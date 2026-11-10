//! Webhook & API: the local REST API (on/off, port, key, endpoints) and the
//! webhook (URL, secret, events, per-number URLs, recent deliveries).

use std::sync::Arc;

use egui::{Align, CentralPanel, Frame, Margin, ScrollArea, Sense, Ui, vec2};

use crate::backend::settings::{masked, new_secret};
use crate::backend::webhook::{Delivery, Outcome};
use crate::backend::{ApiState, Command, SessionView, Settings};
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette};
use crate::ui::widgets::{self, Kind, rich, rich_mono};
use crate::ui::{State, time};

const ENDPOINTS: &[(&str, &str, &str)] = &[
    ("GET", "/healthz", "Cek server hidup (tanpa API key)"),
    ("GET", "/sessions", "Daftar nomor dan statusnya"),
    ("POST", "/sessions", "Tambah nomor baru {name?}"),
    ("GET", "/sessions/{id}", "Status satu nomor"),
    ("GET", "/sessions/{id}/qr", "QR atau kode pairing terbaru"),
    ("POST", "/sessions/{id}/pair-code", "Minta kode pairing {phone}"),
    ("POST", "/sessions/{id}/logout", "Lepas tautan dari HP"),
    ("DELETE", "/sessions/{id}", "Lepas tautan dan hapus nomor"),
    ("GET", "/sessions/{id}/chats", "Daftar chat ?limit=50"),
    ("GET", "/sessions/{id}/statuses", "Status kontak 24 jam terakhir"),
    ("POST", "/sessions/{id}/messages/text", "Kirim teks {to, text, allow_cold?}"),
    ("POST", "/sessions/{id}/messages/media", "Kirim file {to, path|url, caption?, as_sticker?}"),
    ("GET", "/sessions/{id}/messages/{msg_id}", "Status kirim satu pesan"),
    ("PUT", "/sessions/{id}/webhook", "URL webhook khusus nomor ini {url}"),
];

pub fn show(
    ui: &mut Ui,
    state: &mut State,
    sessions: &[SessionView],
    saved: &Arc<Settings>,
    api: &ApiState,
    deliveries: &[Delivery],
) -> Vec<Command> {
    let p = theme::palette(ui.ctx());
    let mut commands = Vec::new();
    CentralPanel::default()
        .frame(theme::page(ui.ctx()))
        .show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 20.0;
                ui.set_max_width(ui.available_width().min(880.0));
                super::page_head(
                    ui,
                    "Webhook & API",
                    "Hubungkan aplikasi lain: kirim pesan lewat REST API, terima pesan masuk lewat webhook.",
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    widgets::tabs(ui, &["REST API", "Webhook", "Pemicu"], &mut state.integrations_tab);
                    // Classic tabs sit on a rule; Forest's pills need none.
                    if !theme::template().top_nav() {
                        widgets::hline(ui, ui.max_rect(), ui.cursor().top() - 0.5, p.border);
                    }
                });
                state.settings_form.sync(saved);
                match state.integrations_tab {
                    0 => api_tab(ui, &p, state, api),
                    1 => webhook_tab(ui, &p, state, sessions, deliveries, &mut commands),
                    _ => super::triggers::tab(ui, &p, state, sessions),
                }
                if let Some(settings) = super::settings::save_bar(ui, &mut state.settings_form) {
                    commands.push(Command::SaveSettings(settings));
                }
            });
        });
    commands
}

fn copy(ui: &Ui, state: &mut State, text: &str, what: &str) {
    ui.ctx().copy_text(text.to_owned());
    state.settings_form.flash(format!("{what} disalin"));
}

fn status_line(ui: &mut Ui, p: &Palette, api: &ApiState, enabled: bool) {
    let (color, text) = match api {
        ApiState::Listening(address) => (p.green, format!("Aktif di {address}")),
        ApiState::Failed(e) => (p.red, format!("Gagal: {e}")),
        ApiState::Off if enabled => (p.amber, "Belum aktif, simpan perubahan untuk menyalakan".to_owned()),
        ApiState::Off => (p.gray, "Mati".to_owned()),
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    widgets::paint_status(ui, rect, color, &text);
}

fn api_tab(ui: &mut Ui, p: &Palette, state: &mut State, api: &ApiState) {
    let saved_key = state.settings_form.draft.as_ref().map(|d| d.api.key.clone()).unwrap_or_default();
    let address = match api {
        ApiState::Listening(address) => address.clone(),
        _ => format!("http://127.0.0.1:{}", state.settings_form.port.trim()),
    };
    super::card(ui, "Server", "Hanya bisa dipanggil dari komputer ini (127.0.0.1). Butuh API key di setiap request.", |ui| {
        let enabled = state.settings_form.draft.as_ref().is_some_and(|d| d.api.enabled);
        status_line(ui, p, api, enabled);
        let form = &mut state.settings_form;
        if let Some(draft) = form.draft.as_mut() {
            widgets::toggle(ui, &mut draft.api.enabled, "Aktifkan REST API");
        }
        ui.horizontal(|ui| {
            ui.label(rich("Port", 13.0, Weight::Medium, p.text_2));
            widgets::field(ui, &mut form.port, "8787", 100.0, true);
            ui.label(rich("1024–65535", 12.0, Weight::Regular, p.muted));
        });
    });

    super::card(ui, "API key", "Kirim sebagai header Authorization: Bearer <key>. Perlakukan seperti kata sandi.", |ui| {
        ui.horizontal(|ui| {
            let shown = if state.settings_form.show_key { saved_key.clone() } else { masked(&saved_key) };
            ui.label(rich_mono(&shown, 13.0, p.text));
        });
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, Some("copy"), "Salin", Kind::Secondary).clicked() {
                copy(ui, state, &saved_key, "API key");
            }
            let label = if state.settings_form.show_key { "Sembunyikan" } else { "Tampilkan" };
            if widgets::button(ui, None, label, Kind::Secondary).clicked() {
                state.settings_form.show_key = !state.settings_form.show_key;
            }
            if widgets::button(ui, Some("rotate-cw"), "Buat key baru", Kind::Secondary).clicked() {
                if let Some(draft) = state.settings_form.draft.as_mut() {
                    draft.api.key = new_secret("wr_");
                }
            }
            ui.label(rich("Key lama berhenti berlaku setelah disimpan.", 12.0, Weight::Regular, p.muted));
        });
    });

    super::card(ui, "Endpoint", "Semua respons JSON. Pesan masuk antrean dengan jeda anti-ban yang sama seperti dari aplikasi.", |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (method, path, purpose) in ENDPOINTS {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                widgets::hline(ui, rect, rect.bottom() - 0.5, p.border);
                let cols = widgets::columns(rect, &[Some(64.0), Some(300.0), None], 12.0);
                widgets::paint_text(ui, cols[0], method, fonts::mono(12.0), p.text_2, Align::Min);
                widgets::paint_text(ui, cols[1], path, fonts::mono(12.0), p.text, Align::Min);
                widgets::paint_text(ui, cols[2], purpose, fonts::ui(12.0, Weight::Regular), p.muted, Align::Min);
            }
        });
        let example = format!(
            "curl -X POST {address}/sessions/<id>/messages/text \\\n  -H 'Authorization: Bearer <API key>' \\\n  -H 'Content-Type: application/json' \\\n  -d '{{\"to\": \"628123456789\", \"text\": \"Halo *tebal*\"}}'"
        );
        ui.label(rich("Contoh", 12.0, Weight::Medium, p.text_2));
        Frame::NONE
            .fill(p.raised)
            .corner_radius(theme::R_MD)
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(rich_mono(&example, 12.0, p.text));
            });
        ui.horizontal(|ui| {
            if widgets::button(ui, Some("copy"), "Salin contoh", Kind::Secondary).clicked() {
                copy(ui, state, &example.replace("<API key>", &saved_key), "Contoh (dengan API key)");
            }
            ui.label(rich(
                "Penerima yang belum pernah mengirim pesan ke nomor Anda ditolak (403) kecuali allow_cold: true.",
                12.0,
                Weight::Regular,
                p.muted,
            ));
        });
    });
}

fn outcome_label(p: &Palette, d: &Delivery) -> (egui::Color32, String) {
    match &d.outcome {
        Outcome::Pending => (p.amber, "Mengirim…".to_owned()),
        Outcome::Done(code) if (200..300).contains(code) => (p.green, format!("{code} OK")),
        Outcome::Done(code) => (p.red, format!("HTTP {code}")),
        Outcome::Failed(e) => (p.red, format!("Gagal: {e}")),
    }
}

fn webhook_tab(
    ui: &mut Ui,
    p: &Palette,
    state: &mut State,
    sessions: &[SessionView],
    deliveries: &[Delivery],
    commands: &mut Vec<Command>,
) {
    let secret = state.settings_form.draft.as_ref().map(|d| d.webhook.secret.clone()).unwrap_or_default();
    super::card(ui, "URL webhook", "Event dikirim sebagai POST JSON. Gagal (timeout/5xx/429) diulang hingga 5 kali dalam ±13 menit.", |ui| {
        let form = &mut state.settings_form;
        let Some(draft) = form.draft.as_mut() else { return };
        let width = ui.available_width().min(560.0);
        widgets::field(ui, &mut draft.webhook.url, "https://server-anda.com/whatsapp/webhook", width, true);
        let url = draft.webhook.url.trim();
        if !url.is_empty() && !(url.starts_with("http://") || url.starts_with("https://")) {
            ui.label(rich("URL harus diawali http:// atau https://", 12.0, Weight::Regular, p.red_text));
        }
        ui.label(rich("Event yang dikirim", 12.0, Weight::Medium, p.text_2));
        widgets::toggle(ui, &mut draft.webhook.message_received, "message.received + status.received: pesan dan status masuk");
        widgets::toggle(ui, &mut draft.webhook.message_status, "message.status: terkirim, diterima, dibaca, gagal");
        widgets::toggle(ui, &mut draft.webhook.session_status, "session.status: terhubung, terputus, logout, dll.");
        let dirty = form.dirty();
        ui.horizontal(|ui| {
            let ready = !dirty && form.draft.as_ref().is_some_and(|d| !d.webhook.url.trim().is_empty());
            let test = ui.add_enabled_ui(ready, |ui| widgets::button(ui, Some("arrow-up-right"), "Kirim tes", Kind::Secondary));
            if test.inner.clicked() {
                commands.push(Command::TestWebhook);
                form.flash("Event webhook.test dikirim");
            }
            let note = if dirty { "Simpan dulu, lalu kirim tes." } else { "Mengirim event webhook.test ke URL global." };
            ui.label(rich(note, 12.0, Weight::Regular, p.muted));
        });
    });

    super::card(
        ui,
        "Tanda tangan",
        "Cocokkan header X-Warust-Signature dengan HMAC-SHA256(secret, body mentah) agar yakin request dari Warust.",
        |ui| {
            let shown = if state.settings_form.show_secret { secret.clone() } else { masked(&secret) };
            ui.label(rich_mono(&shown, 13.0, p.text));
            ui.horizontal(|ui| {
                if widgets::button(ui, Some("copy"), "Salin", Kind::Secondary).clicked() {
                    copy(ui, state, &secret, "Secret");
                }
                let label = if state.settings_form.show_secret { "Sembunyikan" } else { "Tampilkan" };
                if widgets::button(ui, None, label, Kind::Secondary).clicked() {
                    state.settings_form.show_secret = !state.settings_form.show_secret;
                }
                if widgets::button(ui, Some("rotate-cw"), "Buat secret baru", Kind::Secondary).clicked() {
                    if let Some(draft) = state.settings_form.draft.as_mut() {
                        draft.webhook.secret = new_secret("whsec_");
                    }
                }
            });
            ui.label(rich_mono(
                "X-Warust-Event: message.received\nX-Warust-Delivery: d1762591234-1\nX-Warust-Signature: sha256=<hex>",
                12.0,
                p.muted,
            ));
        },
    );

    super::card(ui, "URL per nomor", "Kosongkan untuk memakai URL global.", |ui| {
        if sessions.is_empty() {
            ui.label(rich("Belum ada nomor.", 12.0, Weight::Regular, p.muted));
        }
        let Some(draft) = state.settings_form.draft.as_mut() else { return };
        for s in sessions {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(200.0, 32.0), Sense::hover());
                let label = match &s.phone {
                    Some(phone) => format!("{} · {phone}", s.name),
                    None => s.name.clone(),
                };
                widgets::paint_text(ui, rect, &label, fonts::ui(13.0, Weight::Medium), p.text_2, Align::Min);
                let mut url = draft.webhook.overrides.get(&s.id).cloned().unwrap_or_default();
                let width = (ui.available_width() - 4.0).min(460.0);
                if widgets::field(ui, &mut url, "Pakai URL global", width, true).changed() {
                    if url.trim().is_empty() {
                        draft.webhook.overrides.remove(&s.id);
                    } else {
                        draft.webhook.overrides.insert(s.id.clone(), url);
                    }
                }
            });
        }
    });

    super::card(ui, "Pengiriman terakhir", "Sejak aplikasi dibuka, terbaru di atas.", |ui| {
        if deliveries.is_empty() {
            ui.label(rich("Belum ada event terkirim.", 12.0, Weight::Regular, p.muted));
            return;
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = [Some(70.0), Some(150.0), Some(110.0), None, Some(150.0), Some(60.0)];
            let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            widgets::hline(ui, head, head.bottom() - 0.5, p.border);
            let cols = widgets::columns(head, &widths, 12.0);
            for (i, title) in ["Waktu", "Event", "Nomor", "URL", "Hasil", "Coba"].iter().enumerate() {
                widgets::paint_text(ui, cols[i], title, fonts::ui(12.0, Weight::Medium), p.muted, Align::Min);
            }
            for d in deliveries.iter().take(30) {
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
                widgets::hline(ui, rect, rect.bottom() - 0.5, p.border);
                let cols = widgets::columns(rect, &widths, 12.0);
                let number = match sessions.iter().find(|s| s.id == d.session) {
                    Some(s) => s.name.clone(),
                    None if d.session == "test" => "tes".to_owned(),
                    None => d.session.clone(),
                };
                let (color, result) = outcome_label(p, d);
                widgets::paint_text(ui, cols[0], &time::clock_secs(d.at), fonts::mono(12.0), p.text_2, Align::Min);
                widgets::paint_text(ui, cols[1], &d.event, fonts::mono(12.0), p.text, Align::Min);
                widgets::paint_text(ui, cols[2], &number, fonts::ui(12.0, Weight::Regular), p.text_2, Align::Min);
                widgets::paint_text(ui, cols[3], &d.url, fonts::mono(12.0), p.muted, Align::Min);
                widgets::paint_status(ui, cols[4], color, &result);
                widgets::paint_text(ui, cols[5], &d.attempts.to_string(), fonts::mono(12.0), p.text_2, Align::Min);
                if let Outcome::Failed(e) = &d.outcome {
                    response.on_hover_text(e);
                }
            }
        });
    });
}
