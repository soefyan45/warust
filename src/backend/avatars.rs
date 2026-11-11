//! Profile pictures (preview size) for the chats the UI is showing. Cached in
//! `data/avatars/` and asked from WhatsApp at most once a week per chat (a
//! day when it has none), one request every `GAP`, so scrolling through
//! hundreds of chats never bursts. The newest request goes first, so the
//! rows on screen fill before those scrolled past; a lookup that failed is
//! tried again later instead of counting as "no picture" (#116).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::mpsc::UnboundedReceiver;
use whatsapp_rust::{ProfilePictureRequest, ProfilePictureTarget, ProfilePictureType};
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::iq::contacts::ProfilePictureLookup;
use whatsapp_rust::wacore::net::HttpRequest;

use super::Updater;

const GAP: Duration = Duration::from_millis(1500);
const FRESH: Duration = Duration::from_secs(7 * 24 * 3600);
/// "No picture" is asked again sooner: people add one.
const NONE_FRESH: Duration = Duration::from_secs(24 * 3600);
/// Waits before trying a failed lookup again, then the last one over and over.
const RETRY: [Duration; 4] = [Duration::from_secs(30), Duration::from_secs(120), Duration::from_secs(600), Duration::from_secs(1800)];
/// Written once the `.none` markers from before #116 are dropped: some
/// stood for a lookup that had only failed.
const NONE_RESET: &str = ".none-reset-116";

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("avatars")
}

fn file_stem(jid: &str) -> String {
    jid.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

fn younger_than(path: &Path, age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|a| a < age)
}

/// The cached picture, `Some(None)` when the chat has none, `None` when unknown or stale.
fn cached(dir: &Path, jid: &str) -> Option<Option<PathBuf>> {
    let stem = file_stem(jid);
    let picture = dir.join(format!("{stem}.jpg"));
    if younger_than(&picture, FRESH) {
        return Some(Some(picture));
    }
    if younger_than(&dir.join(format!("{stem}.none")), NONE_FRESH) {
        return Some(None);
    }
    None
}

/// Drops the `.none` markers written before #116, once per data folder.
fn reset_none_markers(dir: &Path) {
    let marker = dir.join(NONE_RESET);
    if marker.exists() {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "none") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let _ = fs::write(marker, b"");
}

/// What a lookup found.
enum Lookup {
    Picture(Vec<u8>),
    /// None set, or hidden from us by its owner's privacy.
    Nothing,
    /// Not connected, the network, or the download: try again later.
    Failed,
}

/// The requests waiting: the newest on top, and the failed ones with when
/// they may go again.
#[derive(Default)]
struct Queue {
    waiting: Vec<String>,
    later: Vec<(Instant, String)>,
}

impl Queue {
    /// A request moves to the top, also when it was already waiting.
    fn push(&mut self, jid: String) {
        self.waiting.retain(|j| j != &jid);
        self.waiting.push(jid);
    }

    /// Failed lookups whose wait is over go back under the fresh requests.
    fn wake(&mut self, now: Instant) {
        let (due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.later).into_iter().partition(|(at, _)| *at <= now);
        self.later = rest;
        for (_, jid) in due {
            if !self.waiting.contains(&jid) {
                self.waiting.insert(0, jid);
            }
        }
    }

    /// Waits longer after each failure (`tries` before this one).
    fn retry(&mut self, jid: String, tries: usize) {
        let wait = RETRY[tries.min(RETRY.len() - 1)];
        self.later.retain(|(_, j)| j != &jid);
        self.later.push((Instant::now() + wait, jid));
    }

    fn next_wake(&self) -> Option<Instant> {
        self.later.iter().map(|(at, _)| *at).min()
    }
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
    reset_none_markers(&dir);
    let mut done: HashSet<String> = HashSet::new();
    let mut queue = Queue::default();
    let mut tries: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    loop {
        // Everything asked meanwhile, so the newest goes first.
        while let Ok(jid) = requests.try_recv() {
            if !done.contains(&jid) {
                queue.push(jid);
            }
        }
        queue.wake(Instant::now());
        let Some(jid) = queue.waiting.pop() else {
            // Nothing to do: wait for a request, or for a retry to be due.
            let wait = queue.next_wake().map(|at| at.saturating_duration_since(Instant::now()));
            let next = match wait {
                Some(wait) => match tokio::time::timeout(wait, requests.recv()).await {
                    Ok(next) => next,
                    Err(_) => continue,
                },
                None => requests.recv().await,
            };
            match next {
                Some(jid) if !done.contains(&jid) => queue.push(jid),
                Some(_) => {}
                None => return,
            }
            continue;
        };
        if let Some(found) = cached(&dir, &jid) {
            done.insert(jid.clone());
            state.set_avatar(&jid, found);
            continue;
        }
        let stem = file_stem(&jid);
        match fetch(&client, &jid, ProfilePictureType::Preview).await {
            Lookup::Picture(bytes) => {
                let path = dir.join(format!("{stem}.jpg"));
                let _ = fs::remove_file(dir.join(format!("{stem}.none")));
                done.insert(jid.clone());
                tries.remove(&jid);
                state.set_avatar(&jid, fs::write(&path, bytes).ok().map(|_| path));
            }
            Lookup::Nothing => {
                let _ = fs::write(dir.join(format!("{stem}.none")), b"");
                done.insert(jid.clone());
                tries.remove(&jid);
                state.set_avatar(&jid, None);
            }
            Lookup::Failed => {
                let n = tries.entry(jid.clone()).or_default();
                queue.retry(jid, *n);
                *n += 1;
            }
        }
        tokio::time::sleep(GAP).await;
    }
}

/// The full-size picture of `jid` (#92), from the cache when fresh, else
/// from WhatsApp; `None` when it has none.
pub async fn full(client: &Arc<Client>, jid: &str) -> Option<PathBuf> {
    let dir = dir(crate::platform::data_dir());
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{}-full.jpg", file_stem(jid)));
    let fresh = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < FRESH);
    if fresh {
        return Some(path);
    }
    let Lookup::Picture(bytes) = fetch(client, jid, ProfilePictureType::Full).await else {
        return None;
    };
    fs::write(&path, bytes).ok().map(|_| path)
}

async fn fetch(client: &Arc<Client>, jid: &str, kind: ProfilePictureType) -> Lookup {
    let Ok(parsed) = jid.parse::<Jid>() else { return Lookup::Nothing };
    let target = if jid.ends_with("@g.us") {
        ProfilePictureTarget::Group(&parsed)
    } else {
        ProfilePictureTarget::Contact(&parsed)
    };
    let request = ProfilePictureRequest::new(target, kind);
    match client.pictures().lookup(request).await {
        Ok(ProfilePictureLookup::Found(picture)) => match client.http_client().execute(HttpRequest::get(picture.url)).await {
            Ok(response) if response.status_code == 200 && !response.body.is_empty() => Lookup::Picture(response.body),
            _ => Lookup::Failed,
        },
        Ok(ProfilePictureLookup::NotFound | ProfilePictureLookup::NotAuthorized) => Lookup::Nothing,
        // Only asked without a cached id, so "unchanged" means nothing came.
        Ok(_) => Lookup::Failed,
        Err(e) => {
            log::debug!("profile picture for a chat failed: {e:?}");
            Lookup::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_request_goes_first_and_failures_wait() {
        let mut q = Queue::default();
        q.push("a".into());
        q.push("b".into());
        q.push("a".into());
        assert_eq!(q.waiting, ["b", "a"]);
        assert_eq!(q.waiting.pop().as_deref(), Some("a"));
        q.retry("c".into(), 0);
        q.wake(Instant::now());
        assert_eq!(q.waiting, ["b"]);
        q.wake(Instant::now() + RETRY[0]);
        assert_eq!(q.waiting, ["c", "b"]);
        q.retry("d".into(), 9);
        assert_eq!(q.next_wake().map(|at| at > Instant::now() + RETRY[2]), Some(true));
    }
}
