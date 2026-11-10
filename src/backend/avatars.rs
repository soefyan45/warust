//! Profile pictures (preview size) for the chats the UI is showing. Cached in
//! `data/avatars/` and asked from WhatsApp at most once a week per chat, one
//! request every `GAP`, so scrolling through hundreds of chats never bursts.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::mpsc::UnboundedReceiver;
use whatsapp_rust::{ProfilePictureRequest, ProfilePictureTarget, ProfilePictureType};
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::iq::contacts::ProfilePictureLookup;
use whatsapp_rust::wacore::net::HttpRequest;

use super::Updater;

const GAP: Duration = Duration::from_millis(1500);
const FRESH: Duration = Duration::from_secs(7 * 24 * 3600);

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("avatars")
}

fn file_stem(jid: &str) -> String {
    jid.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// The cached picture, `Some(None)` when the chat has none, `None` when unknown or stale.
fn cached(dir: &Path, jid: &str) -> Option<Option<PathBuf>> {
    let stem = file_stem(jid);
    let fresh = |path: &Path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < FRESH)
    };
    let picture = dir.join(format!("{stem}.jpg"));
    if fresh(&picture) {
        return Some(Some(picture));
    }
    let none = dir.join(format!("{stem}.none"));
    if fresh(&none) {
        return Some(None);
    }
    None
}

/// Serves avatar requests for one session until the session stops.
pub async fn worker(
    client: Arc<Client>,
    data_dir: PathBuf,
    state: Updater,
    mut requests: UnboundedReceiver<String>,
) {
    let dir = dir(&data_dir);
    let _ = fs::create_dir_all(&dir);
    let mut done: HashSet<String> = HashSet::new();
    while let Some(jid) = requests.recv().await {
        if !done.insert(jid.clone()) {
            continue;
        }
        if let Some(found) = cached(&dir, &jid) {
            state.set_avatar(&jid, found);
            continue;
        }
        let picture = fetch(&client, &jid).await;
        let stem = file_stem(&jid);
        let path = match picture {
            Some(bytes) => {
                let path = dir.join(format!("{stem}.jpg"));
                let _ = fs::remove_file(dir.join(format!("{stem}.none")));
                fs::write(&path, bytes).ok().map(|_| path)
            }
            None => {
                let _ = fs::write(dir.join(format!("{stem}.none")), b"");
                None
            }
        };
        state.set_avatar(&jid, path);
        tokio::time::sleep(GAP).await;
    }
}

async fn fetch(client: &Arc<Client>, jid: &str) -> Option<Vec<u8>> {
    let parsed: Jid = jid.parse().ok()?;
    let target = if jid.ends_with("@g.us") {
        ProfilePictureTarget::Group(&parsed)
    } else {
        ProfilePictureTarget::Contact(&parsed)
    };
    let request = ProfilePictureRequest::new(target, ProfilePictureType::Preview);
    match client.pictures().lookup(request).await {
        Ok(ProfilePictureLookup::Found(picture)) => {
            let response = client.http_client().execute(HttpRequest::get(picture.url)).await.ok()?;
            (response.status_code == 200 && !response.body.is_empty()).then_some(response.body)
        }
        Ok(_) => None,
        Err(e) => {
            log::debug!("profile picture for a chat failed: {e:?}");
            None
        }
    }
}
