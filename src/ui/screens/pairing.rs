//! Pairing screen (Pego `v4/Pairing`) for a real session: the rotating QR
//! from whatsapp-rust, or an 8-character code for a phone number.

use std::time::Instant;

use egui::{Align, Color32, Frame, Layout, Margin, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::backend::{SessionView, Status};
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::qr::Qr;
use crate::ui::theme::{self, Palette, R_LG, R_MD};
use crate::ui::widgets::{self, Kind, rich, rich_mono};
use crate::ui::{State, icons};

const COLUMN: f32 = 580.0;
const INFO: f32 = 320.0;
const GAP: f32 = 32.0;

pub enum Action {
    RequestCode(String),
    /// Start pairing again (new keys, new QR).
    Retry,
}

pub fn show(ui: &mut Ui, state: &mut State, session: Option<&SessionView>) -> Option<Action> {
    let p = theme::palette(ui.ctx());
    let area = ui.max_rect().shrink2(vec2(40.0, 44.0));
    let left = area.left() + ((area.width() - COLUMN - INFO - GAP) / 2.0).max(0.0);
    let column_rect = Rect::from_min_max(pos2(left, area.top()), pos2(left + COLUMN, area.bottom()));
    let info_left = column_rect.right() + GAP;
    let info_rect = Rect::from_min_max(pos2(info_left, area.top()), pos2(info_left + INFO, area.bottom()));
    let action = column(&mut widgets::child(ui, column_rect, Layout::top_down(Align::Min)), &p, state, session);
    let webhook = session.map(|s| super::webhook_label(state, &s.id)).unwrap_or_else(|| "-".to_owned());
    info(&mut widgets::child(ui, info_rect, Layout::top_down(Align::Min)), &p, session, webhook);
    action
}

fn column(ui: &mut Ui, p: &Palette, state: &mut State, session: Option<&SessionView>) -> Option<Action> {
    let mut action = None;
    ui.spacing_mut().item_spacing.y = 24.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(rich(tr("1  Choose a method", "1  Pilih cara"), 12.0, Weight::SemiBold, p.text));
        step_line(ui, p);
        ui.label(rich(tr("2  Confirm on the phone", "2  Konfirmasi di HP"), 12.0, Weight::Regular, p.muted));
        step_line(ui, p);
        ui.label(rich(tr("3  Sync history", "3  Sinkron riwayat"), 12.0, Weight::Regular, p.muted));
    });
    super::page_head(
        ui,
        tr("Link new number", "Tautkan nomor baru"),
        tr(
            "The gateway signs in as a companion device, just like WhatsApp Web.",
            "Gateway masuk sebagai perangkat pendamping, sama seperti WhatsApp Web.",
        ),
    );

    theme::card(ui.ctx())
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if option_head(ui, p, !state.pair_with_code,
                tr("Scan QR code", "Pindai kode QR"),
                tr("For a phone that is near this screen.", "Untuk HP yang ada di dekat layar ini."),
                true,
            ) {
                state.pair_with_code = false;
            }
            if !state.pair_with_code {
                qr_body(ui, p, state, session);
            }
            if option_head(ui, p, state.pair_with_code,
                tr("Use a phone code", "Pakai kode telepon"),
                tr("For a server without a screen. An 8-character code is entered on the phone.", "Untuk server tanpa layar. Kode 8 karakter dimasukkan di HP."),
                false,
            ) {
                state.pair_with_code = true;
            }
            if state.pair_with_code
                && let Some(a) = code_body(ui, p, state, session)
            {
                action = Some(a);
            }
        });

    let (icon, text) = match session.map(|s| &s.status) {
        None | Some(Status::Starting) => ("loader-circle", tr("Preparing the session.", "Menyiapkan sesi.").to_owned()),
        Some(Status::Pairing) => (
            "loader-circle",
            tr(
                "Waiting for confirmation from the phone. This page moves on by itself once connected.",
                "Menunggu konfirmasi dari HP. Halaman ini pindah sendiri setelah tersambung.",
            )
            .to_owned(),
        ),
        Some(Status::Failed(reason)) => ("triangle-alert", reason.clone()),
        Some(Status::Locked) => ("triangle-alert", tr("This session is in use by another process.", "Sesi ini sedang dipakai proses lain.").to_owned()),
        Some(other) => ("triangle-alert", format!("Status: {other:?}")),
    };
    ui.horizontal(|ui| {
        icons::show(ui, icon, 16.0, p.muted);
        ui.label(rich(&text, 13.0, Weight::Regular, p.muted));
        if matches!(session.map(|s| &s.status), Some(Status::Failed(_)))
            && widgets::button(ui, Some("rotate-cw"), tr("Try again", "Coba lagi"), Kind::Secondary).clicked()
        {
            action = Some(Action::Retry);
        }
    });
    action
}

fn step_line(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 12.0), Sense::hover());
    widgets::hline(ui, rect, rect.center().y, p.border_strong);
}

/// One option row with a radio. Returns true when clicked.
fn option_head(ui: &mut Ui, p: &Palette, selected: bool, title: &str, desc: &str, first: bool) -> bool {
    let fill = if selected { p.panel } else { Color32::TRANSPARENT };
    let corner = if first {
        egui::CornerRadius { nw: R_LG, ne: R_LG, sw: 0, se: 0 }
    } else {
        egui::CornerRadius::ZERO
    };
    let response = Frame::NONE
        .fill(fill)
        .corner_radius(corner)
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::row(
                ui,
                38.0,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    let color = if selected { p.text } else { p.muted };
                    ui.painter().circle_stroke(rect.center(), 7.5, Stroke::new(1.0, color));
                    if selected {
                        ui.painter().circle_filled(rect.center(), 4.0, p.text);
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(rich(title, 14.0, Weight::SemiBold, p.text));
                        ui.label(rich(desc, 13.0, Weight::Regular, p.muted));
                    });
                },
                |ui| {
                    if !selected {
                        icons::show(ui, "chevron-right", 16.0, p.muted);
                    }
                },
            );
        })
        .response;
    let clicked = ui
        .interact(response.rect, ui.scope_id().with(("option", title)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked();
    widgets::hline(ui, response.rect, response.rect.bottom() - 0.5, p.border);
    clicked
}

fn qr_body(ui: &mut Ui, p: &Palette, state: &mut State, session: Option<&SessionView>) {
    Frame::NONE
        .fill(p.panel)
        .inner_margin(Margin { left: 48, right: 20, top: 4, bottom: 20 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.0;
                // QR area plus a line for the countdown under it.
                let (space, _) = ui.allocate_exact_size(vec2(240.0, 262.0), Sense::hover());
                let tile = Rect::from_min_size(space.min, vec2(240.0, 240.0));
                let current = session.and_then(|s| s.qr.clone()).filter(|(_, until)| *until > Instant::now());
                match current {
                    Some((payload, until)) => {
                        if state.qr.as_ref().map(|q| q.payload != payload).unwrap_or(true) {
                            state.qr = Qr::encode(&payload);
                        }
                        if let Some(qr) = &state.qr {
                            // Always dark on white, whatever the theme: phones scan it best.
                            qr.paint(ui, tile, Color32::from_rgb(0x0B, 0x0B, 0x0C), Color32::WHITE);
                        }
                        let left = until.saturating_duration_since(Instant::now()).as_secs();
                        let timer = Rect::from_min_max(pos2(space.left(), tile.bottom() + 6.0), space.max);
                        widgets::paint_text(
                            ui,
                            timer,
                            &crate::trf!("New code in {left} s", "Kode baru dalam {left} dtk"),
                            fonts::ui(12.0, Weight::Regular),
                            p.muted,
                            Align::Center,
                        );
                    }
                    None => {
                        ui.painter()
                            .rect(tile, R_MD, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
                        icons::paint(ui, tile.translate(vec2(0.0, -10.0)), "qr-code", 48.0, p.muted);
                        let caption = Rect::from_min_max(pos2(tile.left(), tile.center().y + 24.0), pos2(tile.right(), tile.center().y + 40.0));
                        let text = match session.map(|s| &s.status) {
                            Some(Status::Failed(_)) => tr("QR not available", "QR tidak tersedia"),
                            _ => tr("Preparing the QR code", "Menyiapkan kode QR"),
                        };
                        widgets::paint_text(ui, caption, text, fonts::ui(12.0, Weight::Regular), p.muted, Align::Center);
                    }
                }
                ui.vertical(|ui| {
                    ui.add_space(60.0);
                    ui.spacing_mut().item_spacing.y = 14.0;
                    for (n, text) in [
                        ("1", tr("Open WhatsApp on the phone of the number to link.", "Buka WhatsApp di HP nomor yang akan ditautkan.")),
                        ("2", tr("Settings, Linked devices, then Link a device.", "Setelan, Perangkat tertaut, lalu Tautkan perangkat.")),
                        ("3", tr("Point the camera at the code beside this.", "Arahkan kamera ke kode di samping.")),
                    ] {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            ui.label(rich_mono(n, 12.0, p.muted));
                            ui.add(egui::Label::new(rich(text, 13.0, Weight::Regular, p.text)).wrap());
                        });
                    }
                });
            });
        });
}

fn code_body(ui: &mut Ui, p: &Palette, state: &mut State, session: Option<&SessionView>) -> Option<Action> {
    let mut action = None;
    Frame::NONE
        .fill(p.panel)
        .corner_radius(egui::CornerRadius { nw: 0, ne: 0, sw: R_LG, se: R_LG })
        .inner_margin(Margin { left: 48, right: 20, top: 4, bottom: 20 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(rich(tr("Phone number to link", "Nomor HP yang akan ditautkan"), 13.0, Weight::Medium, p.text));
            ui.horizontal(|ui| {
                Frame::NONE
                    .fill(p.raised)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(theme::R_SM)
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.pair_phone)
                                .frame(Frame::NONE)
                                .hint_text(rich_mono("62812xxxxxxxx", 13.0, p.muted))
                                .font(fonts::mono(13.0))
                                .desired_width(220.0),
                        );
                    });
                // Digits only, without + or spaces, as WhatsApp expects.
                state.pair_phone.retain(|c| c.is_ascii_digit());
                let ready = state.pair_phone.len() >= 9;
                let clicked = ui
                    .add_enabled_ui(ready, |ui| widgets::button(ui, Some("arrow-up-right"), tr("Request code", "Minta kode"), Kind::Primary))
                    .inner
                    .clicked();
                if clicked && ready {
                    action = Some(Action::RequestCode(state.pair_phone.clone()));
                }
            });
            ui.label(rich(
                tr("International format without the + sign, for example 62812...", "Format internasional tanpa tanda +, contoh 62812..."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            if let Some(code) = session.and_then(|s| s.pair_code.as_deref()) {
                let shown = if code.len() == 8 {
                    format!("{}-{}", &code[..4], &code[4..])
                } else {
                    code.to_owned()
                };
                ui.add_space(4.0);
                ui.label(rich(tr("Pairing code", "Kode pairing"), 12.0, Weight::Medium, p.muted));
                ui.label(rich_mono(&shown, 32.0, p.text));
                ui.add(
                    egui::Label::new(rich(
                        tr(
                            "On the phone: Settings, Linked devices, Link a device, then Link with phone number instead. Valid for about 3 minutes.",
                            "Di HP: Setelan, Perangkat tertaut, Tautkan perangkat, lalu Tautkan dengan nomor telepon saja. Berlaku sekitar 3 menit.",
                        ),
                        12.0,
                        Weight::Regular,
                        p.text_2,
                    ))
                    .wrap(),
                );
            }
        });
    action
}

fn info(ui: &mut Ui, p: &Palette, session: Option<&SessionView>, webhook: String) {
    theme::card(ui.ctx())
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(INFO - 40.0);
            ui.spacing_mut().item_spacing.y = 18.0;
            ui.label(rich(tr("Session details", "Detail sesi"), 13.0, Weight::SemiBold, p.text));
            let name = session.map(|s| s.name.clone()).unwrap_or_else(|| "-".to_owned());
            let store = session
                .map(|s| format!("data/sessions/{}.db", s.id))
                .unwrap_or_else(|| "-".to_owned());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                kv(ui, p, tr("Session name", "Nama sesi"), &name, false);
                kv(ui, p, tr("Name in the Linked devices list", "Nama di daftar Perangkat tertaut"), "Warust", false);
                kv(ui, p, tr("Key storage", "Penyimpanan kunci"), &store, true);
                kv(ui, p, "Webhook", &webhook, false);
            });
            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_stroke(line, 0.0, Stroke::new(1.0, p.border), StrokeKind::Inside);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::show(ui, "triangle-alert", 16.0, p.amber);
                ui.add(
                    egui::Label::new(rich(
                        tr(
                            "Unofficial client. Use a number you can accept the risk of account restrictions for, and avoid mass sending to numbers that never chatted with you.",
                            "Klien tidak resmi. Pakai nomor yang siap menanggung risiko pembatasan akun, dan hindari kirim massal ke nomor yang belum pernah chat.",
                        ),
                        12.0,
                        Weight::Regular,
                        p.text_2,
                    ))
                    .wrap(),
                );
            });
        });
}

fn kv(ui: &mut Ui, p: &Palette, key: &str, value: &str, mono: bool) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(rich(key, 12.0, Weight::Regular, p.muted));
        if mono {
            ui.label(rich_mono(value, 12.0, p.text));
        } else {
            ui.label(rich(value, 13.0, Weight::Regular, p.text));
        }
    });
}
