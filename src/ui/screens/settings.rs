//! Pengaturan: theme, send pacing, media, and where the data lives. Also the
//! settings draft shared with the Webhook & API screen: edits stay local until
//! "Simpan", then go to the backend as `Command::SaveSettings`.

use std::sync::Arc;
use std::time::Instant;

use egui::{CentralPanel, ScrollArea, Ui};

use crate::backend::settings::Sending;
use crate::backend::{Command, Settings};
use crate::i18n::tr;
use crate::ui::State;
use crate::ui::fonts::Weight;
use crate::ui::icons;
use crate::ui::theme::{self, Template};
use crate::ui::widgets::{self, Kind, rich, rich_mono};

/// The settings being edited, and the text of the number fields.
#[derive(Default)]
pub struct Form {
    pub draft: Option<Settings>,
    /// What the draft was copied from, to notice saves made elsewhere.
    base: Option<Arc<Settings>>,
    pub port: String,
    pub gap: String,
    pub jitter: String,
    pub show_key: bool,
    pub show_secret: bool,
    /// Last "saved" or "copied" message and when.
    pub note: Option<(String, Instant)>,
}

impl Form {
    /// The draft, (re)copied from the saved settings unless it has unsaved edits.
    pub fn sync(&mut self, saved: &Arc<Settings>) -> &mut Settings {
        let stale = self.base.as_ref().is_none_or(|b| !Arc::ptr_eq(b, saved));
        // Our own save came back: the draft is the saved settings now.
        if stale && self.result().is_ok_and(|r| r == **saved) {
            self.base = Some(saved.clone());
        }
        let stale = self.base.as_ref().is_none_or(|b| !Arc::ptr_eq(b, saved));
        let clean = match (&self.draft, &self.base) {
            (Some(d), Some(b)) => d == b.as_ref() && self.fields_match(d),
            _ => true,
        };
        if self.draft.is_none() || (stale && clean) {
            let fresh = (**saved).clone();
            self.port = fresh.api.port.to_string();
            self.gap = fresh.sending.gap_secs.to_string();
            self.jitter = fresh.sending.jitter_secs.to_string();
            self.draft = Some(fresh);
            self.base = Some(saved.clone());
        }
        self.draft.get_or_insert_with(|| (**saved).clone())
    }

    fn fields_match(&self, d: &Settings) -> bool {
        self.port == d.api.port.to_string()
            && self.gap == d.sending.gap_secs.to_string()
            && self.jitter == d.sending.jitter_secs.to_string()
    }

    /// The draft with the number fields applied, or why they are invalid.
    pub fn result(&self) -> Result<Settings, String> {
        let mut s = self.draft.clone().ok_or(tr("Not loaded yet", "Belum dimuat"))?;
        s.api.port = match self.port.trim().parse::<u16>() {
            Ok(port) if port >= 1024 => port,
            _ => return Err(tr("Port must be a number from 1024 to 65535", "Port harus angka 1024–65535").to_owned()),
        };
        s.sending.gap_secs = match self.gap.trim().parse::<u64>() {
            Ok(gap) if (Sending::MIN_GAP..=120).contains(&gap) => gap,
            _ => return Err(crate::trf!("Minimum gap must be {}–120 seconds", "Jeda minimal harus {}–120 detik", Sending::MIN_GAP)),
        };
        s.sending.jitter_secs = match self.jitter.trim().parse::<u64>() {
            Ok(jitter) if jitter <= 30 => jitter,
            _ => return Err(tr("Jitter must be 0–30 seconds", "Jitter harus 0–30 detik").to_owned()),
        };
        if let Some(problem) = s.rules.iter().find_map(|r| r.problem()) {
            return Err(problem);
        }
        Ok(s)
    }

    pub fn dirty(&self) -> bool {
        match (self.result(), &self.base) {
            (Ok(s), Some(b)) => s != **b,
            (Err(_), _) => true,
            _ => false,
        }
    }

    pub fn flash(&mut self, note: impl Into<String>) {
        self.note = Some((note.into(), Instant::now()));
    }
}

/// "Simpan perubahan" and "Batalkan" with the validation message; returns
/// the settings to save.
pub fn save_bar(ui: &mut Ui, form: &mut Form) -> Option<Settings> {
    let p = theme::palette(ui.ctx());
    let mut save = None;
    let mut undo = false;
    let result = form.result();
    let dirty = form.dirty();
    let note = form.note.clone();
    widgets::row(
        ui,
        36.0,
        |ui| {
            let (icon, text, color) = match (&result, dirty, &note) {
                (Err(e), _, _) => ("triangle-alert", e.clone(), p.red_text),
                (Ok(_), true, _) => ("ellipsis", tr("There are unsaved changes", "Ada perubahan yang belum disimpan").to_owned(), p.amber),
                (Ok(_), false, Some((note, at))) if at.elapsed().as_secs() < 6 => {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    ("check", note.clone(), p.text_2)
                }
                _ => ("check", tr("All changes saved", "Semua perubahan tersimpan").to_owned(), p.muted),
            };
            ui.spacing_mut().item_spacing.x = 6.0;
            icons::show(ui, icon, 14.0, color);
            ui.label(rich(&text, 12.0, Weight::Regular, color));
        },
        |ui| {
            let button = ui.add_enabled_ui(dirty && result.is_ok(), |ui| {
                widgets::button(ui, Some("check"), tr("Save changes", "Simpan perubahan"), Kind::Primary)
            });
            if button.inner.clicked() {
                save = result.clone().ok();
            }
            ui.add_space(8.0);
            undo = ui.add_enabled_ui(dirty, |ui| widgets::button(ui, None, tr("Cancel", "Batalkan"), Kind::Secondary)).inner.clicked();
        },
    );
    if undo {
        form.draft = None;
    }
    if save.is_some() {
        form.flash(tr("Saved", "Tersimpan"));
    }
    save
}

/// A selectable box with a template's name, a colour strip and its description.
fn template_option(ui: &mut Ui, p: &theme::Palette, template: Template, active: bool) -> egui::Response {
    let size = egui::vec2(240.0, 112.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let stroke = if active { egui::Stroke::new(2.0, p.primary) } else { egui::Stroke::new(1.0, p.border) };
    ui.painter().rect(rect, theme::r_inner(), p.panel, stroke, egui::StrokeKind::Inside);
    // The template's own colours, so the choice is visible before saving.
    let swatches = match template {
        Template::Classic => [theme::DARK.bg, theme::DARK.raised, theme::LIGHT.bg, theme::LIGHT.text],
        Template::Forest => [theme::FOREST.bg, theme::FOREST.primary, theme::FOREST.accent, theme::FOREST.panel],
    };
    for (i, color) in swatches.iter().enumerate() {
        let swatch = egui::Rect::from_min_size(rect.min + egui::vec2(14.0 + i as f32 * 26.0, 14.0), egui::vec2(20.0, 20.0));
        ui.painter().rect(swatch, 10.0, *color, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    }
    let name = egui::Rect::from_min_size(rect.min + egui::vec2(14.0, 44.0), egui::vec2(size.x - 28.0, 20.0));
    widgets::paint_text(ui, name, template.name(), crate::ui::fonts::ui(14.0, Weight::SemiBold), p.text, egui::Align::Min);
    let mut job = egui::text::LayoutJob::simple(
        template.description().to_owned(),
        crate::ui::fonts::ui(12.0, Weight::Regular),
        p.muted,
        size.x - 28.0,
    );
    job.wrap.max_rows = 2;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(rect.min + egui::vec2(14.0, 68.0), galley, p.muted);
    if active {
        let check = egui::Rect::from_min_size(egui::pos2(rect.right() - 30.0, rect.top() + 14.0), egui::vec2(16.0, 16.0));
        icons::paint(ui, check, "check", 16.0, p.primary);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A label column, then the control.
/// What the wallpaper picker edits: the app's choice, or a number's (#60).
pub struct WallDraft<'a> {
    pub key: &'a mut String,
    pub file: &'a mut String,
    pub dim: &'a mut u8,
}

/// "Chat wallpaper" (#56): Auto, the six designed wallpapers, None, and the
/// user's own picture with how much it is dimmed. For a number (`app` given)
/// it starts with "Same as the app", the key `app`.
pub fn wallpaper_picker(
    ui: &mut Ui,
    p: &theme::Palette,
    wall: WallDraft,
    picking: &mut Option<std::sync::mpsc::Receiver<Option<Result<std::path::PathBuf, String>>>>,
    dark: bool,
    app: Option<&Settings>,
) {
    // A picture chosen and copied on its own thread.
    if let Some(answer) = picking.as_ref().and_then(|r| r.try_recv().ok()) {
        *picking = None;
        match answer {
            Some(Ok(path)) => {
                forget_custom(wall.file);
                *wall.key = "custom".to_owned();
                *wall.file = path.to_string_lossy().into_owned();
            }
            Some(Err(e)) => log::warn!("wallpaper picture: {e}"),
            None => {}
        }
    }
    if picking.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    ui.add_space(4.0);
    ui.label(rich(tr("Chat wallpaper", "Wallpaper chat"), 13.0, Weight::Medium, p.text_2));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
        let auto = widgets::auto_wallpaper(dark);
        if let Some(app) = app {
            let name = tr("Same as the app", "Ikuti aplikasi");
            let active = wall.key.as_str() == "app";
            let clicked = match app.wallpaper.as_str() {
                "custom" if !app.wallpaper_file.is_empty() => custom_tile(ui, p, &app.wallpaper_file, name, active),
                "none" => wallpaper_tile(ui, p, None, name, active),
                key => wallpaper_tile(ui, p, Some(widgets::WALLPAPERS.iter().find(|w| w.key == key).unwrap_or(auto)), name, active),
            };
            if clicked {
                *wall.key = "app".to_owned();
            }
        }
        if wallpaper_tile(ui, p, Some(auto), tr("Auto", "Otomatis"), wall.key.as_str() == "auto") {
            *wall.key = "auto".to_owned();
        }
        for design in &widgets::WALLPAPERS {
            if wallpaper_tile(ui, p, Some(design), tr(design.name.0, design.name.1), wall.key.as_str() == design.key) {
                *wall.key = design.key.to_owned();
            }
        }
        if wallpaper_tile(ui, p, None, tr("None", "Polos"), wall.key.as_str() == "none") {
            *wall.key = "none".to_owned();
        }
        if !wall.file.is_empty() && custom_tile(ui, p, wall.file, tr("Your picture", "Gambar Anda"), wall.key.as_str() == "custom") {
            *wall.key = "custom".to_owned();
        }
    });
    ui.horizontal(|ui| {
        let busy = picking.is_some();
        let label = if busy { tr("Copying…", "Menyalin…") } else { tr("Choose picture…", "Pilih gambar…") };
        if ui.add_enabled_ui(!busy, |ui| widgets::button(ui, Some("image"), label, Kind::Secondary)).inner.clicked() {
            *picking = Some(pick_wallpaper(ui.ctx()));
        }
        if !wall.file.is_empty() && widgets::button(ui, Some("x"), tr("Remove picture", "Hapus gambar"), Kind::Secondary).clicked() {
            forget_custom(wall.file);
            wall.file.clear();
            if wall.key.as_str() == "custom" {
                *wall.key = if app.is_some() { "app" } else { "auto" }.to_owned();
            }
        }
    });
    if wall.key.as_str() == "custom" {
        let mut dim = f32::from(*wall.dim);
        line(ui, tr("Dim the picture", "Redupkan gambar"), |ui| {
            ui.add(egui::Slider::new(&mut dim, 0.0..=80.0).suffix(" %").step_by(5.0));
        });
        *wall.dim = dim.round() as u8;
    }
    let note = if app.is_some() {
        tr("Used in this number's chats, here and in its own window, once saved.", "Dipakai di chat nomor ini, di sini dan di jendelanya sendiri, setelah disimpan.")
    } else {
        tr(
            "Applies to every open chat once saved, except numbers with their own wallpaper. Bubbles and date pills keep their own background.",
            "Berlaku untuk semua chat setelah disimpan, kecuali nomor yang punya wallpaper sendiri. Bubble dan label tanggal tetap punya latar sendiri.",
        )
    };
    ui.add(egui::Label::new(rich(note, 12.0, Weight::Regular, p.muted)).wrap());
}

/// One choice in the wallpaper picker: a phone-shaped preview and its name.
fn wallpaper_tile(ui: &mut Ui, p: &theme::Palette, wall: Option<&widgets::Wallpaper>, name: &str, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(64.0, 132.0), egui::Sense::click());
    let preview = egui::Rect::from_min_size(rect.min, egui::vec2(64.0, 110.0));
    match wall {
        Some(wall) => widgets::paint_wallpaper_preview(ui, preview, wall),
        None => {
            ui.painter().rect_filled(preview, 8.0, p.chat_bg);
        }
    }
    tile_frame(ui, p, preview, &response, active);
    let label = egui::Rect::from_min_max(egui::pos2(rect.left() - 4.0, preview.bottom() + 4.0), egui::pos2(rect.right() + 4.0, rect.bottom()));
    widgets::paint_text(ui, label, name, crate::ui::fonts::ui(11.5, if active { Weight::SemiBold } else { Weight::Regular }), if active { p.text } else { p.text_2 }, egui::Align::Center);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A picture of the user's own as a picker choice.
fn custom_tile(ui: &mut Ui, p: &theme::Palette, file: &str, name: &str, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(64.0, 132.0), egui::Sense::click());
    let preview = egui::Rect::from_min_size(rect.min, egui::vec2(64.0, 110.0));
    ui.painter().rect_filled(preview, 8.0, p.raised);
    if let Some(uri) = widgets::picture_uri(ui.ctx(), std::path::Path::new(file)) {
        egui::Image::from_uri(uri).corner_radius(8).fit_to_exact_size(preview.size()).paint_at(ui, preview);
    }
    tile_frame(ui, p, preview, &response, active);
    let label = egui::Rect::from_min_max(egui::pos2(rect.left() - 4.0, preview.bottom() + 4.0), egui::pos2(rect.right() + 4.0, rect.bottom()));
    widgets::paint_text(ui, label, name, crate::ui::fonts::ui(11.5, if active { Weight::SemiBold } else { Weight::Regular }), if active { p.text } else { p.text_2 }, egui::Align::Center);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The ring round a picker choice: the active one in the primary colour,
/// a hovered one lifting a little.
fn tile_frame(ui: &Ui, p: &theme::Palette, rect: egui::Rect, response: &egui::Response, active: bool) {
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
    let (width, color) = if active { (2.0, p.primary) } else { (1.0, p.border.lerp_to_gamma(p.border_strong, hover)) };
    ui.painter().rect_stroke(rect, 8.0, egui::Stroke::new(width, color), egui::StrokeKind::Outside);
    if active {
        let badge = egui::Rect::from_center_size(rect.right_top() + egui::vec2(-12.0, 12.0), egui::vec2(18.0, 18.0));
        ui.painter().circle_filled(badge.center(), 9.0, p.primary);
        crate::ui::icons::paint(ui, badge.shrink(3.0), "check", 12.0, p.on_primary);
    }
}

/// Asks for a picture and copies it, made at most 2400 px on its long side,
/// into the data folder, so the original can move and large photos are
/// scaled once rather than every frame. Off the UI thread.
fn pick_wallpaper(ctx: &egui::Context) -> std::sync::mpsc::Receiver<Option<Result<std::path::PathBuf, String>>> {
    let (answer, receive) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let picked = crate::platform::pick_file(tr("Choose a chat wallpaper", "Pilih wallpaper chat"), true);
        let copied = picked.map(|path| copy_wallpaper(&path));
        let _ = answer.send(copied);
        ctx.request_repaint();
    });
    receive
}

fn copy_wallpaper(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let picture = image::open(path).map_err(|e| crate::trf!("Not a picture: {e}", "Bukan gambar: {e}"))?;
    let picture = if picture.width().max(picture.height()) > 2400 { picture.thumbnail(2400, 2400) } else { picture };
    let dir = crate::platform::data_dir().join("wallpapers");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let out = dir.join(format!("wallpaper-{stamp}.jpg"));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&out).map_err(|e| e.to_string())?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 88).encode_image(&picture.to_rgb8()).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Deletes a copied wallpaper picture (never a file the user picked).
pub fn forget_custom(file: &str) {
    crate::backend::settings::forget_wallpaper(&crate::platform::data_dir(), file);
}

fn line(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let p = theme::palette(ui.ctx());
    // Wrapped, so the note after the field moves down in a narrow window.
    let label_w = if ui.available_width() < 480.0 { 130.0 } else { 180.0 };
    ui.horizontal_wrapped(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(label_w, 32.0), egui::Sense::hover());
        widgets::paint_text(ui, rect, label, crate::ui::fonts::ui(13.0, Weight::Medium), p.text_2, egui::Align::Min);
        add(ui);
    });
}

pub fn show(ui: &mut Ui, state: &mut State, saved: &Arc<Settings>) -> Vec<Command> {
    let p = theme::palette(ui.ctx());
    let mut commands = Vec::new();
    let data_dir = crate::platform::data_dir();
    CentralPanel::default()
        .frame(theme::page(ui.ctx()))
        .show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 20.0;
                ui.set_max_width(ui.available_width().min(760.0));
                super::page_head(
                    ui,
                    tr("Settings", "Pengaturan"),
                    tr("Appearance, sending speed, media, and where the app keeps its data.", "Tampilan, kecepatan kirim, media, dan lokasi data aplikasi."),
                );
                let form = &mut state.settings_form;
                form.sync(saved);

                super::card(
                    ui,
                    tr("Appearance", "Tampilan"),
                    tr(
                        "The template sets the page layout and colours; data and features are the same in every template.",
                        "Template mengatur susunan halaman dan warna; data dan fitur sama di semua template.",
                    ),
                    |ui| {
                        let draft = form.draft.as_mut().expect("synced");
                        ui.label(rich(tr("Page template", "Template halaman"), 13.0, Weight::Medium, p.text_2));
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                            for template in Template::ALL {
                                if template_option(ui, &p, template, draft.template == template.key()).clicked() {
                                    draft.template = template.key().to_owned();
                                }
                            }
                        });
                        // Named in both languages, so either can find it.
                        let mut language = usize::from(draft.language == crate::i18n::ID);
                        line(ui, "Language / Bahasa", |ui| widgets::tabs(ui, &["English", "Bahasa Indonesia"], &mut language));
                        draft.language = if language == 1 { crate::i18n::ID } else { crate::i18n::EN }.to_owned();
                        if Template::from_key(&draft.template).has_dark() {
                            let mut tab = usize::from(draft.theme == "light");
                            line(ui, tr("Theme", "Tema"), |ui| widgets::tabs(ui, &[tr("Dark", "Gelap"), tr("Light", "Terang")], &mut tab));
                            draft.theme = if tab == 1 { "light" } else { "dark" }.to_owned();
                        } else {
                            ui.label(rich(tr("This template only has a light mode.", "Template ini hanya punya mode terang."), 12.0, Weight::Regular, p.muted));
                        }
                        // Forest's top pills, or Classic's sidebar as a rail of icons.
                        let mut nav = usize::from(draft.nav_icons_only);
                        line(ui, tr("Navigation", "Navigasi"), |ui| widgets::tabs(ui, &[tr("Icons and labels", "Ikon dan label"), tr("Icons only", "Ikon saja")], &mut nav));
                        draft.nav_icons_only = nav == 1;
                        let dark = ui.ctx().theme() == egui::Theme::Dark;
                        wallpaper_picker(
                        ui,
                        &p,
                        WallDraft { key: &mut draft.wallpaper, file: &mut draft.wallpaper_file, dim: &mut draft.wallpaper_dim },
                        &mut state.wallpaper_picking,
                        dark,
                        None,
                    );
                    },
                );

                super::card(
                    ui,
                    tr("Sending (anti-ban)", "Pengiriman (anti-ban)"),
                    tr(
                        "Gap between messages per number, including messages from the REST API. A short gap raises the risk of the number being restricted by WhatsApp.",
                        "Jeda antar pesan per nomor, termasuk pesan dari REST API. Jeda pendek menaikkan risiko nomor dibatasi WhatsApp.",
                    ),
                    |ui| {
                        line(ui, tr("Minimum gap (seconds)", "Jeda minimal (detik)"), |ui| {
                            widgets::field(ui, &mut form.gap, "4", 90.0, true);
                            ui.label(rich(&crate::trf!("minimum {}", "minimal {}", Sending::MIN_GAP), 12.0, Weight::Regular, p.muted));
                        });
                        line(ui, tr("Random jitter (seconds)", "Jitter acak (detik)"), |ui| {
                            widgets::field(ui, &mut form.jitter, "2", 90.0, true);
                            ui.label(rich(tr("random extra 0–N seconds, 0–30", "tambahan acak 0–N detik, 0–30"), 12.0, Weight::Regular, p.muted));
                        });
                    },
                );

                super::card(ui, "Media", "", |ui| {
                    let draft = form.draft.as_mut().expect("synced");
                    widgets::toggle(ui, &mut draft.auto_stickers, tr("Download stickers automatically when they show in a chat", "Unduh stiker otomatis saat terlihat di chat"));
                    ui.label(rich(
                        tr("Photos, videos, and documents are still downloaded on click.", "Foto, video, dan dokumen tetap diunduh saat diklik."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                });

                super::card(ui, tr("Privacy", "Privasi"), "", |ui| {
                    let draft = form.draft.as_mut().expect("synced");
                    widgets::toggle(ui, &mut draft.send_read, tr("Send read receipts", "Kirim tanda dibaca"));
                    ui.label(rich(
                        tr(
                            "On: opening a chat sends blue ticks to the sender and marks the chat as read on the phone. It is off by default, so only the badge here goes away.",
                            "Menyala: membuka chat mengirim centang biru ke pengirim dan menandai chat dibaca di HP. Bawaannya mati, jadi hanya badge di sini yang hilang.",
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    ui.add_space(6.0);
                    widgets::toggle(ui, &mut draft.share_online, tr("Show when you are online", "Tampilkan saat Anda online"));
                    ui.label(rich(
                        tr(
                            "On: contacts see \"online\" while a warust window is in front, and you see when they are online. Off: neither, as on WhatsApp.",
                            "Menyala: kontak melihat \"online\" selama jendela warust di depan, dan Anda melihat saat mereka online. Mati: keduanya tidak, seperti di WhatsApp.",
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    ui.add_space(6.0);
                    widgets::toggle(ui, &mut draft.typing, tr("Show \"typing\" to the other person while replying", "Tampilkan \"sedang mengetik\" ke lawan chat saat membalas"));
                    ui.label(rich(
                        tr("Status seen receipts are set per number on the Status page.", "Tanda dilihat status diatur per nomor di halaman Status."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                });

                super::card(
                    ui,
                    tr("Notifications", "Notifikasi"),
                    tr("Incoming messages show as desktop notifications; click to open the chat.", "Pesan masuk muncul di notifikasi desktop; klik untuk membuka chatnya."),
                    |ui| {
                        let notify = &mut form.draft.as_mut().expect("synced").notify;
                        widgets::toggle(ui, &mut notify.enabled, tr("Show notifications for incoming messages", "Tampilkan notifikasi pesan masuk"));
                        ui.add_enabled_ui(notify.enabled, |ui| {
                            ui.spacing_mut().item_spacing.y = 14.0;
                            widgets::toggle(ui, &mut notify.groups, tr("Include group messages", "Termasuk pesan grup"));
                            widgets::toggle(ui, &mut notify.preview, tr("Show the message text (turn off to show only the sender name)", "Tampilkan isi pesan (matikan agar hanya nama pengirim)"));
                            widgets::toggle(ui, &mut notify.sound, tr("Play a message sound", "Bunyikan suara pesan"));
                        });
                        ui.label(rich(
                            tr("The chat open in the active window is not notified.", "Chat yang sedang terbuka di jendela aktif tidak dinotifikasi."),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    },
                );

                super::card(ui, tr("Updates", "Pembaruan"), tr("warust looks for new releases on GitHub. Nothing else is sent.", "warust mencari rilis baru di GitHub. Tidak ada data lain yang dikirim."), |ui| {
                    use crate::backend::update::{UpdateState, current};
                    let draft = form.draft.as_mut().expect("synced");
                    line(ui, tr("Version", "Versi"), |ui| {
                        ui.label(rich_mono(&format!("warust {}", current()), 12.0, p.text));
                    });
                    widgets::toggle(ui, &mut draft.updates.auto_check, tr("Check for updates automatically (on start and once a day)", "Cek pembaruan otomatis (saat dibuka dan sekali sehari)"));
                    ui.add_enabled_ui(draft.updates.auto_check, |ui| {
                        widgets::toggle(
                            ui,
                            &mut draft.updates.auto_install,
                            tr("Install updates automatically, then restart (checked against the release's SHA-256)", "Pasang pembaruan otomatis lalu mulai ulang (dicek dengan SHA-256 rilis)"),
                        );
                    });
                    ui.horizontal(|ui| {
                        let checking = state.update == UpdateState::Checking;
                        let clicked = ui.add_enabled_ui(!checking, |ui| widgets::button(ui, Some("rotate-cw"), tr("Check now", "Cek sekarang"), Kind::Secondary)).inner.clicked();
                        if clicked && !checking {
                            commands.push(Command::CheckUpdate);
                        }
                        let (text, color) = match &state.update {
                            UpdateState::Idle => (String::new(), p.muted),
                            UpdateState::Checking => (tr("Checking…", "Mengecek…").to_owned(), p.muted),
                            UpdateState::UpToDate => (tr("You have the latest version.", "Anda memakai versi terbaru.").to_owned(), p.muted),
                            UpdateState::Available(r) => (crate::trf!("warust {} is available.", "warust {} sudah tersedia.", r.version), p.accent_text),
                            UpdateState::Downloading(r) => (crate::trf!("Downloading warust {}…", "Mengunduh warust {}…", r.version), p.accent_text),
                            UpdateState::Ready(r) => (crate::trf!("warust {} is ready; it installs once warust is idle.", "warust {} siap; dipasang saat warust sedang tidak dipakai.", r.version), p.accent_text),
                            UpdateState::Installing(r) => (crate::trf!("Installing warust {}…", "Memasang warust {}…", r.version), p.accent_text),
                            UpdateState::Restarting => (tr("Restarting…", "Memulai ulang…").to_owned(), p.accent_text),
                            UpdateState::Failed(e) => (crate::trf!("Could not check: {e}", "Tidak bisa mengecek: {e}"), p.red_text),
                        };
                        ui.label(rich(&text, 12.0, Weight::Regular, color));
                    });
                    if let UpdateState::Available(r) = &state.update {
                        if widgets::button(ui, Some("download"), tr("Download", "Unduh"), Kind::Primary).clicked() {
                            crate::platform::open_url(&r.download);
                        }
                    }
                });

                super::card(ui, "Data", tr("Sessions, chats, media, and settings are stored on this computer.", "Sesi, chat, media, dan pengaturan disimpan di komputer ini."), |ui| {
                    line(ui, tr("Data folder", "Folder data"), |ui| {
                        ui.add(egui::Label::new(rich_mono(&data_dir.display().to_string(), 12.0, p.text)).truncate());
                    });
                    ui.horizontal(|ui| {
                        if widgets::button(ui, Some("arrow-up-right"), tr("Open folder", "Buka folder"), Kind::Secondary).clicked() {
                            crate::platform::open(data_dir);
                        }
                        ui.label(rich(
                            tr("Do not share the sessions/ folder: it holds your numbers' login keys.", "Jangan bagikan folder sessions/: isinya kunci login nomor Anda."),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    });
                });

                if let Some(settings) = save_bar(ui, form) {
                    commands.push(Command::SaveSettings(settings));
                }
            });
        });
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_validates_and_tracks_edits() {
        let saved = Arc::new(Settings::default());
        let mut form = Form::default();
        form.sync(&saved);
        assert!(!form.dirty());
        form.gap = "2".to_owned();
        assert!(form.result().is_err() && form.dirty());
        form.gap = "6".to_owned();
        assert_eq!(form.result().unwrap().sending.gap_secs, 6);
        // Unsaved edits survive a save made elsewhere.
        let elsewhere = Arc::new(Settings { auto_stickers: false, ..Settings::default() });
        form.sync(&elsewhere);
        assert_eq!(form.gap, "6");
        // A clean form follows it.
        let mut clean = Form::default();
        clean.sync(&saved);
        clean.sync(&elsewhere);
        assert!(!clean.draft.as_ref().unwrap().auto_stickers);

        // After saving, the form is clean again once the saved copy comes back.
        let mut saving = Form::default();
        saving.sync(&saved);
        saving.jitter = "5".to_owned();
        let after = Arc::new(saving.result().unwrap());
        saving.sync(&after);
        assert!(!saving.dirty());
    }
}
