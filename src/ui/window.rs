//! Own window chrome. The OS title bar is turned off (`with_decorations(false)`)
//! so there is one bar only: the app's topbar carries the window controls,
//! moves the window by drag and maximizes on double click.
//!
//! macOS keeps its own chrome instead (`NATIVE`): a transparent title bar under
//! the app's topbar, with the system's window buttons moved into that bar
//! (`place_buttons`), and the system's corners, shadow and resize edges.

use egui::{
    Context, CursorIcon, Id, LayerId, Order, Rect, ResizeDirection, Sense, Stroke, StrokeKind,
    Ui, ViewportCommand, pos2, vec2,
};

use super::theme;
use crate::i18n::tr;

const EDGE: f32 = 5.0;

/// The OS draws the window buttons, edges and border (macOS).
pub const NATIVE: bool = cfg!(target_os = "macos");

/// Left edge of the macOS window buttons, in points from the window's left.
#[cfg(target_os = "macos")]
const BUTTONS_X: f64 = 18.0;

/// Where the content of the bar holding the macOS window buttons may start,
/// from the window's left edge: past the buttons; 0 in full screen (the
/// buttons hide) and on other systems.
pub fn buttons_inset(ctx: &Context) -> f32 {
    if NATIVE && !is_fullscreen(ctx) { 92.0 } else { 0.0 }
}

/// Whether the window is in full screen, where AppKit moves the title bar
/// into a window of its own and the buttons must be left to it.
pub fn is_fullscreen(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
}

/// Centres the macOS window buttons in the app's top bar (`bar` points tall)
/// and moves them to `BUTTONS_X`. AppKit lays them out again on resize and
/// full screen, so this runs every frame and only writes what changed. Not
/// called in full screen (see `is_fullscreen`).
#[cfg(target_os = "macos")]
pub fn place_buttons(frame: &eframe::Frame, bar: f32) {
    use objc2_app_kit::{NSView, NSWindowButton};
    use objc2_foundation::NSRect;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's content view, alive as long as the window; eframe calls
    // `App::ui` on the main thread, where AppKit objects may be used.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let Some(window) = view.window() else {
        return;
    };
    let buttons = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton]
        .map(|kind| window.standardWindowButton(kind));
    let [Some(close), Some(minimize), Some(zoom)] = buttons else {
        return;
    };
    // close → NSTitlebarView → NSTitlebarContainerView.
    // SAFETY: plain view-hierarchy getters on the main thread.
    let Some(titlebar) = (unsafe { close.superview() }) else {
        return;
    };
    let Some(container) = (unsafe { titlebar.superview() }) else {
        return;
    };
    let near = |a: f64, b: f64| (a - b).abs() < 0.5;
    let bar = f64::from(bar);
    let window_height = window.frame().size.height;
    let frame = container.frame();
    if !near(frame.size.height, bar) || !near(frame.origin.y, window_height - bar) {
        let mut frame = frame;
        frame.size.height = bar;
        frame.origin.y = window_height - bar;
        container.setFrame(frame);
    }
    let inner = titlebar.frame();
    if !near(inner.size.height, bar) || !near(inner.origin.y, 0.0) {
        titlebar.setFrame(NSRect::new(objc2_foundation::NSPoint::new(inner.origin.x, 0.0), objc2_foundation::NSSize::new(inner.size.width, bar)));
    }
    let step = minimize.frame().origin.x - close.frame().origin.x;
    for (i, button) in [close, minimize, zoom].iter().enumerate() {
        let frame = button.frame();
        // Centred either way up, so the view's flippedness does not matter.
        let (x, y) = (BUTTONS_X + i as f64 * step, ((bar - frame.size.height) / 2.0).round());
        if !near(frame.origin.x, x) || !near(frame.origin.y, y) {
            button.setFrameOrigin(objc2_foundation::NSPoint::new(x, y));
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn place_buttons(_frame: &eframe::Frame, _bar: f32) {}

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
    if NATIVE {
        return;
    }
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
        Control::Minimize => tr("Minimize", "Kecilkan"),
        Control::Maximize if is_maximized(ui.ctx()) => tr("Restore", "Pulihkan"),
        Control::Maximize => tr("Maximize", "Perbesar"),
        Control::Close => tr("Close", "Tutup"),
    };
    response.on_hover_text(tip)
}

/// Thin areas along the window edges that start an OS resize, plus a 1px
/// border so the undecorated window stays visible on any background.
pub fn edges(ui: &Ui) {
    if NATIVE {
        return;
    }
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
