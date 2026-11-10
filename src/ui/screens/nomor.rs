//! Nomor screen (Pego `v4/Nomor`) with the real sessions: stats, incoming
//! messages per hour, what needs attention, and the sessions table.
//! Sent and queue count what this app sent since it started.

use std::time::Instant;

use egui::{Align, Color32, Margin, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::backend::{SessionView, Status};
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette, R_LG};
use crate::ui::widgets::{self, Kind, rich};
use crate::ui::{State, icons};

const ROW_COLS: [Option<f32>; 7] = [
    Some(260.0),
    Some(170.0),
    Some(90.0),
    Some(120.0),
    Some(70.0),
    None,
    Some(28.0),
];

pub enum Action {
    Add,
    /// Show the pairing screen of a session that is not linked yet.
    Open(String),
    Relink(String),
    Logout(String),
    Remove(String),
    /// Open the "Ganti nama" dialog.
    Rename(String),
}

pub fn show(ui: &mut Ui, state: &mut State, sessions: &[SessionView]) -> Option<Action> {
    let p = theme::palette(ui.ctx());
    let mut action = None;
    ScrollArea::vertical().show(ui, |ui| {
        theme::page(ui.ctx())
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 24.0;
                let connected = sessions.iter().filter(|s| s.status == Status::Connected).count();
                super::page_head(
                    ui,
                    "Nomor",
                    &format!(
                        "{} nomor, {connected} terhubung sebagai perangkat pendamping.",
                        sessions.len()
                    ),
                );
                if sessions.is_empty() {
                    if empty(ui, &p) {
                        action = Some(Action::Add);
                    }
                    return;
                }
                stats(ui, &p, sessions);
                let (chart_row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 196.0), Sense::hover());
                let attention_rect = Rect::from_min_max(pos2(chart_row.right() - 340.0, chart_row.top()), chart_row.max);
                let chart_rect = Rect::from_min_max(chart_row.min, pos2(attention_rect.left() - 16.0, chart_row.bottom()));
                chart(ui, &p, chart_rect, sessions);
                if let Some(a) = attention(ui, &p, attention_rect, sessions) {
                    action = Some(a);
                }
                theme::surface(ui.ctx()).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let problems = sessions.len() - connected;
                    let tabs = [
                        format!("Semua {}", sessions.len()),
                        format!("Terhubung {connected}"),
                        format!("Perlu tindakan {problems}"),
                    ];
                    widgets::row(
                        ui,
                        36.0,
                        |ui| {
                            let labels: Vec<&str> = tabs.iter().map(String::as_str).collect();
                            widgets::tabs(ui, &labels, &mut state.nomor_tab)
                        },
                        |ui| widgets::search(ui, &mut state.nomor_search, "Cari nama atau nomor", 240.0, false),
                    );
                    ui.add_space(8.0);
                    if let Some(a) = table(ui, &p, state, sessions) {
                        action = Some(a);
                    }
                });
                });
            });
    });
    action
}

/// Returns true when "Tautkan nomor pertama" is clicked.
fn empty(ui: &mut Ui, p: &Palette) -> bool {
    let mut add = false;
    theme::card(ui.ctx())
        .inner_margin(Margin::same(40))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                icons::show(ui, "smartphone", 28.0, p.muted);
                ui.label(rich("Belum ada nomor tertaut", 16.0, Weight::SemiBold, p.text));
                ui.label(rich(
                    "Tautkan nomor WhatsApp sebagai perangkat pendamping untuk mulai menerima dan mengirim pesan.",
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
                ui.add_space(6.0);
                add = widgets::button(ui, Some("plus"), "Tautkan nomor pertama", Kind::Primary).clicked();
            });
        });
    add
}

fn stats(ui: &mut Ui, p: &Palette, sessions: &[SessionView]) {
    let connected = sessions.iter().filter(|s| s.status == Status::Connected).count();
    let incoming: usize = sessions.iter().map(SessionView::incoming_today).sum();
    let sent: usize = sessions.iter().map(SessionView::sent_today).sum();
    let queue: usize = sessions.iter().map(|s| s.queue).sum();
    let problems = sessions.len() - connected;
    let cells = [
        ("Terhubung", connected.to_string(), format!("dari {}", sessions.len())),
        ("Pesan masuk", incoming.to_string(), "24 jam terakhir".to_owned()),
        ("Perlu tindakan", problems.to_string(), "nomor".to_owned()),
        ("Terkirim", sent.to_string(), "24 jam terakhir".to_owned()),
        ("Dalam antrean", queue.to_string(), "jeda ± 4 dtk per pesan".to_owned()),
    ];
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
    ui.painter()
        .rect_stroke(rect, R_LG, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let columns = widgets::columns(rect, &[None; 5], 0.0);
    for (i, (cell, (label, value, note))) in columns.iter().zip(cells).enumerate() {
        if i > 0 {
            ui.painter()
                .vline(cell.left(), rect.y_range().shrink(1.0), Stroke::new(1.0, p.border));
        }
        let inner = cell.shrink2(vec2(18.0, 14.0));
        let label_rect = Rect::from_min_size(inner.min, vec2(inner.width(), 16.0));
        widgets::paint_text(ui, label_rect, label, fonts::ui(12.0, Weight::Medium), p.muted, Align::Min);
        let value = ui
            .painter()
            .layout_no_wrap(value, fonts::ui(22.0, Weight::SemiBold), p.text);
        let value_w = value.size().x;
        ui.painter()
            .galley(pos2(inner.left(), inner.bottom() - value.size().y), value, p.text);
        let note_rect = Rect::from_min_max(
            pos2(inner.left() + value_w + 8.0, inner.bottom() - 18.0),
            pos2(inner.right(), inner.bottom()),
        );
        widgets::paint_text(ui, note_rect, &note, fonts::ui(12.0, Weight::Regular), p.muted, Align::Min);
    }
}

/// Incoming messages per hour over the last 24 hours; the rightmost bar is the current hour.
fn chart(ui: &mut Ui, p: &Palette, rect: Rect, sessions: &[SessionView]) {
    let mut buckets = [0u32; 24];
    for t in sessions.iter().flat_map(|s| s.incoming.iter()) {
        let hours_ago = (t.elapsed().as_secs() / 3600) as usize;
        if hours_ago < 24 {
            buckets[23 - hours_ago] += 1;
        }
    }
    let total: u32 = buckets.iter().sum();

    ui.painter()
        .rect_stroke(rect, R_LG, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(18.0, 16.0));
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), 18.0));
    widgets::paint_text(ui, head, "Pesan masuk per jam, 24 jam terakhir", fonts::ui(13.0, Weight::SemiBold), p.text, Align::Min);
    widgets::paint_text(ui, head, &format!("Total {total} pesan"), fonts::ui(12.0, Weight::Regular), p.muted, Align::Max);

    let axis = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 14.0), inner.max);
    let area = Rect::from_min_max(pos2(inner.left(), head.bottom() + 14.0), pos2(inner.right(), axis.top() - 8.0));
    let max = (*buckets.iter().max().unwrap_or(&0)).max(1) as f32;
    let bars = widgets::columns(area, &[None; 24], 4.0);
    for (i, (bar, value)) in bars.iter().zip(buckets).enumerate() {
        let h = (value as f32 / max * area.height()).max(2.0);
        let r = Rect::from_min_max(pos2(bar.left(), bar.bottom() - h), bar.max);
        let color = if i == 23 { p.bar_hi } else { p.bar };
        ui.painter().rect_filled(r, 2.0, color);
    }
    widgets::hline(ui, area, area.bottom() + 0.5, p.border);
    if total == 0 {
        widgets::paint_text(
            ui,
            area,
            "Belum ada pesan masuk sejak aplikasi dibuka",
            fonts::ui(13.0, Weight::Regular),
            p.muted,
            Align::Center,
        );
    }

    let labels = ["24 jam lalu", "18 jam", "12 jam", "6 jam", "sekarang"];
    let slots = widgets::columns(axis, &[None; 5], 0.0);
    for (i, (slot, label)) in slots.iter().zip(labels).enumerate() {
        let align = match i {
            0 => Align::Min,
            4 => Align::Max,
            _ => Align::Center,
        };
        let color = if i == 4 { p.text_2 } else { p.muted };
        widgets::paint_text(ui, *slot, label, fonts::mono(11.0), color, align);
    }
}

/// Icon, color, label and a one-line explanation for a status.
pub fn describe(p: &Palette, status: &Status) -> (&'static str, Color32, String, String) {
    match status {
        Status::Connected => ("check", p.green, "Terhubung".into(), String::new()),
        Status::Starting => ("loader-circle", p.gray, "Memulai".into(), "Membuka sesi".into()),
        Status::Pairing => ("qr-code", p.gray, "Menunggu pairing".into(), "Buka untuk memindai QR atau memakai kode".into()),
        Status::Reconnecting => ("rotate-cw", p.amber, "Menyambung ulang".into(), "Koneksi terputus, mencoba lagi".into()),
        Status::LoggedOut => ("unlink", p.red, "Terputus".into(), "Tautan dilepas dari HP, perlu ditautkan ulang".into()),
        Status::Replaced => ("triangle-alert", p.amber, "Diambil alih".into(), "Sesi yang sama tersambung di tempat lain".into()),
        Status::Locked => ("triangle-alert", p.amber, "Dipakai proses lain".into(), "Hentikan proses lain yang memakai sesi ini".into()),
        Status::Banned(reason) => ("triangle-alert", p.red, "Diblokir sementara".into(), reason.clone()),
        Status::Failed(reason) => ("triangle-alert", p.red, "Gagal".into(), reason.clone()),
    }
}

fn attention(ui: &mut Ui, p: &Palette, rect: Rect, sessions: &[SessionView]) -> Option<Action> {
    let items: Vec<&SessionView> = sessions.iter().filter(|s| s.status != Status::Connected).collect();
    ui.painter()
        .rect_stroke(rect, R_LG, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(18.0, 16.0));
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), 18.0));
    widgets::paint_text(ui, head, "Perlu perhatian", fonts::ui(13.0, Weight::SemiBold), p.text, Align::Min);
    widgets::paint_text(ui, head, &items.len().to_string(), fonts::mono(12.0), p.muted, Align::Max);

    if items.is_empty() {
        let line = Rect::from_min_size(pos2(inner.left(), head.bottom() + 14.0), vec2(inner.width(), 18.0));
        let icon = Rect::from_min_size(line.min, vec2(16.0, 16.0));
        icons::paint(ui, icon, "check", 16.0, p.green);
        let text = Rect::from_min_max(pos2(line.left() + 26.0, line.top()), line.max);
        widgets::paint_text(ui, text, "Semua nomor terhubung", fonts::ui(13.0, Weight::Medium), p.text, Align::Min);
        return None;
    }

    let mut action = None;
    let mut y = head.bottom() + 14.0;
    for session in items.iter().take(3) {
        let item = Rect::from_min_max(pos2(inner.left(), y), pos2(inner.right(), y + 34.0));
        let (icon, color, label, detail) = describe(p, &session.status);
        icons::paint(ui, Rect::from_min_size(item.min, vec2(16.0, 16.0)), icon, 16.0, color);
        let left = item.left() + 26.0;
        let title = Rect::from_min_max(pos2(left, y), pos2(item.right(), y + 17.0));
        let sub = Rect::from_min_max(pos2(left, y + 17.0), pos2(item.right(), y + 33.0));
        widgets::paint_text(ui, title, &format!("{}: {label}", session.name), fonts::ui(13.0, Weight::Medium), p.text, Align::Min);
        widgets::paint_text(ui, sub, &detail, fonts::ui(12.0, Weight::Regular), p.muted, Align::Min);
        let response = ui
            .interact(item, ui.scope_id().with(("attention", &session.id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            action = Some(fix(session));
        }
        y += 46.0;
    }
    if items.len() > 3 {
        let more = Rect::from_min_size(pos2(inner.left() + 26.0, y - 6.0), vec2(inner.width(), 16.0));
        widgets::paint_text(ui, more, &format!("+{} lainnya", items.len() - 3), fonts::ui(12.0, Weight::Regular), p.muted, Align::Min);
    }
    action
}

/// The obvious next step for a session that is not connected.
pub fn fix(session: &SessionView) -> Action {
    match session.status {
        Status::Pairing | Status::Starting => Action::Open(session.id.clone()),
        _ => Action::Relink(session.id.clone()),
    }
}

fn ago(at: Option<Instant>) -> String {
    let Some(at) = at else {
        return "-".to_owned();
    };
    match at.elapsed().as_secs() {
        0..60 => "Baru saja".to_owned(),
        s @ 60..3600 => format!("{} mnt lalu", s / 60),
        s @ 3600..86400 => format!("{} jam lalu", s / 3600),
        s => format!("{} hari lalu", s / 86400),
    }
}

fn table(ui: &mut Ui, p: &Palette, state: &mut State, sessions: &[SessionView]) -> Option<Action> {
    let width = ui.available_width();
    let (head, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
    widgets::hline(ui, head, head.bottom() - 0.5, p.border);
    let cols = widgets::columns(head.shrink2(vec2(12.0, 0.0)), &ROW_COLS, 16.0);
    let titles = ["Nomor", "Status", "Terkirim", "Pesan masuk", "Antrean", "Terakhir aktif", ""];
    for (i, (col, title)) in cols.iter().zip(titles).enumerate() {
        let align = if matches!(i, 2 | 4) { Align::Max } else { Align::Min };
        widgets::paint_text(ui, *col, title, fonts::ui(12.0, Weight::Medium), p.muted, align);
    }

    let query = state.nomor_search.trim().to_lowercase();
    let mut action = None;
    for session in sessions {
        let connected = session.status == Status::Connected;
        let tab_ok = match state.nomor_tab {
            1 => connected,
            2 => !connected,
            _ => true,
        };
        let phone = session.phone.clone().unwrap_or_else(|| "belum ditautkan".to_owned());
        let query_ok = query.is_empty()
            || session.name.to_lowercase().contains(&query)
            || phone.contains(&query);
        if !tab_ok || !query_ok {
            continue;
        }

        let (row, resp) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(row, 0.0, p.panel);
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !connected {
            action = Some(fix(session));
        }
        widgets::hline(ui, row, row.bottom() - 0.5, p.border);
        let cols = widgets::columns(row.shrink2(vec2(12.0, 0.0)), &ROW_COLS, 16.0);

        let tile = Rect::from_min_size(pos2(cols[0].left(), row.center().y - 14.0), vec2(28.0, 28.0));
        ui.painter()
            .rect(tile, theme::R_SM, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
        icons::paint(ui, tile, "smartphone", 14.0, p.text_2);
        let text_x = cols[0].left() + 40.0;
        let name = Rect::from_min_max(pos2(text_x, row.center().y - 18.0), pos2(cols[0].right(), row.center().y));
        let phone_rect = Rect::from_min_max(pos2(text_x, row.center().y), pos2(cols[0].right(), row.center().y + 16.0));
        widgets::paint_text(ui, name, &session.name, fonts::ui(14.0, Weight::Medium), p.text, Align::Min);
        widgets::paint_text(ui, phone_rect, &phone, fonts::mono(12.0), p.muted, Align::Min);

        let (_, color, label, detail) = describe(p, &session.status);
        widgets::paint_status(ui, cols[1], color, &label);
        if !detail.is_empty() && !connected {
            ui.interact(cols[1], ui.scope_id().with(("status", &session.id)), Sense::hover())
                .on_hover_text(detail);
        }
        widgets::paint_text(ui, cols[2], &session.sent_today().to_string(), fonts::mono(13.0), p.text, Align::Max);
        widgets::paint_text(ui, cols[3], &session.incoming_today().to_string(), fonts::mono(13.0), p.text, Align::Min);
        widgets::paint_text(ui, cols[4], &session.queue.to_string(), fonts::mono(13.0), p.text, Align::Max);
        widgets::paint_text(ui, cols[5], &ago(session.last_activity), fonts::ui(13.0, Weight::Regular), p.muted, Align::Min);

        // Row menu.
        let menu = ui
            .interact(cols[6], ui.scope_id().with(("menu", &session.id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if menu.hovered() {
            ui.painter().rect_filled(cols[6].shrink2(vec2(0.0, 14.0)), theme::R_SM, p.raised);
        }
        icons::paint(ui, cols[6], "ellipsis", 16.0, p.muted);
        if let Some(a) = row_menu(&menu, p, session) {
            action = Some(a);
        }
    }
    action
}

/// The menu a number's row opens: pairing, relink, unlink, remove.
pub fn row_menu(anchor: &egui::Response, p: &Palette, session: &SessionView) -> Option<Action> {
    let connected = session.status == Status::Connected;
    let mut action = None;
    egui::Popup::menu(anchor).show(|ui| {
        ui.set_min_width(180.0);
        if ui.button("Ganti nama…").clicked() {
            action = Some(Action::Rename(session.id.clone()));
        }
        // Kept apart from the steps that unlink, so a slip does not relink.
        ui.separator();
        if !connected && ui.button("Tautkan / tampilkan pairing").clicked() {
            action = Some(fix(session));
        }
        if ui.button("Tautkan ulang (pairing baru)").clicked() {
            action = Some(Action::Relink(session.id.clone()));
        }
        if session.phone.is_some() && ui.button("Putuskan tautan").clicked() {
            action = Some(Action::Logout(session.id.clone()));
        }
        ui.separator();
        if ui.button(rich("Hapus nomor", 13.0, Weight::Regular, p.red_text)).clicked() {
            action = Some(Action::Remove(session.id.clone()));
        }
    });
    action
}
