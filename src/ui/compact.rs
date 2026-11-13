//! The phone-sized window (Pego `m5/*` Forest, `m3/*` Monokrom): a slim
//! title bar, one screen at a time and a tab bar along the bottom. Classic
//! dark uses the same layout with its own palette.

use std::path::Path;

use egui::{Align, Color32, CornerRadius, Frame, Margin, Panel, Rect, Response, Sense, Stroke, Theme, Ui, pos2, vec2};

use super::fonts::{self, Weight};
use crate::i18n::tr;
use super::theme::{self, Palette, Template};
use super::widgets::{self, rich};
use super::{Screen, icons, window};
use crate::backend::{SessionView, Status};

/// Below this window width the app switches to the phone layout; the desktop
/// layout needs about 1100 px for its navigation and panes.
pub const WIDTH: f32 = 1100.0;
/// Side padding of the phone screens.
pub const PAD: f32 = 20.0;

/// Mint shrinks in steps instead: below `MINT_RAIL` its sidebar becomes the
/// icon rail, below `MINT_SINGLE` the Inbox shows the list or the chat, not
/// both, and only below `MINT_WIDTH` does the phone layout take over.
pub const MINT_RAIL: f32 = 900.0;
pub const MINT_SINGLE: f32 = 800.0;
pub const MINT_WIDTH: f32 = 500.0;

/// The width below which the phone layout shows, for the active template.
pub fn width() -> f32 {
    if theme::mint() { MINT_WIDTH } else { WIDTH }
}

pub fn is_compact(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < width()
}

fn forest() -> bool {
    theme::template() == Template::Forest
}

/// "Warust" and the window buttons; also the window's drag area.
pub fn title_bar(ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    Panel::top("m_titlebar")
        .exact_size(40.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE.fill(theme::sidebar_fill(ui.ctx())).inner_margin(Margin { left: 14, right: 6, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let rect = ui.max_rect().expand2(vec2(14.0, 0.0));
            window::drag_area(ui, rect, "m_titlebar");
            ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
            ui.horizontal_centered(|ui| {
                // Past the macOS window buttons (the bar's margin is 14).
                ui.add_space((window::buttons_inset(ui.ctx()) - 14.0).max(0.0));
                ui.label(rich("Warust", 13.0, Weight::Medium, p.text_2));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), window::controls);
            });
        });
}

/// The tab a screen belongs to.
fn tab_of(screen: Screen) -> usize {
    match screen {
        Screen::Ringkasan => 0,
        Screen::Inbox => 1,
        Screen::Status => 2,
        Screen::Nomor | Screen::Pairing => 3,
        Screen::More | Screen::Log | Screen::Integrations | Screen::Settings => 4,
    }
}

/// Room Mint's chat list leaves under its last row for the floating tab bar.
pub const TAB_BAR_ROOM: f32 = 86.0;

/// Whether the tab bar floats over `screen` (Mint's Inbox) instead of
/// taking a strip of its own under it.
pub fn floats(screen: Screen) -> bool {
    theme::mint() && screen == Screen::Inbox
}

/// Beranda, Inbox, Status, Perangkat, Lainnya. Returns the screen picked.
pub fn tab_bar(ui: &mut Ui, screen: Screen, unread: u32, unseen: usize) -> Option<Screen> {
    let p = theme::palette(ui.ctx());
    let items = [
        (Screen::Ringkasan, "house", tr("Home", "Beranda"), 0),
        (Screen::Inbox, "message-circle", "Inbox", unread as usize),
        (Screen::Status, "circle-dashed", "Status", unseen),
        (Screen::Nomor, "smartphone", tr("Devices", "Perangkat"), 0),
        (Screen::More, "menu", tr("More", "Lainnya"), 0),
    ];
    let current = tab_of(screen);
    let mut picked = None;
    if theme::mint() {
        // Mint: a capsule with room around it; the tab it is on is a raised
        // tile around the icon and its label together.
        let line = theme::mint_pair(ui.ctx(), theme::MINT_SIDEBAR_LINE);
        let mut draw = |ui: &mut Ui, bar: Rect, floating: bool| {
                // Floating, it casts a soft shadow. It stays opaque: without a blur,
                // the rows passing behind would show through its labels.
                let fill = theme::sidebar_fill(ui.ctx());
                if floating {
                    ui.painter().add(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(28) }.as_shape(bar, bar.height() / 2.0));
                }
                ui.painter().rect(bar, bar.height() / 2.0, fill, Stroke::new(1.0, line), egui::StrokeKind::Inside);
                let inner = bar.shrink2(vec2(6.0, 0.0));
                let w = inner.width() / items.len() as f32;
                for (i, (target, icon, label, badge)) in items.into_iter().enumerate() {
                    let cell = Rect::from_min_size(pos2(inner.left() + i as f32 * w, inner.top()), vec2(w, inner.height()));
                    let response = ui.interact(cell, ui.scope_id().with(("m_tab", i)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                    let selected = i == current;
                    let tile = cell.shrink2(vec2(1.0, 5.0));
                    let field = theme::mint_pair(ui.ctx(), theme::MINT_SIDEBAR_SELECTED);
                    if selected {
                        ui.painter().rect(tile, tile.height() / 2.0, field, Stroke::new(1.0, line), egui::StrokeKind::Inside);
                    } else if response.hovered() {
                        ui.painter().rect_filled(tile, tile.height() / 2.0, field.gamma_multiply(0.5));
                    }
                    let color = if selected { p.primary } else { p.muted };
                    let at = pos2(cell.center().x, cell.center().y - 9.0);
                    icons::paint(ui, Rect::from_center_size(at, vec2(22.0, 22.0)), icon, 22.0, color);
                    if badge > 0 {
                        let text = if badge > 99 { "99+".to_owned() } else { badge.to_string() };
                        let bw = if badge > 9 { 24.0 } else { 17.0 };
                        let b = Rect::from_center_size(pos2(at.x + 13.0, at.y - 9.0), vec2(bw, 17.0));
                        ui.painter().rect_filled(b, 9.0, p.hl);
                        widgets::paint_text(ui, b, &text, fonts::ui(10.0, Weight::SemiBold), p.on_hl, Align::Center);
                    }
                    let text = Rect::from_center_size(pos2(cell.center().x, cell.center().y + 15.0), vec2(w, 14.0));
                    widgets::paint_text(ui, text, label, fonts::ui(10.5, if selected { Weight::SemiBold } else { Weight::Medium }), color, Align::Center);
                    if response.clicked() && !(selected && screen == target) {
                        picked = Some(target);
                    }
                }
        };
        if floats(screen) {
            // Over the Inbox the capsule floats: the chat list runs behind it
            // (and leaves `TAB_BAR_ROOM` under its last row).
            let screen_rect = ui.ctx().content_rect();
            let size = vec2(screen_rect.width() - 24.0, 62.0);
            egui::Area::new(egui::Id::unique("m_tabbar_float"))
                .order(egui::Order::Middle)
                .fixed_pos(pos2(screen_rect.left() + 12.0, screen_rect.bottom() - 12.0 - size.y))
                .show(ui.ctx(), |ui| {
                    let (bar, _) = ui.allocate_exact_size(size, Sense::hover());
                    draw(ui, bar, true);
                });
        } else {
            Panel::bottom("m_tabbar")
                .exact_size(82.0)
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::NONE.fill(p.bg))
                .show(ui, |ui| {
                    let bar = Rect::from_min_max(ui.max_rect().min + vec2(12.0, 8.0), ui.max_rect().max - vec2(12.0, 12.0));
                    draw(ui, bar, false);
                });
        }
        return picked;
    }
    Panel::bottom("m_tabbar")
        .exact_size(64.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE.fill(p.panel))
        .show(ui, |ui| {
            let bar = ui.max_rect();
            ui.painter().hline(bar.x_range(), bar.top() + 0.5, Stroke::new(1.0, p.border));
            let w = bar.width() / items.len() as f32;
            for (i, (target, icon, label, badge)) in items.into_iter().enumerate() {
                let cell = Rect::from_min_size(pos2(bar.left() + i as f32 * w, bar.top()), vec2(w, bar.height()));
                let response = ui.interact(cell, ui.scope_id().with(("m_tab", i)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                let selected = i == current;
                let pill = Rect::from_center_size(pos2(cell.center().x, cell.top() + 22.0), vec2(56.0, 30.0));
                // Forest: lime pill, green icon. Monokrom: black pill, white icon.
                let (pill_fill, icon_color) = match (selected, forest()) {
                    (true, true) => (Some(p.accent), p.primary),
                    (true, false) => (Some(p.primary), p.on_primary),
                    (false, _) => (response.hovered().then_some(p.raised), p.text_2),
                };
                if let Some(fill) = pill_fill {
                    ui.painter().rect_filled(pill, 15.0, fill);
                }
                icons::paint(ui, Rect::from_center_size(pill.center(), vec2(22.0, 22.0)), icon, 22.0, icon_color);
                if badge > 0 {
                    let text = if badge > 99 { "99+".to_owned() } else { badge.to_string() };
                    let bw = if badge > 9 { 24.0 } else { 17.0 };
                    let b = Rect::from_center_size(pos2(pill.center().x + 14.0, pill.top() + 3.0), vec2(bw, 17.0));
                    ui.painter().rect_filled(b.expand(1.5), 10.0, p.panel);
                    ui.painter().rect_filled(b, 9.0, if forest() { p.primary } else { p.red });
                    widgets::paint_text(ui, b, &text, fonts::ui(10.0, Weight::SemiBold), Color32::WHITE, Align::Center);
                }
                let (weight, color) = if selected {
                    (Weight::SemiBold, if forest() { p.primary } else { p.text })
                } else {
                    (Weight::Medium, p.text_2)
                };
                let text = Rect::from_center_size(pos2(cell.center().x, cell.top() + 49.0), vec2(w, 16.0));
                widgets::paint_text(ui, text, label, fonts::ui(11.0, weight), color, Align::Center);
                if response.clicked() && !(selected && screen == target) {
                    picked = Some(target);
                }
            }
        });
    picked
}

/// "‹ Lainnya   Log pesan" above a desktop screen shown in the phone layout.
/// Returns true when the back button is clicked.
pub fn back_bar(ui: &mut Ui, title: &str) -> bool {
    let p = theme::palette(ui.ctx());
    let mut back = false;
    Panel::top("m_backbar")
        .exact_size(52.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE.fill(theme::sidebar_fill(ui.ctx())).inner_margin(Margin { left: 8, right: 12, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let rect = ui.max_rect().expand2(vec2(8.0, 0.0));
            ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
            ui.horizontal_centered(|ui| {
                back = widgets::icon_button(ui, "chevron-left", 36.0, false).on_hover_text(tr("Back", "Kembali")).clicked();
                ui.label(rich(title, 16.0, Weight::SemiBold, p.text));
            });
        });
    back
}

/// Big screen title ("Ringkasan", "Perangkat"), with room on the right for
/// `actions` (laid out right to left).
pub fn page_title(ui: &mut Ui, title: &str, actions: impl FnOnce(&mut Ui)) {
    let p = theme::palette(ui.ctx());
    widgets::row(ui, 52.0, |ui| {
        ui.label(rich(title, 28.0, Weight::SemiBold, p.text));
    }, actions);
}

/// A round icon button with a white fill (search, refresh), as in the design.
pub fn round_button(ui: &mut Ui, icon: &str, primary: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    let (fill, color) = if primary { (p.primary, p.on_primary) } else if theme::mint() { (p.raised, p.text_2) } else { (p.panel, p.text) };
    ui.painter().circle_filled(rect.center(), 20.0, fill);
    if !primary && !forest() && !theme::mint() {
        ui.painter().circle_stroke(rect.center(), 19.5, Stroke::new(1.0, p.border));
    }
    if response.hovered() {
        ui.painter().circle_filled(rect.center(), 20.0, Color32::from_black_alpha(12));
    }
    icons::paint(ui, Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), icon, 18.0, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A white card (Forest) or an outlined one (Monokrom), full width.
pub fn card(ctx: &egui::Context) -> Frame {
    let p = theme::palette(ctx);
    let radius = if forest() { 20 } else { theme::r_card() };
    let frame = Frame::NONE.fill(p.panel).corner_radius(radius).inner_margin(Margin::symmetric(16, 4));
    if forest() { frame } else { frame.stroke(Stroke::new(1.0, p.border)) }
}

/// Paints the card of `card()` behind `rect` (for rows laid out by hand).
pub fn paint_card(ui: &Ui, rect: Rect) {
    let p = theme::palette(ui.ctx());
    let radius = if forest() { 20 } else { theme::r_card() };
    ui.painter().rect_filled(rect, radius, p.panel);
    if !forest() {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    }
}

/// Round avatar: the profile picture when cached, else initials on a soft
/// tile (lime in Forest when `accent`).
pub fn avatar(ui: &Ui, rect: Rect, initials: &str, picture: Option<&Path>, accent: bool) {
    let p = theme::palette(ui.ctx());
    let r = rect.width() / 2.0;
    if let Some(uri) = picture.and_then(|path| widgets::picture_uri(ui.ctx(), path)) {
        egui::Image::from_uri(uri)
            .corner_radius(CornerRadius::same(r as u8))
            .fit_to_exact_size(rect.size())
            .paint_at(ui, rect);
        return;
    }
    let (fill, color) = if accent && forest() { (p.accent_soft, p.accent_text) } else { (p.raised, p.text_2) };
    ui.painter().circle_filled(rect.center(), r, fill);
    if !forest() {
        ui.painter().circle_stroke(rect.center(), r - 0.5, Stroke::new(1.0, p.border));
    }
    widgets::paint_text(ui, rect, initials, fonts::ui(rect.height() * 0.34, Weight::SemiBold), color, Align::Center);
}

/// Label, soft fill and text colour of a number's state tag.
pub fn status_tag(p: &Palette, status: &Status) -> (&'static str, Color32, Color32) {
    let dark = p.bg == theme::DARK.bg;
    match status {
        Status::Connected if forest() => (tr("Connected", "Terhubung"), p.accent_soft, p.accent_text),
        Status::Connected => (tr("Connected", "Terhubung"), p.raised, p.text_2),
        Status::Starting => (tr("Starting", "Memulai"), p.raised, p.text_2),
        Status::Pairing => ("Pairing", if dark { p.raised } else { p.orange_soft }, p.amber),
        Status::Reconnecting | Status::Replaced | Status::Locked => (tr("Interrupted", "Tersendat"), if dark { p.raised } else { p.orange_soft }, p.amber),
        Status::LoggedOut | Status::Banned(_) | Status::Failed(_) => (tr("Disconnected", "Terputus"), if dark { p.raised } else { p.red_soft }, p.red_text),
    }
}

/// "● Terhubung" tag ending at `right`, centred on `y`.
fn paint_tag(ui: &Ui, p: &Palette, right: f32, y: f32, status: &Status) {
    let (label, fill, color) = status_tag(p, status);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(12.0, Weight::SemiBold), color);
    let w = galley.size().x + 28.0;
    let rect = Rect::from_min_size(pos2(right - w, y - 12.0), vec2(w, 24.0));
    ui.painter().rect_filled(rect, 12.0, fill);
    ui.painter().circle_filled(pos2(rect.left() + 11.0, rect.center().y), 3.0, super::screens::devices::status_dot(p, status));
    ui.painter().galley(pos2(rect.left() + 19.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// One number: icon tile, name, masked phone (or what it waits for) and its
/// state tag. `divider` draws the line under it.
pub fn device_row(ui: &mut Ui, session: &SessionView, divider: bool, main: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect.expand2(vec2(8.0, -4.0)), theme::r_inner(), p.raised);
    }
    let connected = session.status == Status::Connected;
    let tile = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(44.0, 44.0));
    if session.status == Status::Pairing {
        let fill = if connected && forest() { p.accent_soft } else { p.raised };
        ui.painter().circle_filled(tile.center(), 22.0, fill);
        icons::paint(ui, Rect::from_center_size(tile.center(), vec2(20.0, 20.0)), "qr-code", 20.0, p.text_2);
    } else {
        // The number's own colour and icon (#142).
        widgets::paint_number_tile(ui, tile, &session.id);
    }
    let left = tile.right() + 14.0;
    let right = rect.right() - 110.0;
    widgets::paint_number_name(ui, Rect::from_min_max(pos2(left, rect.top() + 14.0), pos2(right, rect.top() + 36.0)), &session.name, fonts::ui(16.0, Weight::SemiBold), p.text, main);
    let sub = match &session.phone {
        Some(phone) => phone.clone(),
        None if session.status == Status::Pairing => tr("Not scanned yet", "Belum dipindai").to_owned(),
        None => super::screens::nomor::describe(&p, &session.status).2,
    };
    widgets::paint_text(ui, Rect::from_min_max(pos2(left, rect.top() + 37.0), pos2(right, rect.top() + 56.0)), &sub, fonts::mono(13.0), p.muted, Align::Min);
    paint_tag(ui, &p, rect.right(), rect.center().y, &session.status);
    if divider {
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.border));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What Lainnya can open.
pub enum More {
    Open(Screen),
    ToggleTheme,
}

/// Lainnya: the screens that have no tab of their own.
pub fn more(ui: &mut Ui, api_on: bool, failed_hooks: usize) -> Option<More> {
    let p = theme::palette(ui.ctx());
    let mut picked = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        Frame::NONE.inner_margin(Margin { left: PAD as i8, right: PAD as i8, top: 12, bottom: 20 }).show(ui, |ui| {
            page_title(ui, tr("More", "Lainnya"), |_| {});
            ui.add_space(12.0);
            card(ui.ctx()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                let hooks = if failed_hooks > 0 {
                    crate::trf!("{failed_hooks} deliveries failed", "{failed_hooks} pengiriman gagal")
                } else {
                    tr("Endpoints, triggers and history", "Endpoint, pemicu dan riwayat").to_owned()
                };
                let rows: [(&str, &str, String, Screen); 3] = [
                    ("scroll-text", tr("Message log", "Log pesan"), tr("All outgoing and incoming messages", "Semua pesan keluar dan masuk").to_owned(), Screen::Log),
                    (
                        "webhook",
                        "Webhook & API",
                        format!("API {} · {hooks}", if api_on { tr("running", "berjalan") } else { tr("off", "mati") }),
                        Screen::Integrations,
                    ),
                    ("settings", tr("Settings", "Pengaturan"), tr("Appearance, send delay, stickers", "Tampilan, jeda kirim, stiker").to_owned(), Screen::Settings),
                ];
                for (i, (icon, title, desc, screen)) in rows.into_iter().enumerate() {
                    if menu_row(ui, &p, icon, title, &desc, i < 2).clicked() {
                        picked = Some(More::Open(screen));
                    }
                }
            });
            if theme::template().has_dark() {
                ui.add_space(12.0);
                card(ui.ctx()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let dark = ui.ctx().theme() == Theme::Dark;
                    let (icon, title) = if dark { ("sun", tr("Light theme", "Tema terang")) } else { ("moon", tr("Dark theme", "Tema gelap")) };
                    if menu_row(ui, &p, icon, title, tr("Saved for the next launch", "Disimpan untuk pembukaan berikutnya"), false).clicked() {
                        picked = Some(More::ToggleTheme);
                    }
                });
            }
            ui.add_space(16.0);
            ui.vertical_centered(|ui| ui.label(widgets::rich_mono("warust v0.1.0", 12.0, p.muted)));
        });
    });
    picked
}

fn menu_row(ui: &mut Ui, p: &Palette, icon: &str, title: &str, desc: &str, divider: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect.expand2(vec2(8.0, -4.0)), theme::r_inner(), p.raised);
    }
    let tile = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(40.0, 40.0));
    ui.painter().circle_filled(tile.center(), 20.0, p.raised);
    icons::paint(ui, Rect::from_center_size(tile.center(), vec2(18.0, 18.0)), icon, 18.0, p.text_2);
    let left = tile.right() + 14.0;
    let right = rect.right() - 28.0;
    widgets::paint_text(ui, Rect::from_min_max(pos2(left, rect.top() + 12.0), pos2(right, rect.top() + 32.0)), title, fonts::ui(15.0, Weight::SemiBold), p.text, Align::Min);
    widgets::paint_text(ui, Rect::from_min_max(pos2(left, rect.top() + 33.0), pos2(right, rect.top() + 52.0)), desc, fonts::ui(13.0, Weight::Regular), p.muted, Align::Min);
    icons::paint(ui, Rect::from_center_size(pos2(rect.right() - 10.0, rect.center().y), vec2(16.0, 16.0)), "chevron-right", 16.0, p.muted);
    if divider {
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.border));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
