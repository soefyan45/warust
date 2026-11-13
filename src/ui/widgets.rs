//! Small painted widgets shared by the screens, matching the Pego components
//! (`D/*` dark, `M/*` light).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
    // What does not fit on the left is cut there, never drawn over the right.
    let mut left_ui = child(ui, left_rect, Layout::left_to_right(Align::Center));
    left_ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
    left(&mut left_ui);
    rect
}

pub fn rich(text: &str, size: f32, weight: Weight, color: Color32) -> RichText {
    RichText::new(text).font(fonts::ui(size, weight)).color(color)
}

pub fn rich_mono(text: &str, size: f32, color: Color32) -> RichText {
    RichText::new(text).font(fonts::mono(size)).color(color)
}

/// The warust mark (Pego `Logo · Warust`): a chat bubble with its tail at the
/// bottom left and a stroked W, drawn in `rect` from the 96-unit artwork.
pub fn logo(ui: &Ui, rect: Rect, bubble: Color32, mark: Color32) {
    let unit = rect.width().min(rect.height()) / 96.0;
    let origin = rect.center() - vec2(48.0, 48.0) * unit;
    let at = |x: f32, y: f32| origin + vec2(x, y) * unit;
    let painter = ui.painter();
    // The body: a rounded box with a square bottom-left corner where the tail
    // starts. The tail overlaps the body a little so no seam shows.
    let corner = (16.0 * unit).round() as u8;
    let radius = egui::CornerRadius { nw: corner, ne: corner, sw: 0, se: corner };
    painter.rect_filled(Rect::from_min_max(at(14.0, 14.0), at(82.0, 74.0)), radius, bubble);
    // The tail ends in the curve C17 86 14 84 14 81 from (20, 84).
    let curve = |t: f32| {
        let s = 1.0 - t;
        let (p0, p1, p2, p3) = ((20.0, 84.0), (17.0, 86.0), (14.0, 84.0), (14.0, 81.0));
        let x = s * s * s * p0.0 + 3.0 * s * s * t * p1.0 + 3.0 * s * t * t * p2.0 + t * t * t * p3.0;
        let y = s * s * s * p0.1 + 3.0 * s * s * t * p1.1 + 3.0 * s * t * t * p2.1 + t * t * t * p3.1;
        at(x, y)
    };
    let mut tail = vec![at(14.0, 70.0), at(34.0, 70.0), at(34.0, 74.0)];
    tail.extend((0..=6).map(|i| curve(i as f32 / 6.0)));
    painter.add(egui::Shape::convex_polygon(tail, bubble, Stroke::NONE));
    // The W, with round caps and joins.
    let width = 8.5 * unit;
    let w = [at(28.0, 34.0), at(37.0, 58.0), at(48.0, 42.0), at(59.0, 58.0), at(68.0, 34.0)];
    painter.add(egui::Shape::line(w.to_vec(), Stroke::new(width, mark)));
    for point in w {
        painter.circle_filled(point, width / 2.0, mark);
    }
}

/// The texture of the latest video frame in `latest`, uploaded again only
/// when a new frame arrived; kept in egui's memory under `key`.
pub fn frame_texture(ctx: &egui::Context, key: &str, latest: &crate::backend::video::Latest) -> Option<egui::TextureHandle> {
    let picture = latest.lock().ok()?.clone()?;
    let id = egui::Id::NULL.with(("frame_texture", key));
    let cached: Option<(egui::TextureHandle, u64)> = ctx.data(|d| d.get_temp(id));
    if let Some((handle, generation)) = &cached {
        if *generation == picture.generation {
            return Some(handle.clone());
        }
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([picture.width, picture.height], &picture.rgba);
    let handle = match cached {
        Some((mut handle, _)) => {
            handle.set(image, egui::TextureOptions::LINEAR);
            handle
        }
        None => ctx.load_texture(key, image, egui::TextureOptions::LINEAR),
    };
    ctx.data_mut(|d| d.insert_temp(id, (handle.clone(), picture.generation)));
    Some(handle)
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
        let (fill, stroke) = theme::sidebar_selected(ui.ctx(), selected);
        ui.painter().rect(rect, theme::r_control(), fill, stroke, StrokeKind::Inside);
    }
    let icon_rect = Rect::from_min_size(pos2(rect.left() + 10.0, rect.center().y - 8.0), Vec2::splat(16.0));
    // Mint marks the page it is on with the highlight colour.
    let icon_color = if !selected { p.muted } else if theme::mint() { p.hl } else { p.text };
    icons::paint(ui, icon_rect, icon, 16.0, icon_color);
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

/// A page in the Classic sidebar's icon rail (#102): the icon in a square,
/// the label in the hint, a count as a small badge on its corner.
pub fn nav_icon(ui: &mut Ui, icon: &str, label: &str, count: Option<&str>, selected: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let tile = Rect::from_center_size(row.center(), Vec2::splat(40.0));
    if selected || response.hovered() {
        let (fill, stroke) = theme::sidebar_selected(ui.ctx(), selected);
        ui.painter().rect(tile, theme::r_control(), fill, stroke, StrokeKind::Inside);
    }
    let icon_color = if !selected { p.muted } else if theme::mint() { p.hl } else { p.text };
    icons::paint(ui, Rect::from_center_size(tile.center(), Vec2::splat(18.0)), icon, 18.0, icon_color);
    if let Some(count) = count {
        let galley = ui.painter().layout_no_wrap(count.to_owned(), fonts::ui(9.5, Weight::SemiBold), p.on_hl);
        let size = vec2((galley.size().x + 8.0).max(15.0), 15.0);
        let badge = Rect::from_min_size(pos2(tile.right() - size.x + 2.0, tile.top() - 1.0), size);
        ui.painter().rect_filled(badge, 7.5, p.hl);
        ui.painter().galley(badge.center() - galley.size() / 2.0, galley, p.on_hl);
    }
    let hint = match count {
        Some(count) => format!("{label} · {count}"),
        None => label.to_owned(),
    };
    response.on_hover_text(hint).on_hover_cursor(CursorIcon::PointingHand)
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
        ui.painter().rect_filled(line, 0.0, if theme::mint() { p.hl } else { p.text });
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

/// The Forest navigation: each pill an icon and its label, like the classic
/// sidebar; with `icons_only` the label moves to the hint.
pub fn icon_pills(ui: &mut Ui, items: &[(&str, &str)], selected: &mut usize, icons_only: bool) -> bool {
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
                for (i, (icon, label)) in items.iter().enumerate() {
                    let active = *selected == i;
                    let color = if active { p.on_primary } else { p.text_2 };
                    let weight = if active { Weight::SemiBold } else { Weight::Medium };
                    let galley = (!icons_only).then(|| ui.painter().layout_no_wrap((*label).to_owned(), fonts::ui(14.0, weight), color));
                    let width = match &galley {
                        Some(g) => 16.0 + 7.0 + g.size().x + 32.0,
                        None => 44.0,
                    };
                    let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
                    if active {
                        ui.painter().rect_filled(rect, 17.0, p.primary);
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 17.0, p.raised);
                    }
                    let icon_x = if galley.is_some() { rect.left() + 16.0 + 8.0 } else { rect.center().x };
                    icons::paint(ui, Rect::from_center_size(pos2(icon_x, rect.center().y), vec2(16.0, 16.0)), icon, 16.0, color);
                    if let Some(g) = galley {
                        ui.painter().galley(pos2(icon_x + 8.0 + 7.0, rect.center().y - g.size().y / 2.0), g, color);
                    }
                    let response = response.on_hover_cursor(CursorIcon::PointingHand);
                    let response = if icons_only { response.on_hover_text(*label) } else { response };
                    if response.clicked() && !active {
                        *selected = i;
                        changed = true;
                    }
                }
            });
        });
    changed
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
            ui.painter().rect_filled(rect, 10.0, if *on { p.hl } else { p.border_strong });
            let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
            let knob = if *on { p.on_hl } else { p.text_2 };
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

/// A search box. While it has text, an × at its end (or Esc) clears it and
/// keeps the focus in the box (#141).
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
                let room_for_clear = if text.is_empty() { 0.0 } else { 24.0 };
                let edit = egui::TextEdit::singleline(text)
                    .frame(Frame::NONE)
                    .hint_text(rich(hint, 13.0, Weight::Regular, p.muted))
                    .font(fonts::ui(13.0, Weight::Regular))
                    .align(egui::Align2::LEFT_CENTER)
                    // Whatever is left after the icon, minus room for the Ctrl K hint
                    // and the clear button.
                    .desired_width((ui.available_width() - if kbd { 54.0 } else { 0.0 } - room_for_clear).max(0.0));
                let field = ui.add(edit);
                // Esc takes the focus away from a text box; with text, it clears it.
                let esc = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
                if !text.is_empty() && (esc || clear_button(ui, &p)) {
                    text.clear();
                    field.request_focus();
                }
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


/// The round × at the end of a search box with text; true when clicked.
fn clear_button(ui: &mut Ui, p: &theme::Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    let fill = if response.hovered() { p.border_strong } else { p.border };
    ui.painter().circle_filled(rect.center(), 9.0, fill);
    icons::paint(ui, rect.shrink(5.0), "x", 10.0, p.text_2);
    response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(crate::i18n::tr("Clear search", "Hapus pencarian")).clicked()
}

/// The profile picture when cached, else the initials tile.
pub fn paint_avatar(ui: &Ui, rect: Rect, initials: &str, picture: Option<&Path>, filled: bool) {
    let p = theme::palette(ui.ctx());
    // A rounded square, or a circle in Mint.
    let radius = if theme::mint() { (rect.height() / 2.0).min(127.0) as u8 } else { R_MD };
    if let Some(uri) = picture.and_then(|path| picture_uri(ui.ctx(), path)) {
        egui::Image::from_uri(uri)
            .corner_radius(radius)
            .fit_to_exact_size(rect.size())
            .paint_at(ui, rect);
        ui.painter()
            .rect_stroke(rect, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
        return;
    }
    ui.painter().rect(
        rect,
        radius,
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

/// A picture file's size in pixels, read from its header once and remembered.
pub fn picture_size(path: &Path) -> Option<(u32, u32)> {
    static SIZES: OnceLock<Mutex<HashMap<PathBuf, (u32, u32)>>> = OnceLock::new();
    let mut sizes = SIZES.get_or_init(Default::default).lock().ok()?;
    if let Some(size) = sizes.get(path) {
        return Some(*size);
    }
    let size = image::image_dimensions(path).ok()?;
    sizes.insert(path.to_owned(), size);
    Some(size)
}


/// A number's name in `rect`, followed by a small "Main" tag when it is the
/// main number (#138); the name gives way ("…") so the tag still shows.
pub fn paint_number_name(ui: &Ui, rect: Rect, name: &str, font: egui::FontId, color: Color32, main: bool) {
    if !main {
        paint_text(ui, rect, name, font, color, Align::Min);
        return;
    }
    let p = theme::palette(ui.ctx());
    let tag = ui.painter().layout_no_wrap(crate::i18n::tr("Main", "Utama").to_owned(), fonts::ui(10.5, Weight::SemiBold), p.accent_text);
    let tag_w = tag.size().x + 12.0;
    let name_w = ui.painter().layout_no_wrap(name.to_owned(), font.clone(), color).size().x.min((rect.width() - tag_w - 6.0).max(0.0));
    paint_text(ui, Rect::from_min_size(rect.min, vec2(name_w, rect.height())), name, font, color, Align::Min);
    let pill = Rect::from_min_size(pos2(rect.left() + name_w + 6.0, rect.center().y - 8.0), vec2(tag_w, 16.0));
    ui.painter().rect_filled(pill, 8.0, p.accent_soft);
    ui.painter().galley(pill.center() - tag.size() / 2.0, tag, p.accent_text);
}

/// The colours a number can have (#142): mid tones that read on light and
/// dark backgrounds, as a tint on a soft fill of themselves.
pub const NUMBER_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x2E, 0x8B, 0x57),
    Color32::from_rgb(0x2F, 0x6F, 0xDB),
    Color32::from_rgb(0x7B, 0x4F, 0xD6),
    Color32::from_rgb(0xE0, 0x7B, 0x24),
    Color32::from_rgb(0xD6, 0x45, 0x7A),
    Color32::from_rgb(0x13, 0x8A, 0x8A),
    Color32::from_rgb(0xB8, 0x86, 0x0B),
    Color32::from_rgb(0x5B, 0x6B, 0x7A),
];

/// The icons a number can have (#142), the phone first (the default).
pub const NUMBER_ICONS: [&str; 8] = ["smartphone", "briefcase", "store", "headset", "house", "user", "star", "message-circle"];

fn number_looks() -> &'static Mutex<std::collections::HashMap<String, crate::backend::settings::NumberLook>> {
    static LOOKS: OnceLock<Mutex<std::collections::HashMap<String, crate::backend::settings::NumberLook>>> = OnceLock::new();
    LOOKS.get_or_init(Default::default)
}

/// Takes the numbers' colours and icons from the settings (cheap; called every frame).
pub fn set_number_looks(settings: &crate::backend::Settings) {
    if let Ok(mut looks) = number_looks().lock() {
        if *looks != settings.number_looks {
            *looks = settings.number_looks.clone();
        }
    }
}

/// A number's colour index: its own, else one picked from its id, so
/// numbers differ before anyone chooses.
pub fn number_color_index(id: &str) -> usize {
    let chosen = number_looks().lock().ok().and_then(|looks| looks.get(id).map(|l| l.color as usize));
    chosen.unwrap_or_else(|| id.bytes().fold(0usize, |sum, b| sum.wrapping_mul(31).wrapping_add(b as usize))) % NUMBER_COLORS.len()
}

/// A number's colour and icon (#142).
pub fn number_look(id: &str) -> (Color32, &'static str) {
    let icon = number_looks()
        .lock()
        .ok()
        .and_then(|looks| looks.get(id).and_then(|l| NUMBER_ICONS.iter().copied().find(|i| *i == l.icon)))
        .unwrap_or(NUMBER_ICONS[0]);
    (NUMBER_COLORS[number_color_index(id)], icon)
}

/// A number's round tile: its icon in its colour on a soft fill of it.
pub fn paint_number_tile(ui: &Ui, rect: Rect, id: &str) {
    let (color, icon) = number_look(id);
    let r = rect.width().min(rect.height()) / 2.0;
    ui.painter().circle_filled(rect.center(), r, color.gamma_multiply(0.16));
    let size = (r * 0.95).round();
    icons::paint(ui, Rect::from_center_size(rect.center(), vec2(size, size)), icon, size, color);
}

/// A designed chat wallpaper (Pego "Mobile · Chat wallpaper"): settings key,
/// names, the colour under it, and the image.
pub struct Wallpaper {
    pub key: &'static str,
    pub name: (&'static str, &'static str),
    pub background: Color32,
    pub uri: &'static str,
    pub bytes: &'static [u8],
}

/// The six designed wallpapers: the templates' own, then the ones users pick.
pub const WALLPAPERS: [Wallpaper; 6] = [
    Wallpaper { key: "forest", name: ("Forest", "Forest"), background: Color32::from_rgb(0xEC, 0xE8, 0xE2), uri: "bytes://wallpaper/forest.webp", bytes: include_bytes!("../../assets/wallpaper/forest.webp") },
    Wallpaper { key: "mono", name: ("Monochrome", "Monokrom"), background: Color32::from_rgb(0xFA, 0xFA, 0xFA), uri: "bytes://wallpaper/mono.webp", bytes: include_bytes!("../../assets/wallpaper/mono.webp") },
    Wallpaper { key: "dark", name: ("Dark", "Dark"), background: Color32::from_rgb(0x0B, 0x0B, 0x0C), uri: "bytes://wallpaper/dark.webp", bytes: include_bytes!("../../assets/wallpaper/dark.webp") },
    Wallpaper { key: "forest-dark", name: ("Forest dark", "Forest Gelap"), background: Color32::from_rgb(0x0F, 0x2A, 0x1F), uri: "bytes://wallpaper/forest-dark.webp", bytes: include_bytes!("../../assets/wallpaper/forest-dark.webp") },
    Wallpaper { key: "geometric", name: ("Geometric", "Geometri"), background: Color32::from_rgb(0xF3, 0xF1, 0xEA), uri: "bytes://wallpaper/geometric.webp", bytes: include_bytes!("../../assets/wallpaper/geometric.webp") },
    Wallpaper { key: "botanical", name: ("Botanical", "Botani"), background: Color32::from_rgb(0xE9, 0xF0, 0xE6), uri: "bytes://wallpaper/botanical.webp", bytes: include_bytes!("../../assets/wallpaper/botanical.webp") },
];

/// A chosen background: a wallpaper key, the custom picture, its dimming.
#[derive(Clone, Default, PartialEq)]
struct Choice {
    key: String,
    file: String,
    dim: u8,
}

/// The app's background and each number's own (#60), set from the settings
/// every frame.
#[derive(Clone, Default, PartialEq)]
struct Chosen {
    app: Choice,
    numbers: std::collections::HashMap<String, Choice>,
}

fn chosen() -> &'static Mutex<Chosen> {
    static CHOSEN: OnceLock<Mutex<Chosen>> = OnceLock::new();
    CHOSEN.get_or_init(Default::default)
}

/// Takes the wallpaper choices from the settings (cheap; called every frame).
pub fn set_wallpapers(settings: &crate::backend::Settings) {
    let app = Choice { key: settings.wallpaper.clone(), file: settings.wallpaper_file.clone(), dim: settings.wallpaper_dim };
    let numbers = settings.number_wallpapers.iter().map(|(id, w)| (id.clone(), Choice { key: w.key.clone(), file: w.file.clone(), dim: w.dim })).collect();
    let next = Chosen { app, numbers };
    if let Ok(mut current) = chosen().lock() {
        if *current != next {
            *current = next;
        }
    }
}

/// The designed wallpaper `auto` means: Forest for the Forest template,
/// Monochrome for Classic light, Dark for Classic dark.
pub fn auto_wallpaper(dark: bool) -> &'static Wallpaper {
    let key = match (theme::template(), dark) {
        (theme::Template::Forest, _) => "forest",
        (_, false) => "mono",
        (_, true) => "dark",
    };
    WALLPAPERS.iter().find(|w| w.key == key).unwrap_or(&WALLPAPERS[0])
}

/// The chat background behind an open chat of number `session`: its own
/// wallpaper (#60), else the one chosen in Settings (#56). Designed
/// wallpapers repeat every 390 px so the doodles keep their size; a custom
/// picture covers the thread, dimmed as chosen.
pub fn wallpaper(ui: &Ui, rect: Rect, session: &str) {
    let p = theme::palette(ui.ctx());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0.0, p.chat_bg);
    let choice = chosen().lock().map(|c| c.numbers.get(session).unwrap_or(&c.app).clone()).unwrap_or_default();
    let dark = ui.ctx().theme() == egui::Theme::Dark;
    match choice.key.as_str() {
        "none" => {}
        "custom" if !choice.file.is_empty() => {
            let Some(uri) = picture_uri(ui.ctx(), Path::new(&choice.file)) else { return };
            let Ok(egui::load::TexturePoll::Ready { texture }) = egui::Image::from_uri(uri).load_for_size(ui.ctx(), rect.size()) else {
                return;
            };
            // Cover: fill the thread, cropping what does not fit, centred.
            let size = texture.size;
            let scale = (rect.width() / size.x.max(1.0)).max(rect.height() / size.y.max(1.0));
            let shown = Rect::from_center_size(rect.center(), size * scale);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(texture.id, shown, uv, Color32::WHITE);
            let dim = f32::from(choice.dim.min(80)) / 100.0;
            if dim > 0.0 {
                painter.rect_filled(rect, 0.0, p.chat_bg.gamma_multiply(dim));
            }
        }
        // Mint's own backdrop is what `auto` means there; a picked wallpaper still wins.
        "auto" if theme::mint() => mint_backdrop(&painter, rect, dark),
        key => {
            let wall = WALLPAPERS.iter().find(|w| w.key == key).unwrap_or_else(|| auto_wallpaper(dark));
            paint_tiles(ui, &painter, rect, wall);
        }
    }
}

/// Mint's chat background: a soft green glow from the top right corner under
/// a faint dot grid. The grid stands still while the messages scroll.
fn mint_backdrop(painter: &egui::Painter, rect: Rect, dark: bool) {
    mint_glow(painter, rect, dark, 0.0);
    // The grid: a dot every 28 px.
    let dot = if dark { Color32::from_white_alpha(18) } else { Color32::from_black_alpha(20) };
    const STEP: f32 = 28.0;
    let clip = painter.clip_rect();
    let mut y = rect.top() + STEP / 2.0;
    while y < rect.bottom() {
        if y >= clip.top() - 2.0 && y <= clip.bottom() + 2.0 {
            let mut x = rect.left() + STEP / 2.0;
            while x < rect.right() {
                painter.circle_filled(pos2(x, y), 1.0, dot);
                x += STEP;
            }
        }
        y += STEP;
    }
}

/// The glow of Mint's chat background alone, over whatever `rect` is filled
/// with; `radius` rounds its corners, for a card.
pub fn mint_glow(painter: &egui::Painter, rect: Rect, dark: bool, radius: f32) {
    // The glow: an ellipse 75% of the width by 85% of the height around the
    // top right corner, strongest there. egui has no gradients, so it is a
    // mesh with a colour per vertex.
    let (glow, strength) = if dark { (theme::MINT_GLOW.1, 0.16) } else { (theme::MINT_GLOW.0, 1.0) };
    const CELLS: usize = 24;
    let mut mesh = egui::Mesh::default();
    for row in 0..=CELLS {
        for col in 0..=CELLS {
            let (u, v) = (col as f32 / CELLS as f32, row as f32 / CELLS as f32);
            let d = (((1.0 - u) / 0.75).powi(2) + (v / 0.85).powi(2)).sqrt();
            // 100% at the corner, 45% a third of the way out, gone at three quarters.
            let alpha = if d < 0.35 { egui::lerp(1.0..=0.45, d / 0.35) } else { egui::lerp(0.45..=0.0, ((d - 0.35) / 0.4).min(1.0)) };
            let at = pos2(rect.left() + u * rect.width(), rect.top() + v * rect.height());
            mesh.colored_vertex(rounded(rect, radius, at), glow.gamma_multiply(alpha * strength));
        }
    }
    let stride = (CELLS + 1) as u32;
    for row in 0..CELLS as u32 {
        for col in 0..CELLS as u32 {
            let i = row * stride + col;
            mesh.add_triangle(i, i + 1, i + stride);
            mesh.add_triangle(i + 1, i + stride + 1, i + stride);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// `at` pulled onto the nearest corner arc of `rect` rounded by `radius`,
/// when it lies in the part a rounded corner cuts off.
fn rounded(rect: Rect, radius: f32, at: egui::Pos2) -> egui::Pos2 {
    if radius <= 0.0 {
        return at;
    }
    let inner = rect.shrink(radius);
    let centre = pos2(at.x.clamp(inner.left(), inner.right()), at.y.clamp(inner.top(), inner.bottom()));
    let out = at - centre;
    if out.x != 0.0 && out.y != 0.0 && out.length() > radius { centre + out.normalized() * radius } else { at }
}

/// `wall` repeated every 390 px across `rect`.
fn paint_tiles(ui: &Ui, painter: &egui::Painter, rect: Rect, wall: &Wallpaper) {
    painter.rect_filled(rect, 0.0, wall.background);
    let image = egui::Image::from_bytes(wall.uri, wall.bytes);
    let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(ui.ctx(), vec2(390.0, 700.0)) else {
        return;
    };
    let tile = vec2(390.0, 390.0 * texture.size.y / texture.size.x.max(1.0));
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

/// A small preview of a designed wallpaper, for the picker in Settings.
pub fn paint_wallpaper_preview(ui: &Ui, rect: Rect, wall: &Wallpaper) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 8.0, wall.background);
    egui::Image::from_bytes(wall.uri, wall.bytes).corner_radius(8).fit_to_exact_size(rect.size()).paint_at(ui, rect);
}

#[cfg(test)]
mod number_look_tests {
    use super::*;

    #[test]
    fn a_number_without_a_choice_gets_a_steady_colour_and_the_phone() {
        let id = "s1794296306";
        assert_eq!(number_color_index(id), number_color_index(id));
        assert!(number_color_index(id) < NUMBER_COLORS.len());
        assert_eq!(number_look("s-unknown").1, "smartphone");
    }
}
