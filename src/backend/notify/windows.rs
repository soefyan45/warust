//! Windows toasts, so a click opens the chat. Toasts need an app id; warust
//! is not installed through a package, so the id is registered under HKCU
//! with its name and icon. Clicks are seen while warust runs (the Activated
//! event); after it quits a toast in Action Center only dismisses.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::core::{HSTRING, IInspectable, Ref};

use super::{Updater, parse_key};

/// Same as the macOS bundle id.
const APP_ID: &str = "io.github.soefyan45.warust";
/// The toast group; with a tag per chat a newer toast replaces the older one.
const GROUP: &str = "warust";

static STATE: OnceLock<Updater> = OnceLock::new();

/// The shown toast per chat key. Kept alive so its Activated handler still
/// runs; dropped when a newer toast replaces it.
fn shown() -> &'static Mutex<HashMap<String, ToastNotification>> {
    static SHOWN: OnceLock<Mutex<HashMap<String, ToastNotification>>> = OnceLock::new();
    SHOWN.get_or_init(Default::default)
}

/// Registers the app id with its display name and icon (once per run; the
/// values are rewritten in case the data folder moved).
pub(super) fn init(state: &Updater) {
    if STATE.set(state.clone()).is_err() {
        return;
    }
    let icon = state.data_dir.join("warust-256.png");
    let _ = std::fs::write(&icon, include_bytes!("../../../assets/app/warust-256.png"));
    let key = format!(r"HKCU\Software\Classes\AppUserModelId\{APP_ID}");
    let icon = icon.to_string_lossy().into_owned();
    for (name, value) in [("DisplayName", "warust"), ("IconUri", icon.as_str())] {
        let added = crate::platform::command("reg")
            .args(["add", &key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"])
            .output();
        if !added.is_ok_and(|o| o.status.success()) {
            log::warn!("could not register the notification app id ({name})");
        }
    }
}

/// Text inside toast XML.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Toast tags are short (64 characters); the chat key can be longer.
fn tag(key: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn toast_xml(title: &str, body: &str) -> String {
    // Silent: warust plays its own sound, once per burst.
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio silent="true"/></toast>"#,
        xml_escape(title),
        xml_escape(body)
    )
}

/// Shows or replaces the chat's toast.
pub(super) fn show(key: &str, title: &str, body: &str) -> windows::core::Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(toast_xml(title, body)))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    toast.SetTag(&HSTRING::from(tag(key)))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    let clicked = key.to_owned();
    toast.Activated(&TypedEventHandler::new(move |_: Ref<ToastNotification>, _: Ref<IInspectable>| {
        if let (Some(state), Some((session, jid))) = (STATE.get(), parse_key(&clicked)) {
            state.request_open(session, jid);
        }
        Ok(())
    }))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)?;
    if let Ok(mut shown) = shown().lock() {
        shown.insert(key.to_owned(), toast);
    }
    Ok(())
}

/// Lets go of the toasts on quit; Windows keeps them in Action Center.
pub(super) fn forget_all() {
    if let Ok(mut shown) = shown().lock() {
        shown.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_is_escaped_and_tags_are_short() {
        let xml = toast_xml("Budi & <Ani>", "\"ya\" 'kan'");
        assert!(xml.contains("<text>Budi &amp; &lt;Ani&gt;</text>"));
        assert!(xml.contains("<text>&quot;ya&quot; &apos;kan&apos;</text>"));
        let long = format!("s1|{}@g.us", "1".repeat(80));
        assert_eq!(tag(&long).len(), 16);
        assert_eq!(tag(&long), tag(&long));
    }
}
