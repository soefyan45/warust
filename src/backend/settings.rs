//! App settings in `data/settings.json`: the local REST API, the webhook, and
//! send pacing. Missing fields take their defaults, so older files still load.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use whatsapp_rust::serde_json;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Api {
    pub enabled: bool,
    /// Always bound on 127.0.0.1.
    pub port: u16,
    /// `Authorization: Bearer <key>` on every request but `/healthz`.
    pub key: String,
}

impl Default for Api {
    fn default() -> Self {
        Self { enabled: false, port: 8787, key: new_secret("wr_") }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Webhook {
    /// Default URL for every number; empty means off.
    pub url: String,
    /// Signs each body: `X-Warust-Signature: sha256=<hex HMAC>`.
    pub secret: String,
    /// URL per session id, used instead of `url`.
    pub overrides: HashMap<String, String>,
    pub message_received: bool,
    pub message_status: bool,
    pub session_status: bool,
}

impl Default for Webhook {
    fn default() -> Self {
        Self {
            url: String::new(),
            secret: new_secret("whsec_"),
            overrides: HashMap::new(),
            message_received: true,
            message_status: true,
            session_status: true,
        }
    }
}

impl Webhook {
    /// Where events of this session go, if anywhere.
    pub fn url_for(&self, session: &str) -> Option<&str> {
        self.overrides
            .get(session)
            .map(String::as_str)
            .filter(|u| !u.trim().is_empty())
            .or_else(|| Some(self.url.as_str()).filter(|u| !u.trim().is_empty()))
    }

    pub fn wants(&self, event: &str) -> bool {
        match event {
            "message.received" | "status.received" | "message.deleted" | "message.edited" => self.message_received,
            "message.status" => self.message_status,
            "session.status" => self.session_status,
            _ => true,
        }
    }
}

/// Desktop notifications for incoming messages.
/// Looking for new releases (#93).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Updates {
    /// Check GitHub on start and once a day.
    pub auto_check: bool,
    /// Download, check and install a new release by itself, then restart.
    pub auto_install: bool,
    /// A version the user put off with "Later": no banner for it.
    pub skipped: String,
}

impl Default for Updates {
    fn default() -> Self {
        Self { auto_check: true, auto_install: true, skipped: String::new() }
    }
}

/// Anti-peek mode (#57): keeps chats unreadable to people behind the user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub enabled: bool,
    /// Message text and pictures stay frosted until the cursor is on them.
    pub blur_chat: bool,
    /// Chat list previews (and notification text) are frosted.
    pub blur_previews: bool,
    /// Contact names, numbers and pictures are hidden too.
    pub hide_names: bool,
    /// Lock after this many idle minutes (0: never).
    pub lock_minutes: u32,
}

impl Default for Privacy {
    fn default() -> Self {
        Self { enabled: false, blur_chat: true, blur_previews: true, hide_names: false, lock_minutes: 2 }
    }
}

impl Privacy {
    pub fn blur_chat(&self) -> bool {
        self.enabled && self.blur_chat
    }
    pub fn blur_previews(&self) -> bool {
        self.enabled && self.blur_previews
    }
    pub fn hide_names(&self) -> bool {
        self.enabled && self.hide_names
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notify {
    pub enabled: bool,
    pub groups: bool,
    /// Show the message text, not only who sent it.
    pub preview: bool,
    pub sound: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Self { enabled: true, groups: true, preview: true, sound: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sending {
    /// Minimum seconds between two sends of one number (anti-ban, at least 3).
    pub gap_secs: u64,
    /// Random extra wait of up to this many seconds.
    pub jitter_secs: u64,
}

impl Default for Sending {
    fn default() -> Self {
        Self { gap_secs: 4, jitter_secs: 2 }
    }
}

impl Sending {
    pub const MIN_GAP: u64 = 3;

    pub fn gap(&self) -> Duration {
        Duration::from_secs(self.gap_secs.max(Self::MIN_GAP))
    }

    pub fn jitter(&self) -> Duration {
        Duration::from_secs(self.jitter_secs.min(30))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub api: Api,
    pub webhook: Webhook,
    pub sending: Sending,
    /// Download received stickers as soon as they show on screen.
    pub auto_stickers: bool,
    /// `dark` or `light`.
    pub theme: String,
    /// Page template: `classic` (v4) or `forest` (v5).
    pub template: String,
    /// Webhook triggers, checked top to bottom; several may fire.
    pub rules: Vec<super::rules::Rule>,
    /// Numbers (session ids) that view statuses without sending "seen".
    pub status_seen_off: Vec<String>,
    pub notify: Notify,
    /// Let contacts see "sedang mengetik" while a reply is being typed.
    pub typing: bool,
    /// Opening a chat sends read receipts (blue ticks) and syncs "read" to
    /// the phone. Off by default (it only clears the local badge).
    pub send_read: bool,
    /// UI language: `en` (the default, also for older files) or `id`.
    pub language: String,
    /// Chat background: `auto` (the template's own), one of the designed
    /// wallpapers (`forest`, `mono`, `dark`, `forest-dark`, `geometric`,
    /// `botanical`), `none`, or `custom` with `wallpaper_file`.
    pub wallpaper: String,
    /// The user's own picture, copied into the data folder.
    pub wallpaper_file: String,
    /// How much a custom picture is dimmed toward the chat colour, 0 to 80 %.
    pub wallpaper_dim: u8,
    /// A number's own chat wallpaper, by session id (#60). A number not in
    /// here uses the app's.
    pub number_wallpapers: HashMap<String, NumberWallpaper>,
    pub privacy: Privacy,
    pub updates: Updates,
}

/// A number's chat wallpaper: the same choices as the app's `wallpaper`,
/// `wallpaper_file` and `wallpaper_dim`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberWallpaper {
    pub key: String,
    pub file: String,
    pub dim: u8,
}

/// Deletes a copied wallpaper picture, only ever inside the data folder's
/// `wallpapers` (never a file the user picked).
pub fn forget_wallpaper(data_dir: &Path, file: &str) {
    let dir = data_dir.join("wallpapers");
    let path = Path::new(file);
    if !file.is_empty() && path.parent() == Some(dir.as_path()) {
        let _ = std::fs::remove_file(path);
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api: Api::default(),
            webhook: Webhook::default(),
            sending: Sending::default(),
            auto_stickers: true,
            theme: "dark".to_owned(),
            template: "classic".to_owned(),
            rules: Vec::new(),
            status_seen_off: Vec::new(),
            notify: Notify::default(),
            typing: true,
            send_read: false,
            language: crate::i18n::EN.to_owned(),
            wallpaper: "auto".to_owned(),
            wallpaper_file: String::new(),
            wallpaper_dim: 30,
            number_wallpapers: HashMap::new(),
            privacy: Privacy::default(),
            updates: Updates::default(),
        }
    }
}

impl Settings {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("settings.json")
    }

    /// Loads the file, or makes (and saves) fresh settings with new secrets.
    pub fn load(data_dir: &Path) -> Self {
        let path = Self::path(data_dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("settings.json unreadable ({e}); using defaults");
                Self::default()
            }),
            Err(_) => {
                let fresh = Self::default();
                fresh.save(data_dir);
                fresh
            }
        }
    }

    pub fn save(&self, data_dir: &Path) {
        let path = Self::path(data_dir);
        let _ = std::fs::create_dir_all(data_dir);
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        // Write then rename, so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        // Holds the API key and webhook secret: owner only, where the file
        // system supports it (not on NTFS/FAT mounts).
        let write = std::fs::write(&tmp, text).and_then(|_| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            }
            std::fs::rename(&tmp, &path)
        });
        if let Err(e) = write {
            log::error!("cannot save settings: {e}");
        }
    }
}

/// A random secret such as `wr_3f9a…` (32 random bytes, hex).
pub fn new_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    if !crate::platform::random_bytes(&mut bytes) {
        // Not expected on any supported OS; still never a fixed key.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes[..16].copy_from_slice(&nanos.to_le_bytes());
        bytes[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}{hex}")
}

/// `wr_3f9a…c41b` for showing a secret without revealing it.
pub fn masked(secret: &str) -> String {
    let (prefix, rest) = secret.split_once('_').map(|(p, r)| (format!("{p}_"), r)).unwrap_or((String::new(), secret));
    if rest.len() <= 8 {
        return "••••".to_owned();
    }
    format!("{prefix}{}••••{}", &rest[..4], &rest[rest.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_overrides_and_masks() {
        let a = new_secret("wr_");
        assert!(a.starts_with("wr_") && a.len() == 3 + 64);
        assert_ne!(a, new_secret("wr_"));
        assert!(masked(&a).starts_with(&a[..7]) && !masked(&a).contains(&a[10..60]));

        let mut hook = Webhook { url: "https://a.test/hook".to_owned(), ..Webhook::default() };
        hook.overrides.insert("s2".to_owned(), "https://b.test".to_owned());
        hook.overrides.insert("s3".to_owned(), " ".to_owned());
        assert_eq!(hook.url_for("s1"), Some("https://a.test/hook"));
        assert_eq!(hook.url_for("s2"), Some("https://b.test"));
        assert_eq!(hook.url_for("s3"), Some("https://a.test/hook"));

        // Older files without newer fields still load, with defaults.
        let old: Settings = serde_json::from_str(r#"{"api":{"port":9000}}"#).unwrap();
        assert_eq!(old.api.port, 9000);
        assert!(!old.api.enabled && old.api.key.starts_with("wr_"));
        assert_eq!(old.sending.gap(), Duration::from_secs(4));
        assert_eq!(Sending { gap_secs: 1, jitter_secs: 0 }.gap(), Duration::from_secs(3));
    }

    #[test]
    fn send_read_is_off_unless_set() {
        assert!(!Settings::default().send_read);
        let old: Settings = serde_json::from_str(r#"{"typing":false}"#).unwrap();
        assert!(!old.send_read);
        let on: Settings = serde_json::from_str(r#"{"send_read":true}"#).unwrap();
        assert!(on.send_read);
    }
}
