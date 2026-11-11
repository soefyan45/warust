//! Anti-peek mode (issue #57, Pego `v5/Privasi`): chat text stays frosted
//! until the cursor is on it, previews and (if chosen) names are hidden, and
//! the app locks after a while without input. Frosting draws rounded bars in
//! place of each line, so no text is on screen to read, and nothing is
//! computed from it but its line lengths.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use egui::{Color32, Galley, Painter, Pos2, Rect, pos2, vec2};

use crate::backend::settings::Privacy;

fn current() -> &'static Mutex<Privacy> {
    static CURRENT: OnceLock<Mutex<Privacy>> = OnceLock::new();
    CURRENT.get_or_init(|| Mutex::new(Privacy::default()))
}

/// Takes the anti-peek options from the settings (every frame).
pub fn set(privacy: &Privacy) {
    if let Ok(mut current) = current().lock() {
        if *current != *privacy {
            *current = privacy.clone();
        }
    }
}

fn get() -> Privacy {
    current().lock().map(|p| p.clone()).unwrap_or_default()
}

pub fn blur_chat() -> bool {
    get().blur_chat()
}

pub fn blur_previews() -> bool {
    get().blur_previews()
}

pub fn hide_names() -> bool {
    get().hide_names()
}

/// Rounded bars where each line of `galley` (drawn at `at`) would be.
pub fn frost_galley(painter: &Painter, at: Pos2, galley: &Galley, color: Color32) {
    for row in &galley.rows {
        let rect = row.rect().translate(at.to_vec2());
        if rect.width() < 1.0 {
            continue;
        }
        let bar = Rect::from_center_size(rect.center(), vec2(rect.width(), (rect.height() * 0.55).max(4.0)));
        painter.rect_filled(bar, bar.height() / 2.0, color);
    }
}

/// One bar filling most of `rect` vertically centred, for single lines
/// (names, previews) of a width that says nothing about the text.
pub fn frost_line(painter: &Painter, rect: Rect, fraction: f32, color: Color32) {
    let height = (rect.height() * 0.5).clamp(4.0, 10.0);
    let bar = Rect::from_min_size(pos2(rect.left(), rect.center().y - height / 2.0), vec2(rect.width() * fraction, height));
    painter.rect_filled(bar, height / 2.0, color);
}

/// When the app last saw input, for the auto-lock.
#[derive(Default)]
pub struct Lock {
    last_input: Option<Instant>,
    pub locked: bool,
    /// Locked because the camera saw someone else (#57).
    pub peeked: bool,
    /// The camera saw someone else last frame.
    crowded: bool,
}

impl Lock {
    /// Notes input, locks after the chosen idle time, and unlocks on a click
    /// or key. Returns how long until it would lock, to wake up then.
    pub fn update(&mut self, ctx: &egui::Context, privacy: &Privacy) -> Option<Duration> {
        let now = Instant::now();
        let input = ctx.input(|i| {
            i.pointer.is_moving() || i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)))
        });
        if self.locked {
            let unlock = ctx.input(|i| i.pointer.any_click() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
            if unlock || !privacy.enabled {
                self.locked = false;
                self.peeked = false;
                self.last_input = Some(now);
            }
            return None;
        }
        if input || self.last_input.is_none() {
            self.last_input = Some(now);
        }
        if !privacy.enabled || privacy.lock_minutes == 0 {
            return None;
        }
        let after = Duration::from_secs(u64::from(privacy.lock_minutes) * 60);
        let idle = self.last_input.map(|t| now - t).unwrap_or_default();
        if idle >= after {
            self.locked = true;
            return None;
        }
        Some(after - idle)
    }

    /// Locks when the camera starts to see someone else. Once dismissed it
    /// waits for them to leave and come back, so a click is not undone.
    pub fn watch(&mut self, crowded: bool) {
        if crowded && !self.crowded && !self.locked {
            self.locked = true;
            self.peeked = true;
        }
        self.crowded = crowded;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_need_the_mode_on() {
        let off = Privacy { enabled: false, blur_chat: true, blur_previews: true, hide_names: true, lock_minutes: 2, watch_behind: true };
        assert!(!off.blur_chat() && !off.blur_previews() && !off.hide_names() && !off.watch_behind());
        let on = Privacy { enabled: true, ..off };
        assert!(on.blur_chat() && on.blur_previews() && on.hide_names() && on.watch_behind());
    }
}
