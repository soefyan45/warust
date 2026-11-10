pub mod devices;
pub mod inbox;
pub mod integrations;
pub mod log;
pub mod nomor;
pub mod pairing;
pub mod ringkasan;
pub mod settings;
pub mod status;
pub mod triggers;

use egui::{Margin, Ui};

use super::fonts::Weight;
use super::icons;
use super::theme::{self, Palette};
use crate::i18n::tr;
use super::widgets::rich;
use crate::backend::{Book, Chat, mask_phone};

/// Page title and one-line description, shared by the screens.
pub fn page_head(ui: &mut Ui, title: &str, desc: &str) {
    let p = theme::palette(ui.ctx());
    // In the phone layout the back bar already shows the title.
    let phone = crate::ui::compact::is_compact(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        if !phone {
            ui.label(rich(title, 24.0, Weight::SemiBold, p.text));
        }
        ui.label(rich(desc, 14.0, Weight::Regular, p.muted));
    });
}

/// Where a number's webhook events go: the URL's host, or "Tidak diatur".
pub fn webhook_label(state: &super::State, session: &str) -> String {
    match state.settings.webhook.url_for(session) {
        Some(url) => {
            let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
            rest.split('/').next().unwrap_or(rest).to_owned()
        }
        None => tr("Not set", "Tidak diatur").to_owned(),
    }
}

/// Bordered section with a title and an optional one-line description.
pub fn card<R>(ui: &mut Ui, title: &str, desc: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui.ctx());
    theme::card(ui.ctx())
        .inner_margin(Margin::same(24))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(rich(title, 15.0, Weight::SemiBold, p.text));
                if !desc.is_empty() {
                    ui.label(rich(desc, 13.0, Weight::Regular, p.muted));
                }
            });
            add(ui)
        })
        .inner
}

/// Centered icon, title and one line, for screens or panels with nothing to show.
pub fn empty_state(ui: &mut Ui, p: &Palette, icon: &str, title: &str, desc: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 60.0).max(24.0));
        ui.spacing_mut().item_spacing.y = 10.0;
        icons::show(ui, icon, 28.0, p.muted);
        ui.label(rich(title, 16.0, Weight::SemiBold, p.text));
        ui.label(rich(desc, 13.0, Weight::Regular, p.muted));
    });
}

pub use crate::backend::short_jid;

/// Contact or group name, else the masked number.
pub fn chat_title(book: &Book, chat: &Chat) -> String {
    if let Some(name) = book.name_of(&chat.jid) {
        return name.to_owned();
    }
    if chat.is_group() {
        tr("Unnamed group", "Grup tanpa nama").to_owned()
    } else if chat.jid.ends_with("@s.whatsapp.net") {
        mask_phone(&chat.jid)
    } else {
        crate::trf!("Contact {}", "Kontak {}", short_jid(&chat.jid))
    }
}

/// A group member's name, else their masked number.
pub fn sender_label(book: &Book, jid: &str) -> String {
    book.name_of(jid).map(str::to_owned).unwrap_or_else(|| short_jid(jid))
}

pub fn chat_subtitle(chat: &Chat) -> String {
    if chat.is_group() { tr("Group", "Grup").to_owned() } else { short_jid(&chat.jid) }
}

/// Two letters for an avatar tile.
pub fn initials(title: &str) -> String {
    let letters: String = title
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .collect();
    if letters.is_empty() { "?".to_owned() } else { letters.to_uppercase() }
}
