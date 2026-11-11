//! Which sessions exist and what they are called: `data/sessions.json`, plus
//! any `data/sessions/<id>.db` found on disk (for example the Phase 0 spike).
//! Each session's keys live in its own SQLite device store.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use whatsapp_rust::serde_json::{self, Value, json};

#[derive(Clone)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// How the number shows on the phone's Linked devices list (#82).
    pub device: Device,
}

/// The linked-device name and type a session sends when it pairs.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub kind: DeviceKind,
}

impl Default for Device {
    fn default() -> Self {
        Self { name: DEFAULT_DEVICE_NAME.to_owned(), kind: DeviceKind::Desktop }
    }
}

/// This computer's OS as WhatsApp accepts it in "<Browser> (<OS>)": any
/// other text becomes "Linux" on the phone.
pub fn this_os() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Mac OS"
    } else {
        "Linux"
    }
}

/// Never the hostname, which would leak the machine's name to every linked phone.
pub const DEFAULT_DEVICE_NAME: &str = "Warust";

/// The device types offered for the Linked devices list; WhatsApp's
/// `DeviceProps.PlatformType` has more, these are the computer ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Desktop,
    Chrome,
    Firefox,
    Safari,
    Edge,
}

impl DeviceKind {
    pub const ALL: [DeviceKind; 5] = [DeviceKind::Desktop, DeviceKind::Chrome, DeviceKind::Firefox, DeviceKind::Safari, DeviceKind::Edge];

    /// The key in `sessions.json` and the REST API.
    pub fn code(self) -> &'static str {
        match self {
            DeviceKind::Desktop => "desktop",
            DeviceKind::Chrome => "chrome",
            DeviceKind::Firefox => "firefox",
            DeviceKind::Safari => "safari",
            DeviceKind::Edge => "edge",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == code.trim().to_ascii_lowercase())
    }

    /// Desktop shows the typed name on the phone; WhatsApp names browser
    /// devices itself, as "<Browser> (<OS>)".
    pub fn named(self) -> bool {
        self == DeviceKind::Desktop
    }

    /// How the phone's Linked devices list shows a device of this type.
    pub fn on_phone(self, name: &str) -> String {
        match self {
            DeviceKind::Desktop => name.to_owned(),
            DeviceKind::Chrome => format!("Google Chrome ({})", this_os()),
            other => format!("{} ({})", other.label(), this_os()),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DeviceKind::Desktop => "Desktop",
            DeviceKind::Chrome => "Chrome",
            DeviceKind::Firefox => "Firefox",
            DeviceKind::Safari => "Safari",
            DeviceKind::Edge => "Edge",
        }
    }
}

pub struct Registry {
    dir: PathBuf,
    pub entries: Vec<Entry>,
}

impl Registry {
    pub fn load(data_dir: &Path) -> Self {
        let dir = data_dir.join("sessions");
        let _ = fs::create_dir_all(&dir);
        let mut entries: Vec<Entry> = fs::read_to_string(data_dir.join("sessions.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|item| {
                let text = |key: &str| item.get(key).and_then(Value::as_str);
                let device = Device {
                    name: text("device_name").map(str::to_owned).unwrap_or_else(|| DEFAULT_DEVICE_NAME.to_owned()),
                    kind: text("device_type").and_then(DeviceKind::parse).unwrap_or(DeviceKind::Desktop),
                };
                Some(Entry { id: text("id")?.to_owned(), name: text("name")?.to_owned(), device })
            })
            .collect();
        // Device stores without an entry (created before the registry existed).
        if let Ok(files) = fs::read_dir(&dir) {
            let mut found: Vec<String> = files
                .flatten()
                .filter_map(|f| f.file_name().to_str()?.strip_suffix(".db").map(str::to_owned))
                .filter(|id| !entries.iter().any(|e| &e.id == id))
                .collect();
            found.sort();
            entries.extend(found.into_iter().map(|id| Entry { name: id.clone(), id, device: Device::default() }));
        }
        let registry = Self { dir, entries };
        registry.save();
        registry
    }

    fn save(&self) {
        let list: Vec<Value> = self
            .entries
            .iter()
            .map(|e| json!({ "id": e.id, "name": e.name, "device_name": e.device.name, "device_type": e.device.kind.code() }))
            .collect();
        let path = self.dir.with_file_name("sessions.json");
        let text = serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_owned());
        if let Err(e) = fs::write(&path, text) {
            log::error!("cannot save {}: {e}", path.display());
        }
    }

    pub fn add(&mut self, id: &str, name: &str, device: Device) {
        self.entries.push(Entry { id: id.to_owned(), name: name.to_owned(), device });
        self.save();
    }

    /// What the session sends when it pairs; the default for an unknown id.
    pub fn device(&self, id: &str) -> Device {
        self.entries.iter().find(|e| e.id == id).map(|e| e.device.clone()).unwrap_or_default()
    }

    /// Changes the linked-device name and type; `clean_name` checks the name first.
    pub fn set_device(&mut self, id: &str, device: Device) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.device = device;
            self.save();
        }
    }

    /// Gives a number a new name; `clean_name` checks it first.
    pub fn rename(&mut self, id: &str, name: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.name = name.to_owned();
            self.save();
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.entries.retain(|e| e.id != id);
        self.save();
        self.delete_store(id);
        let _ = fs::remove_file(self.lock_path(id));
    }

    /// Whether another process holds the session's lock right now.
    pub fn in_use(&self, id: &str) -> bool {
        matches!(self.lock(id), Ok(None))
    }

    pub fn db_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.db"))
    }

    fn lock_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.lock"))
    }

    /// Deletes the device store (keys), so the next start pairs from scratch.
    pub fn delete_store(&self, id: &str) {
        let db = self.db_path(id);
        for suffix in ["", "-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{suffix}", db.display()));
            let _ = fs::remove_file(path);
        }
    }

    /// One process per session: two connections with the same keys kick each
    /// other off (`StreamReplaced`). The lock is held while the File lives.
    pub fn lock(&self, id: &str) -> io::Result<Option<File>> {
        let file = File::create(self.lock_path(id))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

/// Longest name a number may have.
pub const NAME_MAX: usize = 40;

/// A number's name as typed: trimmed, one line, not empty, at most
/// `NAME_MAX` characters. The error is shown to the user.
pub fn clean_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(crate::i18n::tr("Name cannot be empty", "Nama tidak boleh kosong"));
    }
    if name.chars().count() > NAME_MAX {
        return Err(crate::i18n::tr("Name is at most 40 characters", "Nama maksimal 40 karakter"));
    }
    Ok(name)
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn only_desktop_shows_the_typed_name_on_the_phone() {
        assert_eq!(DeviceKind::Desktop.on_phone("komputer kantor"), "komputer kantor");
        assert_eq!(DeviceKind::Chrome.on_phone("komputer kantor"), format!("Google Chrome ({})", this_os()));
        assert_eq!(DeviceKind::Firefox.on_phone("x"), format!("Firefox ({})", this_os()));
        assert!(["Windows", "Mac OS", "Linux"].contains(&this_os()));
    }

    #[test]
    fn device_name_and_type_are_kept_per_session() {
        let dir = std::env::temp_dir().join(format!("warust-registry-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut registry = Registry::load(&dir);
        registry.add("s1", "CS", Device::default());
        registry.set_device("s1", Device { name: "Kasir".into(), kind: DeviceKind::Chrome });
        let again = Registry::load(&dir);
        assert_eq!(again.device("s1"), Device { name: "Kasir".into(), kind: DeviceKind::Chrome });
        // Older entries without the fields, and unknown ids, are "Warust" on Desktop.
        assert_eq!(again.device("nope"), Device::default());
        assert_eq!(DeviceKind::parse(" Firefox "), Some(DeviceKind::Firefox));
        assert_eq!(DeviceKind::parse("tv"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  CS   Utama \n"), Ok("CS Utama".to_owned()));
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"a".repeat(41)).is_err());
        assert_eq!(clean_name(&"é".repeat(40)).map(|n| n.chars().count()), Ok(40));
    }
}
