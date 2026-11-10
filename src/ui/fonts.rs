//! Embedded fonts: Inter for the UI, JetBrains Mono for numbers, phone
//! numbers and IDs, Noto Color Emoji (OFL) for emoji in colour, as on phones.
//! egui has no font weights, so each weight is a family.

use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily, FontId};

const MEDIUM: &str = "inter-medium";
const SEMIBOLD: &str = "inter-semibold";
const EMOJI: &str = "noto-color-emoji";
/// Emoji font first: for runs that are emoji, so characters Inter also has
/// (❤, digits in keycaps) still come out as colour emoji.
const EMOJI_FIRST: &str = "emoji-first";

#[derive(Clone, Copy)]
pub enum Weight {
    Regular,
    Medium,
    SemiBold,
}

pub fn install(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("inter", include_bytes!("../../assets/fonts/Inter-Regular.ttf"));
    add(MEDIUM, include_bytes!("../../assets/fonts/Inter-Medium.ttf"));
    add(SEMIBOLD, include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"));
    add("mono", include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"));
    // Drawn as colour bitmaps through epaint's `color_fonts` feature.
    add(EMOJI, include_bytes!("../../assets/fonts/NotoColorEmoji.ttf"));

    // Emoji right after our fonts, then egui's defaults (symbols) as fallbacks.
    let defaults = fonts.families[&FontFamily::Proportional].clone();
    let with_fallback = |primary: &str| {
        let mut list = vec![primary.to_owned(), EMOJI.to_owned()];
        list.extend(defaults.iter().cloned());
        list
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with_fallback("inter"));
    fonts
        .families
        .insert(FontFamily::Name(MEDIUM.into()), with_fallback(MEDIUM));
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), with_fallback(SEMIBOLD));
    let mut emoji_first = vec![EMOJI.to_owned(), "inter".to_owned()];
    emoji_first.extend(defaults.iter().cloned());
    fonts.families.insert(FontFamily::Name(EMOJI_FIRST.into()), emoji_first);
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .splice(0..0, ["mono".to_owned(), EMOJI.to_owned()]);
    ctx.set_fonts(fonts);
}

pub fn ui(size: f32, weight: Weight) -> FontId {
    let family = match weight {
        Weight::Regular => FontFamily::Proportional,
        Weight::Medium => FontFamily::Name(MEDIUM.into()),
        Weight::SemiBold => FontFamily::Name(SEMIBOLD.into()),
    };
    FontId::new(size, family)
}

/// The family for a run of emoji; see `emoji::append`.
pub fn emoji_family() -> FontFamily {
    FontFamily::Name(EMOJI_FIRST.into())
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}
