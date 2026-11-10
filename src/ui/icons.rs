//! Embedded lucide icons (ISC, assets/icons/LICENSE.txt). The SVGs stroke
//! with `currentColor`; it is turned white once so `Image::tint` can color them.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::load::Bytes;
use egui::{Color32, Image, Rect, Ui, Vec2};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../assets/icons/", $name, ".svg")) as &[u8])),*]
    };
}

const ICONS: &[(&str, &[u8])] = icons![
    "smartphone", "inbox", "scroll-text", "webhook", "settings", "search", "plus",
    "book-open", "chevrons-up-down", "chevron-down", "chevron-right", "bell", "archive",
    "ellipsis", "unlink", "gauge", "arrow-up-right", "arrow-down-left", "x", "check",
    "panel-right", "paperclip", "image", "arrow-up", "download", "copy", "terminal",
    "rotate-cw", "triangle-alert", "qr-code", "loader-circle", "sun", "moon", "smile",
    "sticker", "file-text", "gif", "chevron-left", "video", "eye", "lock", "panel-left", "house", "message-circle",
    "circle-dashed", "menu", "ellipsis-vertical", "send", "arrow-right", "layers", "columns-2", "heart",
    "pause", "play", "clock", "check-check", "circle-alert", "mic",
];

fn white_svgs() -> &'static HashMap<&'static str, Bytes> {
    static CACHE: OnceLock<HashMap<&'static str, Bytes>> = OnceLock::new();
    CACHE.get_or_init(|| {
        ICONS
            .iter()
            .map(|(name, svg)| {
                let white = String::from_utf8_lossy(svg).replace("currentColor", "#ffffff");
                (*name, Bytes::Shared(Arc::from(white.into_bytes())))
            })
            .collect()
    })
}

fn image(name: &str, size: f32, color: Color32) -> Image<'static> {
    let bytes = white_svgs()
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("unknown icon {name}"));
    Image::from_bytes(format!("bytes://icon/{name}.svg"), bytes)
        .fit_to_exact_size(Vec2::splat(size))
        .tint(color)
}

/// Adds an icon as a widget in the current layout.
pub fn show(ui: &mut Ui, name: &str, size: f32, color: Color32) {
    ui.add(image(name, size, color));
}

/// Paints an icon centered in `rect` without taking layout space.
pub fn paint(ui: &Ui, rect: Rect, name: &str, size: f32, color: Color32) {
    let at = Rect::from_center_size(rect.center(), Vec2::splat(size));
    image(name, size, color).paint_at(ui, at);
}
