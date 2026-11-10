//! The Inbox's device panel (Pego `v5/Inbox · Perangkat`): "every number"
//! or one number, with unread counts, and the numbers that cannot reply now.

use egui::{Align, Color32, Rect, Sense, Ui, pos2, vec2};

use crate::backend::{SessionView, Status};
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette};
use crate::ui::widgets::{self, Kind, rich};
use crate::ui::{State, icons};

const ITEM: f32 = 56.0;

fn unread(session: &SessionView) -> u32 {
    session.book.chats.iter().map(|c| c.unread).sum()
}

/// Short label of a number for chips: name and the last four digits.
pub fn chip_label(session: &SessionView) -> String {
    let digits: String = session.phone.as_deref().unwrap_or_default().chars().filter(char::is_ascii_digit).collect();
    if digits.len() >= 4 {
        format!("{} · {}", session.name, &digits[digits.len() - 4..])
    } else {
        session.name.clone()
    }
}

pub fn status_dot(p: &Palette, status: &Status) -> Color32 {
    match status {
        Status::Connected => p.green,
        Status::Pairing | Status::Starting => p.blue,
        Status::Reconnecting | Status::Replaced | Status::Locked => p.amber,
        _ => p.red,
    }
}

/// One row: icon tile with a status dot, title, subtitle, unread badge.
#[allow(clippy::too_many_arguments)]
fn item(
    ui: &mut Ui,
    p: &Palette,
    icon: &str,
    title: &str,
    subtitle: &str,
    subtitle_color: Color32,
    mono: bool,
    dot: Option<Color32>,
    badge: u32,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ITEM), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, theme::r_inner(), p.selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, theme::r_inner(), p.raised);
    }
    let tile = Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), vec2(34.0, 34.0));
    ui.painter().circle_filled(tile.center(), 17.0, if selected { p.panel } else { p.raised });
    icons::paint(ui, tile, icon, 16.0, p.text_2);
    if let Some(dot) = dot {
        let at = pos2(tile.right() - 4.0, tile.bottom() - 4.0);
        ui.painter().circle_filled(at, 5.0, p.panel);
        ui.painter().circle_filled(at, 3.5, dot);
    }
    let left = tile.right() + 12.0;
    let right = rect.right() - if badge > 0 { 40.0 } else { 10.0 };
    let name = Rect::from_min_max(pos2(left, rect.top() + 9.0), pos2(right, rect.top() + 28.0));
    widgets::paint_text(ui, name, title, fonts::ui(14.0, Weight::Medium), p.text, Align::Min);
    let sub = Rect::from_min_max(pos2(left, rect.top() + 29.0), pos2(right, rect.top() + 46.0));
    let font = if mono { fonts::mono(12.0) } else { fonts::ui(12.0, Weight::Regular) };
    widgets::paint_text(ui, sub, subtitle, font, subtitle_color, Align::Min);
    if badge > 0 {
        let label = if badge > 99 { "99+".to_owned() } else { badge.to_string() };
        let w = if badge > 9 { 28.0 } else { 20.0 };
        let b = Rect::from_center_size(pos2(rect.right() - 10.0 - w / 2.0, rect.center().y), vec2(w, 20.0));
        ui.painter().rect_filled(b, 10.0, p.primary);
        widgets::paint_text(ui, b, &label, fonts::ui(11.0, Weight::SemiBold), p.on_primary, Align::Center);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Right-click on a number: its inbox in a window of its own, or a new name.
fn number_menu(row: &egui::Response, state: &mut State, session: &SessionView) {
    row.context_menu(|ui| {
        if ui.button("Buka di jendela baru").clicked() {
            state.open_window_requested = Some(session.id.clone());
        }
        if ui.button("Ganti nama…").clicked() {
            state.rename = Some((session.id.clone(), session.name.clone()));
        }
    });
}

pub fn show(ui: &mut Ui, p: &Palette, state: &mut State, sessions: &[SessionView]) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let connected: Vec<&SessionView> = sessions.iter().filter(|s| s.status == Status::Connected).collect();
    let others: Vec<&SessionView> = sessions.iter().filter(|s| s.status != Status::Connected).collect();
    ui.add_space(14.0);
    ui.label(rich("Perangkat", 18.0, Weight::SemiBold, p.text));
    ui.label(rich(&format!("{} dari {} nomor terhubung", connected.len(), sessions.len()), 12.0, Weight::Regular, p.muted));
    ui.add_space(10.0);

    let total: u32 = sessions.iter().map(unread).sum();
    let all = item(
        ui,
        p,
        "inbox",
        "Semua nomor",
        &format!("Gabungan {} nomor", sessions.len()),
        p.accent_text,
        false,
        None,
        total,
        state.inbox_scope.is_none(),
    );
    if all.clicked() {
        state.inbox_scope = None;
    }
    for s in &connected {
        let phone = s.phone.clone().unwrap_or_default();
        let selected = state.inbox_scope.as_deref() == Some(s.id.as_str());
        let row = item(ui, p, "smartphone", &s.name, &phone, p.muted, true, Some(p.green), unread(s), selected);
        if row.clicked() {
            state.inbox_scope = Some(s.id.clone());
        }
        number_menu(&row, state, s);
    }
    if !others.is_empty() {
        ui.add_space(10.0);
        ui.label(rich("TIDAK BISA MEMBALAS", 11.0, Weight::SemiBold, p.muted));
        for s in &others {
            let (icon, _, label, _) = super::nomor::describe(p, &s.status);
            let chats = s.book.chats.len();
            let detail = if chats > 0 { format!("{label} · {chats} chat lama") } else { label };
            let selected = state.inbox_scope.as_deref() == Some(s.id.as_str());
            let dot = status_dot(p, &s.status);
            let row = item(ui, p, icon, &s.name, &detail, dot, false, Some(dot), unread(s), selected);
            if row.clicked() {
                state.inbox_scope = Some(s.id.clone());
            }
            number_menu(&row, state, s);
        }
    }
    ui.add_space(12.0);
    egui::Frame::NONE
        .fill(p.raised)
        .corner_radius(theme::r_inner())
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::show(ui, "lock", 14.0, p.text_2);
                ui.add(
                    egui::Label::new(rich(
                        "Balasan selalu dikirim dari nomor yang menerima chat. Nomor pengirim tidak bisa diganti.",
                        12.0,
                        Weight::Regular,
                        p.text_2,
                    ))
                    .wrap(),
                );
            });
        });
    ui.add_space(8.0);
    if widgets::button(ui, Some("plus"), "Tautkan nomor", Kind::Secondary).clicked() {
        state.add_number_requested = true;
    }
}

/// Perangkat in the phone layout (Pego `m5/Perangkat`, `m3/Perangkat`): the
/// numbers that can reply, the ones that cannot, and "Tambah nomor". A row
/// opens the same menu as on the Nomor screen.
pub fn compact(ui: &mut Ui, sessions: &[SessionView]) -> Option<super::nomor::Action> {
    use crate::ui::compact::{self, PAD};
    let p = theme::palette(ui.ctx());
    let mut action = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin { left: PAD as i8, right: PAD as i8, top: 12, bottom: 24 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            compact::page_title(ui, "Perangkat", |_| {});
            ui.add_space(16.0);
            let forest = theme::template().top_nav();
            let (fill, color) = if forest { (p.accent_soft, p.accent_text) } else { (p.raised, p.text_2) };
            egui::Frame::NONE.fill(fill).corner_radius(16).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    icons::show(ui, "lock", 18.0, color);
                    ui.add(egui::Label::new(rich("Balasan selalu dikirim dari nomor yang menerima chat. Nomor pembalas tidak bisa diganti.", 14.0, Weight::Regular, color)).wrap());
                });
            });
            let groups: [(&str, Vec<&SessionView>); 2] = [
                ("Bisa membalas", sessions.iter().filter(|s| s.status == Status::Connected).collect()),
                ("Tidak bisa membalas", sessions.iter().filter(|s| s.status != Status::Connected).collect()),
            ];
            for (label, group) in groups {
                if group.is_empty() {
                    continue;
                }
                ui.add_space(24.0);
                ui.label(rich(&format!("{label} · {}", group.len()), 14.0, Weight::SemiBold, p.text_2));
                ui.add_space(10.0);
                compact::card(ui.ctx()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, s) in group.iter().enumerate() {
                        let row = compact::device_row(ui, s, i + 1 < group.len());
                        if let Some(a) = super::nomor::row_menu(&row, &p, s) {
                            action = Some(a);
                        }
                    }
                });
            }
            if sessions.is_empty() {
                ui.add_space(24.0);
                ui.label(rich("Belum ada nomor. Tautkan nomor WhatsApp untuk mulai.", 14.0, Weight::Regular, p.muted));
            }
            ui.add_space(24.0);
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
            let hover = if response.hovered() { 0.92 } else { 1.0 };
            ui.painter().rect_filled(rect, 26.0, p.primary.gamma_multiply(hover));
            let label = ui.painter().layout_no_wrap("Tambah nomor".to_owned(), fonts::ui(16.0, Weight::SemiBold), p.on_primary);
            let total = label.size().x + 28.0;
            let left = rect.center().x - total / 2.0;
            icons::paint(ui, Rect::from_center_size(pos2(left + 9.0, rect.center().y), vec2(18.0, 18.0)), "plus", 18.0, p.on_primary);
            ui.painter().galley(pos2(left + 28.0, rect.center().y - label.size().y / 2.0), label, p.on_primary);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                action = Some(super::nomor::Action::Add);
            }
        });
    });
    action
}
