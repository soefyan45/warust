//! Ringkasan (Pego `v5/Ringkasan`): today at a glance. Linked numbers, what
//! was sent, queued and failed, chats waiting for a reply, webhook health, the
//! hourly send chart, what needs attention, and the latest chats. Every number
//! comes from the chat stores and the live state; nothing is sample data.

use egui::{Align, Color32, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::backend::webhook::{Delivery as HookDelivery, Outcome};
use crate::backend::{Chat, Delivery, SessionView, Status};
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette, Template};
use crate::ui::widgets::{self, Kind};
use crate::ui::{State, icons, time};

use super::nomor;

pub enum Action {
    Nomor(nomor::Action),
    /// Open a chat in the Inbox.
    OpenChat { session: String, jid: String },
    OpenInbox,
    /// Phone layout: the Perangkat tab.
    Devices,
}

const GAP: f32 = 16.0;

/// Today's outgoing messages across every number.
#[derive(Default)]
struct Today {
    sent: usize,
    received: usize,
    failed: usize,
    queued: usize,
    per_hour: [usize; 24],
    /// Chats whose last message is from the contact (not a group), last 24 h.
    waiting: usize,
    chats_today: usize,
}

fn today(sessions: &[SessionView]) -> Today {
    let now = time::now();
    let day = time::day(now);
    let mut t = Today::default();
    for s in sessions {
        t.queued += s.queue;
        for chat in &s.book.chats {
            if chat.messages.iter().any(|m| time::day(m.ts) == day) {
                t.chats_today += 1;
            }
            if waiting(chat, now) {
                t.waiting += 1;
            }
            for m in chat.messages.iter().rev().take_while(|m| time::day(m.ts) == day) {
                if !m.from_me {
                    t.received += 1;
                    continue;
                }
                match m.status {
                    Delivery::Sent | Delivery::Delivered | Delivery::Read => {
                        t.sent += 1;
                        t.per_hour[time::hour(m.ts)] += 1;
                    }
                    Delivery::Failed(_) => t.failed += 1,
                    _ => {}
                }
            }
        }
    }
    t
}

/// Needs a reply (see `Chat::needs_reply`) and came in within a day.
fn waiting(chat: &Chat, now: i64) -> bool {
    chat.needs_reply() && chat.messages.last().is_some_and(|m| now - m.ts < 24 * 3600)
}

/// Thousands with a comma in English (`1,332`) and a dot in Indonesian (`1.332`).
fn number(n: usize) -> String {
    let sep = tr(",", ".");
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(c);
    }
    out
}

fn percent(part: usize, total: usize) -> String {
    if total == 0 {
        return "-".to_owned();
    }
    let text = format!("{:.1}%", part as f64 * 100.0 / total as f64);
    if crate::i18n::indonesian() { text.replace('.', ",") } else { text }
}

fn forest() -> bool {
    theme::template() == Template::Forest
}

/// A card behind `rect`: white in Forest, outlined in Classic.
fn card(ui: &Ui, p: &Palette, rect: Rect, fill: Option<Color32>) {
    let r = theme::r_card();
    match (fill, forest()) {
        (Some(fill), _) => ui.painter().rect_filled(rect, r, fill),
        (None, true) => ui.painter().rect_filled(rect, r, p.panel),
        (None, false) => ui.painter().rect_stroke(rect, r, Stroke::new(1.0, p.border), StrokeKind::Inside),
    };
}

/// A small status tag: soft fill, coloured text.
fn tag(ui: &Ui, at: egui::Pos2, label: &str, fill: Color32, color: Color32) -> Rect {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(12.0, Weight::Medium), color);
    let rect = Rect::from_min_size(at, vec2(galley.size().x + 18.0, 22.0));
    ui.painter().rect_filled(rect, 11.0, fill);
    ui.painter().galley(pos2(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, color);
    rect
}

fn status_tag(p: &Palette, status: &Status) -> (&'static str, Color32, Color32) {
    match status {
        Status::Connected => (tr("Connected", "Terhubung"), p.accent_soft, p.accent_text),
        Status::Pairing | Status::Starting => (tr("Waiting for pairing", "Menunggu pairing"), p.blue_soft, p.blue),
        Status::Reconnecting | Status::Replaced | Status::Locked => (tr("Interrupted", "Tersendat"), p.orange_soft, p.amber),
        Status::LoggedOut | Status::Banned(_) | Status::Failed(_) => (tr("Disconnected", "Terputus"), p.red_soft, p.red),
    }
}

pub fn show(ui: &mut Ui, state: &mut State, sessions: &[SessionView], deliveries: &[HookDelivery]) -> Option<Action> {
    let p = theme::palette(ui.ctx());
    let mut action = None;
    let t = today(sessions);
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        theme::page(ui.ctx()).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            let now = time::now();
            super::page_head(ui, tr("Overview", "Ringkasan"), &crate::trf!("{} · updated {}", "{} · diperbarui {}", time::long_date(now), time::clock(now)));
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
            let width = ui.available_width();

            // Row 1: five tiles, the first one wider.
            let (row, _) = ui.allocate_exact_size(vec2(width, 150.0), Sense::hover());
            let unit = (width - GAP * 4.0) / 5.8;
            let mut x = row.left();
            let mut next = |w: f32| {
                let r = Rect::from_min_size(pos2(x, row.top()), vec2(w, row.height()));
                x += w + GAP;
                r
            };
            devices_tile(ui, &p, next(unit * 1.8), sessions);
            let total_out = t.sent + t.failed + t.queued;
            stat_tile(ui, &p, next(unit), "arrow-up-right", tr("Sent today", "Terkirim hari ini"), &number(t.sent), tr("messages", "pesan"), &crate::trf!("{} of {} queued", "{} dari {} antrean", percent(t.sent, total_out), number(total_out)));
            stat_tile(ui, &p, next(unit), "loader-circle", tr("Waiting to send", "Menunggu kirim"), &number(t.queued), tr("messages", "pesan"), &crate::trf!("{} failed since 00:00", "{} gagal sejak 00.00", number(t.failed)));
            stat_tile(ui, &p, next(unit), "inbox", tr("Not replied", "Belum dibalas"), &number(t.waiting), tr("chats", "chat"), &crate::trf!("of {} conversations today", "dari {} percakapan hari ini", number(t.chats_today)));
            let finished: Vec<&HookDelivery> = deliveries.iter().filter(|d| !matches!(d.outcome, Outcome::Pending)).collect();
            let ok = finished.iter().filter(|d| matches!(d.outcome, Outcome::Done(c) if (200..300).contains(&c))).count();
            let host = if state.settings.webhook.url.trim().is_empty() {
                tr("global webhook not set", "webhook global belum diatur").to_owned()
            } else {
                super::webhook_label(state, "")
            };
            stat_tile(ui, &p, next(unit), "webhook", tr("Webhook success", "Webhook berhasil"), &percent(ok, finished.len()), "", &crate::trf!("{} deliveries · {host}", "{} pengiriman · {host}", finished.len()));

            // Row 2: two columns.
            let left_w = ((width - GAP) * 0.66).floor();
            let right_w = width - GAP - left_w;
            let top = ui.cursor().top();
            let left_rect = Rect::from_min_size(pos2(row.left(), top), vec2(left_w, 4000.0));
            let right_rect = Rect::from_min_size(pos2(row.left() + left_w + GAP, top), vec2(right_w, 4000.0));
            let mut left = widgets::child(ui, left_rect, egui::Layout::top_down(Align::Min));
            left.spacing_mut().item_spacing.y = GAP;
            flow_card(&mut left, &p, &t);
            if let Some(a) = numbers_card(&mut left, &p, sessions) {
                action = Some(a);
            }
            let mut right = widgets::child(ui, right_rect, egui::Layout::top_down(Align::Min));
            right.spacing_mut().item_spacing.y = GAP;
            if let Some(a) = attention_card(&mut right, &p, sessions, &t, &finished) {
                action = Some(a);
            }
            if let Some(a) = recent_card(&mut right, &p, state, sessions) {
                action = Some(a);
            }
            let used = left.min_rect().height().max(right.min_rect().height());
            ui.allocate_exact_size(vec2(width, used), Sense::hover());
        });
    });
    action
}

fn tile_head(ui: &Ui, rect: Rect, icon: &str, label: &str, color: Color32, circle: Color32) {
    let badge = Rect::from_min_size(rect.min + vec2(20.0, 20.0), vec2(28.0, 28.0));
    ui.painter().circle_filled(badge.center(), 14.0, circle);
    icons::paint(ui, badge, icon, 15.0, color);
    let text = Rect::from_min_max(pos2(badge.right() + 10.0, badge.top()), pos2(rect.right() - 16.0, badge.bottom()));
    widgets::paint_text(ui, text, label, fonts::ui(13.0, Weight::Medium), color, Align::Min);
}

fn big_number(ui: &Ui, at: egui::Pos2, value: &str, unit: &str, color: Color32, muted: Color32) {
    let galley = ui.painter().layout_no_wrap(value.to_owned(), fonts::ui(30.0, Weight::SemiBold), color);
    let w = galley.size().x;
    ui.painter().galley(at, galley, color);
    if !unit.is_empty() {
        let unit_galley = ui.painter().layout_no_wrap(unit.to_owned(), fonts::ui(13.0, Weight::Regular), muted);
        ui.painter().galley(pos2(at.x + w + 6.0, at.y + 17.0), unit_galley, muted);
    }
}

fn stat_tile(ui: &Ui, p: &Palette, rect: Rect, icon: &str, label: &str, value: &str, unit: &str, detail: &str) {
    card(ui, p, rect, None);
    tile_head(ui, rect, icon, label, p.text_2, p.raised);
    big_number(ui, rect.min + vec2(20.0, 62.0), value, unit, p.text, p.muted);
    let line = Rect::from_min_max(rect.min + vec2(20.0, 108.0), pos2(rect.right() - 16.0, rect.min.y + 126.0));
    widgets::paint_text(ui, line, detail, fonts::ui(12.0, Weight::Regular), p.text_2, Align::Min);
}

fn status_color(p: &Palette, status: &Status) -> Color32 {
    match status {
        Status::Connected => p.green,
        Status::Pairing | Status::Starting => p.blue,
        Status::Reconnecting | Status::Replaced | Status::Locked => p.amber,
        _ => p.red,
    }
}

fn devices_tile(ui: &Ui, p: &Palette, rect: Rect, sessions: &[SessionView]) {
    // The lime tile in Forest; a plain card in Classic.
    let (fill, text, muted) = if forest() { (Some(p.accent), p.primary, p.text_2) } else { (None, p.text, p.muted) };
    card(ui, p, rect, fill);
    let circle = if forest() { p.panel } else { p.raised };
    tile_head(ui, rect, "smartphone", tr("Connected devices", "Perangkat terhubung"), text, circle);
    let connected = sessions.iter().filter(|s| s.status == Status::Connected).count();
    big_number(ui, rect.min + vec2(20.0, 62.0), &connected.to_string(), &crate::trf!("of {} linked devices", "dari {} perangkat tertaut", sessions.len()), text, muted);
    // One segment per number, coloured by its state.
    let bar = Rect::from_min_max(rect.min + vec2(20.0, 106.0), pos2(rect.right() - 20.0, rect.min.y + 112.0));
    let n = sessions.len().max(1) as f32;
    let seg = (bar.width() - 4.0 * (n - 1.0)) / n;
    for (i, s) in sessions.iter().enumerate() {
        let r = Rect::from_min_size(pos2(bar.left() + i as f32 * (seg + 4.0), bar.top()), vec2(seg, bar.height()));
        ui.painter().rect_filled(r, 3.0, status_color(p, &s.status));
    }
    if sessions.is_empty() {
        ui.painter().rect_filled(bar, 3.0, p.border);
    }
    let waiting = sessions.iter().filter(|s| matches!(s.status, Status::Pairing | Status::Starting)).count();
    let down = sessions.len() - connected - waiting;
    let mut x = rect.left() + 20.0;
    for (color, label) in [
        (p.green, crate::trf!("{connected} connected", "{connected} terhubung")),
        (p.blue, crate::trf!("{waiting} waiting for pairing", "{waiting} menunggu pairing")),
        (p.red, crate::trf!("{down} disconnected", "{down} terputus")),
    ] {
        let y = rect.top() + 128.0;
        ui.painter().circle_filled(pos2(x + 3.5, y), 3.5, color);
        let galley = ui.painter().layout_no_wrap(label, fonts::ui(12.0, Weight::Regular), text);
        let w = galley.size().x;
        ui.painter().galley(pos2(x + 12.0, y - galley.size().y / 2.0), galley, text);
        x += w + 20.0;
    }
}

fn flow_card(ui: &mut Ui, p: &Palette, t: &Today) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 400.0), Sense::hover());
    // Forest's dark green hero; a plain card in Classic.
    let (fill, text, muted, track, line, hi) = if forest() {
        (Some(p.primary), p.on_primary, theme::FOREST_MUTED, theme::FOREST_2, theme::FOREST_LINE, p.accent)
    } else {
        (None, p.text, p.muted, p.raised, p.bar, p.bar_hi)
    };
    card(ui, p, rect, fill);
    let inner = rect.shrink2(vec2(28.0, 26.0));
    let title = Rect::from_min_size(inner.min, vec2(inner.width(), 24.0));
    widgets::paint_text(ui, title, tr("Today's sending flow", "Alur pengiriman hari ini"), fonts::ui(18.0, Weight::SemiBold), text, Align::Min);
    let total = t.sent + t.failed + t.queued;
    let sub = Rect::from_min_size(inner.min + vec2(0.0, 30.0), vec2(inner.width(), 18.0));
    let summary = if total == 0 {
        tr("No outgoing messages today yet.", "Belum ada pesan keluar hari ini.").to_owned()
    } else {
        crate::trf!(
            "{} outgoing messages since 00:00: {} sent, {} waiting for the delay, {} failed.",
            "{} pesan keluar sejak 00.00: {} terkirim, {} menunggu jeda, {} gagal.",
            number(total),
            number(t.sent),
            number(t.queued),
            number(t.failed)
        )
    };
    widgets::paint_text(ui, sub, &summary, fonts::ui(13.0, Weight::Regular), muted, Align::Min);

    // Four figures across.
    let figures = [(tr("QUEUED", "DIANTREKAN"), total, text), (tr("SENT", "TERKIRIM"), t.sent, hi), (tr("WAITING", "MENUNGGU"), t.queued, text), (tr("FAILED", "GAGAL"), t.failed, text)];
    let col = inner.width() / 4.0;
    for (i, (label, value, color)) in figures.iter().enumerate() {
        let x = inner.left() + i as f32 * col;
        let align = if i == 3 { Align::Max } else { Align::Min };
        let head = Rect::from_min_size(pos2(x, inner.top() + 66.0), vec2(col, 14.0));
        widgets::paint_text(ui, head, label, fonts::ui(11.0, Weight::SemiBold), if i == 1 { hi } else { muted }, align);
        let val = Rect::from_min_size(pos2(x, inner.top() + 84.0), vec2(col, 20.0));
        widgets::paint_text(ui, val, &number(*value), fonts::ui(16.0, Weight::SemiBold), *color, align);
    }

    // Share bar: sent, waiting, failed.
    let bar = Rect::from_min_size(pos2(inner.left(), inner.top() + 122.0), vec2(inner.width(), 46.0));
    ui.painter().rect_filled(bar, 14.0, track);
    let inside = bar.shrink(4.0);
    if total > 0 {
        let mut x = inside.left();
        for (count, color) in [(t.sent, hi), (t.queued, muted), (t.failed, p.red)] {
            if count == 0 {
                continue;
            }
            let w = (inside.width() * count as f32 / total as f32).max(8.0);
            let seg = Rect::from_min_size(pos2(x, inside.top()), vec2(w.min(inside.right() - x), inside.height()));
            ui.painter().rect_filled(seg, 11.0, color);
            x += w + 3.0;
        }
        let label_color = if forest() { p.primary } else { p.on_primary };
        let label = Rect::from_min_size(inside.min + vec2(14.0, 0.0), vec2(inside.width(), inside.height()));
        widgets::paint_text(ui, label, &crate::trf!("{} sent", "{} terkirim", percent(t.sent, total)), fonts::ui(13.0, Weight::SemiBold), label_color, Align::Min);
    }

    // Outgoing per hour.
    let chart_top = inner.top() + 192.0;
    let head = Rect::from_min_size(pos2(inner.left(), chart_top), vec2(inner.width(), 16.0));
    widgets::paint_text(ui, head, tr("Outgoing messages per hour", "Pesan keluar per jam"), fonts::ui(12.0, Weight::SemiBold), text, Align::Min);
    let (peak_hour, peak) = t.per_hour.iter().enumerate().max_by_key(|(_, n)| **n).map(|(h, n)| (h, *n)).unwrap_or((0, 0));
    let peak_text = if peak == 0 {
        tr("None yet", "Belum ada").to_owned()
    } else {
        crate::trf!("Peak {peak_hour:02}:00, {} messages", "Puncak {peak_hour:02}.00, {} pesan", number(peak))
    };
    widgets::paint_text(ui, head, &peak_text, fonts::ui(12.0, Weight::Regular), muted, Align::Max);
    let chart = Rect::from_min_max(pos2(inner.left(), chart_top + 28.0), pos2(inner.right(), inner.bottom() - 24.0));
    let max = peak.max(1) as f32;
    let current = time::hour(time::now());
    let bw = chart.width() / 24.0;
    for (h, n) in t.per_hour.iter().enumerate() {
        let height = (chart.height() * *n as f32 / max).max(if *n > 0 { 6.0 } else { 3.0 });
        let r = Rect::from_min_max(pos2(chart.left() + h as f32 * bw + 2.0, chart.bottom() - height), pos2(chart.left() + (h + 1) as f32 * bw - 2.0, chart.bottom()));
        let color = if h == current { hi } else if *n == peak && peak > 0 { muted } else { line };
        ui.painter().rect_filled(r, 2.0, color);
    }
    for (h, label) in [(0usize, "00.00"), (6, "06.00"), (12, "12.00"), (18, "18.00"), (23, "23.00")] {
        let x = chart.left() + h as f32 * bw;
        let r = Rect::from_min_size(pos2(x, chart.bottom() + 8.0), vec2(bw * 2.0, 14.0));
        let align = if h == 23 { Align::Max } else { Align::Min };
        let r = if h == 23 { Rect::from_min_max(pos2(chart.right() - 60.0, r.top()), pos2(chart.right(), r.bottom())) } else { r };
        widgets::paint_text(ui, r, label, fonts::mono(11.0), if h == 23 { hi } else { muted }, align);
    }
}

fn numbers_card(ui: &mut Ui, p: &Palette, sessions: &[SessionView]) -> Option<Action> {
    let width = ui.available_width();
    let rows = sessions.len().max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 132.0 + rows * 62.0), Sense::hover());
    card(ui, p, rect, None);
    let inner = rect.shrink2(vec2(24.0, 22.0));
    let connected = sessions.iter().filter(|s| s.status == Status::Connected).count();
    let title = Rect::from_min_size(inner.min, vec2(inner.width(), 24.0));
    widgets::paint_text(ui, title, tr("Linked numbers", "Nomor tertaut"), fonts::ui(18.0, Weight::SemiBold), p.text, Align::Min);
    let sub = Rect::from_min_size(inner.min + vec2(0.0, 28.0), vec2(inner.width() - 180.0, 18.0));
    let desc = crate::trf!(
        "{} numbers linked as companion devices, {connected} connected now.",
        "{} nomor tertaut sebagai perangkat pendamping, {connected} sedang terhubung.",
        sessions.len()
    );
    widgets::paint_text(ui, sub, &desc, fonts::ui(13.0, Weight::Regular), p.muted, Align::Min);
    let mut out = None;
    let button_rect = Rect::from_min_size(pos2(inner.right() - 160.0, inner.top()), vec2(160.0, 36.0));
    let mut b = widgets::child(ui, button_rect, egui::Layout::right_to_left(Align::Center));
    if widgets::button(&mut b, Some("plus"), tr("Link number", "Tautkan nomor"), Kind::Primary).clicked() {
        out = Some(Action::Nomor(nomor::Action::Add));
    }

    let head = Rect::from_min_size(pos2(inner.left(), inner.top() + 66.0), vec2(inner.width(), 22.0));
    let cols = widgets::columns(head, &[None, Some(160.0), Some(80.0), Some(70.0)], 12.0);
    for (i, label) in [tr("Number", "Nomor"), "Status", tr("Sent", "Terkirim"), tr("Queued", "Antrean")].iter().enumerate() {
        let align = if i >= 2 { Align::Max } else { Align::Min };
        widgets::paint_text(ui, cols[i], label, fonts::ui(12.0, Weight::Medium), p.muted, align);
    }
    widgets::hline(ui, head, head.bottom() + 4.0, p.border);
    if sessions.is_empty() {
        let empty = Rect::from_min_size(pos2(inner.left(), head.bottom() + 20.0), vec2(inner.width(), 20.0));
        widgets::paint_text(ui, empty, tr("No numbers yet. Link your first number.", "Belum ada nomor. Tautkan nomor pertama."), fonts::ui(13.0, Weight::Regular), p.muted, Align::Min);
    }
    for (i, s) in sessions.iter().enumerate() {
        let row = Rect::from_min_size(pos2(inner.left(), head.bottom() + 6.0 + i as f32 * 62.0), vec2(inner.width(), 62.0));
        let response = ui.interact(row, ui.scope_id().with(("summary_number", &s.id)), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(row, theme::r_inner(), p.raised);
        }
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            out = Some(Action::Nomor(nomor::fix(s)));
        }
        widgets::hline(ui, row, row.bottom() - 0.5, p.border);
        let cols = widgets::columns(row, &[None, Some(160.0), Some(80.0), Some(70.0)], 12.0);
        let icon = Rect::from_center_size(pos2(cols[0].left() + 20.0, row.center().y), vec2(38.0, 38.0));
        ui.painter().circle_filled(icon.center(), 19.0, p.raised);
        let glyph = match s.status {
            Status::Connected => "smartphone",
            Status::Pairing | Status::Starting => "qr-code",
            _ => "unlink",
        };
        icons::paint(ui, icon, glyph, 16.0, p.text_2);
        let name = Rect::from_min_max(pos2(icon.right() + 12.0, row.top() + 12.0), pos2(cols[0].right(), row.top() + 31.0));
        widgets::paint_text(ui, name, &s.name, fonts::ui(14.0, Weight::Medium), p.text, Align::Min);
        let phone = Rect::from_min_max(pos2(icon.right() + 12.0, row.top() + 32.0), pos2(cols[0].right(), row.top() + 50.0));
        let phone_text = s.phone.clone().unwrap_or_else(|| tr("not linked yet", "belum ditautkan").to_owned());
        widgets::paint_text(ui, phone, &phone_text, fonts::mono(12.0), p.muted, Align::Min);
        let (label, fill, color) = status_tag(p, &s.status);
        tag(ui, pos2(cols[1].left(), row.center().y - 11.0), label, fill, color);
        widgets::paint_text(ui, cols[2], &number(s.sent_today()), fonts::ui(14.0, Weight::Medium), p.text, Align::Max);
        widgets::paint_text(ui, cols[3], &number(s.queue), fonts::ui(14.0, Weight::Regular), p.text_2, Align::Max);
    }
    out
}

fn attention_card(ui: &mut Ui, p: &Palette, sessions: &[SessionView], t: &Today, deliveries: &[&HookDelivery]) -> Option<Action> {
    struct Item {
        icon: &'static str,
        title: String,
        when: String,
        tag: (&'static str, Color32, Color32),
        action: Option<Action>,
    }
    let mut items: Vec<Item> = sessions
        .iter()
        .filter(|s| s.status != Status::Connected)
        .map(|s| {
            let (icon, _, label, _) = nomor::describe(p, &s.status);
            let queued = if s.queue > 0 { crate::trf!(", {} messages held", ", {} pesan tertahan", s.queue) } else { String::new() };
            Item {
                icon,
                title: format!("{}: {}{queued}", s.name, label.to_lowercase()),
                when: s.last_activity.map(|_| tr("Last activity recorded", "Aktivitas terakhir tercatat").to_owned()).unwrap_or_else(|| tr("Since the app was opened", "Sejak aplikasi dibuka").to_owned()),
                tag: status_tag(p, &s.status),
                action: Some(Action::Nomor(nomor::fix(s))),
            }
        })
        .collect();
    if t.failed > 0 {
        items.push(Item {
            icon: "triangle-alert",
            title: crate::trf!("{} messages failed to send today", "{} pesan gagal terkirim hari ini", number(t.failed)),
            when: tr("See the Failed tab in Message log", "Lihat tab Gagal di Log pesan").to_owned(),
            tag: (tr("Failed", "Gagal"), p.red_soft, p.red),
            action: None,
        });
    }
    let failed_hooks: Vec<&&HookDelivery> = deliveries
        .iter()
        .filter(|d| match &d.outcome {
            Outcome::Done(code) => !(200..300).contains(code),
            Outcome::Failed(_) => true,
            Outcome::Pending => false,
        })
        .collect();
    if let Some(last) = failed_hooks.first() {
        let reason = match &last.outcome {
            Outcome::Done(code) => format!("HTTP {code}"),
            Outcome::Failed(e) => e.chars().take(40).collect(),
            Outcome::Pending => String::new(),
        };
        items.push(Item {
            icon: "webhook",
            title: crate::trf!("Webhook failed {} times, {reason}", "Webhook gagal {} kali, {reason}", failed_hooks.len()),
            when: time::clock(last.at),
            tag: if last.attempts < 5 { (tr("Retrying", "Dicoba ulang"), p.blue_soft, p.blue) } else { (tr("Gave up", "Menyerah"), p.red_soft, p.red) },
            action: None,
        });
    }
    let width = ui.available_width();
    let rows = items.len().max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 72.0 + rows * 74.0), Sense::hover());
    card(ui, p, rect, None);
    let inner = rect.shrink2(vec2(24.0, 22.0));
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), 22.0));
    widgets::paint_text(ui, head, tr("Needs attention", "Perlu perhatian"), fonts::ui(16.0, Weight::SemiBold), p.text, Align::Min);
    widgets::paint_text(ui, head, &crate::trf!("{} items", "{} hal", items.len()), fonts::ui(12.0, Weight::Regular), p.muted, Align::Max);
    if items.is_empty() {
        let line = Rect::from_min_size(pos2(inner.left(), head.bottom() + 18.0), vec2(inner.width(), 20.0));
        icons::paint(ui, Rect::from_min_size(line.min, vec2(16.0, 16.0)), "check", 16.0, p.green);
        let text = Rect::from_min_max(pos2(line.left() + 26.0, line.top()), line.max);
        widgets::paint_text(ui, text, tr("All good: numbers connected, nothing failed.", "Semua lancar: nomor terhubung, tidak ada yang gagal."), fonts::ui(13.0, Weight::Regular), p.text_2, Align::Min);
        return None;
    }
    let mut out = None;
    for (i, item) in items.into_iter().enumerate() {
        let top = head.bottom() + 16.0 + i as f32 * 74.0;
        let row = Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), 66.0));
        icons::paint(ui, Rect::from_min_size(row.min, vec2(18.0, 18.0)), item.icon, 16.0, p.text_2);
        let title = Rect::from_min_max(pos2(row.left() + 28.0, row.top()), pos2(row.right(), row.top() + 20.0));
        widgets::paint_text(ui, title, &item.title, fonts::ui(14.0, Weight::Regular), p.text, Align::Min);
        let when = ui.painter().layout_no_wrap(item.when.clone(), fonts::ui(12.0, Weight::Regular), p.muted);
        let when_w = when.size().x;
        ui.painter().galley(pos2(row.left() + 28.0, row.top() + 33.0), when, p.muted);
        let (label, fill, color) = item.tag;
        tag(ui, pos2(row.left() + 40.0 + when_w, row.top() + 29.0), label, fill, color);
        if let Some(action) = item.action {
            let response = ui.interact(row, ui.scope_id().with(("attention_item", i)), Sense::click());
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                out = Some(action);
            }
        }
    }
    out
}

fn recent_card(ui: &mut Ui, p: &Palette, state: &mut State, sessions: &[SessionView]) -> Option<Action> {
    let now = time::now();
    let mut chats: Vec<(&SessionView, &Chat)> = sessions.iter().flat_map(|s| s.book.chats.iter().map(move |c| (s, c))).collect();
    chats.sort_by_key(|(_, c)| std::cmp::Reverse(c.last_ts()));
    let waiting: Vec<(&SessionView, &Chat)> = chats.iter().copied().filter(|(_, c)| waiting(c, now)).collect();
    let shown: Vec<(&SessionView, &Chat)> = if state.summary_tab == 0 { waiting.clone() } else { chats.clone() };
    let shown = &shown[..shown.len().min(5)];
    let width = ui.available_width();
    let rows = shown.len().max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 128.0 + rows * 62.0), Sense::hover());
    card(ui, p, rect, None);
    let inner = rect.shrink2(vec2(24.0, 22.0));
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), 22.0));
    widgets::paint_text(ui, head, tr("Latest messages", "Pesan terbaru"), fonts::ui(16.0, Weight::SemiBold), p.text, Align::Min);
    let mut out = None;
    let link = Rect::from_min_max(pos2(head.right() - 90.0, head.top()), head.max);
    let link_response = ui.interact(link, ui.scope_id().with("open_inbox"), Sense::click());
    widgets::paint_text(ui, link, tr("Open inbox", "Buka inbox"), fonts::ui(13.0, Weight::SemiBold), p.primary, Align::Max);
    if link_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        out = Some(Action::OpenInbox);
    }
    let tabs_rect = Rect::from_min_size(pos2(inner.left(), head.bottom() + 14.0), vec2(inner.width(), 40.0));
    let mut tabs_ui = widgets::child(ui, tabs_rect, egui::Layout::left_to_right(Align::Center));
    let labels = [crate::trf!("Not replied · {}", "Belum dibalas · {}", waiting.len()), crate::trf!("All · {}", "Semua · {}", chats.len())];
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    widgets::tabs(&mut tabs_ui, &labels, &mut state.summary_tab);
    let list_top = tabs_rect.bottom() + 10.0;
    if shown.is_empty() {
        let empty = Rect::from_min_size(pos2(inner.left(), list_top + 12.0), vec2(inner.width(), 20.0));
        let text = if state.summary_tab == 0 { tr("No chats waiting for a reply.", "Tidak ada chat yang menunggu balasan.") } else { tr("No chats yet.", "Belum ada chat.") };
        widgets::paint_text(ui, empty, text, fonts::ui(13.0, Weight::Regular), p.muted, Align::Min);
    }
    for (i, (session, chat)) in shown.iter().enumerate() {
        let row = Rect::from_min_size(pos2(inner.left(), list_top + i as f32 * 62.0), vec2(inner.width(), 62.0));
        let response = ui.interact(row, ui.scope_id().with(("recent", &session.id, &chat.jid)), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(row, theme::r_inner(), p.raised);
        }
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            out = Some(Action::OpenChat { session: session.id.clone(), jid: chat.jid.clone() });
        }
        widgets::hline(ui, row, row.bottom() - 0.5, p.border);
        let title = super::chat_title(&session.book, chat);
        let avatar = Rect::from_center_size(pos2(row.left() + 20.0, row.center().y), vec2(38.0, 38.0));
        ui.painter().circle_filled(avatar.center(), 19.0, if forest() { p.accent_soft } else { p.raised });
        let initials = if chat.is_group() { "GR".to_owned() } else { super::initials(&title) };
        widgets::paint_text(ui, avatar, &initials, fonts::ui(12.0, Weight::SemiBold), p.accent_text, Align::Center);
        let left = avatar.right() + 12.0;
        let when = time::short(chat.last_ts());
        let name = Rect::from_min_max(pos2(left, row.top() + 11.0), pos2(row.right() - 60.0, row.top() + 30.0));
        widgets::paint_text(ui, name, &title, fonts::ui(14.0, Weight::Medium), p.text, Align::Min);
        let time_rect = Rect::from_min_max(pos2(row.right() - 60.0, row.top() + 11.0), pos2(row.right(), row.top() + 28.0));
        widgets::paint_text(ui, time_rect, &when, fonts::ui(12.0, Weight::Regular), p.muted, Align::Max);
        let preview = chat
            .messages
            .last()
            .map(|m| if m.from_me { format!("{}: {}", tr("You", "Anda"), m.text.replace('\n', " ")) } else { m.text.replace('\n', " ") })
            .unwrap_or_default();
        let line = Rect::from_min_max(pos2(left, row.top() + 31.0), pos2(row.right() - 34.0, row.top() + 50.0));
        widgets::paint_text(ui, line, &preview, fonts::ui(13.0, Weight::Regular), p.text_2, Align::Min);
        if chat.unread > 0 {
            let badge = Rect::from_center_size(pos2(row.right() - 10.0, row.top() + 40.0), vec2(20.0, 20.0));
            ui.painter().circle_filled(badge.center(), 10.0, p.primary);
            widgets::paint_text(ui, badge, &chat.unread.min(99).to_string(), fonts::ui(11.0, Weight::SemiBold), p.on_primary, Align::Center);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_numbers() {
        assert_eq!(number(0), "0");
        assert_eq!(number(1332), "1,332");
        assert_eq!(number(1_204_000), "1,204,000");
        assert_eq!(percent(1332, 1402), "95.0%");
        assert_eq!(percent(1, 0), "-");
    }
}

/// "14 menit", "3 jam": how long a chat has waited.
fn waited(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => tr("less than 1 minute", "kurang dari 1 menit").to_owned(),
        s if s < 3600 => {
            let n = s / 60;
            let en = if n == 1 { "minute" } else { "minutes" };
            crate::trf!("{n} {en}", "{n} menit")
        }
        s => {
            let n = s / 3600;
            let en = if n == 1 { "hour" } else { "hours" };
            crate::trf!("{n} {en}", "{n} jam")
        }
    }
}

/// Beranda in the phone layout (Pego `m5/Beranda`, `m3/Beranda`): today's
/// messages, chats waiting for a reply, and the numbers.
pub fn compact(ui: &mut Ui, sessions: &[SessionView], deliveries: &[HookDelivery], main: Option<&str>) -> Option<Action> {
    use crate::ui::compact::{self, PAD};
    let p = theme::palette(ui.ctx());
    let mut action = None;
    let t = today(sessions);
    let now = time::now();
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin { left: PAD as i8, right: PAD as i8, top: 12, bottom: 24 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let width = ui.available_width();
            // Brand.
            widgets::row(ui, 44.0, |ui| {
                let (logo, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                let mark = if forest() { p.accent } else { p.on_primary };
                widgets::logo(ui, logo, p.primary, mark);
                ui.add_space(4.0);
                ui.label(widgets::rich("warust", 18.0, Weight::SemiBold, p.text));
            }, |_| {});
            ui.add_space(16.0);
            ui.label(widgets::rich(tr("Overview", "Ringkasan"), 28.0, Weight::SemiBold, p.text));
            ui.add_space(4.0);
            let connected = sessions.iter().filter(|s| s.status == Status::Connected).count();
            ui.label(widgets::rich(&crate::trf!("{} · {connected} numbers can reply", "{} · {connected} nomor bisa membalas", time::long_date(now)), 14.0, Weight::Regular, p.muted));
            ui.add_space(20.0);

            // Today's messages: the dark (Forest green / black) card.
            let (hero, _) = ui.allocate_exact_size(vec2(width, 190.0), Sense::hover());
            ui.painter().rect_filled(hero, 20.0, p.primary);
            let (on, muted) = (p.on_primary, p.on_primary_muted);
            let figure = if forest() { p.accent } else { p.on_primary };
            widgets::paint_text(ui, Rect::from_min_size(hero.min + vec2(20.0, 18.0), vec2(width - 40.0, 20.0)), tr("Messages today", "Pesan hari ini"), fonts::ui(14.0, Weight::Regular), muted, Align::Min);
            big_number_sized(ui, hero.min + vec2(20.0, 46.0), &number(t.received + t.sent), tr("messages", "pesan"), 40.0, on, muted);
            ui.painter().hline((hero.left() + 20.0)..=(hero.right() - 20.0), hero.top() + 112.0, Stroke::new(1.0, muted.gamma_multiply(0.4)));
            let finished: Vec<&HookDelivery> = deliveries.iter().filter(|d| !matches!(d.outcome, Outcome::Pending)).collect();
            let ok = finished.iter().filter(|d| matches!(d.outcome, Outcome::Done(c) if (200..300).contains(&c))).count();
            let third = (width - 40.0) / 3.0;
            for (i, (label, value)) in [(tr("In", "Masuk"), number(t.received)), (tr("Out", "Keluar"), number(t.sent)), ("Webhook", percent(ok, finished.len()))].into_iter().enumerate() {
                let x = hero.left() + 20.0 + i as f32 * third;
                widgets::paint_text(ui, Rect::from_min_size(pos2(x, hero.top() + 126.0), vec2(third, 18.0)), label, fonts::ui(13.0, Weight::Regular), muted, Align::Min);
                widgets::paint_text(ui, Rect::from_min_size(pos2(x, hero.top() + 146.0), vec2(third, 26.0)), &value, fonts::ui(20.0, Weight::SemiBold), figure, Align::Min);
            }
            ui.add_space(20.0);

            // Chats waiting for a reply.
            let oldest = sessions
                .iter()
                .flat_map(|s| s.book.chats.iter().filter(|c| waiting(c, now)).map(move |c| (s, c)))
                .min_by_key(|(_, c)| c.last_ts());
            let (card_rect, response) = ui.allocate_exact_size(vec2(width, 72.0), Sense::click());
            compact::paint_card(ui, card_rect);
            let (title, detail) = match oldest {
                Some((s, c)) => (
                    crate::trf!("{} chats need a reply", "{} chat perlu dibalas", t.waiting),
                    crate::trf!("Longest wait {}, from {}", "Paling lama {}, dari {}", waited(now - c.last_ts()), s.name),
                ),
                None => (
                    tr("All chats replied to", "Semua chat sudah dibalas").to_owned(),
                    tr("No private chats waiting within 24 hours", "Tidak ada chat pribadi yang menunggu dalam 24 jam").to_owned(),
                ),
            };
            let text_right = card_rect.right() - 64.0;
            widgets::paint_text(ui, Rect::from_min_max(card_rect.min + vec2(20.0, 14.0), pos2(text_right, card_rect.top() + 36.0)), &title, fonts::ui(16.0, Weight::SemiBold), p.text, Align::Min);
            widgets::paint_text(ui, Rect::from_min_max(card_rect.min + vec2(20.0, 38.0), pos2(text_right, card_rect.top() + 58.0)), &detail, fonts::ui(14.0, Weight::Regular), p.text_2, Align::Min);
            let circle = pos2(card_rect.right() - 36.0, card_rect.center().y);
            let (fill, color) = if forest() { (p.orange_soft, theme::FOREST_ORANGE) } else { (p.primary, p.on_primary) };
            ui.painter().circle_filled(circle, 20.0, fill);
            icons::paint(ui, Rect::from_center_size(circle, vec2(18.0, 18.0)), "arrow-right", 18.0, color);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                action = Some(Action::OpenInbox);
            }
            ui.add_space(28.0);

            // Numbers.
            let mut more = false;
            widgets::row(ui, 28.0, |ui| {
                ui.label(widgets::rich(tr("Devices", "Perangkat"), 18.0, Weight::SemiBold, p.text));
            }, |ui| {
                let link = if forest() { p.primary } else { p.text };
                more = ui
                    .add(egui::Label::new(widgets::rich(tr("See all", "Lihat semua"), 15.0, Weight::SemiBold, link)).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked();
            });
            if more {
                action = Some(Action::Devices);
            }
            ui.add_space(12.0);
            compact::card(ui.ctx()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                if sessions.is_empty() {
                    ui.add_space(16.0);
                    ui.label(widgets::rich(tr("No linked numbers yet.", "Belum ada nomor tertaut."), 14.0, Weight::Regular, p.muted));
                    ui.add_space(10.0);
                    if widgets::button(ui, Some("plus"), tr("Add number", "Tambah nomor"), Kind::Primary).clicked() {
                        action = Some(Action::Nomor(nomor::Action::Add));
                    }
                    ui.add_space(16.0);
                }
                let shown = sessions.len().min(4);
                for (i, s) in sessions.iter().take(shown).enumerate() {
                    if compact::device_row(ui, s, i + 1 < shown, main == Some(s.id.as_str())).clicked() {
                        action = Some(Action::Devices);
                    }
                }
            });
        });
    });
    action
}

fn big_number_sized(ui: &Ui, at: egui::Pos2, value: &str, unit: &str, size: f32, color: Color32, muted: Color32) {
    let galley = ui.painter().layout_no_wrap(value.to_owned(), fonts::ui(size, Weight::SemiBold), color);
    let (w, h) = (galley.size().x, galley.size().y);
    ui.painter().galley(at, galley, color);
    let unit_galley = ui.painter().layout_no_wrap(unit.to_owned(), fonts::ui(15.0, Weight::Regular), muted);
    let uh = unit_galley.size().y;
    ui.painter().galley(pos2(at.x + w + 8.0, at.y + h - uh - 6.0), unit_galley, muted);
}
