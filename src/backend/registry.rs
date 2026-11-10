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
                Some(Entry {
                    id: item.get("id")?.as_str()?.to_owned(),
                    name: item.get("name")?.as_str()?.to_owned(),
                })
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
            entries.extend(found.into_iter().map(|id| Entry { name: id.clone(), id }));
        }
        let registry = Self { dir, entries };
        registry.save();
        registry
    }

    fn save(&self) {
        let list: Vec<Value> = self
            .entries
            .iter()
            .map(|e| json!({ "id": e.id, "name": e.name }))
            .collect();
        let path = self.dir.with_file_name("sessions.json");
        let text = serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_owned());
        if let Err(e) = fs::write(&path, text) {
            log::error!("cannot save {}: {e}", path.display());
        }
    }

    pub fn add(&mut self, id: &str, name: &str) {
        self.entries.push(Entry { id: id.to_owned(), name: name.to_owned() });
        self.save();
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
        return Err("Nama tidak boleh kosong");
    }
    if name.chars().count() > NAME_MAX {
        return Err("Nama maksimal 40 karakter");
    }
    Ok(name)
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  CS   Utama \n"), Ok("CS Utama".to_owned()));
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"a".repeat(41)).is_err());
        assert_eq!(clean_name(&"é".repeat(40)).map(|n| n.chars().count()), Ok(40));
    }
}
