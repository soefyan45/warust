//! Color tokens and page templates from the Pego design (project `warust`).
//! Classic: v4 dark (`d-*`) and v3 light (`m-*`), sidebar layout. Forest: v5
//! (`f-*`, page "App v5 · Forest"), top navigation and white cards on a warm
//! background; light only, as designed. Mint ("Warust Mint", v6): Classic's
//! sidebar layout on neutral greys with a green highlight, dark and light.

use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Color32, Context, CornerRadius, Frame, Margin, Stroke, Theme, ThemePreference, Visuals};

use crate::i18n::tr;

/// A page template: palette, navigation and surfaces. Data and screens are
/// the same in every template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Classic,
    Forest,
    Mint,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::Classic, Template::Forest, Template::Mint];

    /// Name in `settings.json`.
    pub fn key(self) -> &'static str {
        match self {
            Template::Classic => "classic",
            Template::Forest => "forest",
            Template::Mint => "mint",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL.into_iter().find(|t| t.key() == key).unwrap_or(Template::Classic)
    }

    pub fn name(self) -> &'static str {
        match self {
            Template::Classic => tr("Classic (v4)", "Klasik (v4)"),
            Template::Forest => "Forest (v5)",
            Template::Mint => "Warust Mint (v6)",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Template::Classic => tr("Left sidebar, monochrome, dark and light mode.", "Sidebar kiri, monokrom, mode gelap dan terang."),
            Template::Forest => tr("Top navigation, white cards, green-lime accent. Light only.", "Navigasi atas, kartu putih, aksen hijau-lime. Terang saja."),
            Template::Mint => tr("Left sidebar, neutral greys, green highlight. Dark and light mode.", "Sidebar kiri, abu netral, aksen hijau. Mode gelap dan terang."),
        }
    }

    /// Whether the dark/light toggle applies.
    pub fn has_dark(self) -> bool {
        self != Template::Forest
    }

    /// Navigation along the top instead of the sidebar.
    pub fn top_nav(self) -> bool {
        self == Template::Forest
    }
}

static ACTIVE: AtomicU8 = AtomicU8::new(0);

pub fn template() -> Template {
    match ACTIVE.load(Ordering::Relaxed) {
        1 => Template::Forest,
        2 => Template::Mint,
        _ => Template::Classic,
    }
}

/// Switches the template; Forest also switches to light.
pub fn set_template(ctx: &Context, template: Template) {
    ACTIVE.store(template as u8, Ordering::Relaxed);
    let (light, dark) = match template {
        Template::Classic => (&LIGHT, &DARK),
        Template::Forest => (&FOREST, &DARK),
        Template::Mint => (&MINT_LIGHT, &MINT_DARK),
    };
    ctx.set_visuals_of(Theme::Light, visuals(Visuals::light(), light));
    ctx.set_visuals_of(Theme::Dark, visuals(Visuals::dark(), dark));
    if !template.has_dark() {
        ctx.set_theme(ThemePreference::Light);
    }
    ctx.request_repaint();
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_2: Color32,
    pub muted: Color32,
    pub primary: Color32,
    pub on_primary: Color32,
    pub on_primary_muted: Color32,
    pub green: Color32,
    pub amber: Color32,
    pub red: Color32,
    pub red_text: Color32,
    pub gray: Color32,
    pub bar: Color32,
    pub bar_hi: Color32,
    /// Highlight (Forest lime) and its soft fill and text colour.
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub blue: Color32,
    pub blue_soft: Color32,
    pub orange_soft: Color32,
    pub red_soft: Color32,
    /// Selected list row.
    pub selected: Color32,
    /// Behind the bubbles of an open chat, and incoming bubbles.
    pub chat_bg: Color32,
    pub bubble_in: Color32,
    /// Marker colour (unread badges, the active tab's line, switches) and what
    /// sits on it. The same as `primary` in Classic and Forest; green in Mint,
    /// where `primary` stays neutral.
    pub hl: Color32,
    pub on_hl: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const DARK: Palette = Palette {
    bg: hex(0x0B0B0C),
    panel: hex(0x111113),
    raised: hex(0x19191C),
    border: hex(0x26262A),
    border_strong: hex(0x3A3A40),
    text: hex(0xEDEDEF),
    text_2: hex(0xC4C4CA),
    muted: hex(0x8E8E96),
    primary: hex(0xEDEDEF),
    on_primary: hex(0x0B0B0C),
    on_primary_muted: hex(0x55555C),
    green: hex(0x22C55E),
    amber: hex(0xF59E0B),
    red: hex(0xEF4444),
    red_text: hex(0xF87171),
    gray: hex(0x71717A),
    bar: hex(0x3A3A40),
    bar_hi: hex(0xEDEDEF),
    accent: hex(0xEDEDEF),
    accent_soft: hex(0x19191C),
    accent_text: hex(0xEDEDEF),
    blue: hex(0xC4C4CA),
    blue_soft: hex(0x19191C),
    orange_soft: hex(0x19191C),
    red_soft: hex(0x19191C),
    selected: hex(0x19191C),
    chat_bg: hex(0x0B0B0C),
    bubble_in: hex(0x19191C),
    hl: hex(0xEDEDEF),
    on_hl: hex(0x0B0B0C),
};

pub const LIGHT: Palette = Palette {
    bg: hex(0xFFFFFF),
    panel: hex(0xFAFAFA),
    raised: hex(0xF4F4F5),
    border: hex(0xEBEBEB),
    border_strong: hex(0xD4D4D8),
    text: hex(0x0A0A0A),
    text_2: hex(0x3F3F46),
    muted: hex(0x66666E),
    primary: hex(0x0A0A0A),
    on_primary: hex(0xFFFFFF),
    on_primary_muted: hex(0xD4D4D8),
    green: hex(0x16A34A),
    amber: hex(0xD97706),
    red: hex(0xDC2626),
    red_text: hex(0xB91C1C),
    gray: hex(0xA1A1AA),
    bar: hex(0xD4D4D8),
    bar_hi: hex(0x0A0A0A),
    accent: hex(0x0A0A0A),
    accent_soft: hex(0xF4F4F5),
    accent_text: hex(0x0A0A0A),
    blue: hex(0x3F3F46),
    blue_soft: hex(0xF4F4F5),
    orange_soft: hex(0xF4F4F5),
    red_soft: hex(0xF4F4F5),
    selected: hex(0xF4F4F5),
    chat_bg: hex(0xFFFFFF),
    bubble_in: hex(0xF4F4F5),
    hl: hex(0x0A0A0A),
    on_hl: hex(0xFFFFFF),
};

/// v5 "Forest" (`f-*` variables).
pub const FOREST: Palette = Palette {
    bg: hex(0xECE8E2),
    panel: hex(0xFFFFFF),
    raised: hex(0xF5F3EF),
    border: hex(0xE7E2DA),
    border_strong: hex(0xC9C4B5),
    text: hex(0x1D211F),
    text_2: hex(0x3E4440),
    muted: hex(0x5A5F5A),
    primary: hex(0x1D5A3F),
    on_primary: hex(0xFFFFFF),
    on_primary_muted: hex(0xB5CFC0),
    green: hex(0x1D5A3F),
    amber: hex(0x9E4317),
    red: hex(0xB42318),
    red_text: hex(0xB42318),
    gray: hex(0xC9C4B5),
    bar: hex(0x4C8468),
    bar_hi: hex(0xD6F39A),
    accent: hex(0xD6F39A),
    accent_soft: hex(0xE8F6CF),
    accent_text: hex(0x3E6A12),
    blue: hex(0x1F5E9E),
    blue_soft: hex(0xE1EEFA),
    orange_soft: hex(0xFCE8DA),
    red_soft: hex(0xFBE4E1),
    selected: hex(0xE8F6CF),
    chat_bg: hex(0xF5F3EF),
    bubble_in: hex(0xFFFFFF),
    hl: hex(0x1D5A3F),
    on_hl: hex(0xFFFFFF),
};

/// v6 "Warust Mint", light: neutral greys, charcoal controls, green markers.
pub const MINT_LIGHT: Palette = Palette {
    bg: hex(0xFFFFFF),
    panel: hex(0xFAFAFA),
    raised: hex(0xF4F4F5),
    border: hex(0xE8E8EA),
    border_strong: hex(0xD1D1D6),
    text: hex(0x18181B),
    text_2: hex(0x3F3F46),
    muted: hex(0x71717A),
    primary: hex(0x27272A),
    on_primary: hex(0xFFFFFF),
    on_primary_muted: hex(0xD4D4D8),
    green: hex(0x1FA855),
    amber: hex(0xD97706),
    red: hex(0xDC2626),
    red_text: hex(0xB91C1C),
    gray: hex(0xA1A1AA),
    bar: hex(0xD1D1D6),
    bar_hi: hex(0x27272A),
    accent: hex(0x27272A),
    accent_soft: hex(0xE6F5EC),
    accent_text: hex(0x0F6B48),
    blue: hex(0x3F3F46),
    blue_soft: hex(0xF4F4F5),
    orange_soft: hex(0xF4F4F5),
    red_soft: hex(0xF4F4F5),
    selected: hex(0xEFEFF1),
    chat_bg: hex(0xFFFFFF),
    bubble_in: hex(0xFFFFFF),
    hl: hex(0x12855A),
    on_hl: hex(0xFFFFFF),
};

/// v6 "Warust Mint", dark.
pub const MINT_DARK: Palette = Palette {
    bg: hex(0x0B0B0C),
    panel: hex(0x111113),
    raised: hex(0x19191C),
    border: hex(0x26262A),
    border_strong: hex(0x3A3A40),
    text: hex(0xEDEDEF),
    text_2: hex(0xC4C4CA),
    muted: hex(0x8E8E96),
    primary: hex(0xE4E4E7),
    on_primary: hex(0x18181B),
    on_primary_muted: hex(0x71717A),
    green: hex(0x22C55E),
    amber: hex(0xF59E0B),
    red: hex(0xEF4444),
    red_text: hex(0xF87171),
    gray: hex(0x71717A),
    bar: hex(0x3A3A40),
    bar_hi: hex(0xEDEDEF),
    accent: hex(0xE4E4E7),
    accent_soft: hex(0x10261C),
    accent_text: hex(0x6EE7B0),
    blue: hex(0xC4C4CA),
    blue_soft: hex(0x19191C),
    orange_soft: hex(0x19191C),
    red_soft: hex(0x19191C),
    selected: hex(0x1D1D21),
    chat_bg: hex(0x0B0B0C),
    bubble_in: hex(0x19191C),
    hl: hex(0x3ECF8E),
    on_hl: hex(0x06281A),
};

/// Mint's frame (sidebar) and what sits on it: its fill, the fields and the
/// selected row on it, and its lines. `(light, dark)`.
pub const MINT_SIDEBAR: (Color32, Color32) = (hex(0xF5F5F6), hex(0x131315));
pub const MINT_SIDEBAR_FIELD: (Color32, Color32) = (hex(0xFFFFFF), hex(0x1C1C20));
pub const MINT_SIDEBAR_SELECTED: (Color32, Color32) = (hex(0xFFFFFF), hex(0x242428));
pub const MINT_SIDEBAR_LINE: (Color32, Color32) = (hex(0xE4E4E7), hex(0x26262A));
/// The soft glow in the top right of Mint's chat background.
pub const MINT_GLOW: (Color32, Color32) = (hex(0xD5F2E3), hex(0x34D399));

/// Forest's dark green surfaces (hero cards) and what sits on them.
pub const FOREST_2: Color32 = hex(0x2A6B4E);
/// `f-orange`: the arrow on the soft orange circle.
pub const FOREST_ORANGE: Color32 = hex(0x9E4317);
pub const FOREST_LINE: Color32 = hex(0x4C8468);
pub const FOREST_MUTED: Color32 = hex(0xB5CFC0);

pub const R_SM: u8 = 6;
pub const R_MD: u8 = 8;
pub const R_LG: u8 = 12;

/// Corner radius of cards: 12 in Classic, 24 in Forest (`f-r-card`).
pub fn r_card() -> u8 {
    match template() {
        Template::Classic => R_LG,
        Template::Forest => 24,
        Template::Mint => 16,
    }
}

/// Buttons and inputs: small radius in Classic, pills in Forest (`f-r-pill`).
pub fn r_control() -> u8 {
    match template() {
        Template::Classic => R_SM,
        Template::Forest => 16,
        Template::Mint => 10,
    }
}

/// Inner boxes inside cards: 8 in Classic, 14 in Forest (`f-r-inner`).
pub fn r_inner() -> u8 {
    match template() {
        Template::Classic => R_MD,
        Template::Forest => 14,
        Template::Mint => 10,
    }
}

pub fn palette(ctx: &Context) -> Palette {
    match (template(), ctx.theme()) {
        (Template::Forest, _) => FOREST,
        (Template::Mint, Theme::Dark) => MINT_DARK,
        (Template::Mint, Theme::Light) => MINT_LIGHT,
        (_, Theme::Dark) => DARK,
        (_, Theme::Light) => LIGHT,
    }
}

/// Gap around and between Forest's cards.
pub const GAP: i8 = 12;

/// Frame of a side panel or a pane of a screen: flush in Classic, a white
/// card with a gap around it in Forest.
pub fn pane(ctx: &Context, classic_fill: Color32, inner: Margin) -> Frame {
    let p = palette(ctx);
    match template() {
        Template::Classic | Template::Mint => Frame::NONE.fill(classic_fill).inner_margin(inner),
        Template::Forest => Frame::NONE
            .fill(p.panel)
            .corner_radius(r_card())
            .inner_margin(inner)
            .outer_margin(Margin::same(GAP)),
    }
}

/// Background of the parts inside a pane (chat header, composer).
pub fn pane_bg(ctx: &Context) -> Color32 {
    let p = palette(ctx);
    match template() {
        Template::Classic | Template::Mint => p.bg,
        Template::Forest => p.panel,
    }
}

/// Rounded top corners of a pane's first part (none in Classic).
pub fn top_corners() -> CornerRadius {
    match template() {
        Template::Classic | Template::Mint => CornerRadius::ZERO,
        Template::Forest => CornerRadius { nw: r_card(), ne: r_card(), sw: 0, se: 0 },
    }
}

pub fn bottom_corners() -> CornerRadius {
    match template() {
        Template::Classic | Template::Mint => CornerRadius::ZERO,
        Template::Forest => CornerRadius { nw: 0, ne: 0, sw: r_card(), se: r_card() },
    }
}

/// A bordered card in Classic, a white card in Forest.
pub fn card(ctx: &Context) -> Frame {
    let p = palette(ctx);
    match template() {
        Template::Classic | Template::Mint => Frame::NONE.stroke(Stroke::new(1.0, p.border)).corner_radius(r_card()),
        Template::Forest => Frame::NONE.fill(p.panel).corner_radius(r_card()),
    }
}

/// Margin of the root area under the navigation: none in Classic; in
/// Forest, so panes line up with the navigation's left edge.
pub fn root_margin() -> Margin {
    match template() {
        Template::Classic | Template::Mint => Margin::ZERO,
        Template::Forest => Margin { left: 20, right: 20, top: 0, bottom: 20 },
    }
}

/// Frame of a scrolling page (Nomor, Log, Webhook & API, Pengaturan).
pub fn page(ctx: &Context) -> Frame {
    let p = palette(ctx);
    match template() {
        _ if ctx.content_rect().width() < super::compact::WIDTH => Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(16, 12)),
        Template::Classic | Template::Mint => Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(40, 28)),
        Template::Forest => Frame::NONE.fill(p.bg).inner_margin(Margin { left: 12, right: 12, top: 8, bottom: 12 }),
    }
}

/// Wraps content that sits straight on the page in Classic (tables) in a white
/// card in Forest.
pub fn surface(ctx: &Context) -> Frame {
    let p = palette(ctx);
    match template() {
        Template::Classic | Template::Mint => Frame::NONE,
        Template::Forest => Frame::NONE.fill(p.panel).corner_radius(r_card()).inner_margin(Margin::same(20)),
    }
}

/// Installs egui visuals for both themes so built-in widgets (scroll bars,
/// text edits, panel separators) match the tokens.
pub fn install(ctx: &Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(Visuals::dark(), &DARK));
    ctx.set_visuals_of(Theme::Light, visuals(Visuals::light(), &LIGHT));
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 6.0);
        });
    }
}

fn visuals(mut v: Visuals, p: &Palette) -> Visuals {
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.extreme_bg_color = p.raised;
    v.faint_bg_color = p.panel;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.border_strong;
    v.selection.stroke = Stroke::new(1.0, p.text);
    let r = CornerRadius::same(R_SM);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
        w.fg_stroke = Stroke::new(1.0, p.text);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.bg_fill = p.bg;
    v.widgets.inactive.bg_fill = p.raised;
    v.widgets.inactive.weak_bg_fill = p.raised;
    v.widgets.hovered.bg_fill = p.border;
    v.widgets.hovered.weak_bg_fill = p.border;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    v.widgets.active.bg_fill = p.border_strong;
    v.widgets.active.weak_bg_fill = p.border_strong;
    v
}

/// Whether Mint is the active template: where a screen draws Mint's own
/// shapes (round avatars, the dot grid) instead of Classic's.
pub fn mint() -> bool {
    template() == Template::Mint
}

/// One of Mint's `(light, dark)` pairs for the current mode.
pub fn mint_pair(ctx: &Context, pair: (Color32, Color32)) -> Color32 {
    if ctx.theme() == Theme::Dark { pair.1 } else { pair.0 }
}

/// Fill of the sidebar: the panel colour, or Mint's own frame grey.
pub fn sidebar_fill(ctx: &Context) -> Color32 {
    if mint() { mint_pair(ctx, MINT_SIDEBAR) } else { palette(ctx).panel }
}

/// Fill and outline of the selected (or hovered) item in the sidebar. Mint's
/// sidebar is grey already, so its selected item is a lighter field with a line.
pub fn sidebar_selected(ctx: &Context, selected: bool) -> (Color32, Stroke) {
    if !mint() {
        return (palette(ctx).raised, Stroke::NONE);
    }
    let fill = mint_pair(ctx, MINT_SIDEBAR_SELECTED);
    if selected {
        (fill, Stroke::new(1.0, mint_pair(ctx, MINT_SIDEBAR_LINE)))
    } else {
        (fill.gamma_multiply(0.6), Stroke::NONE)
    }
}
