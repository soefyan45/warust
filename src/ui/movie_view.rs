//! The in-app media preview (#84, #86): one modal for what a chat or a
//! status carries. A photo full size with zoom; a video or a sound played
//! through ffmpeg (play/pause, a seek bar with the time, mute); a plain-text
//! document read in place; anything else as its name, type and size. "Open
//! in another app" stays as a way out. Esc, × or a click outside closes it.

use std::path::{Path, PathBuf};

use egui::{Align, Color32, CursorIcon, Frame, Margin, Rect, ScrollArea, Sense, Stroke, pos2, vec2};

use crate::backend::movie;
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, R_LG, R_MD};
use crate::ui::widgets::{self, Kind, rich};
use crate::ui::icons;
use crate::ui::zoom::{self, Span};

/// How a file is previewed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    Photo,
    /// Played: a video, or sound alone.
    Play { video: bool },
    Text,
    /// Its name, type and size, and the way out.
    Info,
}

/// The file open in the preview.
pub struct Viewer {
    /// Player id: the file itself, so the same video reopens where it was.
    id: String,
    pub file: PathBuf,
    pub title: String,
    look: Look,
    /// Photo zoom: 1 fits the stage.
    zoom: f32,
    /// The zoom the photo was last laid out at, and how far it was scrolled
    /// then: what a zoom is measured from to stay under the pointer.
    laid: f32,
    scroll: egui::Vec2,
    /// A text document's content, read once.
    text: Option<String>,
    /// The chat's photos, oldest first, when it was opened from one: the
    /// arrow keys go through them (#184). `at` is the one on screen.
    album: Vec<(PathBuf, String)>,
    at: usize,
}

impl Viewer {
    pub fn new(file: PathBuf, title: String) -> Self {
        let look = look_of(&file);
        Self { id: file.to_string_lossy().into_owned(), file, title, look, zoom: 1.0, laid: 1.0, scroll: egui::Vec2::ZERO, text: None, album: Vec::new(), at: 0 }
    }

    /// Photo `at` of `album` (a chat's photos, oldest first), with the
    /// others a left or right arrow away.
    pub fn in_album(album: Vec<(PathBuf, String)>, at: usize) -> Option<Self> {
        let (file, title) = album.get(at)?.clone();
        let mut viewer = Self::new(file, title);
        viewer.album = album;
        viewer.at = at;
        Some(viewer)
    }

    /// Moves `step` photos along the album (−1 older, +1 newer), stopping at
    /// either end; the new photo starts fitted.
    fn step(&mut self, step: isize) {
        let Some(at) = self.at.checked_add_signed(step).filter(|&at| at < self.album.len()) else { return };
        let (file, title) = self.album[at].clone();
        *self = Self { album: std::mem::take(&mut self.album), at, ..Self::new(file, title) };
    }
}

/// What a file is, by its extension; video and sound need ffmpeg.
pub fn look_of(file: &Path) -> Look {
    let ext = file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let playable = movie::available();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" => Look::Photo,
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "3gp" if playable => Look::Play { video: true },
        "mp3" | "m4a" | "aac" | "wav" | "ogg" | "opus" | "oga" | "flac" | "amr" if playable => Look::Play { video: false },
        "txt" | "csv" | "json" | "log" | "md" | "xml" | "tsv" | "ini" | "yaml" | "yml" => Look::Text,
        _ => Look::Info,
    }
}

/// Text documents are read up to this much.
const TEXT_LIMIT: usize = 256 * 1024;

/// "1:05", or "1:02:05" past an hour.
fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 { format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0) } else { format!("{} KB", bytes.div_ceil(1024)) }
}

pub fn show(ctx: &egui::Context, open: &mut Option<Viewer>) {
    let Some(viewer) = open.as_mut() else { return };
    let p = theme::palette(ctx);
    let screen = ctx.content_rect();
    let playing = matches!(viewer.look, Look::Play { .. });
    let status = if playing {
        if movie::status(&viewer.id).is_none() {
            movie::play(&viewer.id, &viewer.file, movie::repaint_for(ctx));
        }
        // A paused one still needs asking about now and then, or it stops.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        movie::status(&viewer.id)
    } else {
        None
    };
    if viewer.look == Look::Text && viewer.text.is_none() {
        let bytes = std::fs::read(&viewer.file).unwrap_or_default();
        let cut = bytes.len() > TEXT_LIMIT;
        let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(TEXT_LIMIT)]).into_owned();
        if cut {
            text.push_str(tr("\n\n… (cut short; open it in another app for the rest)", "\n\n… (dipotong; buka di aplikasi lain untuk selebihnya)"));
        }
        viewer.text = Some(text);
    }
    // Left and right go through the chat's photos (#184).
    if viewer.album.len() > 1 {
        let (left, right) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)));
        if left {
            viewer.step(-1);
        } else if right {
            viewer.step(1);
        }
    }
    let mut close = false;
    // Fits a phone-sized window too: a 12 px margin there, 40 px elsewhere.
    let margin = if screen.width() < 700.0 { 24.0 } else { 80.0 };
    let size = vec2((screen.width() - margin).clamp(280.0, 1100.0), (screen.height() - margin).clamp(260.0, 900.0));
    let icon = match viewer.look {
        Look::Photo => "image",
        Look::Play { video: true } => "video",
        Look::Play { video: false } => "volume-2",
        Look::Text | Look::Info => "file-text",
    };
    let modal = egui::Modal::new(egui::Id::unique("media_preview"))
        .frame(Frame::NONE.fill(p.panel).stroke(Stroke::new(1.0, p.border_strong)).corner_radius(R_LG).inner_margin(Margin::same(14)))
        .show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            let narrow = size.x < 560.0;
            widgets::row(
                ui,
                32.0,
                |ui| {
                    icons::show(ui, icon, 16.0, p.text_2);
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(rich(&viewer.title, 14.0, Weight::SemiBold, p.text)).truncate());
                    if viewer.album.len() > 1 {
                        ui.add_space(8.0);
                        ui.label(widgets::rich_mono(&format!("{} / {}", viewer.at + 1, viewer.album.len()), 12.0, p.muted));
                    }
                },
                |ui| {
                    close = widgets::icon_button(ui, "x", 28.0, false).on_hover_text(tr("Close (Esc)", "Tutup (Esc)")).clicked();
                    ui.add_space(8.0);
                    let label = tr("Open in another app", "Buka di aplikasi lain");
                    let out = if narrow { widgets::icon_button(ui, "arrow-up-right", 28.0, false).on_hover_text(label) } else { widgets::button(ui, None, label, Kind::Secondary) };
                    if out.clicked() {
                        movie::stop();
                        crate::platform::open(&viewer.file);
                        close = true;
                    }
                },
            );
            ui.add_space(10.0);
            match viewer.look {
                Look::Photo => photo(ui, viewer),
                Look::Play { video } => play(ui, &p, viewer, status.as_ref(), video),
                Look::Text => {
                    let text = viewer.text.as_deref().unwrap_or_default();
                    Frame::NONE.fill(p.raised).corner_radius(R_MD).inner_margin(Margin::same(12)).show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                            ui.add(egui::Label::new(widgets::rich_mono(text, 12.5, p.text)).selectable(true).extend());
                        });
                    });
                }
                Look::Info => info(ui, &p, viewer, icon),
            }
        });
    if close || modal.should_close() {
        movie::stop();
        *open = None;
    }
}

/// Photo zoom limits: 1 fits the stage.
const PHOTO_ZOOM: (f32, f32) = (1.0, 6.0);
/// Where a double click on a fitted photo zooms to.
const PHOTO_DOUBLE: f32 = 2.5;

/// A photo fitted to the stage, zoomed with −/+, a pinch, Cmd/Ctrl+scroll or
/// a double click (around the pointer), and moved around by scrolling (or
/// dragging) once it is bigger than the stage.
fn photo(ui: &mut egui::Ui, viewer: &mut Viewer) {
    let controls_h = 36.0;
    let stage = vec2(ui.available_width(), (ui.available_height() - controls_h - 8.0).max(120.0));
    let area = Rect::from_min_size(ui.cursor().min, stage);
    let uri = widgets::picture_uri(ui.ctx(), &viewer.file);
    let natural = uri.as_ref().and_then(|uri| match egui::Image::from_uri(uri.clone()).load_for_size(ui.ctx(), stage) {
        Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
        _ => None,
    });
    Frame::NONE.fill(Color32::from_rgb(0x12, 0x14, 0x13)).corner_radius(R_LG).show(ui, |ui| {
        ui.set_min_size(stage);
        ui.set_max_size(stage);
        match (uri, natural) {
            (Some(uri), Some(natural)) => {
                // At 1× the photo fits the stage, never past its own size.
                let fitted = natural * (stage.x / natural.x).min(stage.y / natural.y).min(1.0);
                // The −/+ buttons zoom around the middle, a gesture around the pointer.
                let mut anchor = stage / 2.0;
                if let Some(gesture) = zoom::gesture(ui, area) {
                    anchor = gesture.anchor;
                    viewer.zoom = if gesture.double {
                        zoom::toggled(viewer.zoom, PHOTO_ZOOM.0, PHOTO_DOUBLE)
                    } else {
                        zoom::scaled(viewer.zoom, gesture.factor, PHOTO_ZOOM.0, PHOTO_ZOOM.1)
                    };
                }
                let shown = fitted * viewer.zoom;
                let mut scroll = ScrollArea::both().auto_shrink(false).scroll_source(egui::scroll_area::ScrollSource::ALL);
                if viewer.zoom != viewer.laid {
                    let before = fitted * viewer.laid;
                    scroll = scroll.scroll_offset(vec2(
                        zoom::keep(anchor.x, viewer.scroll.x, Span::centred(before.x, stage.x), Span::centred(shown.x, stage.x), stage.x),
                        zoom::keep(anchor.y, viewer.scroll.y, Span::centred(before.y, stage.y), Span::centred(shown.y, stage.y), stage.y),
                    ));
                    viewer.laid = viewer.zoom;
                }
                let out = scroll.show(ui, |ui| {
                    // Centred while it fits, from the corner once it is bigger.
                    let (content, _) = ui.allocate_exact_size(shown.max(stage), Sense::hover());
                    egui::Image::from_uri(uri).fit_to_exact_size(shown).paint_at(ui, Rect::from_center_size(content.center(), shown));
                });
                viewer.scroll = out.state.clamped_offset();
                arrows(ui, area, viewer);
            }
            _ => {
                let (rect, _) = ui.allocate_exact_size(stage, Sense::hover());
                widgets::paint_text(ui, rect, tr("Loading…", "Memuat…"), fonts::ui(13.0, Weight::Medium), Color32::from_white_alpha(200), Align::Center);
            }
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if widgets::icon_button(ui, "minus", 30.0, false).on_hover_text(tr("Zoom out", "Perkecil")).clicked() {
            viewer.zoom = (viewer.zoom / 1.25).max(PHOTO_ZOOM.0);
        }
        if widgets::button(ui, None, &format!("{:.0}%", viewer.zoom * 100.0), Kind::Secondary).on_hover_text(tr("Fit to the window", "Pas di jendela")).clicked() {
            viewer.zoom = 1.0;
        }
        let hint = tr("Zoom in (or pinch, Ctrl/Cmd+scroll, double click)", "Perbesar (atau cubit, Ctrl/Cmd+gulir, klik dua kali)");
        if widgets::icon_button(ui, "plus", 30.0, false).on_hover_text(hint).clicked() {
            viewer.zoom = (viewer.zoom * 1.25).min(PHOTO_ZOOM.1);
        }
    });
}

/// Round ‹ and › buttons on the stage's sides, when there is a photo that
/// way; a click goes there as the arrow keys do.
fn arrows(ui: &mut egui::Ui, stage: Rect, viewer: &mut Viewer) {
    if viewer.album.len() < 2 {
        return;
    }
    let mut step = 0;
    for (icon, dir, x, there, hint) in [
        ("chevron-left", -1, stage.left() + 30.0, viewer.at > 0, tr("Previous photo (←)", "Foto sebelumnya (←)")),
        ("chevron-right", 1, stage.right() - 30.0, viewer.at + 1 < viewer.album.len(), tr("Next photo (→)", "Foto berikutnya (→)")),
    ] {
        if !there {
            continue;
        }
        let button = Rect::from_center_size(pos2(x, stage.center().y), vec2(40.0, 40.0));
        let response = ui.interact(button, ui.scope_id().with(("album_arrow", dir)), Sense::click());
        let fill = if response.hovered() { Color32::from_black_alpha(200) } else { Color32::from_black_alpha(130) };
        ui.painter().circle_filled(button.center(), 20.0, fill);
        icons::paint(ui, Rect::from_center_size(button.center(), vec2(20.0, 20.0)), icon, 20.0, Color32::WHITE);
        if response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(hint).clicked() {
            step = dir;
        }
    }
    if step != 0 {
        viewer.step(step);
    }
}

/// A video on its stage, or a sound's card; play/pause, the seek bar with
/// the time, mute.
fn play(ui: &mut egui::Ui, p: &theme::Palette, viewer: &Viewer, status: Option<&movie::Status>, video: bool) {
    let controls_h = 44.0;
    let stage_h = (ui.available_height() - controls_h - 10.0).max(120.0);
    let (stage, response) = ui.allocate_exact_size(vec2(ui.available_width(), stage_h), Sense::click());
    ui.painter().rect_filled(stage, R_LG, Color32::from_rgb(0x12, 0x14, 0x13));
    let texture = status.filter(|_| video).and_then(|s| widgets::frame_texture(ui.ctx(), "chat_video", &s.picture));
    if let Some(texture) = &texture {
        let tex = texture.size_vec2();
        let scale = (stage.width() / tex.x).min(stage.height() / tex.y);
        let shown = Rect::from_center_size(stage.center(), tex * scale);
        egui::Image::from_texture(texture).fit_to_exact_size(shown.size()).paint_at(ui, shown);
    }
    let sound = !video || status.is_some_and(|s| s.sound_only);
    if sound {
        // Sound alone: a large speaker that moves while it plays.
        let t = ui.input(|i| i.time) as f32;
        let beat = if status.is_some_and(|s| s.playing) { 1.0 + 0.06 * (t * 6.0).sin() } else { 1.0 };
        ui.painter().circle_filled(stage.center(), 46.0 * beat, p.primary.gamma_multiply(0.85));
        icons::paint(ui, Rect::from_center_size(stage.center(), vec2(34.0, 34.0)), "volume-2", 34.0, p.on_primary);
        if status.is_some_and(|s| s.playing) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
    }
    let loading = status.is_none_or(|s| s.loading) && texture.is_none() && !sound;
    let paused = status.is_some_and(|s| !s.playing);
    if loading {
        widgets::paint_text(ui, stage, tr("Loading…", "Memuat…"), fonts::ui(13.0, Weight::Medium), Color32::from_white_alpha(200), Align::Center);
    } else if paused && !sound {
        ui.painter().circle_filled(stage.center(), 30.0, Color32::from_black_alpha(150));
        icons::paint(ui, Rect::from_center_size(stage.center(), vec2(26.0, 26.0)), "play", 26.0, Color32::WHITE);
    }
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        movie::toggle(&viewer.id);
    }
    ui.add_space(10.0);
    let Some(s) = status else { return };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if widgets::icon_button(ui, if s.playing { "pause" } else { "play" }, 32.0, false).on_hover_text(if s.playing { tr("Pause", "Jeda") } else { tr("Play", "Putar") }).clicked() {
            movie::toggle(&viewer.id);
        }
        let time = format!("{} / {}", clock(s.position), clock(s.duration));
        let time_w = ui.painter().layout_no_wrap(time.clone(), fonts::mono(12.0), p.muted).size().x;
        let bar_w = (ui.available_width() - time_w - 60.0).max(60.0);
        let (bar, seek) = ui.allocate_exact_size(vec2(bar_w, 24.0), Sense::click_and_drag());
        let track = Rect::from_center_size(bar.center(), vec2(bar.width(), 4.0));
        ui.painter().rect_filled(track, 2.0, p.border);
        let done = if s.duration > 0.0 { (s.position / s.duration).clamp(0.0, 1.0) } else { 0.0 };
        ui.painter().rect_filled(Rect::from_min_size(track.min, vec2(track.width() * done, 4.0)), 2.0, p.primary);
        let knob = ui.ctx().animate_bool_with_time(seek.id, seek.hovered() || seek.dragged(), 0.12);
        ui.painter().circle_filled(pos2(track.left() + track.width() * done, track.center().y), 5.0 + 2.0 * knob, p.primary);
        if seek.clicked() || seek.drag_stopped() {
            if let Some(at) = seek.interact_pointer_pos() {
                movie::seek(&viewer.id, (at.x - track.left()) / track.width().max(1.0));
            }
        }
        ui.label(widgets::rich_mono(&time, 12.0, p.muted));
        let hint = if s.muted { tr("Sound on", "Nyalakan suara") } else { tr("Mute", "Bisukan") };
        if widgets::icon_button(ui, if s.muted { "volume-x" } else { "volume-2" }, 32.0, s.muted).on_hover_text(hint).clicked() {
            movie::set_muted(!s.muted);
        }
    });
}

/// A file with no preview: its icon, name, type and size, and the way out.
fn info(ui: &mut egui::Ui, p: &theme::Palette, viewer: &Viewer, icon: &str) {
    let (stage, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    ui.painter().rect_filled(stage, R_LG, p.raised);
    let center = stage.center();
    let tile = Rect::from_center_size(center - vec2(0.0, 50.0), vec2(72.0, 72.0));
    ui.painter().rect_filled(tile, 12.0, p.accent_soft);
    icons::paint(ui, tile, icon, 34.0, p.accent_text);
    widgets::paint_text(ui, Rect::from_center_size(center + vec2(0.0, 6.0), vec2(stage.width() - 40.0, 22.0)), &viewer.title, fonts::ui(15.0, Weight::SemiBold), p.text, Align::Center);
    let ext = viewer.file.extension().and_then(|e| e.to_str()).map(str::to_ascii_uppercase).unwrap_or_else(|| tr("File", "File").to_owned());
    let size = std::fs::metadata(&viewer.file).map(|m| human_size(m.len())).unwrap_or_default();
    let line = if size.is_empty() { ext } else { format!("{ext} · {size}") };
    widgets::paint_text(ui, Rect::from_center_size(center + vec2(0.0, 30.0), vec2(stage.width() - 40.0, 18.0)), &line, fonts::ui(12.5, Weight::Regular), p.muted, Align::Center);
    widgets::paint_text(
        ui,
        Rect::from_center_size(center + vec2(0.0, 58.0), vec2(stage.width() - 40.0, 18.0)),
        tr("No preview for this type. Open it in another app.", "Tipe ini tidak bisa dipratinjau. Buka di aplikasi lain."),
        fonts::ui(12.0, Weight::Regular),
        p.muted,
        Align::Center,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_go_through_the_album_and_stop_at_its_ends() {
        let album: Vec<(PathBuf, String)> = ["a.jpg", "b.jpg", "c.jpg"].iter().map(|f| (PathBuf::from(f), f.to_string())).collect();
        assert!(Viewer::in_album(album.clone(), 3).is_none());
        let mut viewer = Viewer::in_album(album, 1).expect("in the album");
        assert_eq!(viewer.file, PathBuf::from("b.jpg"));
        viewer.zoom = 3.0;
        viewer.step(1);
        assert_eq!((viewer.file.clone(), viewer.at, viewer.zoom), (PathBuf::from("c.jpg"), 2, 1.0));
        // The newest is the last one: no further.
        viewer.step(1);
        assert_eq!(viewer.at, 2);
        viewer.step(-1);
        viewer.step(-1);
        assert_eq!((viewer.file.clone(), viewer.title.as_str(), viewer.album.len()), (PathBuf::from("a.jpg"), "a.jpg", 3));
        viewer.step(-1);
        assert_eq!(viewer.at, 0);
    }

    #[test]
    fn files_get_the_right_preview() {
        assert_eq!(look_of(Path::new("a.JPG")), Look::Photo);
        assert_eq!(look_of(Path::new("notes.csv")), Look::Text);
        assert_eq!(look_of(Path::new("report.docx")), Look::Info);
        assert_eq!(look_of(Path::new("archive.zip")), Look::Info);
    }
}
