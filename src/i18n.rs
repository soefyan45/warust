//! The app's two languages: English (the default) and Indonesian. Each text
//! sits where it is used, as `tr("English", "Indonesia")`, or `trf!` for
//! formatted text, so both versions are read side by side. The language is
//! one global switch, taken from the settings every frame.

use std::sync::atomic::{AtomicBool, Ordering};

static INDONESIAN: AtomicBool = AtomicBool::new(false);

/// Settings key of each language.
pub const EN: &str = "en";
pub const ID: &str = "id";

/// Switches the language; anything but `id` is English.
pub fn set(key: &str) {
    INDONESIAN.store(key == ID, Ordering::Relaxed);
}

pub fn indonesian() -> bool {
    INDONESIAN.load(Ordering::Relaxed)
}

/// `en` or `id`, whichever language is on.
pub fn tr(en: &'static str, id: &'static str) -> &'static str {
    pick(indonesian(), en, id)
}

fn pick<'a>(indonesian: bool, en: &'a str, id: &'a str) -> &'a str {
    if indonesian { id } else { en }
}

/// `format!` in the language that is on: `trf!("{n} chats", "{n} chat")`.
#[macro_export]
macro_rules! trf {
    ($en:literal, $id:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::indonesian() { format!($id $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests never call `set`: they run in parallel and share the switch, so
    // every other test sees the default, English.
    #[test]
    fn english_by_default() {
        assert!(!indonesian());
        assert_eq!(tr("Send", "Kirim"), "Send");
        let n = 3;
        assert_eq!(crate::trf!("{n} chats", "{n} chat"), "3 chats");
        assert_eq!(pick(true, "Send", "Kirim"), "Kirim");
    }
}
