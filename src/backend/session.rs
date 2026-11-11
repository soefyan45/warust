//! One WhatsApp session: device store, bot, and its events turned into
//! [`SessionView`] updates and chat records. Built like the Phase 0 spike
//! (src/bin/spike.rs).

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use whatsapp_rust::PinDuration;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::*;
use whatsapp_rust::serde_json::{Value, json};
use whatsapp_rust::wacore::history_sync::{HistorySyncStream, MAX_DECOMPRESSED};
use whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info;
use whatsapp_rust::wacore::store::DevicePropsOverride;
use whatsapp_rust::wacore::types::message::AddressingMode;
use whatsapp_rust::wacore_binary::JidExt;
use whatsapp_rust::wacore::types::presence::{ChatActivity, ReceiptType};

use super::media;
use crate::i18n::tr;
use super::registry::Registry;
use super::store::{self, Delivery, Store};
use super::{Status, Updater};

/// What WhatsApp is told this device is, for the phone's Linked devices list.
fn platform(kind: super::registry::DeviceKind) -> wa::device_props::PlatformType {
    use super::registry::DeviceKind;
    use wa::device_props::PlatformType;
    match kind {
        DeviceKind::Desktop => PlatformType::DESKTOP,
        DeviceKind::Chrome => PlatformType::CHROME,
        DeviceKind::Firefox => PlatformType::FIREFOX,
        DeviceKind::Safari => PlatformType::SAFARI,
        DeviceKind::Edge => PlatformType::EDGE,
    }
}


/// Messages asked for per "load older" click.
const OLDER_BATCH: i32 = 50;

struct Outgoing {
    local_id: String,
    chat: String,
    body: Body,
}

/// What one queued message carries.
pub enum Body {
    Text(String),
    /// A photo or document; `caption` may be empty.
    File { path: PathBuf, caption: String },
    Sticker(PathBuf),
    /// A voice note recorded in the composer (Ogg Opus), sent push-to-talk.
    Voice(PathBuf),
    /// A comment or reaction on a contact's status, quoting it (sent to the
    /// poster's chat, as WhatsApp does).
    StatusReply { text: String, status: store::StatusPost },
    /// Text that mentions group members (`mentions` are their JIDs, the text
    /// holds `@<user part>` for each).
    Mention { text: String, mentions: Vec<String> },
    /// A text that quotes an earlier message of the same chat, and may
    /// mention group members like `Mention`.
    Reply { text: String, quote: ReplyQuote, mentions: Vec<String> },
    /// A forwarded text message (stage 1: no media).
    Forward(String),
    /// A contact card: the name and the phone number (digits only) (#71).
    Contact { name: String, number: String },
    /// A pin or unpin (`None`). Not a chat message: it shows no bubble, is
    /// never stored as queued, and so is lost if the app closes before it goes out.
    Pin { target: PinTarget, length: Option<store::PinLength> },
    /// Delete a message for everyone (our own messages only), or for me (this
    /// account's devices). Like a pin it shows no bubble of its own.
    Delete { target: PinTarget, everyone: bool },
    /// A new text for one of our own sent text messages (#73).
    Edit { target: PinTarget, text: String },
}

/// The message a `Body::Reply` quotes.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplyQuote {
    /// Id of the quoted message.
    pub id: String,
    /// Who wrote it, as WhatsApp sent it; `None` for our own message.
    pub participant: Option<String>,
    /// Its text or caption, for the quote box.
    pub text: String,
}

/// Builds the reply body for `text` quoting message `reply_to` (a real or
/// local id) of `chat`; `None` when that chat has no such message.
pub fn reply_body(book: &store::Book, chat: &str, text: String, reply_to: &str, mentions: Vec<String>) -> Option<Body> {
    let real = book.aliases.get(reply_to).map(String::as_str).unwrap_or(reply_to);
    let quoted = book.chats.iter().find(|c| c.jid == chat)?.messages.iter().find(|m| m.id == real || m.id == reply_to)?;
    Some(Body::Reply { text, quote: reply_quote(quoted), mentions })
}

fn reply_quote(m: &store::ChatMsg) -> ReplyQuote {
    ReplyQuote { id: m.id.clone(), participant: (!m.from_me).then(|| m.sender.clone()), text: m.text.clone() }
}

/// The message to pin or unpin. `sender` is its author as stored, `None` for
/// our own messages (groups need the author to address a message).
/// A message addressed for a pin or a delete.
pub struct PinTarget {
    pub chat: String,
    pub msg_id: String,
    pub sender: Option<String>,
}

pub struct Running {
    handle: BotHandle,
    client: Arc<Client>,
    queue: UnboundedSender<Outgoing>,
    avatars: UnboundedSender<String>,
    _lock: File,
}

impl Running {
    pub fn is_linked(&self) -> bool {
        self.client.pn().is_some()
    }

    pub async fn stop(self) {
        self.handle.shutdown().await;
    }

    /// Unlinks this device from the phone, then stops.
    pub async fn logout(self) {
        let report = self.client.logout().await;
        log::info!("logout: {report:?}");
        self.handle.shutdown().await;
    }

    /// Asks the phone for up to `OLDER_BATCH` messages before the given one; they
    /// arrive later as an on-demand `Event::HistorySync` and are stored like any history.
    pub fn fetch_older(&self, id: &str, chat: &str, msg_id: &str, from_me: bool, ts: i64) {
        let Ok(jid) = chat.parse::<Jid>() else {
            return;
        };
        let (client, id, msg_id) = (self.client.clone(), id.to_owned(), msg_id.to_owned());
        tokio::spawn(async move {
            // The field is named "ms" but takes Unix seconds (as whatsmeow sends).
            match client.fetch_message_history(&jid, &msg_id, from_me, ts, OLDER_BATCH).await {
                Ok(request) => log::info!("session {id}: asked the phone for older messages ({request})"),
                Err(e) => log::warn!("session {id}: older messages request failed: {e}"),
            }
        });
    }

    /// Downloads a received file in the background; progress and failures go
    /// to `Shared::downloads`, the finished file into the chat store.
    pub fn download(&self, id: &str, msg_id: String, media: Value, state: &Updater, store: Arc<Mutex<Store>>) {
        state.set_download(&msg_id, Some(None));
        let (client, id, state) = (self.client.clone(), id.to_owned(), state.clone());
        tokio::spawn(async move {
            let dir = media::dir(&state.data_dir, &id);
            match media::download(&client, &media, &dir, &msg_id).await {
                Ok(path) => {
                    state.record(&id, &store, &[store::file_record(&msg_id, &path)]);
                    state.set_download(&msg_id, None);
                }
                Err(e) => {
                    log::warn!("session {id}: download failed: {e}");
                    state.set_download(&msg_id, Some(Some(e)));
                }
            }
        });
    }

    /// Subscribes to the contact's presence, so their typing reaches us.
    pub fn watch_presence(&self, id: &str, chat: &str) {
        let Ok(jid) = chat.parse::<Jid>() else {
            return;
        };
        let (client, id) = (self.client.clone(), id.to_owned());
        tokio::spawn(async move {
            if let Err(e) = client.presence().subscribe(jid).await {
                log::debug!("session {id}: presence subscribe failed: {e}");
            }
        });
    }

    /// Tells the chat we are typing, or (with `false`) that we stopped. Best
    /// effort: a lost indicator only means the dots do not show.
    pub fn chat_state(&self, id: &str, chat: &str, typing: bool) {
        let Ok(jid) = chat.parse::<Jid>() else {
            return;
        };
        let (client, id) = (self.client.clone(), id.to_owned());
        tokio::spawn(async move {
            let sent = if typing { client.chatstate().send_composing(&jid).await } else { client.chatstate().send_paused(&jid).await };
            if let Err(e) = sent {
                log::debug!("session {id}: typing indicator not sent: {e}");
            }
        });
    }

    /// "Recording audio" (or stopped) for `chat`, as WhatsApp shows while a
    /// voice note is recorded.
    pub fn recording_state(&self, id: &str, chat: &str, on: bool) {
        let Ok(jid) = chat.parse::<Jid>() else {
            return;
        };
        let (client, id) = (self.client.clone(), id.to_owned());
        tokio::spawn(async move {
            let sent = if on { client.chatstate().send_recording(&jid).await } else { client.chatstate().send_paused(&jid).await };
            if let Err(e) = sent {
                log::debug!("session {id}: recording indicator not sent: {e}");
            }
        });
    }

    /// Sends the "seen" receipt for a contact's status update, as opening it
    /// on the phone does; the poster sees this number among the viewers.
    pub fn mark_status_seen(&self, id: &str, sender: &str, msg_id: String, state: &Updater, store: Arc<Mutex<Store>>) {
        let Ok(sender) = sender.parse::<Jid>() else {
            return;
        };
        let (client, id, state) = (self.client.clone(), id.to_owned(), state.clone());
        tokio::spawn(async move {
            match client.mark_as_read(&Jid::status_broadcast(), Some(&sender), &[msg_id.as_str()]).await {
                Ok(()) => state.record(&id, &store, &[store::status_seen_record(&msg_id)]),
                Err(e) => log::warn!("session {id}: status seen receipt failed: {e}"),
            }
        });
    }

    /// Fetches a group's members into `Shared::members` and teaches the book
    /// the LID <-> phone mappings it learns on the way, so `@lid` mentions
    /// resolve to names. Runs in the background; failures only log.
    pub fn group_members(&self, id: &str, chat: &str, state: &Updater, store: Arc<Mutex<Store>>) {
        let Ok(group) = chat.parse::<Jid>() else {
            return;
        };
        let (client, id, state) = (self.client.clone(), id.to_owned(), state.clone());
        tokio::spawn(async move {
            let mut meta = match client.groups().fetch_metadata(&group).await {
                Ok(meta) => meta,
                Err(e) => {
                    log::warn!("session {id}: group members failed: {e:?}");
                    return;
                }
            };
            client.groups().resolve_participant_addresses(&mut meta).await;
            let own = own_users(&client);
            let book = state.book(&id);
            let mut members = Vec::new();
            let mut records = Vec::new();
            for p in &meta.participants {
                let phone = p.phone_number.clone().or_else(|| (!p.jid.is_lid()).then(|| p.jid.clone()));
                let lid = p.lid.clone().or_else(|| p.jid.is_lid().then(|| p.jid.clone()));
                let is_own = [Some(&p.jid), phone.as_ref(), lid.as_ref()].into_iter().flatten().any(|j| own.iter().any(|o| o == user_part(&j.to_string())));
                if is_own {
                    continue;
                }
                let (jid, phone) = (store::bare(&p.jid.to_string()), phone.map(|j| store::bare(&j.to_string())));
                if let (Some(lid), Some(pn)) = (&lid, &phone) {
                    let lid = store::bare(&lid.to_string());
                    // Only what the book does not know yet.
                    if book.as_ref().is_some_and(|b| b.normalize(&lid) == lid) {
                        records.push(store::lid_record(&lid, pn));
                    }
                }
                members.push(super::Member { jid, phone });
            }
            state.record(&id, &store, &records);
            state.set_members(&id, &group.to_string(), members);
        });
    }

    /// Read receipts (blue ticks) for incoming messages, given as
    /// `(sender JID, message id)`, plus the app-state "read" flag so the phone
    /// clears (or sets) its badge. `receipts` is false for "mark unread".
    pub fn mark_chat_read(&self, id: &str, chat: &str, messages: Vec<(String, String)>, read: bool, receipts: bool) {
        let Ok(chat) = chat.parse::<Jid>() else {
            return;
        };
        let group = chat.to_string().ends_with("@g.us");
        let (client, id) = (self.client.clone(), id.to_owned());
        tokio::spawn(async move {
            if receipts && read {
                // One receipt per sender: in a group each member is its own author.
                let mut by_sender: Vec<(Option<Jid>, Vec<String>)> = Vec::new();
                for (sender, msg_id) in messages {
                    let sender = group.then(|| sender.parse::<Jid>().ok()).flatten();
                    match by_sender.iter_mut().find(|(s, _)| *s == sender) {
                        Some((_, ids)) => ids.push(msg_id),
                        None => by_sender.push((sender, vec![msg_id])),
                    }
                }
                for (sender, ids) in by_sender {
                    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
                    if let Err(e) = client.mark_as_read(&chat, sender.as_ref(), &ids).await {
                        log::warn!("session {id}: read receipt failed: {e}");
                    }
                }
            }
            if let Err(e) = client.chat_actions().mark_chat_as_read(&chat, read, None).await {
                log::warn!("session {id}: mark chat as read failed: {e}");
            }
        });
    }

    /// Archives or unarchives a chat on the phone's app state.
    pub fn set_archived(&self, id: &str, chat: &str, archived: bool) {
        let Ok(chat) = chat.parse::<Jid>() else {
            return;
        };
        let (client, id) = (self.client.clone(), id.to_owned());
        tokio::spawn(async move {
            let actions = client.chat_actions();
            let result = if archived { actions.archive_chat(&chat, None).await } else { actions.unarchive_chat(&chat, None).await };
            if let Err(e) = result {
                log::warn!("session {id}: archive sync failed: {e}");
            }
        });
    }

    /// Queues a pin (`Some(length)`) or unpin (`None`) for everyone. It waits
    /// in the same paced queue as messages, so pinning in a row keeps the
    /// anti-ban gap. The chat store only changes once WhatsApp accepted it.
    pub fn pin(&self, target: PinTarget, length: Option<store::PinLength>) {
        let chat = target.chat.clone();
        let _ = self.queue.send(Outgoing { local_id: String::new(), chat, body: Body::Pin { target, length } });
    }

    /// Deletes a message. For me it leaves this chat at once and the phone
    /// follows through the app state; for everyone it changes once WhatsApp
    /// took the revoke. Both wait in the paced queue like a send.
    pub fn delete(&self, id: &str, target: PinTarget, everyone: bool, state: &Updater, store: &Mutex<Store>) {
        if !everyone {
            remove_media_file(state, id, &target.msg_id);
            state.record(id, store, &[store::hidden_record(&target.msg_id)]);
            // A message that never went out is only here: nothing to tell the phone.
            if target.msg_id.starts_with("local-") {
                return;
            }
        }
        let chat = target.chat.clone();
        let _ = self.queue.send(Outgoing { local_id: String::new(), chat, body: Body::Delete { target, everyone } });
    }

    /// Queues a new text for one of our own messages; the bubble changes once
    /// WhatsApp took the edit.
    pub fn edit(&self, target: PinTarget, text: String) {
        let chat = target.chat.clone();
        let _ = self.queue.send(Outgoing { local_id: String::new(), chat, body: Body::Edit { target, text } });
    }

    /// The WhatsApp client, for calls placed from a chat.
    pub fn client(&self) -> Arc<Client> {
        self.client.clone()
    }

    pub fn request_avatar(&self, jid: String) {
        let _ = self.avatars.send(jid);
    }

    /// Shows the message as queued right away; the send worker sends it later.
    pub fn enqueue(&self, id: &str, chat: String, body: Body, state: &Updater, store: &Mutex<Store>) {
        self.enqueue_as(id, chat, body, format!("local-{}", unix_nanos()), state, store);
    }

    /// Like `enqueue`, under an id the caller already handed out (the REST API).
    pub fn enqueue_as(&self, id: &str, chat: String, body: Body, local_id: String, state: &Updater, store: &Mutex<Store>) {
        if matches!(body, Body::Pin { .. } | Body::Delete { .. } | Body::Edit { .. }) {
            return log::warn!("session {id}: pins, deletes and edits go through `pin`, `delete` and `edit`, not the message queue");
        }
        let shown = match &body {
            Body::Text(text) => text.clone(),
            Body::File { path, caption } => media::label(path, caption),
            Body::Sticker(_) => "[Stiker]".to_owned(),
            Body::Voice(_) => "[Pesan suara]".to_owned(),
            Body::Contact { name, .. } => format!("[Kontak] {name}"),
            Body::StatusReply { text, .. } | Body::Mention { text, .. } | Body::Reply { text, .. } | Body::Forward(text) => text.clone(),
            Body::Pin { .. } | Body::Delete { .. } | Body::Edit { .. } => unreachable!("returned above"),
        };
        let media = match &body {
            Body::Text(_) | Body::StatusReply { .. } | Body::Mention { .. } | Body::Reply { .. } | Body::Forward(_) | Body::Contact { .. } => None,
            Body::File { path, .. } => Some(media::describe_sent(path)),
            Body::Sticker(path) => Some(media::describe_sent_sticker(path)),
            Body::Voice(path) => Some(media::describe_sent_voice(path)),
            Body::Pin { .. } | Body::Delete { .. } | Body::Edit { .. } => unreachable!("returned above"),
        };
        let record = store::msg_record(&chat, &local_id, true, "", &shown, unix_secs(), &Delivery::Queued, false);
        let mut record = store::with_media(record, media);
        record = store::with_forwarded(record, matches!(body, Body::Forward(_)));
        if let Body::Mention { mentions, .. } | Body::Reply { mentions, .. } = &body {
            record = store::with_mentions(record, mentions, false);
        }
        if let Body::StatusReply { status, .. } = &body {
            let text = if status.media.is_some() && status.caption().is_empty() { status.text.clone() } else { status.caption() };
            record = store::with_quote(record, &store::Quote { id: status.id.clone(), text, status: true });
        }
        if let Body::Reply { quote, .. } = &body {
            record = store::with_quote(record, &store::Quote { id: quote.id.clone(), text: quote.text.clone(), status: false });
        }
        state.record(id, store, &[record]);
        state.update(id, |v| v.queue += 1);
        let _ = self.queue.send(Outgoing { local_id, chat, body });
    }

    /// Puts a message that failed back in the send queue, under its own id,
    /// the way queued messages go out again after a restart.
    pub fn retry(&self, id: &str, msg_id: &str, state: &Updater, store: &Mutex<Store>) {
        let Some(book) = state.book(id) else { return };
        let found = book.chats.iter().find_map(|c| c.messages.iter().find(|m| m.id == msg_id).map(|m| (c.jid.clone(), m)));
        let Some((chat, m)) = found.filter(|(_, m)| m.from_me && matches!(m.status, Delivery::Failed(_))) else {
            return log::warn!("session {id}: retry: no failed message {msg_id}");
        };
        let Some(body) = queued_body(&book, m) else {
            let why = tr("Cannot be resent: the file or the original status is gone", "Tidak bisa dikirim ulang: file atau status aslinya sudah tidak ada");
            return state.record(id, store, &[store::status_record(msg_id, &Delivery::Failed(why.to_owned()))]);
        };
        state.record(id, store, &[store::status_record(msg_id, &Delivery::Queued)]);
        state.update(id, |v| v.queue += 1);
        let _ = self.queue.send(Outgoing { local_id: msg_id.to_owned(), chat, body });
    }
}

fn unix_nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn unix_secs() -> i64 {
    (unix_nanos() / 1_000_000_000) as i64
}

/// Messages queued longer than this when the app closed are not sent later.
const QUEUE_LIFETIME: i64 = 24 * 3600;

/// Messages still queued when the app last closed go back into the queue, in
/// their old order, once the number connects. Ones older than a day, or whose
/// file or status is gone, are marked failed so nothing stale goes out.
fn restore_queue(id: &str, queue: &UnboundedSender<Outgoing>, state: &Updater, store: &Mutex<Store>) {
    let Some(book) = state.book(id) else {
        return;
    };
    let now = unix_secs();
    let mut pending = Vec::new();
    let mut failed = Vec::new();
    for chat in &book.chats {
        for m in chat.messages.iter().filter(|m| m.from_me && matches!(m.status, Delivery::Queued)) {
            let why = if now - m.ts > QUEUE_LIFETIME {
                Some(tr("Expired: still queued for more than 24 hours when the app was closed", "Kedaluwarsa: masih antre lebih dari 24 jam saat aplikasi ditutup"))
            } else {
                match queued_body(&book, m) {
                    Some(body) => {
                        pending.push((m.ts, Outgoing { local_id: m.id.clone(), chat: chat.jid.clone(), body }));
                        None
                    }
                    None => Some(tr("Cannot be resent: the file or the original status is gone", "Tidak bisa dikirim ulang: file atau status aslinya sudah tidak ada")),
                }
            };
            if let Some(why) = why {
                failed.push(store::status_record(&m.id, &Delivery::Failed(why.to_owned())));
            }
        }
    }
    if !failed.is_empty() {
        state.record(id, store, &failed);
    }
    if pending.is_empty() {
        return;
    }
    log::info!("session {id}: {} queued message(s) from the last run go out again", pending.len());
    pending.sort_by_key(|(ts, _)| *ts);
    state.update(id, |v| v.queue += pending.len());
    for (_, message) in pending {
        let _ = queue.send(message);
    }
}

/// What a stored queued message sends: its text (with mentions), file, sticker, or a reply to a status or message.
fn queued_body(book: &store::Book, m: &store::ChatMsg) -> Option<Body> {
    if let Some(quote) = m.quote.as_ref().filter(|q| q.status) {
        let status = book.statuses.iter().find(|s| s.id == quote.id)?.clone();
        return Some(Body::StatusReply { text: m.text.clone(), status });
    }
    let Some(media) = &m.media else {
        if m.forwarded {
            return Some(Body::Forward(m.text.clone()));
        }
        if let Some(quote) = &m.quote {
            // A reply: the quoted message must still be in the book (it gives the participant).
            let real = book.aliases.get(&quote.id).unwrap_or(&quote.id);
            let quoted = book.chats.iter().flat_map(|c| &c.messages).find(|q| q.id == *real || q.id == quote.id)?;
            return Some(Body::Reply { text: m.text.clone(), quote: reply_quote(quoted), mentions: m.mentions.clone() });
        }
        if !m.mentions.is_empty() {
            return Some(Body::Mention { text: m.text.clone(), mentions: m.mentions.clone() });
        }
        // A contact card keeps only its label here: it cannot go out again.
        if m.text.starts_with("[Kontak]") {
            return None;
        }
        return Some(Body::Text(m.text.clone()));
    };
    let path = media.file.clone().filter(|p| p.is_file())?;
    if media.kind == "sticker" {
        return Some(Body::Sticker(path));
    }
    if media.kind == "audio" {
        return Some(Body::Voice(path));
    }
    // The text is the label plus the caption ("[Gambar] caption").
    let label = media::label(&path, "");
    let caption = m.text.strip_prefix(&label).unwrap_or("").trim().to_owned();
    Some(Body::File { path, caption })
}

/// Opens the store and connects. Returns `None` when the session cannot run
/// here (locked by another process, or the store fails to open).
pub async fn start(id: &str, registry: &Registry, state: &Updater, chats: Arc<Mutex<Store>>) -> Option<Running> {
    let lock = match registry.lock(id) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            state.update(id, |v| v.status = Status::Locked);
            return None;
        }
        Err(e) => {
            state.update(id, |v| v.status = Status::Failed(crate::trf!("Session lock failed: {e}", "Kunci sesi gagal: {e}")));
            return None;
        }
    };
    let path = registry.db_path(id);
    let device = match SqliteStore::open(&path.to_string_lossy()).await {
        Ok(store) => store,
        Err(e) => {
            state.update(id, |v| v.status = Status::Failed(crate::trf!("Device store could not be opened: {e}", "Device store gagal dibuka: {e}")));
            return None;
        }
    };

    // Made before the bot, so its first "connected" can refill the queue.
    let (queue, outgoing) = unbounded_channel();
    let restored = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let events = (state.clone(), id.to_owned(), chats.clone(), queue.clone(), restored);
    // Groups whose subject was already asked for, so it is fetched once.
    let asked: Arc<Mutex<HashSet<String>>> = Arc::default();
    let messages = (state.clone(), id.to_owned(), chats.clone(), asked);
    // Sent while pairing: the name and type the phone lists this device under (#82).
    let shown = registry.device(id);
    let bot = Bot::builder()
        .with_backend(device)
        .with_device_props(
            DevicePropsOverride::new()
                .with_os(shown.name.as_str())
                .with_platform_type(platform(shown.kind)),
        )
        .on_event(move |event, client| {
            let (state, id, chats, queue, restored) = events.clone();
            async move { on_event(&state, &id, &chats, &queue, &restored, event, &client).await }
        })
        .on_message(move |ctx| {
            let (state, id, chats, asked) = messages.clone();
            async move { on_message(&state, &id, &chats, &asked, &ctx) }
        })
        .build()
        .await;
    match bot {
        Ok(bot) => {
            let handle = bot.spawn();
            let client = handle.client();
            tokio::spawn(send_worker(id.to_owned(), client.clone(), state.clone(), chats, outgoing));
            let (avatars, requests) = unbounded_channel();
            tokio::spawn(super::avatars::worker(client.clone(), state.data_dir.clone(), state.clone(), requests));
            Some(Running { handle, client, queue, avatars, _lock: lock })
        }
        Err(e) => {
            state.update(id, |v| v.status = Status::Failed(crate::trf!("Failed to start: {e}", "Gagal memulai: {e}")));
            None
        }
    }
}

/// What a message shows in the chat list and bubbles. `None` for messages
/// without anything to show (protocol messages, reactions, ...).
fn body(message: &wa::Message) -> Option<String> {
    if let Some(text) = message.text_content() {
        return Some(text.to_owned());
    }
    let base = message.get_base_message();
    let caption = |c: Option<&String>| c.filter(|c| !c.is_empty()).map(|c| format!(" {c}")).unwrap_or_default();
    if let Some(m) = base.image_message.as_option() {
        return Some(format!("[Gambar]{}", caption(m.caption.as_ref())));
    }
    if let Some(m) = base.video_message.as_option() {
        return Some(format!("[Video]{}", caption(m.caption.as_ref())));
    }
    if let Some(m) = base.document_message.as_option() {
        return Some(format!("[Dokumen] {}", m.file_name.clone().unwrap_or_default()));
    }
    if base.audio_message.as_option().is_some() {
        return Some("[Pesan suara]".to_owned());
    }
    if base.sticker_message.as_option().is_some() {
        return Some("[Stiker]".to_owned());
    }
    if base.location_message.as_option().is_some() {
        return Some("[Lokasi]".to_owned());
    }
    if let Some(card) = base.contact_message.as_option() {
        let name = card.display_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        return Some(name.map_or_else(|| "[Kontak]".to_owned(), |n| format!("[Kontak] {n}")));
    }
    None
}

/// JIDs the message mentions, from the first `ContextInfo` it carries
/// (extended text, or an image, video or document caption).
fn mentions_of(message: &wa::Message) -> Vec<String> {
    message.context_info().map(|c| c.mentioned_jid.clone()).unwrap_or_default()
}

/// User part of a JID, without device and server: `123:5@lid` -> `123`.
fn user_part(jid: &str) -> &str {
    let user = jid.split('@').next().unwrap_or(jid);
    user.split(':').next().unwrap_or(user)
}

/// This number's user parts: phone number and LID (either may be unknown).
/// Read from the device each time, so it is right before `Connected` too.
fn own_users(client: &Client) -> Vec<String> {
    [client.pn(), client.lid()].into_iter().flatten().map(|j| user_part(&j.to_string()).to_owned()).collect()
}

/// An incoming group message that mentions one of `own` (user parts).
fn mentions_me(group: bool, from_me: bool, mentions: &[String], own: &[String]) -> bool {
    group && !from_me && mentions.iter().any(|m| own.iter().any(|o| !o.is_empty() && o == user_part(m)))
}

/// The pin/unpin a `pinInChatMessage` stands for. The pin lasts for the
/// duration in its context info (7 days when absent), counted from `ts`.
fn pin_record_of(message: &wa::Message, chat: &str, ts: i64) -> Option<Value> {
    use wa::message::pin_in_chat_message::Type;
    let base = message.get_base_message();
    let pin = base.pin_in_chat_message.as_option()?;
    let target = pin.key.as_option()?.id.as_deref()?;
    match pin.r#type {
        Some(Type::PinForAll) => {
            let secs = base
                .message_context_info
                .as_option()
                .and_then(|c| c.message_add_on_duration_in_secs)
                .map_or(0, i64::from);
            Some(store::pin_record(chat, target, ts, store::PinLength::from_secs(secs).secs()))
        }
        Some(Type::UnpinForAll) => Some(store::unpin_record(chat, target)),
        _ => None,
    }
}

/// The id of the message an edit changes, and its new text or caption.
fn edited(message: &wa::Message) -> Option<(String, String)> {
    use wa::message::protocol_message::Type;
    let protocol = message.get_base_message().protocol_message.as_option()?;
    if protocol.r#type != Some(Type::MESSAGE_EDIT) {
        return None;
    }
    let target = protocol.key.as_option()?.id.clone().filter(|id| !id.is_empty())?;
    let new = protocol.edited_message.as_option()?;
    let text = new.text_content().or_else(|| new.get_caption())?.to_owned();
    Some((target, text))
}

/// An edit from the contact or from our phone: the bubble shows the new text.
fn on_edit(state: &Updater, id: &str, chats: &Arc<Mutex<Store>>, chat: &str, target: &str, text: &str) {
    let Some(book) = state.book(id) else { return };
    if !book.chats.iter().flat_map(|c| &c.messages).any(|m| m.id == target) {
        return log::info!("session {id}: edited message {target} is not here");
    }
    state.record(id, chats, &[store::edit_record(target, text)]);
    state.emit(id, "message.edited", json!({ "id": target, "chat": chat, "text": text }));
}

/// Removes the downloaded file of a message that is being deleted.
fn remove_media_file(state: &Updater, id: &str, msg_id: &str) {
    let file = state.book(id).and_then(|book| book.media_of(msg_id).and_then(|m| m.file.clone()));
    if let Some(file) = file {
        let _ = std::fs::remove_file(file);
    }
}

/// The id of the message a "delete for everyone" takes back.
fn revoked(message: &wa::Message) -> Option<String> {
    use wa::message::protocol_message::Type;
    let protocol = message.get_base_message().protocol_message.as_option()?;
    if protocol.r#type != Some(Type::REVOKE) {
        return None;
    }
    protocol.key.as_option()?.id.clone().filter(|id| !id.is_empty())
}

/// A message deleted for everyone: its downloaded file goes, the bubble
/// stays as "This message was deleted", and the webhook hears of it.
fn on_revoke(state: &Updater, id: &str, chats: &Arc<Mutex<Store>>, chat: &str, target: &str) {
    let Some(book) = state.book(id) else { return };
    if !book.chats.iter().flat_map(|c| &c.messages).any(|m| m.id == target) {
        return log::info!("session {id}: deleted message {target} is not here");
    }
    remove_media_file(state, id, target);
    state.record(id, chats, &[store::deleted_record(target)]);
    state.emit(id, "message.deleted", json!({ "id": target, "chat": chat }));
}

/// Background and text colour of a text status.
fn status_colors(message: &wa::Message) -> (Option<u32>, Option<u32>) {
    match message.get_base_message().extended_text_message.as_option() {
        Some(m) => (m.background_argb, m.text_argb),
        None => (None, None),
    }
}

/// A status update: stored apart from the chats, sent to the webhook.
fn on_status(state: &Updater, id: &str, chats: &Arc<Mutex<Store>>, ctx: &MessageContext) {
    let source = &ctx.info.source;
    // Revokes, reactions and other protocol messages have nothing to show.
    let Some(text) = body(&ctx.message) else {
        return;
    };
    let msg_id = ctx.info.id.to_string();
    let sender = source.sender.to_string();
    let mut records = Vec::new();
    let push_name = ctx.info.push_name.to_string();
    if !source.is_from_me && !push_name.is_empty() {
        records.push(store::push_record(&store::bare(&sender), &push_name));
    }
    let media = media::describe(&ctx.message, &media::dir(&state.data_dir, id), &msg_id);
    let summary = media.as_ref().map(|m| {
        json!({ "kind": m.get("k"), "name": m.get("name"), "mime": m.get("mime"), "size": m.get("size") })
    });
    let status = if source.is_from_me { Delivery::Sent } else { Delivery::Received };
    let ts = ctx.info.timestamp.timestamp();
    let record = store::msg_record(store::STATUS_CHAT, &msg_id, source.is_from_me, &sender, &text, ts, &status, false);
    let (background, color) = status_colors(&ctx.message);
    records.push(store::with_colors(store::with_media(record, media), background, color));
    state.record(id, chats, &records);
    if !source.is_from_me {
        state.emit(id, "status.received", json!({
            "id": msg_id,
            "from": store::bare(&sender),
            "push_name": (!push_name.is_empty()).then_some(&push_name),
            "text": text,
            "timestamp": ts,
            "media": summary,
        }));
    }
}

fn on_message(
    state: &Updater,
    id: &str,
    chats: &Arc<Mutex<Store>>,
    asked: &Mutex<HashSet<String>>,
    ctx: &MessageContext,
) {
    let source = &ctx.info.source;
    let chat = source.chat.to_string();
    if chat == store::STATUS_CHAT {
        on_status(state, id, chats, ctx);
        return;
    }
    if chat.starts_with("status@") {
        return;
    }
    // A message from the contact ends their typing.
    if !source.is_from_me {
        if let Some(book) = state.book(id) {
            state.set_typing(id, &book.normalize(&chat), None);
        }
    }
    // A pin or unpin (from the phone or the contact) updates the chat's pins;
    // it has no bubble of its own.
    if let Some(record) = pin_record_of(&ctx.message, &chat, ctx.info.timestamp.timestamp()) {
        state.record(id, chats, &[record]);
        return;
    }
    if let Some(target) = revoked(&ctx.message) {
        on_revoke(state, id, chats, &chat, &target);
        return;
    }
    if let Some((target, text)) = edited(&ctx.message) {
        on_edit(state, id, chats, &chat, &target, &text);
        return;
    }
    let Some(text) = body(&ctx.message) else {
        return;
    };
    let mut records: Vec<Value> = Vec::new();
    // A 1:1 chat addressed by LID tells us the phone number too.
    if chat.ends_with("@lid") && !source.is_group {
        let alt = if source.is_from_me { &source.recipient_alt } else { &source.sender_alt };
        if let Some(pn) = alt {
            records.push(store::lid_record(&chat, &pn.to_string()));
        }
    }
    let push_name = ctx.info.push_name.to_string();
    if !source.is_from_me && !push_name.is_empty() {
        records.push(store::push_record(&store::bare(&source.sender.to_string()), &push_name));
    }
    if source.is_group && state.book(id).is_some_and(|b| b.name_of(&chat).is_none()) {
        let first_time = asked.lock().map(|mut a| a.insert(chat.clone())).unwrap_or(false);
        if first_time {
            fetch_group_name(state, id, chats, ctx.client.clone(), source.chat.clone());
        }
    }
    let status = if source.is_from_me { Delivery::Sent } else { Delivery::Received };
    let msg_id = ctx.info.id.to_string();
    let media = media::describe(&ctx.message, &media::dir(&state.data_dir, id), &msg_id);
    let record = store::msg_record(
        &chat,
        &msg_id,
        source.is_from_me,
        &source.sender.to_string(),
        &text,
        ctx.info.timestamp.timestamp(),
        &status,
        !source.is_from_me,
    );
    let summary = media.as_ref().map(|m| {
        json!({ "kind": m.get("k"), "name": m.get("name"), "mime": m.get("mime"), "size": m.get("size") })
    });
    let mentions = mentions_of(&ctx.message);
    let mentioned = mentions_me(source.is_group, source.is_from_me, &mentions, &own_users(&ctx.client));
    let record = store::with_mentions(record, &mentions, mentioned);
    let record = store::with_forwarded(record, ctx.message.is_forwarded());
    records.push(store::with_media(record, media));
    state.record(id, chats, &records);
    if !source.is_from_me {
        state.emit(id, "message.received", json!({
            "id": msg_id,
            "chat": chat,
            "from": store::bare(&source.sender.to_string()),
            "push_name": (!push_name.is_empty()).then_some(&push_name),
            "is_group": source.is_group,
            "text": text,
            "timestamp": ctx.info.timestamp.timestamp(),
            "media": summary,
        }));
        let kind = summary.as_ref().and_then(|m| m["kind"].as_str()).unwrap_or("text").to_owned();
        let caption = if kind != "text" && text.starts_with('[') {
            text.split_once(']').map(|(_, rest)| rest.trim().to_owned()).unwrap_or_default()
        } else {
            text.clone()
        };
        let book = state.book(id);
        let sender = store::bare(&source.sender.to_string());
        let name = book
            .as_ref()
            .and_then(|b| b.name_of(&sender).map(str::to_owned))
            .unwrap_or_else(|| push_name.clone());
        let own_number = state.own_number(id).unwrap_or_default();
        super::webhook::fire_rules(state, &super::rules::Incoming {
            session: id.to_owned(),
            own_number,
            id: msg_id.clone(),
            chat: chat.clone(),
            sender_jid: sender,
            sender_name: name,
            text: caption,
            kind,
            is_group: source.is_group,
            timestamp: ctx.info.timestamp.timestamp(),
        });
        if let Some(book) = &book {
            let shown = book.with_mention_names(&text, &mentions);
            super::notify::incoming(state, id, book, &chat, &store::bare(&source.sender.to_string()), &push_name, &shown, source.is_group, mentioned);
        }
    }
    if !source.is_from_me {
        state.update(id, |v| {
            let now = Instant::now();
            v.incoming.push(now);
            v.last_activity = Some(now);
        });
    }
}

async fn on_event(
    state: &Updater,
    id: &str,
    chats: &Arc<Mutex<Store>>,
    queue: &UnboundedSender<Outgoing>,
    restored: &std::sync::atomic::AtomicBool,
    event: Arc<Event>,
    client: &Arc<Client>,
) {
    match &*event {
        Event::PairingQrCode(qr) => state.update(id, |v| {
            v.status = Status::Pairing;
            v.qr = Some((qr.code.clone(), Instant::now() + qr.timeout));
        }),
        Event::PairingCode(code) => state.update(id, |v| {
            v.status = Status::Pairing;
            v.pair_code = Some(code.code.clone());
        }),
        Event::PairingQrCodesExhausted(_) => state.update(id, |v| {
            v.qr = None;
            v.status = Status::Failed(tr("QR expired, link again", "QR kedaluwarsa, tautkan ulang").to_owned());
        }),
        Event::PairingCodeError(e) => state.update(id, |v| {
            v.pair_code = None;
            v.status = Status::Failed(crate::trf!("Pairing code failed: {e:?}", "Kode pairing gagal: {e:?}"));
        }),
        Event::Connected(_) => {
            fetch_group_names(state, id, chats, client.clone());
            let phone = client.pn().map(|jid| mask_phone(&jid.to_string()));
            let own_number = client.pn().map(|jid| store::bare(&jid.to_string()).split('@').next().unwrap_or_default().to_owned());
            let own_lid = client.lid().map(|jid| user_part(&jid.to_string()).to_owned());
            state.update(id, |v| {
                v.status = Status::Connected;
                v.qr = None;
                v.pair_code = None;
                v.last_activity = Some(Instant::now());
                if phone.is_some() {
                    v.phone = phone;
                    v.own_number = own_number;
                }
                if own_lid.is_some() {
                    v.own_lid = own_lid;
                }
            });
            // Once per start: what was still queued when the app closed.
            if !restored.swap(true, std::sync::atomic::Ordering::Relaxed) {
                restore_queue(id, queue, state, chats);
            }
        }
        Event::Disconnected(_) => state.update(id, |v| {
            if v.status == Status::Connected {
                v.status = Status::Reconnecting;
            }
        }),
        Event::ChatPresence(update) => {
            let chat = update.source.chat.to_string();
            if chat.starts_with("status@") {
                return;
            }
            let Some(book) = state.book(id) else {
                return;
            };
            let typing = match update.state {
                ChatActivity::Idle => None,
                activity => Some(super::Typing {
                    who: update.source.is_group.then(|| book.normalize(&update.source.sender.to_string())),
                    recording: activity == ChatActivity::RecordingAudio,
                    since: Instant::now(),
                }),
            };
            state.set_typing(id, &book.normalize(&chat), typing);
        }
        Event::IncomingCall(call) => super::call::on_incoming(state, id, client, chats, call),
        Event::MissedCall(missed) => super::call::on_missed(state, id, chats, missed),
        Event::CallEndedElsewhere(ended) => super::call::on_elsewhere(state, ended),
        Event::LoggedOut(_) => state.update(id, |v| v.status = Status::LoggedOut),
        Event::StreamReplaced(_) => state.update(id, |v| v.status = Status::Replaced),
        Event::TemporaryBan(ban) => {
            log::error!("session {id}: temporary ban {ban:?}");
            state.update(id, |v| v.status = Status::Banned(format!("{:?}", ban.code)));
        }
        Event::ConnectFailure(f) => {
            log::warn!("session {id}: connect failure {f:?}");
            state.update(id, |v| v.status = Status::Failed(format!("{:?}", f.reason)));
        }
        Event::Receipt(receipt) => {
            let status = match receipt.r#type {
                ReceiptType::Delivered => Delivery::Delivered,
                ReceiptType::Read | ReceiptType::Played => Delivery::Read,
                _ => return,
            };
            let records: Vec<Value> = receipt
                .message_ids
                .iter()
                .map(|m| store::status_record(&m.to_string(), &status))
                .collect();
            state.record(id, chats, &records);
            for m in &receipt.message_ids {
                state.emit(id, "message.status", json!({ "id": m.to_string(), "status": status.code() }));
            }
        }
        Event::ArchiveUpdate(update) => {
            let archived = update.action.archived.unwrap_or(true);
            state.record(id, chats, &[store::archive_record(&update.jid.to_string(), archived)]);
        }
        Event::MarkChatAsReadUpdate(update) => {
            let record = if update.action.read.unwrap_or(true) {
                store::seen_record(&update.jid.to_string())
            } else {
                store::unread_record(&update.jid.to_string())
            };
            state.record(id, chats, &[record]);
        }
        Event::HistorySync(lazy) => {
            let compressed = lazy.compressed_bytes().clone();
            // Decompressing and decoding can take a while; keep it off the event loop.
            let media_dir = media::dir(&state.data_dir, id);
            let own = own_users(client);
            let now = unix_secs();
            let records = tokio::task::spawn_blocking(move || history_records(&compressed, &media_dir, &own, now)).await;
            match records {
                Ok(Ok(records)) => {
                    log::info!("session {id}: stored {} history records", records.len());
                    state.record(id, chats, &records);
                }
                Ok(Err(e)) => log::warn!("session {id}: history chunk unreadable: {e}"),
                Err(e) => log::warn!("session {id}: history worker failed: {e}"),
            }
        }
        _ => {}
    }
}

/// Subjects of every group this number is in; asked once per connection.
fn fetch_group_names(state: &Updater, id: &str, chats: &Arc<Mutex<Store>>, client: Arc<Client>) {
    let (state, id, chats) = (state.clone(), id.to_owned(), chats.clone());
    tokio::spawn(async move {
        match client.groups().list_participating().await {
            Ok(groups) => {
                let records: Vec<Value> = groups
                    .iter()
                    .filter_map(|g| Some(store::name_record(&g.id.to_string(), g.subject.as_ref()?)))
                    .collect();
                state.record(&id, &chats, &records);
            }
            Err(e) => log::warn!("session {id}: group list failed: {e:?}"),
        }
    });
}

/// Subject of a group seen for the first time.
fn fetch_group_name(state: &Updater, id: &str, chats: &Arc<Mutex<Store>>, client: Arc<Client>, group: Jid) {
    let (state, id, chats) = (state.clone(), id.to_owned(), chats.clone());
    tokio::spawn(async move {
        match client.groups().fetch_metadata(&group).await {
            Ok(meta) => {
                if let Some(subject) = meta.subject {
                    state.record(&id, &chats, &[store::name_record(&group.to_string(), &subject)]);
                }
            }
            Err(e) => log::warn!("session {id}: group metadata failed: {e:?}"),
        }
    });
}

/// Mentions in history older than this are not "waiting for a reply".
const HISTORY_MENTION_WINDOW: i64 = 24 * 3600;

/// Whether a history message counts as a pending mention of us: a live
/// mention, and recent enough that a reply is still expected.
fn history_mentions_me(group: bool, from_me: bool, mentions: &[String], own: &[String], ts: i64, now: i64) -> bool {
    now - ts <= HISTORY_MENTION_WINDOW && mentions_me(group, from_me, mentions, own)
}

/// Turns one history-sync chunk into chat records: LID mappings, push names,
/// then conversations (their own names win over push names).
fn history_records(compressed: &[u8], media_dir: &Path, own: &[String], now: i64) -> Result<Vec<Value>, String> {
    let mut stream = HistorySyncStream::new(compressed, MAX_DECOMPRESSED);
    let mut conversations = Vec::new();
    while let Some(conversation) = stream.next_conversation().map_err(|e| e.to_string())? {
        conversations.push(conversation);
    }
    let remainder = stream.remainder().map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    for m in &remainder.phone_number_to_lid_mappings {
        if let (Some(lid), Some(pn)) = (&m.lid_jid, &m.pn_jid) {
            records.push(store::lid_record(lid, pn));
        }
    }
    for p in &remainder.pushnames {
        if let (Some(jid), Some(name)) = (&p.id, &p.pushname) {
            records.push(store::push_record(jid, name));
        }
    }
    for conversation in conversations {
        let chat = conversation.id.clone();
        if chat.starts_with("status@") && chat != store::STATUS_CHAT {
            continue;
        }
        if let Some(name) = conversation.name.as_ref().filter(|n| !n.is_empty()) {
            records.push(store::name_record(&chat, name));
        }
        for entry in &conversation.messages {
            let Some(info) = entry.message.as_option() else { continue };
            let Some(key) = info.key.as_option() else { continue };
            let Some(msg_id) = key.id.as_ref().filter(|i| !i.is_empty()) else { continue };
            let Some(message) = info.message.as_option() else { continue };
            let Some(text) = body(message) else { continue };
            let from_me = key.from_me.unwrap_or(false);
            let sender = key
                .participant
                .clone()
                .or_else(|| info.participant.clone())
                .unwrap_or_else(|| chat.clone());
            let status = if from_me { Delivery::Sent } else { Delivery::Received };
            let ts = info.message_timestamp.unwrap_or(0) as i64;
            let record = store::msg_record(&chat, msg_id, from_me, &sender, &text, ts, &status, false);
            let mentions = mentions_of(message);
            let record = store::with_mentions(record, &mentions, history_mentions_me(chat.ends_with("@g.us"), from_me, &mentions, own, ts, now));
            let record = store::with_media(record, media::describe(message, media_dir, msg_id));
            let record = store::with_forwarded(record, message.is_forwarded());
            let (background, color) = status_colors(message);
            records.push(store::with_colors(record, background, color));
        }
    }
    Ok(records)
}

/// Sends queued messages one by one with the anti-ban gap between them.
async fn send_worker(
    id: String,
    client: Arc<Client>,
    state: Updater,
    chats: Arc<Mutex<Store>>,
    mut outgoing: UnboundedReceiver<Outgoing>,
) {
    let mut last: Option<Instant> = None;
    while let Some(message) = outgoing.recv().await {
        // Anti-ban pacing: a minimum gap per number plus random jitter,
        // both from the settings.
        if let Some(last) = last {
            let sending = state.settings().sending.clone();
            let jitter_ms = sending.jitter().as_millis().max(1);
            let gap = sending.gap() + Duration::from_millis((unix_nanos() % jitter_ms) as u64);
            if last.elapsed() < gap {
                tokio::time::sleep(gap - last.elapsed()).await;
            }
        }
        let message = match message {
            Outgoing { body: Body::Pin { target, length }, .. } => {
                send_pin(&id, &client, &state, &chats, target, length).await;
                last = Some(Instant::now());
                continue;
            }
            Outgoing { body: Body::Delete { target, everyone }, .. } => {
                send_delete(&id, &client, &state, &chats, target, everyone).await;
                last = Some(Instant::now());
                continue;
            }
            Outgoing { body: Body::Edit { target, text }, .. } => {
                send_edit(&id, &client, &state, &chats, target, text).await;
                last = Some(Instant::now());
                continue;
            }
            other => other,
        };
        let outcome: Result<String, String> = match message.chat.parse::<Jid>() {
            Err(e) => Err(crate::trf!("Invalid JID: {e:?}", "JID tidak valid: {e:?}")),
            Ok(jid) => match prepare(&client, &jid, message.body).await {
                Err(e) => {
                    log::warn!("session {id}: file not sent: {e}");
                    Err(e)
                }
                Ok(prepared) => match client.send_message(&jid, prepared).await {
                    Ok(result) => Ok(result.message_id.to_string()),
                    Err(e) => {
                        log::warn!("session {id}: send failed: {e}");
                        Err(e.to_string())
                    }
                },
            },
        };
        let records = match &outcome {
            Ok(real_id) => {
                state.update(&id, |v| v.sent.push(Instant::now()));
                vec![
                    store::rekey_record(&message.local_id, real_id),
                    store::status_record(real_id, &Delivery::Sent),
                ]
            }
            Err(e) => vec![store::status_record(&message.local_id, &Delivery::Failed(e.clone()))],
        };
        state.record(&id, &chats, &records);
        let (msg_id, status, error) = match &outcome {
            Ok(real_id) => (real_id.as_str(), "sent", None),
            Err(e) => (message.local_id.as_str(), "failed", Some(e.as_str())),
        };
        state.emit(&id, "message.status", json!({
            "id": msg_id,
            "local_id": message.local_id,
            "chat": message.chat,
            "status": status,
            "error": error,
        }));
        state.update(&id, |v| v.queue = v.queue.saturating_sub(1));
        last = Some(Instant::now());
    }
}

/// Sends one queued pin or unpin and, once WhatsApp accepted it, records it.
async fn send_pin(id: &str, client: &Client, state: &Updater, chats: &Mutex<Store>, target: PinTarget, length: Option<store::PinLength>) {
    let Ok(chat_jid) = target.chat.parse::<Jid>() else {
        return;
    };
    let sender_jid = target.sender.as_deref().and_then(|s| s.parse::<Jid>().ok());
    let from_me = target.sender.is_none();
    let PinTarget { chat, msg_id, .. } = target;
    let addressed = MessageId::new(msg_id.as_str()).and_then(|mid| MessageRef::new(&chat_jid, mid, sender_jid.as_ref(), from_me));
    let addressed = match addressed {
        Ok(addressed) => addressed,
        Err(e) => return log::warn!("session {id}: cannot address message {msg_id} to pin: {e}"),
    };
    let sent = match length {
        Some(length) => {
            let duration = match length {
                store::PinLength::Hours24 => PinDuration::Hours24,
                store::PinLength::Days7 => PinDuration::Days7,
                store::PinLength::Days30 => PinDuration::Days30,
            };
            client.pin_message(&addressed, duration).await
        }
        None => client.unpin_message(&addressed).await,
    };
    match sent {
        Ok(_) => {
            let record = match length {
                Some(length) => store::pin_record(&chat, &msg_id, unix_secs(), length.secs()),
                None => store::unpin_record(&chat, &msg_id),
            };
            state.record(id, chats, &[record]);
        }
        Err(e) => log::warn!("session {id}: pin failed: {e}"),
    }
}

/// Sends a delete: a revoke for everyone, or the phone's "delete for me".
async fn send_delete(id: &str, client: &Client, state: &Updater, chats: &Mutex<Store>, target: PinTarget, everyone: bool) {
    let Ok(chat_jid) = target.chat.parse::<Jid>() else {
        return;
    };
    let sender_jid = target.sender.as_deref().and_then(|s| s.parse::<Jid>().ok());
    let from_me = target.sender.is_none();
    let PinTarget { chat, msg_id, .. } = target;
    let addressed = match MessageId::new(msg_id.as_str()).and_then(|mid| MessageRef::new(&chat_jid, mid, sender_jid.as_ref(), from_me)) {
        Ok(addressed) => addressed,
        Err(e) => return log::warn!("session {id}: cannot address message {msg_id} to delete: {e}"),
    };
    if !everyone {
        if let Err(e) = client.chat_actions().delete_message_for_me(&addressed, true, None).await {
            log::warn!("session {id}: delete for me sync failed: {e}");
        }
        return;
    }
    match client.revoke_message(&addressed).await {
        Ok(_) => {
            remove_media_file(state, id, &msg_id);
            state.record(id, chats, &[store::deleted_record(&msg_id)]);
            state.emit(id, "message.deleted", json!({ "id": msg_id, "chat": chat }));
        }
        Err(e) => log::warn!("session {id}: delete for everyone failed: {e}"),
    }
}

/// Sends a new text for one of our own messages.
async fn send_edit(id: &str, client: &Client, state: &Updater, chats: &Mutex<Store>, target: PinTarget, text: String) {
    let Ok(chat_jid) = target.chat.parse::<Jid>() else {
        return;
    };
    let PinTarget { chat, msg_id, .. } = target;
    let addressed = match MessageId::new(msg_id.as_str()).and_then(|mid| MessageRef::new(&chat_jid, mid, None, true)) {
        Ok(addressed) => addressed,
        Err(e) => return log::warn!("session {id}: cannot address message {msg_id} to edit: {e}"),
    };
    match client.edit_message(whatsapp_rust::EditRequest::new(addressed, wa::Message::text(text.clone()))).await {
        Ok(_) => {
            state.record(id, chats, &[store::edit_record(&msg_id, &text)]);
            state.emit(id, "message.edited", json!({ "id": msg_id, "chat": chat, "text": text }));
        }
        Err(e) => log::warn!("session {id}: edit failed: {e}"),
    }
}

/// A vCard 3.0 for one phone number, as WhatsApp sends contact cards; the
/// `waid` lets the card open the person's chat.
fn vcard(name: &str, number: &str) -> String {
    let name = name.replace(['\r', '\n'], " ").replace(';', "\\;").replace(',', "\\,");
    format!("BEGIN:VCARD\nVERSION:3.0\nN:;{name};;;\nFN:{name}\nTEL;type=CELL;type=VOICE;waid={number}:+{number}\nEND:VCARD")
}

/// Our own JID (no device part) as the author of a quoted message: the LID in
/// LID-addressed groups, else the phone number, like the crate's own add-on keys.
async fn own_author(client: &Client, chat: &Jid) -> Result<Jid, String> {
    let pn = client.pn().ok_or(tr("Number is not linked yet", "Nomor belum tertaut"))?;
    if chat.is_group()
        && let Ok(routing) = client.groups().routing_info(chat).await
        && routing.addressing_mode == AddressingMode::Lid
    {
        return Ok(client.lid().unwrap_or(pn).to_non_ad());
    }
    Ok(pn.to_non_ad())
}

async fn prepare(client: &Client, chat: &Jid, body: Body) -> Result<wa::Message, String> {
    match body {
        Body::Text(text) => Ok(wa::Message::text(text)),
        Body::Mention { text, mentions } => {
            let mut context = wa::ContextInfo::default();
            context.mentioned_jid = mentions;
            Ok(wa::Message::text_with_context(text, context))
        }
        Body::File { path, caption } => media::message(client, &path, &caption).await,
        Body::Sticker(path) => media::sticker_message(client, &path).await,
        Body::Voice(path) => media::voice_message(client, &path).await,
        Body::Contact { name, number } => {
            let mut card = wa::message::ContactMessage::default();
            card.display_name = Some(name.clone());
            card.vcard = Some(vcard(&name, &number));
            let mut message = wa::Message::default();
            message.contact_message = MessageField::some(card);
            Ok(message)
        }
        Body::StatusReply { text, status } => {
            let poster = status.sender.parse::<Jid>().map_err(|e| crate::trf!("Invalid status sender JID: {e:?}", "JID pengirim status tidak valid: {e:?}"))?;
            // remote_jid = status@broadcast marks the quote as a status, like WA Web.
            let context = build_quote_context_with_info(status.id.clone(), &poster, &Jid::status_broadcast(), chat, &quoted_status(&status));
            Ok(wa::Message::text_with_context(text, context))
        }
        Body::Reply { text, quote, mentions } => {
            let participant = match &quote.participant {
                Some(sender) => sender.parse::<Jid>().map_err(|e| crate::trf!("Invalid message sender JID: {e:?}", "JID pengirim pesan tidak valid: {e:?}"))?.to_non_ad(),
                None => own_author(client, chat).await?,
            };
            // Same chat: no remote_jid, like a reply in WA Web.
            let mut context = build_quote_context_with_info(quote.id, &participant, chat, chat, &wa::Message::text(quote.text));
            context.mentioned_jid = mentions;
            Ok(wa::Message::text_with_context(text, context))
        }
        // The original wa::Message is not kept, so the text is rebuilt with
        // the same markers `prepare_for_forward` sets (score 1: first hop).
        Body::Pin { .. } | Body::Delete { .. } | Body::Edit { .. } => Err("a pin, delete or edit is not a message".to_owned()),
        Body::Forward(text) => {
            let mut context = wa::ContextInfo::default();
            context.is_forwarded = Some(true);
            context.forwarding_score = Some(1);
            Ok(wa::Message::text_with_context(text, context))
        }
    }
}

/// The status as the quote shows it: its text with colours, or the photo's
/// or video's caption with the inline thumbnail. Enough for the quote box;
/// the update itself is not sent again.
fn quoted_status(post: &store::StatusPost) -> wa::Message {
    let mut message = wa::Message::default();
    let caption = Some(post.caption()).filter(|c| !c.is_empty());
    let thumb = post.media.as_ref().and_then(|m| m.thumb.as_ref()).and_then(|p| std::fs::read(p).ok());
    match post.media.as_ref().map(|m| m.kind.as_str()) {
        Some("image") => {
            let mut image = wa::message::ImageMessage::default();
            image.caption = caption;
            image.jpeg_thumbnail = thumb;
            image.mimetype = Some("image/jpeg".to_owned());
            message.image_message = MessageField::some(image);
        }
        Some("video") => {
            let mut video = wa::message::VideoMessage::default();
            video.caption = caption;
            video.jpeg_thumbnail = thumb;
            video.mimetype = Some("video/mp4".to_owned());
            message.video_message = MessageField::some(video);
        }
        _ => {
            let mut text = wa::message::ExtendedTextMessage::default();
            text.text = Some(post.text.clone());
            text.background_argb = post.background;
            text.text_argb = post.text_color;
            message.extended_text_message = MessageField::some(text);
        }
    }
    message
}

pub fn request_pair_code(running: &Running, id: &str, phone: String, state: &Updater) {
    let client = running.client.clone();
    let (state, id) = (state.clone(), id.to_owned());
    state.update(&id, |v| v.pair_code = None);
    tokio::spawn(async move {
        let options = PairCodeOptions {
            phone_number: phone,
            ..Default::default()
        };
        // Success also arrives as Event::PairingCode; set it here too in case.
        match client.pair_with_code(options).await {
            Ok(code) => state.update(&id, |v| v.pair_code = Some(code)),
            Err(e) => state.update(&id, |v| {
                v.status = Status::Failed(crate::trf!("Pairing code failed: {e:?}", "Kode pairing gagal: {e:?}"));
            }),
        }
    });
}

/// `628121234410@s.whatsapp.net` or `628121234410:5@s.whatsapp.net`
/// → `+62 812-****-4410`. Never show a full number in the UI.
pub fn mask_phone(jid: &str) -> String {
    let user = jid.split('@').next().unwrap_or(jid);
    let digits: String = user.split(':').next().unwrap_or(user).chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 9 {
        return format!("+{digits}");
    }
    format!("+{} {}-****-{}", &digits[..2], &digits[2..5], &digits[digits.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::{Body, HISTORY_MENTION_WINDOW, ReplyQuote, history_mentions_me, mask_phone, mentions_me, queued_body, reply_body, vcard};
    use crate::backend::store::{self, Delivery, Quote, Store};

    #[test]
    fn a_contact_card_is_a_vcard_with_the_whatsapp_id() {
        let card = vcard("Budi; Santoso, Jr", "6281234567890");
        assert!(card.starts_with("BEGIN:VCARD\nVERSION:3.0\n"));
        assert!(card.contains("FN:Budi\\; Santoso\\, Jr\n"));
        assert!(card.contains("TEL;type=CELL;type=VOICE;waid=6281234567890:+6281234567890\n"));
        assert!(card.ends_with("END:VCARD"));
    }

    /// A stored queued message comes back as what it was: text, file with
    /// its caption, a reply to a status, a voice note; a file that is gone
    /// does not.
    #[test]
    fn queued_messages_rebuild_their_body() {
        let dir = std::env::temp_dir().join(format!("warust-queue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("nota.pdf");
        std::fs::write(&doc, b"%PDF-1.4").unwrap();
        let note = dir.join("voice-1.ogg");
        std::fs::write(&note, b"OggS").unwrap();
        let chat = "628111000001@s.whatsapp.net";
        let q = |id: &str, text: &str| store::msg_record(chat, id, true, "", text, 100, &Delivery::Queued, false);
        let status = store::msg_record(store::STATUS_CHAT, "ST1", false, chat, "Promo", 90, &Delivery::Received, false);
        let records = [
            status,
            q("a", "halo"),
            store::with_media(q("b", "[Dokumen] nota.pdf tolong dicek"), Some(super::media::describe_sent(&doc))),
            store::with_media(q("c", "[Dokumen] hilang.pdf"), Some(super::media::describe_sent(&dir.join("hilang.pdf")))),
            store::with_quote(q("d", "👍"), &Quote { id: "ST1".to_owned(), text: "Promo".to_owned(), status: true }),
            store::with_forwarded(q("e", "info penting"), true),
            store::with_media(q("f", "[Pesan suara]"), Some(super::media::describe_sent_voice(&note))),
        ];
        let lines: String = records.iter().map(|r| format!("{r}\n")).collect();
        let path = dir.join("s.jsonl");
        std::fs::write(&path, lines).unwrap();
        let (_, book) = Store::open(&path);
        let body = |id: &str| queued_body(&book, book.chats[0].messages.iter().find(|m| m.id == id).unwrap());
        assert!(matches!(body("a"), Some(Body::Text(t)) if t == "halo"));
        assert!(matches!(body("b"), Some(Body::File { path, caption }) if path == doc && caption == "tolong dicek"));
        assert!(body("c").is_none());
        assert!(matches!(body("d"), Some(Body::StatusReply { text, status }) if text == "👍" && status.id == "ST1"));
        assert!(matches!(body("e"), Some(Body::Forward(t)) if t == "info penting"));
        assert!(matches!(body("f"), Some(Body::Voice(path)) if path == note));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reply keeps its quote through a restart; the participant is the
    /// sender of the quoted message, or `None` for our own.
    #[test]
    fn queued_replies_rebuild_their_quote() {
        let dir = std::env::temp_dir().join(format!("warust-reply-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let group = "120363000000000001@g.us";
        let them = "628111000002@s.whatsapp.net";
        let incoming = store::msg_record(group, "IN1", false, them, "kapan kirim?", 90, &Delivery::Received, false);
        let own = store::msg_record(group, "OUT1", true, "", "besok", 95, &Delivery::Sent, false);
        let reply = |id: &str, to: &str, text: &str| {
            let record = store::msg_record(group, id, true, "", text, 100, &Delivery::Queued, false);
            store::with_quote(record, &Quote { id: to.to_owned(), text: "x".to_owned(), status: false })
        };
        let records = [incoming, own, reply("r1", "IN1", "jam 5"), reply("r2", "OUT1", "ok"), reply("r3", "GONE", "?")];
        let lines: String = records.iter().map(|r| format!("{r}\n")).collect();
        let path = dir.join("s.jsonl");
        std::fs::write(&path, lines).unwrap();
        let (_, book) = Store::open(&path);
        let body = |id: &str| queued_body(&book, book.chats[0].messages.iter().find(|m| m.id == id).unwrap());
        let quote = |id: &str, participant: Option<&str>, text: &str| ReplyQuote { id: id.to_owned(), participant: participant.map(str::to_owned), text: text.to_owned() };
        assert!(matches!(body("r1"), Some(Body::Reply { text, quote: q, .. }) if text == "jam 5" && q == quote("IN1", Some(them), "kapan kirim?")));
        assert!(matches!(body("r2"), Some(Body::Reply { quote: q, .. }) if q == quote("OUT1", None, "besok")));
        assert!(body("r3").is_none());
        // The REST/UI path builds the same quote; an unknown id gives nothing.
        assert!(matches!(reply_body(&book, group, "hai".to_owned(), "IN1", Vec::new()), Some(Body::Reply { quote: q, .. }) if q == quote("IN1", Some(them), "kapan kirim?")));
        assert!(reply_body(&book, group, "hai".to_owned(), "NOPE", Vec::new()).is_none());
        assert!(reply_body(&book, "other@g.us", "hai".to_owned(), "IN1", Vec::new()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_history_mentions_are_not_pending() {
        let men = vec!["99@lid".to_owned()];
        let own = vec!["99".to_owned()];
        let now = 1_000_000;
        assert!(history_mentions_me(true, false, &men, &own, now - 60, now));
        assert!(history_mentions_me(true, false, &men, &own, now - HISTORY_MENTION_WINDOW, now));
        assert!(!history_mentions_me(true, false, &men, &own, now - HISTORY_MENTION_WINDOW - 1, now));
        assert!(!history_mentions_me(false, false, &men, &own, now, now), "not a group");
    }

    #[test]
    fn queued_mention_comes_back_as_mention() {
        let dir = std::env::temp_dir().join(format!("warust-mention-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chat = "123@g.us";
        let men = vec!["99@lid".to_owned(), "628111@s.whatsapp.net".to_owned()];
        let rec = store::with_mentions(store::msg_record(chat, "m", true, "", "@99 @628111 halo", 100, &Delivery::Queued, false), &men, false);
        let path = dir.join("s.jsonl");
        std::fs::write(&path, format!("{rec}\n")).unwrap();
        let (_, book) = Store::open(&path);
        let msg = &book.chats[0].messages[0];
        assert!(matches!(queued_body(&book, msg), Some(Body::Mention { text, mentions }) if text == "@99 @628111 halo" && mentions == men));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mentions_me_matches_number_or_lid_ignoring_device() {
        let own = vec!["628111".to_owned(), "99".to_owned()];
        let m = |j: &str| vec![j.to_owned()];
        assert!(mentions_me(true, false, &m("628111@s.whatsapp.net"), &own));
        assert!(mentions_me(true, false, &m("99:3@lid"), &own));
        assert!(!mentions_me(true, false, &m("55@lid"), &own));
        assert!(!mentions_me(false, false, &m("99@lid"), &own), "not a group");
        assert!(!mentions_me(true, true, &m("99@lid"), &own), "from us");
        assert!(!mentions_me(true, false, &m("@lid"), &["".to_owned()]));
    }

    #[test]
    fn masks_the_middle_of_a_number() {
        assert_eq!(mask_phone("6281212344410@s.whatsapp.net"), "+62 812-****-4410");
        assert_eq!(mask_phone("6285200001234:12@s.whatsapp.net"), "+62 852-****-1234");
        assert_eq!(mask_phone("12345"), "+12345");
    }
}
