//! Emoji picker for the composer, and colour emoji in laid-out text. Emoji
//! come from the bundled Noto Color Emoji; epaint shapes skin tones, ZWJ
//! sequences and flags into single glyphs.

use std::ops::Range;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::text_edit::TextEditState;
use egui::{Id, PopupCloseBehavior, Response, ScrollArea, Sense, Ui, vec2};

use super::fonts::{self, Weight};
use super::theme::{self, R_SM};
use super::widgets::{self, rich};
use crate::i18n::tr;

const CATEGORIES: &[(&str, &str)] = &[
    ("Wajah", "😀 😃 😄 😁 😆 😅 🤣 😂 🙂 🙃 🫠 😉 😊 😇 🥰 😍 🤩 😘 😗 ☺️ 😚 😙 🥲 😋 😛 😜 🤪 😝 🤑 🤗 🤭 🫢 🫣 🤫 🤔 🫡 🤐 🤨 😐 😑 😶 🫥 😏 😒 🙄 😬 🤥 🫨 😌 😔 😪 🤤 😴 😷 🤒 🤕 🤢 🤮 🤧 🥵 🥶 🥴 😵 🤯 🤠 🥳 🥸 😎 🤓 🧐 😕 🫤 😟 🙁 😮 😯 😲 😳 🥺 🥹 😦 😧 😨 😰 😥 😢 😭 😱 😖 😣 😞 😓 😩 😫 🥱 😤 😡 😠 🤬 😈 👿 💀 ☠️ 💩 🤡 👹 👺 👻 👽 👾 🤖 😺 😸 😹 😻 😼 😽 🙀 😿 😾 🙈 🙉 🙊"),
    ("Gestur", "👋 🤚 🖐 ✋️ 🖖 🫱 🫲 🫳 🫴 👌 🤌 🤏 ✌️ 🤞 🫰 🤟 🤘 🤙 👈 👉 👆 🖕 👇 ☝️ 🫵 👍 👎 ✊️ 👊 🤛 🤜 👏 🙌 🫶 👐 🤲 🤝 🙏 ✍️ 💅 🤳 💪 🦾 🦵 🦶 👂 🦻 👃 🧠 🫀 🫁 🦷 🦴 👀 👁 👅 👄 🫦 💋"),
    ("Orang", "👶 🧒 👦 👧 🧑 👱 👨 🧔 👩 🧓 👴 👵 🙍 🙎 🙅 🙆 💁 🙋 🧏 🙇 🤦 🤷 👮 🕵 💂 🥷 👷 🫅 🤴 👸 👳 👲 🧕 🤵 👰 🤰 🫄 🤱 👼 🎅 🤶 🦸 🦹 🧙 🧚 🧛 🧜 🧝 🧞 🧟 💆 💇 🚶 🧍 🧎 🏃 💃 🕺 👯 🧖 🧗 🤺 🏇 ⛷️ 🏂 🏌 🏄 🚣 🏊 ⛹️ 🏋 🚴 🚵 🤸 🤼 🤽 🤾 🤹 🧘 🛀 🛌 👭 👫 👬 💏 💑 👪"),
    ("Hati", "❤️ 🩷 🧡 💛 💚 💙 🩵 💜 🤎 🖤 🩶 🤍 💔 ❣️ 💕 💞 💓 💗 💖 💘 💝 💟 💌 💯 💢 💥 💫 💦 💨 🕳 💬 🗨 🗯 💭 💤"),
    ("Alam", "🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐻‍❄️ 🐨 🐯 🦁 🐮 🐷 🐽 🐸 🐵 🐔 🐧 🐦 🐤 🐣 🐥 🦆 🦅 🦉 🦇 🐺 🐗 🐴 🦄 🫎 🐝 🪱 🐛 🦋 🐌 🐞 🐜 🪰 🪲 🪳 🦟 🦗 🕷 🦂 🐢 🐍 🦎 🦖 🦕 🐙 🦑 🦐 🦞 🦀 🪼 🐡 🐠 🐟 🐬 🐳 🐋 🦈 🦭 🐊 🐅 🐆 🦓 🦍 🦧 🦣 🐘 🦛 🦏 🐪 🐫 🦒 🦘 🦬 🐃 🐂 🐄 🐎 🐖 🐏 🐑 🦙 🐐 🦌 🐕 🐩 🦮 🐈 🐓 🦃 🦤 🦚 🦜 🦢 🦩 🕊 🐇 🦝 🦨 🦡 🦫 🦦 🦥 🐁 🐀 🐿 🦔 🌵 🎄 🌲 🌳 🌴 🪵 🌱 🌿 ☘️ 🍀 🎍 🪴 🎋 🍃 🍂 🍁 🪺 🪹 🍄 🐚 🪸 🪨 🌾 💐 🌷 🌹 🥀 🪻 🪷 🌺 🌸 🌼 🌻 🌞 🌝 🌛 🌜 🌚 🌕 🌙 🌎 🌍 🌏 🪐 💫 ⭐️ 🌟 ✨️ ⚡️ ☄️ 💥 🔥 🌪 🌈 ☀️ 🌤 ⛅️ 🌥 ☁️ 🌦 🌧 ⛈️ 🌩 🌨 ❄️ ☃️ ⛄️ 🌬 💨 💧 💦 🫧 ☔️ ☂️ 🌊"),
    ("Makanan", "🍏 🍎 🍐 🍊 🍋 🍌 🍉 🍇 🍓 🫐 🍈 🍒 🍑 🥭 🍍 🥥 🥝 🍅 🍆 🥑 🥦 🫛 🥬 🥒 🌶 🫑 🌽 🥕 🫒 🧄 🧅 🥔 🍠 🫚 🥐 🥯 🍞 🥖 🥨 🧀 🥚 🍳 🧈 🥞 🧇 🥓 🥩 🍗 🍖 🦴 🌭 🍔 🍟 🍕 🫓 🥪 🥙 🧆 🌮 🌯 🫔 🥗 🥘 🫕 🥫 🍝 🍜 🍲 🍛 🍣 🍱 🥟 🦪 🍤 🍙 🍚 🍘 🍥 🥠 🥮 🍢 🍡 🍧 🍨 🍦 🥧 🧁 🍰 🎂 🍮 🍭 🍬 🍫 🍿 🍩 🍪 🌰 🥜 🍯 🥛 🍼 🫖 ☕️ 🍵 🧃 🥤 🧋 🍶 🍺 🍻 🥂 🍷 🥃 🍸 🍹 🧉 🍾 🧊 🥄 🍴 🍽 🥣 🥡 🥢 🧂"),
    ("Aktivitas", "⚽️ 🏀 🏈 ⚾️ 🥎 🎾 🏐 🏉 🥏 🎱 🪀 🏓 🏸 🏒 🏑 🥍 🏏 🪃 🥅 ⛳️ 🪁 🏹 🎣 🤿 🥊 🥋 🎽 🛹 🛼 🛷 ⛸️ 🥌 🎿 🏆 🥇 🥈 🥉 🏅 🎖 🏵 🎗 🎫 🎟 🎪 🎭 🩰 🎨 🎬 🎤 🎧 🎼 🎹 🥁 🪘 🎷 🎺 🪗 🎸 🪕 🎻 🪈 🎲 ♟️ 🎯 🎳 🎮 🎰 🧩"),
    ("Perjalanan", "🚗 🚕 🚙 🚌 🚎 🏎 🚓 🚑 🚒 🚐 🛻 🚚 🚛 🚜 🛵 🏍 🛺 🚲 🛴 🚨 🚔 🚍 🚘 🚖 🚡 🚠 🚟 🚃 🚋 🚞 🚝 🚄 🚅 🚈 🚂 🚆 🚇 🚊 🚉 ✈️ 🛫 🛬 🛩 💺 🛰 🚀 🛸 🚁 🛶 ⛵️ 🚤 🛥 🛳 ⛴️ 🚢 ⚓️ 🪝 ⛽️ 🚧 🚦 🚥 🚏 🗺 🗿 🗽 🗼 🏰 🏯 🏟 🎡 🎢 🎠 ⛲️ ⛱️ 🏖 🏝 🏜 🌋 ⛰️ 🏔 🗻 🏕 ⛺️ 🛖 🏠 🏡 🏘 🏚 🏗 🏭 🏢 🏬 🏣 🏤 🏥 🏦 🏨 🏪 🏫 🏩 💒 🏛 ⛪️ 🕌 🕍 🛕 🕋 ⛩️ 🛤 🛣 🗾 🎑 🏞 🌅 🌄 🌠 🎇 🎆 🌇 🌆 🏙 🌃 🌌 🌉 🌁"),
    ("Benda", "⌚️ 📱 📲 💻 ⌨️ 🖥 🖨 🖱 🖲 🕹 🗜 💽 💾 💿 📀 📼 📷 📸 📹 🎥 📽 🎞 📞 ☎️ 📟 📠 📺 📻 🎙 🎚 🎛 🧭 ⏱️ ⏲️ ⏰️ 🕰 ⌛️ ⏳️ 📡 🔋 🪫 🔌 💡 🔦 🕯 🪔 🧯 🛢 💸 💵 💴 💶 💷 🪙 💰 💳 💎 ⚖️ 🪜 🧰 🪛 🔧 🔨 ⚒️ 🛠 ⛏️ 🪚 🔩 ⚙️ 🪤 🧱 ⛓️ 🧲 🔫 💣 🧨 🪓 🔪 🗡 ⚔️ 🛡 🚬 ⚰️ 🪦 ⚱️ 🏺 🔮 📿 🧿 🪬 💈 ⚗️ 🔭 🔬 🕳 🩹 🩺 🩻 🩼 💊 💉 🩸 🧬 🦠 🧫 🧪 🌡 🧹 🪠 🧺 🧻 🚽 🚰 🚿 🛁 🛀 🧼 🪥 🪒 🧽 🪣 🧴 🛎 🔑 🗝 🚪 🪑 🛋 🛏 🛌 🧸 🪆 🖼 🪞 🪟 🛍 🛒 🎁 🎈 🎏 🎀 🪄 🪅 🎊 🎉 🎎 🏮 🎐 🧧 ✉️ 📩 📨 📧 💌 📥 📤 📦 🏷 🪧 📪 📫 📬 📭 📮 📯 📜 📃 📄 📑 🧾 📊 📈 📉 🗒 🗓 📆 📅 🗑 📇 🗃 🗳 🗄 📋 📁 📂 🗂 🗞 📰 📓 📔 📒 📕 📗 📘 📙 📚 📖 🔖 🧷 🔗 📎 🖇 📐 📏 🧮 📌 📍 ✂️ 🖊 🖋 ✒️ 🖌 🖍 📝 ✏️ 🔍 🔎 🔏 🔐 🔒 🔓"),
    ("Simbol", "✅️ ☑️ ✔️ ❌️ ❎️ ➕️ ➖️ ➗️ ✖️ 🟰 ❓️ ❔️ ❕️ ❗️ ‼️ ⁉️ 〰️ 💱 💲 ⚕️ ♻️ ⚜️ 🔱 📛 🔰 ⭕️ ⛔️ 🚫 📵 🔞 ☢️ ☣️ ⚠️ 🚸 ⬆️ ↗️ ➡️ ↘️ ⬇️ ↙️ ⬅️ ↖️ ↕️ ↔️ ↩️ ↪️ ⤴️ ⤵️ 🔃 🔄 🔙 🔚 🔛 🔜 🔝 🛐 ⚛️ 🕉 ✡️ ☸️ ☯️ ✝️ ☦️ ☪️ ☮️ 🕎 🔯 ♈️ ♉️ ♊️ ♋️ ♌️ ♍️ ♎️ ♏️ ♐️ ♑️ ♒️ ♓️ ⛎️ 🔀 🔁 🔂 ▶️ ⏩️ ⏭️ ⏯️ ◀️ ⏪️ ⏮️ 🔼 ⏫️ 🔽 ⏬️ ⏸️ ⏹️ ⏺️ ⏏️ 🎦 🔅 🔆 📶 🛜 📳 📴 ♀️ ♂️ ⚧️ ✳️ ❇️ ✴️ 🆗 🆕 🆓 🆙 🆒 🆘 🆚 🔴 🟠 🟡 🟢 🔵 🟣 🟤 ⚫️ ⚪️ 🟥 🟧 🟨 🟩 🟦 🟪 🟫 ⬛️ ⬜️ 🔶 🔷 🔸 🔹 🔺 🔻 💠 🔘 🔳 🔲 🎵 🎶 ©️ ®️ ™️ 🔔 🔕 📣 📢 💤 🏁 🚩 🎌 🏴 🏳"),
    ("Bendera", "🇮🇩 🇲🇾 🇸🇬 🇧🇳 🇹🇱 🇵🇭 🇹🇭 🇻🇳 🇯🇵 🇰🇷 🇨🇳 🇭🇰 🇹🇼 🇮🇳 🇵🇰 🇧🇩 🇸🇦 🇦🇪 🇶🇦 🇰🇼 🇪🇬 🇹🇷 🇺🇸 🇨🇦 🇲🇽 🇧🇷 🇦🇷 🇬🇧 🇩🇪 🇫🇷 🇳🇱 🇮🇹 🇪🇸 🇵🇹 🇷🇺 🇦🇺 🇳🇿 🇿🇦 🇳🇬 🏳️‍🌈"),
];

/// A category's name in the language that is on, keyed by its Indonesian name above.
fn category_name(name: &str) -> &'static str {
    match name {
        "Wajah" => tr("Faces", "Wajah"),
        "Gestur" => tr("Gestures", "Gestur"),
        "Orang" => tr("People", "Orang"),
        "Hati" => tr("Hearts", "Hati"),
        "Alam" => tr("Nature", "Alam"),
        "Makanan" => tr("Food", "Makanan"),
        "Aktivitas" => tr("Activities", "Aktivitas"),
        "Perjalanan" => tr("Travel", "Perjalanan"),
        "Benda" => tr("Objects", "Benda"),
        "Simbol" => tr("Symbols", "Simbol"),
        "Bendera" => tr("Flags", "Bendera"),
        _ => "",
    }
}

const CELL: f32 = 34.0;
const COLUMNS: usize = 9;

/// A character that begins an emoji: pictographs, symbols and dingbats, flag
/// letters, or anything followed by an emoji variation selector or keycap.
fn starts_emoji(c: char, next: Option<char>) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x2300..=0x23FF)
        || matches!(next, Some('\u{FE0F}' | '\u{20E3}'))
}

/// A character that belongs to the emoji before it.
fn continues_emoji(c: char) -> bool {
    matches!(c, '\u{FE0F}' | '\u{200D}' | '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}')
}

/// Splits text into byte ranges of plain text and of emoji (`true`).
fn runs(text: &str) -> Vec<(Range<usize>, bool)> {
    let mut out: Vec<(Range<usize>, bool)> = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut after_zwj = false;
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|(_, n)| *n);
        let in_emoji = out.last().is_some_and(|(_, e)| *e);
        let emoji = (in_emoji && (continues_emoji(c) || after_zwj)) || starts_emoji(c, next);
        after_zwj = emoji && c == '\u{200D}';
        let end = i + c.len_utf8();
        match out.last_mut() {
            Some((range, e)) if *e == emoji => range.end = end,
            _ => out.push((i..end, emoji)),
        }
    }
    out
}

/// Appends text to a layout job, emoji runs in the emoji-first family so they
/// are drawn in colour.
pub fn append(job: &mut LayoutJob, text: &str, format: TextFormat) {
    for (range, emoji) in runs(text) {
        let mut format = format.clone();
        if emoji {
            format.font_id.family = fonts::emoji_family();
        }
        job.append(&text[range], 0.0, format);
    }
}

/// Opens the picker under `button`; a chosen emoji goes in at the caret of the
/// TextEdit `editor`, replacing any selection.
pub fn picker(ui: &Ui, button: &Response, editor: Id, text: &mut String) {
    let p = theme::palette(ui.ctx());
    egui::Popup::menu(button)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(COLUMNS as f32 * CELL + 16.0)
        .show(|ui| {
            let tab_id = editor.with("emoji_tab");
            let mut tab: usize = ui.data(|d| d.get_temp(tab_id)).unwrap_or(0);
            // Category tabs, each shown by its first emoji.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let width = (COLUMNS as f32 * CELL) / CATEGORIES.len() as f32;
                for (i, (name, list)) in CATEGORIES.iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
                    if i == tab || response.hovered() {
                        ui.painter().rect_filled(rect, R_SM, p.raised);
                    }
                    let first = list.split(' ').next().unwrap_or_default();
                    widgets::paint_text(ui, rect, first, fonts::ui(16.0, Weight::Regular), p.text, egui::Align::Center);
                    if response.on_hover_text(category_name(name)).clicked() {
                        tab = i;
                    }
                }
            });
            ui.data_mut(|d| d.insert_temp(tab_id, tab));
            let (name, list) = CATEGORIES[tab.min(CATEGORIES.len() - 1)];
            widgets::section_label(ui, category_name(name));
            ScrollArea::vertical().id_salt(("emoji", tab)).max_height(300.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let emoji: Vec<&str> = list.split(' ').collect();
                for chunk in emoji.chunks(COLUMNS) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for e in chunk {
                            let (rect, response) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            if response.hovered() {
                                ui.painter().rect_filled(rect, R_SM, p.raised);
                            }
                            widgets::paint_text(ui, rect, e, fonts::ui(22.0, Weight::Regular), p.text, egui::Align::Center);
                            if response.clicked() {
                                insert(ui, editor, text, e);
                            }
                        }
                    });
                }
            });
            ui.add_space(4.0);
            ui.label(rich(tr("Click outside to close", "Klik di luar untuk menutup"), 11.0, Weight::Regular, p.muted));
        });
}

fn insert(ui: &Ui, editor: Id, text: &mut String, emoji: &str) {
    let mut state = TextEditState::load(ui.ctx(), editor).unwrap_or_default();
    let end = text.chars().count();
    let [start, stop] = state
        .cursor
        .char_range()
        .map(|r| r.sorted_cursors())
        .unwrap_or([CCursor::new(end), CCursor::new(end)]);
    let byte = |c: CCursor| text.char_indices().nth(c.index.0).map_or(text.len(), |(b, _)| b);
    let (from, to) = (byte(start), byte(stop));
    text.replace_range(from..to, emoji);
    let caret = CCursor::new(start.index.0 + emoji.chars().count());
    state.cursor.set_char_range(Some(CCursorRange::one(caret)));
    state.store(ui.ctx(), editor);
    ui.memory_mut(|m| m.request_focus(editor));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_runs() {
        let text = "hai ❤️ 1️⃣ ok 👍🏼👨‍👩‍👧 🇮🇩!";
        let pieces: Vec<(&str, bool)> = runs(text).into_iter().map(|(r, e)| (&text[r], e)).collect();
        assert_eq!(
            pieces,
            vec![
                ("hai ", false),
                ("❤️", true),
                (" ", false),
                ("1️⃣", true),
                (" ok ", false),
                ("👍🏼👨‍👩‍👧", true),
                (" ", false),
                ("🇮🇩", true),
                ("!", false),
            ]
        );
        assert_eq!(runs("plain 123"), vec![(0..9, false)]);
    }
}

#[cfg(test)]
mod font_tests {
    use super::*;

    /// Every emoji offered must come out as one colour glyph in every weight;
    /// a missing font or feature shows boxes or monochrome glyphs.
    #[test]
    fn picker_emoji_have_glyphs() {
        let ctx = egui::Context::default();
        fonts::install(&ctx);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        for weight in [Weight::Regular, Weight::Medium, Weight::SemiBold] {
            let font = fonts::ui(20.0, weight);
            for (name, list) in CATEGORIES {
                for e in list.split(' ') {
                    let mut job = LayoutJob::default();
                    append(&mut job, e, TextFormat::simple(font.clone(), egui::Color32::WHITE));
                    let galley = ctx.fonts_mut(|f| f.layout_job(job));
                    let glyphs: Vec<_> = galley.rows.iter().flat_map(|r| &r.glyphs).collect();
                    assert!(glyphs.first().is_some_and(|g| g.is_color), "{name}: {e} is not a colour emoji");
                }
            }
        }
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    }
}
