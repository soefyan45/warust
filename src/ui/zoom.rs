//! Zoom shared by the photo and the PDF preview (#160): what the pointer
//! asked for (a pinch, Cmd/Ctrl+scroll, a double click), the limits, and
//! where the scroll area must stand so the point under the pointer stays
//! under it.

use egui::{PointerButton, Rect, Vec2};

/// What the pointer asked of a preview this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gesture {
    /// A pinch or Cmd/Ctrl+scroll: the zoom is multiplied by this.
    pub factor: f32,
    /// A double click (a double tap on a trackpad).
    pub double: bool,
    /// The pointer, measured from the area's top-left corner.
    pub anchor: Vec2,
}

/// The zoom gesture made over `area` this frame, if any. A trackpad pinch
/// arrives without a modifier, so none is asked for.
pub fn gesture(ui: &egui::Ui, area: Rect) -> Option<Gesture> {
    if !ui.rect_contains_pointer(area) {
        return None;
    }
    let (factor, double, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.button_double_clicked(PointerButton::Primary), i.pointer.latest_pos()));
    let anchor = pointer? - area.min;
    (factor != 1.0 || double).then_some(Gesture { factor, double, anchor })
}

/// `zoom` multiplied by a gesture's `factor`, kept within `min..=max`. A
/// factor that is not a positive number changes nothing.
pub fn scaled(zoom: f32, factor: f32, min: f32, max: f32) -> f32 {
    let factor = if factor.is_finite() && factor > 0.0 { factor } else { 1.0 };
    (zoom * factor).clamp(min, max)
}

/// The next of `steps` (ascending) above or below `zoom`, wherever a pinch
/// left it; the last or the first one when there is none further.
pub fn stepped(zoom: f32, steps: &[f32], up: bool) -> f32 {
    // Closer than this to a step counts as standing on it.
    const NEAR: f32 = 0.005;
    let next = if up { steps.iter().copied().find(|s| *s > zoom + NEAR) } else { steps.iter().rev().copied().find(|s| *s < zoom - NEAR) };
    let end = if up { steps.last() } else { steps.first() };
    next.or(end.copied()).unwrap_or(zoom)
}

/// A double click: in to `near` from the resting zoom, back to `rest` from
/// anywhere else.
pub fn toggled(zoom: f32, rest: f32, near: f32) -> f32 {
    if (zoom - rest).abs() > 0.01 { rest } else { near }
}

/// One axis of what a scroll area holds: the zoomed thing's `size`, and the
/// `pad` before it (and as much after it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub pad: f32,
    pub size: f32,
}

impl Span {
    /// Something centred in the viewport while it is smaller than it.
    pub fn centred(size: f32, viewport: f32) -> Self {
        Self { pad: ((viewport - size) / 2.0).max(0.0), size }
    }
}

/// The scroll offset on one axis after a zoom from `before` to `after` that
/// keeps the point under `anchor` (measured from the viewport's start) where
/// it is, as far as the content's ends allow. `offset` is the one before.
pub fn keep(anchor: f32, offset: f32, before: Span, after: Span, viewport: f32) -> f32 {
    if before.size <= 0.0 {
        return 0.0;
    }
    // How far into the zoomed thing the anchor points; its nearest edge when
    // the anchor is on the padding.
    let at = ((offset + anchor - before.pad) / before.size).clamp(0.0, 1.0);
    let most = (2.0 * after.pad + after.size - viewport).max(0.0);
    (after.pad + at * after.size - anchor).clamp(0.0, most)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, pos2, vec2};

    #[test]
    fn zoom_stays_within_its_limits() {
        assert_eq!(scaled(1.0, 1.5, 1.0, 6.0), 1.5);
        assert_eq!(scaled(5.0, 1.5, 1.0, 6.0), 6.0);
        assert_eq!(scaled(1.2, 0.5, 1.0, 6.0), 1.0);
        // A broken factor leaves the zoom alone.
        for bad in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            assert_eq!(scaled(2.0, bad, 1.0, 6.0), 2.0, "{bad}");
        }
    }

    #[test]
    fn buttons_step_from_wherever_a_pinch_left_the_zoom() {
        let steps = [0.5, 1.0, 2.0, 4.0];
        assert_eq!(stepped(1.0, &steps, true), 2.0);
        assert_eq!(stepped(1.0, &steps, false), 0.5);
        assert_eq!(stepped(1.37, &steps, true), 2.0);
        assert_eq!(stepped(1.37, &steps, false), 1.0);
        // A hair off a step is that step.
        assert_eq!(stepped(1.999, &steps, true), 4.0);
        assert_eq!(stepped(2.001, &steps, false), 1.0);
        // The ends hold.
        assert_eq!(stepped(4.0, &steps, true), 4.0);
        assert_eq!(stepped(0.5, &steps, false), 0.5);
        assert_eq!(stepped(1.0, &[], true), 1.0);
    }

    #[test]
    fn double_click_zooms_in_and_back() {
        assert_eq!(toggled(1.0, 1.0, 2.5), 2.5);
        assert_eq!(toggled(2.5, 1.0, 2.5), 1.0);
        assert_eq!(toggled(1.7, 1.0, 2.5), 1.0);
    }

    /// Where on the zoomed thing (0 to 1) the anchor points.
    fn pointed(anchor: f32, offset: f32, span: Span) -> f32 {
        (offset + anchor - span.pad) / span.size
    }

    #[test]
    fn the_point_under_the_pointer_stays_there() {
        let viewport = 800.0;
        // Bigger than the viewport before and after, scrolled a little.
        let (before, after) = (Span::centred(1000.0, viewport), Span::centred(1600.0, viewport));
        let offset = keep(600.0, 120.0, before, after, viewport);
        assert!((pointed(600.0, offset, after) - pointed(600.0, 120.0, before)).abs() < 1e-4, "{offset}");
        // Zooming out again lands on the same offset.
        assert!((keep(600.0, offset, after, before, viewport) - 120.0).abs() < 1e-3);
        // From fitted (centred, padded) to bigger than the viewport.
        let fitted = Span::centred(400.0, viewport);
        assert_eq!(fitted.pad, 200.0);
        let offset = keep(300.0, 0.0, fitted, after, viewport);
        assert!((pointed(300.0, offset, after) - 0.25).abs() < 1e-4, "{offset}");
    }

    #[test]
    fn the_offset_stops_at_the_content_ends() {
        let viewport = 800.0;
        let big = Span::centred(1600.0, viewport);
        // Still fitting: nothing to scroll.
        assert_eq!(keep(100.0, 0.0, Span::centred(300.0, viewport), Span::centred(600.0, viewport), viewport), 0.0);
        // Zooming at the far edge cannot scroll past the end,
        assert_eq!(keep(800.0, 200.0, Span::centred(1000.0, viewport), big, viewport), 800.0);
        // nor before the start.
        assert_eq!(keep(0.0, 0.0, Span::centred(1000.0, viewport), big, viewport), 0.0);
        // The pointer on the padding beside a small picture: its nearest edge.
        assert_eq!(keep(50.0, 0.0, Span::centred(400.0, viewport), big, viewport), 0.0);
        // Nothing to measure against.
        assert_eq!(keep(10.0, 30.0, Span { pad: 0.0, size: 0.0 }, big, viewport), 0.0);
    }

    /// The gesture seen over a 400 × 300 area at (100, 100) after `events`.
    fn seen(events: Vec<Event>) -> Option<Gesture> {
        let ctx = egui::Context::default();
        let area = Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 300.0));
        let mut seen = None;
        // The pointer arrives a frame before the gesture, as it does in use.
        for events in [vec![Event::PointerMoved(pos2(200.0, 250.0))], events] {
            let input = egui::RawInput { events, ..Default::default() };
            ctx.run_ui(input, |ui| seen = gesture(ui, area)).textures_delta.clear();
        }
        seen
    }

    #[test]
    fn a_pinch_needs_no_modifier() {
        // A trackpad pinch is a bare zoom event.
        let pinch = seen(vec![Event::Zoom(1.25)]).expect("a pinch over the area");
        assert_eq!(pinch, Gesture { factor: 1.25, double: false, anchor: vec2(100.0, 150.0) });
        // Nothing asked.
        assert_eq!(seen(vec![]), None);
        // The pointer elsewhere: the pinch is not for this preview.
        assert_eq!(seen(vec![Event::PointerMoved(pos2(20.0, 20.0)), Event::Zoom(1.25)]), None);
    }
}
