//! Own window chrome. The OS title bar is turned off (`with_decorations(false)`)
//! so there is one bar only: the app's topbar carries the window controls,
//! moves the window by drag and maximizes on double click.

use egui::{
    Context, CursorIcon, Id, LayerId, Order, Rect, ResizeDirection, Sense, Stroke, StrokeKind,
    Ui, ViewportCommand, pos2, vec2,
};

use super::theme;

const EDGE: f32 = 5.0;

pub fn is_maximized(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn toggle_maximized(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized(ctx)));
}

/// Makes `rect` a title-bar area. Call it before adding the bar's widgets so
/// buttons added afterwards stay on top and keep their clicks.
pub fn drag_area(ui: &Ui, rect: Rect, id: &str) {
    let response = ui.interact(rect, ui.scope_id().with(("window_drag", id)), Sense::click_and_drag());
    if response.double_clicked() {
        toggle_maximized(ui.ctx());
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

#[derive(Clone, Copy)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

/// Minimize, maximize/restore and close, for a right-to-left row: close ends
/// up rightmost.
pub fn controls(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.spacing_mut().item_spacing.x = 2.0;
    if button(ui, Control::Close).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    if button(ui, Control::Maximize).clicked() {
        toggle_maximized(&ctx);
    }
    if button(ui, Control::Minimize).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn button(ui: &mut Ui, control: Control) -> egui::Response {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 28.0), Sense::click());
    let hovered = response.hovered();
    let close = matches!(control, Control::Close);
    if hovered {
        let fill = if close { p.red } else { p.raised };
        ui.painter().rect_filled(rect, theme::R_SM, fill);
    }
    let color = if hovered && close { egui::Color32::WHITE } else { p.text_2 };
    let stroke = Stroke::new(1.2, color);
    let c = rect.center();
    let painter = ui.painter();
    match control {
        Control::Minimize => {
            painter.hline(c.x - 5.0..=c.x + 5.0, c.y, stroke);
        }
        Control::Maximize if is_maximized(ui.ctx()) => {
            // Restore: two overlapping squares.
            let back = Rect::from_center_size(c + vec2(1.5, -1.5), vec2(8.0, 8.0));
            let front = Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0));
            painter.rect_stroke(back, 1.0, stroke, StrokeKind::Middle);
            painter.rect_filled(front, 1.0, if hovered { p.raised } else { p.bg });
            painter.rect_stroke(front, 1.0, stroke, StrokeKind::Middle);
        }
        Control::Maximize => {
            let square = Rect::from_center_size(c, vec2(10.0, 10.0));
            painter.rect_stroke(square, 1.0, stroke, StrokeKind::Middle);
        }
        Control::Close => {
            painter.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], stroke);
            painter.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], stroke);
        }
    }
    let tip = match control {
        Control::Minimize => "Kecilkan",
        Control::Maximize if is_maximized(ui.ctx()) => "Pulihkan",
        Control::Maximize => "Perbesar",
        Control::Close => "Tutup",
    };
    response.on_hover_text(tip)
}

/// Thin areas along the window edges that start an OS resize, plus a 1px
/// border so the undecorated window stays visible on any background.
pub fn edges(ui: &Ui) {
    let ctx = ui.ctx();
    let window = ui.max_rect();
    if !is_maximized(ctx) {
        let (l, r, t, b) = (window.left(), window.right(), window.top(), window.bottom());
        let zones = [
            (Rect::from_min_max(pos2(l, t), pos2(l + EDGE, t + EDGE)), ResizeDirection::NorthWest, CursorIcon::ResizeNwSe),
            (Rect::from_min_max(pos2(r - EDGE, t), pos2(r, t + EDGE)), ResizeDirection::NorthEast, CursorIcon::ResizeNeSw),
            (Rect::from_min_max(pos2(l, b - EDGE), pos2(l + EDGE, b)), ResizeDirection::SouthWest, CursorIcon::ResizeNeSw),
            (Rect::from_min_max(pos2(r - EDGE, b - EDGE), pos2(r, b)), ResizeDirection::SouthEast, CursorIcon::ResizeNwSe),
            (Rect::from_min_max(pos2(l + EDGE, t), pos2(r - EDGE, t + EDGE)), ResizeDirection::North, CursorIcon::ResizeVertical),
            (Rect::from_min_max(pos2(l + EDGE, b - EDGE), pos2(r - EDGE, b)), ResizeDirection::South, CursorIcon::ResizeVertical),
            (Rect::from_min_max(pos2(l, t + EDGE), pos2(l + EDGE, b - EDGE)), ResizeDirection::West, CursorIcon::ResizeHorizontal),
            (Rect::from_min_max(pos2(r - EDGE, t + EDGE), pos2(r, b - EDGE)), ResizeDirection::East, CursorIcon::ResizeHorizontal),
        ];
        for (i, (rect, direction, cursor)) in zones.into_iter().enumerate() {
            let response = ui
                .interact(rect, ui.scope_id().with(("window_edge", i)), Sense::drag())
                .on_hover_cursor(cursor);
            if response.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
            }
        }
        let p = theme::palette(ctx);
        ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("window_border")))
            .rect_stroke(window, 0.0, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    }
}
