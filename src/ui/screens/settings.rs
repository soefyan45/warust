//! Pengaturan: theme, send pacing, media, and where the data lives. Also the
//! settings draft shared with the Webhook & API screen: edits stay local until
//! "Simpan", then go to the backend as `Command::SaveSettings`.

use std::sync::Arc;
use std::time::Instant;

use egui::{CentralPanel, ScrollArea, Ui};

use crate::backend::settings::Sending;
use crate::backend::{Command, Settings};
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
        let mut s = self.draft.clone().ok_or("Belum dimuat")?;
        s.api.port = match self.port.trim().parse::<u16>() {
            Ok(port) if port >= 1024 => port,
            _ => return Err("Port harus angka 1024–65535".to_owned()),
        };
        s.sending.gap_secs = match self.gap.trim().parse::<u64>() {
            Ok(gap) if (Sending::MIN_GAP..=120).contains(&gap) => gap,
            _ => return Err(format!("Jeda minimal harus {}–120 detik", Sending::MIN_GAP)),
        };
        s.sending.jitter_secs = match self.jitter.trim().parse::<u64>() {
            Ok(jitter) if jitter <= 30 => jitter,
            _ => return Err("Jitter harus 0–30 detik".to_owned()),
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
                (Ok(_), true, _) => ("ellipsis", "Ada perubahan yang belum disimpan".to_owned(), p.amber),
                (Ok(_), false, Some((note, at))) if at.elapsed().as_secs() < 6 => {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    ("check", note.clone(), p.text_2)
                }
                _ => ("check", "Semua perubahan tersimpan".to_owned(), p.muted),
            };
            ui.spacing_mut().item_spacing.x = 6.0;
            icons::show(ui, icon, 14.0, color);
            ui.label(rich(&text, 12.0, Weight::Regular, color));
        },
        |ui| {
            let button = ui.add_enabled_ui(dirty && result.is_ok(), |ui| {
                widgets::button(ui, Some("check"), "Simpan perubahan", Kind::Primary)
            });
            if button.inner.clicked() {
                save = result.clone().ok();
            }
            ui.add_space(8.0);
            undo = ui.add_enabled_ui(dirty, |ui| widgets::button(ui, None, "Batalkan", Kind::Secondary)).inner.clicked();
        },
    );
    if undo {
        form.draft = None;
    }
    if save.is_some() {
        form.flash("Tersimpan");
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
                super::page_head(ui, "Pengaturan", "Tampilan, kecepatan kirim, media, dan lokasi data aplikasi.");
                let form = &mut state.settings_form;
                form.sync(saved);

                super::card(ui, "Tampilan", "Template mengatur susunan halaman dan warna; data dan fitur sama di semua template.", |ui| {
                    let draft = form.draft.as_mut().expect("synced");
                    ui.label(rich("Template halaman", 13.0, Weight::Medium, p.text_2));
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                        for template in Template::ALL {
                            if template_option(ui, &p, template, draft.template == template.key()).clicked() {
                                draft.template = template.key().to_owned();
                            }
                        }
                    });
                    if Template::from_key(&draft.template).has_dark() {
                        let mut tab = usize::from(draft.theme == "light");
                        line(ui, "Tema", |ui| widgets::tabs(ui, &["Gelap", "Terang"], &mut tab));
                        draft.theme = if tab == 1 { "light" } else { "dark" }.to_owned();
                    } else {
                        ui.label(rich("Template ini hanya punya mode terang.", 12.0, Weight::Regular, p.muted));
                    }
                });

                super::card(
                    ui,
                    "Pengiriman (anti-ban)",
                    "Jeda antar pesan per nomor, termasuk pesan dari REST API. Jeda pendek menaikkan risiko nomor dibatasi WhatsApp.",
                    |ui| {
                        line(ui, "Jeda minimal (detik)", |ui| {
                            widgets::field(ui, &mut form.gap, "4", 90.0, true);
                            ui.label(rich(&format!("minimal {}", Sending::MIN_GAP), 12.0, Weight::Regular, p.muted));
                        });
                        line(ui, "Jitter acak (detik)", |ui| {
                            widgets::field(ui, &mut form.jitter, "2", 90.0, true);
                            ui.label(rich("tambahan acak 0–N detik, 0–30", 12.0, Weight::Regular, p.muted));
                        });
                    },
                );

                super::card(ui, "Media", "", |ui| {
                    let draft = form.draft.as_mut().expect("synced");
                    widgets::toggle(ui, &mut draft.auto_stickers, "Unduh stiker otomatis saat terlihat di chat");
                    ui.label(rich(
                        "Foto, video, dan dokumen tetap diunduh saat diklik.",
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                });

                super::card(ui, "Notifikasi", "Pesan masuk muncul di notifikasi desktop; klik untuk membuka chatnya.", |ui| {
                    let notify = &mut form.draft.as_mut().expect("synced").notify;
                    widgets::toggle(ui, &mut notify.enabled, "Tampilkan notifikasi pesan masuk");
                    ui.add_enabled_ui(notify.enabled, |ui| {
                        ui.spacing_mut().item_spacing.y = 14.0;
                        widgets::toggle(ui, &mut notify.groups, "Termasuk pesan grup");
                        widgets::toggle(ui, &mut notify.preview, "Tampilkan isi pesan (matikan agar hanya nama pengirim)");
                        widgets::toggle(ui, &mut notify.sound, "Bunyikan suara pesan");
                    });
                    ui.label(rich(
                        "Chat yang sedang terbuka di jendela aktif tidak dinotifikasi.",
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                });

                super::card(ui, "Data", "Sesi, chat, media, dan pengaturan disimpan di komputer ini.", |ui| {
                    line(ui, "Folder data", |ui| {
                        ui.add(egui::Label::new(rich_mono(&data_dir.display().to_string(), 12.0, p.text)).truncate());
                    });
                    ui.horizontal(|ui| {
                        if widgets::button(ui, Some("arrow-up-right"), "Buka folder", Kind::Secondary).clicked() {
                            crate::platform::open(data_dir);
                        }
                        ui.label(rich(
                            "Jangan bagikan folder sessions/: isinya kunci login nomor Anda.",
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
