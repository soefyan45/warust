//! Small painted widgets shared by the screens, matching the Pego components
//! (`D/*` dark, `M/*` light).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Color32, CursorIcon, FontId, Frame, Layout, Margin, Rect, Response,
    RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::fonts::{self, Weight};
use super::icons;
use super::theme::{self, R_MD, R_SM};

/// A child Ui confined to `rect`. Unlike `with_layout`/`allocate_ui` it never
/// takes the rest of the parent's space or inherits its direction.
pub fn child(ui: &mut Ui, rect: Rect, layout: Layout) -> Ui {
    ui.new_child(UiBuilder::new().max_rect(rect).layout(layout))
}

/// One row of fixed height with content on the left and on the right. The
/// right side is laid out first; the left gets what is left, so long text
/// there truncates instead of running under the right side.
pub fn row(
    ui: &mut Ui,
    height: f32,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut right_ui = child(ui, rect, Layout::right_to_left(Align::Center));
    right(&mut right_ui);
    let used = right_ui.min_rect().width();
    let left_rect = Rect::from_min_max(rect.min, pos2((rect.right() - used - 12.0).max(rect.left()), rect.bottom()));
    left(&mut child(ui, left_rect, Layout::left_to_right(Align::Center)));
    rect
}

pub fn rich(text: &str, size: f32, weight: Weight, color: Color32) -> RichText {
    RichText::new(text).font(fonts::ui(size, weight)).color(color)
}

pub fn rich_mono(text: &str, size: f32, color: Color32) -> RichText {
    RichText::new(text).font(fonts::mono(size)).color(color)
}

/// Paints one line of text inside `rect`, cut with an ellipsis when too wide.
pub fn paint_text(ui: &Ui, rect: Rect, text: &str, font: FontId, color: Color32, align: Align) {
    let mut job = LayoutJob { break_on_newline: false, ..Default::default() };
    super::emoji::append(&mut job, text, egui::text::TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(rect.width().max(1.0));
    let galley = ui.painter().layout_job(job);
    let size = galley.size();
    let x = match align {
        Align::Min => rect.left(),
        Align::Center => rect.center().x - size.x / 2.0,
        Align::Max => rect.right() - size.x,
    };
    ui.painter()
        .galley(pos2(x, rect.center().y - size.y / 2.0), galley, color);
}

/// Splits a row into column rects. `None` widths share the space left.
pub fn columns(rect: Rect, widths: &[Option<f32>], gap: f32) -> Vec<Rect> {
    let fixed: f32 = widths.iter().flatten().sum();
    let flex_count = widths.iter().filter(|w| w.is_none()).count().max(1) as f32;
    let gaps = gap * (widths.len().saturating_sub(1)) as f32;
    let flex = ((rect.width() - fixed - gaps) / flex_count).max(0.0);
    let mut x = rect.left();
    widths
        .iter()
        .map(|w| {
            let width = w.unwrap_or(flex);
            let col = Rect::from_min_max(pos2(x, rect.top()), pos2(x + width, rect.bottom()));
            x += width + gap;
            col
        })
        .collect()
}

pub fn hline(ui: &Ui, rect: Rect, y: f32, color: Color32) {
    ui.painter()
        .hline(rect.x_range(), y, Stroke::new(1.0, color));
}

pub fn section_label(ui: &mut Ui, text: &str) {
    let p = theme::palette(ui.ctx());
    ui.label(rich(text, 12.0, Weight::Medium, p.muted));
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
}

pub fn button(ui: &mut Ui, icon: Option<&str>, label: &str, kind: Kind) -> Response {
    let p = theme::palette(ui.ctx());
    let (fg, fill, stroke, weight) = match kind {
        Kind::Primary => (p.on_primary, p.primary, Stroke::NONE, Weight::SemiBold),
        Kind::Secondary => (p.text, p.raised, Stroke::new(1.0, p.border_strong), Weight::Medium),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), fonts::ui(13.0, weight), fg);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 22.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() {
        fill.gamma_multiply(0.85)
    } else {
        fill
    };
    let painter = ui.painter();
    painter.rect(rect, theme::r_control(), fill, stroke, StrokeKind::Inside);
    let mut x = rect.left() + 11.0;
    if let Some(icon) = icon {
        let icon_rect = Rect::from_min_size(pos2(x, rect.center().y - 8.0), Vec2::splat(16.0));
        icons::paint(ui, icon_rect, icon, 16.0, fg);
        x += icon_w;
    }
    let text_pos = pos2(x, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, fg);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn icon_button(ui: &mut Ui, icon: &str, size: f32, filled: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if filled || response.hovered() {
        ui.painter().rect_filled(rect, R_SM, p.raised);
    }
    icons::paint(ui, rect, icon, 16.0, if filled { p.text } else { p.muted });
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn paint_status(ui: &Ui, rect: Rect, color: Color32, label: &str) {
    let p = theme::palette(ui.ctx());
    ui.painter()
        .circle_filled(pos2(rect.left() + 3.5, rect.center().y), 3.5, color);
    let text = Rect::from_min_max(pos2(rect.left() + 15.0, rect.top()), rect.max);
    paint_text(ui, text, label, fonts::ui(13.0, Weight::Regular), p.text_2, Align::Min);
}

pub fn nav_item(ui: &mut Ui, icon: &str, label: &str, count: Option<&str>, selected: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(rect, R_SM, p.raised);
    }
    let icon_rect = Rect::from_min_size(pos2(rect.left() + 10.0, rect.center().y - 8.0), Vec2::splat(16.0));
    icons::paint(ui, icon_rect, icon, 16.0, if selected { p.text } else { p.muted });
    let (weight, color) = if selected {
        (Weight::SemiBold, p.text)
    } else {
        (Weight::Medium, p.text_2)
    };
    let text = Rect::from_min_max(pos2(rect.left() + 36.0, rect.top()), pos2(rect.right() - 36.0, rect.bottom()));
    paint_text(ui, text, label, fonts::ui(14.0, weight), color, Align::Min);
    if let Some(count) = count {
        let r = Rect::from_min_max(pos2(rect.right() - 34.0, rect.top()), pos2(rect.right() - 10.0, rect.bottom()));
        paint_text(ui, r, count, fonts::ui(12.0, Weight::Medium), p.muted, Align::Max);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Underlined tab, like `D/Tab` and `D/Tab/Active`.
pub fn tab(ui: &mut Ui, label: &str, active: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let (weight, color) = if active {
        (Weight::SemiBold, p.text)
    } else {
        (Weight::Medium, p.muted)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), fonts::ui(13.0, weight), color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 4.0, 36.0), Sense::click());
    let color = if response.hovered() && !active { p.text_2 } else { color };
    ui.painter().galley(
        pos2(rect.left() + 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if active {
        let line = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.max);
        ui.painter().rect_filled(line, 0.0, p.text);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// One item of Forest's pill navigation and segmented controls.
pub fn pill(ui: &mut Ui, label: &str, active: bool) -> Response {
    pill_sized(ui, label, active, 14.0, 32.0, 34.0)
}

fn pill_sized(ui: &mut Ui, label: &str, active: bool, size: f32, pad: f32, height: f32) -> Response {
    let p = theme::palette(ui.ctx());
    let weight = if active { Weight::SemiBold } else { Weight::Medium };
    let color = if active { p.on_primary } else { p.text_2 };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(size, weight), color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + pad, height), Sense::click());
    if active {
        ui.painter().rect_filled(rect, height / 2.0, p.primary);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, height / 2.0, p.raised);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Pills in a white rounded track (Forest navigation).
pub fn pills(ui: &mut Ui, labels: &[&str], selected: &mut usize) -> bool {
    pill_track(ui, labels, selected, false)
}

fn pill_track(ui: &mut Ui, labels: &[&str], selected: &mut usize, compact: bool) -> bool {
    let p = theme::palette(ui.ctx());
    let mut changed = false;
    Frame::NONE
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(21)
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, label) in labels.iter().enumerate() {
                    let response = if compact {
                        pill_sized(ui, label, *selected == i, 13.0, 24.0, 30.0)
                    } else {
                        pill(ui, label, *selected == i)
                    };
                    if response.clicked() && *selected != i {
                        *selected = i;
                        changed = true;
                    }
                }
            });
        });
    changed
}

pub fn tabs(ui: &mut Ui, labels: &[&str], selected: &mut usize) {
    // Forest shows tabs as pills.
    if theme::template() == theme::Template::Forest {
        pill_track(ui, labels, selected, true);
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (i, label) in labels.iter().enumerate() {
            if tab(ui, label, *selected == i).clicked() {
                *selected = i;
            }
        }
    });
}

/// Search field with an icon and an optional `Ctrl K` hint.
/// Single-line input in the same style as `search`; `width` is the outer width.
pub fn field(ui: &mut Ui, text: &mut String, hint: &str, width: f32, mono: bool) -> Response {
    let p = theme::palette(ui.ctx());
    Frame::NONE
        .fill(p.raised)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::r_control())
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            let font = if mono { fonts::mono(13.0) } else { fonts::ui(13.0, Weight::Regular) };
            // Outer width minus margins (2 x 10) and stroke (2 x 1).
            ui.add(
                egui::TextEdit::singleline(text)
                    .frame(Frame::NONE)
                    .hint_text(rich(hint, 13.0, Weight::Regular, p.muted))
                    .font(font)
                    .desired_width((width - 22.0).max(20.0)),
            )
        })
        .inner
}

/// On/off switch followed by its label; clicking either flips `on`.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (rect, switch) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
            let t = ui.ctx().animate_bool(switch.id, *on);
            ui.painter().rect_filled(rect, 10.0, if *on { p.primary } else { p.border_strong });
            let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
            let knob = if *on { p.on_primary } else { p.text_2 };
            ui.painter().circle_filled(pos2(x, rect.center().y), 7.0, knob);
            let text = ui.add(egui::Label::new(rich(label, 13.0, Weight::Regular, p.text)).sense(Sense::click()));
            switch | text
        })
        .inner;
    if response.clicked() {
        *on = !*on;
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn search(ui: &mut Ui, text: &mut String, hint: &str, width: f32, kbd: bool) {
    let p = theme::palette(ui.ctx());
    Frame::NONE
        .fill(p.raised)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::r_control())
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            // Inner width = outer width minus margins (2 x 8) and stroke (2 x 1).
            // Overshooting makes a resizable panel grow every frame.
            let inner = width - 18.0;
            // Always left to right, even inside a right-to-left row.
            let ltr = Layout::left_to_right(Align::Center);
            ui.allocate_ui_with_layout(vec2(inner, 22.0), ltr, |ui| {
                ui.set_width(inner);
                icons::show(ui, "search", 15.0, p.muted);
                let edit = egui::TextEdit::singleline(text)
                    .frame(Frame::NONE)
                    .hint_text(rich(hint, 13.0, Weight::Regular, p.muted))
                    .font(fonts::ui(13.0, Weight::Regular))
                    .align(egui::Align2::LEFT_CENTER)
                    // Whatever is left after the icon, minus room for the Ctrl K hint.
                    .desired_width((ui.available_width() - if kbd { 54.0 } else { 0.0 }).max(0.0));
                ui.add(edit);
                if kbd {
                    Frame::NONE
                        .fill(p.border)
                        .corner_radius(4)
                        .inner_margin(Margin::symmetric(6, 2))
                        .show(ui, |ui| {
                            ui.label(rich("Ctrl K", 11.0, Weight::Medium, p.text_2));
                        });
                }
            });
        });
}

/// Rounded square with initials, like the chat and contact avatars.
pub fn avatar(ui: &mut Ui, initials: &str, picture: Option<&Path>, size: f32, filled: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_avatar(ui, rect, initials, picture, filled);
}

/// The profile picture when cached, else the initials tile.
pub fn paint_avatar(ui: &Ui, rect: Rect, initials: &str, picture: Option<&Path>, filled: bool) {
    let p = theme::palette(ui.ctx());
    if let Some(uri) = picture.and_then(|path| picture_uri(ui.ctx(), path)) {
        egui::Image::from_uri(uri)
            .corner_radius(R_MD)
            .fit_to_exact_size(rect.size())
            .paint_at(ui, rect);
        ui.painter()
            .rect_stroke(rect, R_MD, Stroke::new(1.0, p.border), StrokeKind::Inside);
        return;
    }
    ui.painter().rect(
        rect,
        R_MD,
        if filled { p.border } else { p.raised },
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    paint_text(
        ui,
        rect,
        initials,
        fonts::ui(rect.height() * 0.36, Weight::SemiBold),
        p.text_2,
        Align::Center,
    );
}

/// Hands a picture file to egui's image loader once; later frames use the URI.
pub fn picture_uri(ctx: &egui::Context, path: &Path) -> Option<String> {
    static LOADED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let uri = format!("bytes://picture/{}", path.display());
    let mut loaded = LOADED.get_or_init(Default::default).lock().ok()?;
    if !loaded.contains(&uri) {
        let bytes = std::fs::read(path).ok()?;
        ctx.include_bytes(uri.clone(), bytes);
        loaded.insert(uri.clone());
    }
    Some(uri)
}


/// The doodle wallpaper behind an open chat (Pego "Mobile · Chat wallpaper"),
/// repeated every 390 px so the doodles keep their size. Dark mode stays plain.
pub fn wallpaper(ui: &Ui, rect: Rect) {
    let p = theme::palette(ui.ctx());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0.0, p.chat_bg);
    let (uri, bytes): (&str, &'static [u8]) = match (theme::template(), ui.ctx().theme()) {
        (theme::Template::Forest, _) => ("bytes://wallpaper/forest.webp", include_bytes!("../../assets/wallpaper/forest.webp")),
        (_, egui::Theme::Light) => ("bytes://wallpaper/mono.webp", include_bytes!("../../assets/wallpaper/mono.webp")),
        _ => return,
    };
    // The source is 941 x 1672.
    let tile = vec2(390.0, 390.0 * 1672.0 / 941.0);
    let image = egui::Image::from_bytes(uri, bytes);
    let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(ui.ctx(), tile) else {
        return;
    };
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let mut y = rect.top();
    while y < rect.bottom() {
        let mut x = rect.left();
        while x < rect.right() {
            painter.image(texture.id, Rect::from_min_size(pos2(x, y), tile), uv, Color32::WHITE);
            x += tile.x;
        }
        y += tile.y;
    }
}
