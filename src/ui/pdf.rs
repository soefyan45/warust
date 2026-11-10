//! PDF pages as pictures for chat previews and the in-app viewer. Rendered by
//! poppler's `pdftoppm` (page count and size from `pdfinfo`) on one background
//! thread, one job at a time, and cached in `data/pdf-pages/` (never next to
//! the file: a sent PDF lives in the user's own folders).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

use egui::{Align, Frame, Key, Margin, ScrollArea, Sense, Stroke, StrokeKind, vec2};

use super::fonts::{self, Weight};
use super::icons;
use super::theme::{self, R_LG, R_MD};
use super::widgets::{self, Kind, rich};
use crate::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub pages: u32,
    /// Height over width of the first page.
    pub aspect: f32,
}

enum Slot<T> {
    Pending,
    Ready(T),
    Failed,
}

enum Job {
    Info(PathBuf),
    Page { file: PathBuf, page: u32, width: u32, target: PathBuf },
}

#[derive(Default)]
struct Cache {
    infos: HashMap<PathBuf, Slot<Info>>,
    pages: HashMap<PathBuf, Slot<PathBuf>>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The render thread, started on first use.
fn queue(ctx: &egui::Context) -> &'static Mutex<Sender<Job>> {
    static QUEUE: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for job in rx {
                match job {
                    Job::Info(file) => {
                        let info = read_info(&file);
                        if let Ok(mut c) = cache().lock() {
                            c.infos.insert(file, info.map_or(Slot::Failed, Slot::Ready));
                        }
                    }
                    Job::Page { file, page, width, target } => {
                        let done = render(&file, page, width, &target);
                        if let Ok(mut c) = cache().lock() {
                            c.pages.insert(target.clone(), if done { Slot::Ready(target) } else { Slot::Failed });
                        }
                    }
                }
                ctx.request_repaint();
            }
        });
        Mutex::new(tx)
    })
}

pub fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// `Pages: 12` and `Page size: 595.276 x 841.89 pts (A4)` from `pdfinfo`.
fn parse_info(text: &str) -> Option<Info> {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name).map(|rest| rest.trim_start_matches(':').trim().to_owned()))
    };
    let pages = field("Pages")?.parse().ok()?;
    let size = field("Page size").unwrap_or_default();
    let mut numbers = size.split_whitespace().filter_map(|w| w.parse::<f32>().ok());
    let aspect = match (numbers.next(), numbers.next()) {
        (Some(w), Some(h)) if w > 0.0 => h / w,
        _ => 1.414,
    };
    Some(Info { pages, aspect })
}

fn read_info(file: &Path) -> Option<Info> {
    let out = std::process::Command::new("pdfinfo")
        .arg(file)
        .output()
        .inspect_err(|e| log::warn!("pdfinfo failed: {e}"))
        .ok()?;
    parse_info(&String::from_utf8_lossy(&out.stdout))
}

fn render(file: &Path, page: u32, width: u32, target: &Path) -> bool {
    let Some(dir) = target.parent() else { return false };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    // pdftoppm adds ".png" to the prefix itself.
    let prefix = target.with_extension("");
    let status = std::process::Command::new("pdftoppm")
        .args(["-png", "-singlefile", "-scale-to-y", "-1", "-scale-to-x"])
        .arg(width.to_string())
        .args(["-f", &page.to_string(), "-l", &page.to_string()])
        .arg(file)
        .arg(&prefix)
        .status();
    match status {
        Ok(s) if s.success() => target.exists(),
        other => {
            log::warn!("pdftoppm failed for page {page}: {other:?}");
            false
        }
    }
}

/// Page count and shape; `None` while `pdfinfo` runs or when it failed.
pub fn info(ctx: &egui::Context, file: &Path) -> Option<Info> {
    let mut c = cache().lock().ok()?;
    match c.infos.get(file) {
        Some(Slot::Ready(info)) => return Some(*info),
        Some(_) => return None,
        None => {}
    }
    c.infos.insert(file.to_owned(), Slot::Pending);
    drop(c);
    let _ = queue(ctx).lock().map(|q| q.send(Job::Info(file.to_owned())));
    None
}

/// A rendered page (1-based) about `width` pixels wide; `None` until ready.
/// Widths are rounded up to steps of 256 px so resizing does not re-render.
pub fn page(ctx: &egui::Context, file: &Path, page: u32, width: f32) -> Option<PathBuf> {
    let width = ((width / 256.0).ceil() as u32 * 256).clamp(256, 2048);
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        file.hash(&mut hasher);
        std::fs::metadata(file).and_then(|m| m.modified()).ok().hash(&mut hasher);
        hasher.finish()
    };
    let dir = crate::platform::data_dir().join("pdf-pages").join(format!("{key:016x}"));
    let target = dir.join(format!("p{page}-w{width}.png"));
    let mut c = cache().lock().ok()?;
    match c.pages.get(&target) {
        Some(Slot::Ready(path)) => return Some(path.clone()),
        Some(_) => return None,
        None => {}
    }
    if target.exists() {
        c.pages.insert(target.clone(), Slot::Ready(target.clone()));
        return Some(target);
    }
    c.pages.insert(target.clone(), Slot::Pending);
    drop(c);
    let _ = queue(ctx).lock().map(|q| q.send(Job::Page { file: file.to_owned(), page, width, target }));
    None
}

/// The PDF open in the viewer.
pub struct Viewer {
    pub file: PathBuf,
    pub title: String,
    pub zoom: f32,
}

impl Viewer {
    pub fn new(file: PathBuf, title: String) -> Self {
        Self { file, title, zoom: 1.0 }
    }
}

const ZOOMS: [f32; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
/// Space inside the viewer's frame, around its content.
const PADDING: f32 = 16.0;
/// Gap kept between the viewer and the window's edges.
const EDGE: f32 = 12.0;
/// Below this content width the title row drops what does not fit.
const NARROW: f32 = 520.0;

/// The viewer's content size: most of the window, at least a useful size on a
/// wide one, and never more than the window has room for.
fn viewer_size(screen: egui::Vec2) -> egui::Vec2 {
    let room = (screen - egui::Vec2::splat(2.0 * (EDGE + PADDING))).max(egui::Vec2::ZERO);
    vec2((screen.x * 0.8).max(480.0).min(room.x), (screen.y * 0.88).max(360.0).min(room.y))
}

/// Draws the viewer over the window; clears `open` when it is closed.
pub fn show(ctx: &egui::Context, open: &mut Option<Viewer>) {
    let Some(viewer) = open.as_mut() else { return };
    let p = theme::palette(ctx);
    let screen = ctx.content_rect();
    let info = info(ctx, &viewer.file);
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("pdf_viewer"))
        .frame(
            Frame::NONE
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(R_LG)
                .inner_margin(Margin::same(PADDING as i8)),
        )
        .show(ctx, |ui| {
            let size = viewer_size(screen.size());
            let narrow = size.x < NARROW;
            ui.set_width(size.x);
            ui.set_height(size.y);
            widgets::row(
                ui,
                32.0,
                |ui| {
                    icons::show(ui, "file-text", 16.0, p.text_2);
                    ui.add_space(6.0);
                    // The page count first, so the title takes what is left and truncates.
                    let pages = info.filter(|_| !narrow).map(|info| crate::trf!("{} pages", "{} halaman", info.pages));
                    let pages_width = pages
                        .as_ref()
                        .map(|text| ui.painter().layout_no_wrap(text.clone(), fonts::ui(12.0, Weight::Regular), p.muted).size().x + 8.0)
                        .unwrap_or(0.0);
                    let title_width = (ui.available_width() - pages_width).max(0.0);
                    ui.allocate_ui(vec2(title_width, 20.0), |ui| {
                        ui.add(egui::Label::new(rich(&viewer.title, 14.0, Weight::SemiBold, p.text)).truncate());
                    });
                    if let Some(pages) = pages {
                        ui.label(rich(&pages, 12.0, Weight::Regular, p.muted));
                    }
                },
                |ui| {
                    close = widgets::icon_button(ui, "x", 28.0, false).on_hover_text(tr("Close (Esc)", "Tutup (Esc)")).clicked();
                    ui.add_space(8.0);
                    let open_elsewhere = if narrow {
                        widgets::icon_button(ui, "arrow-up-right", 28.0, false).on_hover_text(tr("Open in another app", "Buka di aplikasi lain"))
                    } else {
                        widgets::button(ui, None, tr("Open in another app", "Buka di aplikasi lain"), Kind::Secondary)
                    };
                    if open_elsewhere.clicked() {
                        crate::platform::open(&viewer.file);
                    }
                    ui.add_space(8.0);
                    let at = ZOOMS.iter().position(|z| *z == viewer.zoom).unwrap_or(2);
                    if widgets::button(ui, None, "+", Kind::Secondary).on_hover_text(tr("Zoom in", "Perbesar")).clicked() {
                        viewer.zoom = ZOOMS[(at + 1).min(ZOOMS.len() - 1)];
                    }
                    ui.label(rich_zoom(viewer.zoom, &p));
                    if widgets::button(ui, None, "−", Kind::Secondary).on_hover_text(tr("Zoom out", "Perkecil")).clicked() {
                        viewer.zoom = ZOOMS[at.saturating_sub(1)];
                    }
                },
            );
            ui.add_space(8.0);
            let Some(info) = info else {
                ui.label(rich(tr("Reading PDF…", "Membaca PDF…"), 13.0, Weight::Regular, p.muted));
                return;
            };
            let width = (ui.available_width() - 24.0) * viewer.zoom;
            let height = width * info.aspect;
            let ppp = ui.ctx().pixels_per_point();
            ScrollArea::both().auto_shrink(false).show_rows(ui, height + 12.0, info.pages as usize, |ui, rows| {
                for index in rows {
                    let number = index as u32 + 1;
                    let (row, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), height + 12.0), Sense::hover());
                    let x = row.left() + ((row.width() - width) / 2.0).max(0.0);
                    let rect = egui::Rect::from_min_size(egui::pos2(x, row.top()), vec2(width, height));
                    ui.painter().rect_filled(rect, R_MD, egui::Color32::WHITE);
                    match page(ui.ctx(), &viewer.file, number, width * ppp) {
                        Some(picture) => {
                            if let Some(uri) = widgets::picture_uri(ui.ctx(), &picture) {
                                egui::Image::from_uri(uri).fit_to_exact_size(rect.size()).paint_at(ui, rect);
                            }
                        }
                        None => widgets::paint_text(
                            ui,
                            rect,
                            &crate::trf!("Loading page {number}…", "Memuat halaman {number}…"),
                            fonts::ui(13.0, Weight::Regular),
                            egui::Color32::GRAY,
                            Align::Center,
                        ),
                    }
                    ui.painter().rect_stroke(rect, R_MD, Stroke::new(1.0, p.border), StrokeKind::Inside);
                }
            });
        });
    if close || modal.should_close() || ctx.input(|i| i.key_pressed(Key::Escape)) {
        *open = None;
    }
}

fn rich_zoom(zoom: f32, p: &theme::Palette) -> egui::RichText {
    rich(&format!("{:.0}%", zoom * 100.0), 12.0, Weight::Medium, p.text_2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_fits_the_window() {
        // A wide window: most of it, as before.
        assert_eq!(viewer_size(vec2(1440.0, 900.0)), vec2(1152.0, 792.0));
        // The smallest window warust allows: all of the viewer stays inside.
        let small = vec2(360.0, 560.0);
        let size = viewer_size(small);
        let outer = size + egui::Vec2::splat(2.0 * PADDING);
        assert!(outer.x <= small.x - 2.0 * EDGE && outer.y <= small.y - 2.0 * EDGE, "{outer:?}");
        assert!(size.x >= 280.0 && size.y >= 360.0, "{size:?}");
        // Never negative, whatever the window reports.
        assert_eq!(viewer_size(vec2(10.0, 10.0)), egui::Vec2::ZERO);
    }

    #[test]
    fn reads_pdfinfo_output() {
        let text = "Title:          Laporan\nPages:          12\nPage size:      595.276 x 841.89 pts (A4)\n";
        let info = parse_info(text).unwrap();
        assert_eq!(info.pages, 12);
        assert!((info.aspect - 1.414).abs() < 0.01);
        assert_eq!(parse_info("Producer: x\n"), None);
    }

    /// Renders a real PDF when poppler is installed (skipped otherwise).
    #[test]
    fn renders_a_page() {
        let sample = Path::new("/usr/share/cups/data/confidential.pdf");
        if !sample.exists() || std::process::Command::new("pdftoppm").arg("-v").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("warust-pdf-{}", std::process::id()));
        let info = read_info(sample).expect("pdfinfo reads the sample");
        assert!(info.pages >= 1 && info.aspect > 0.5);
        let target = dir.join("p1-w256.png");
        assert!(render(sample, 1, 256, &target));
        let (w, _) = image::image_dimensions(&target).unwrap();
        assert_eq!(w, 256);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
