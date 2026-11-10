//! One WhatsApp session: device store, bot, and its events turned into
//! [`SessionView`] updates and chat records. Built like the Phase 0 spike
//! (src/bin/spike.rs).

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::*;
use whatsapp_rust::serde_json::{Value, json};
use whatsapp_rust::wacore::history_sync::{HistorySyncStream, MAX_DECOMPRESSED};
use whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info;
use whatsapp_rust::wacore::store::DevicePropsOverride;
use whatsapp_rust::wacore::types::presence::{ChatActivity, ReceiptType};

use super::media;
use super::registry::Registry;
use super::store::{self, Delivery, Store};
use super::{Status, Updater};

/// Name shown in the phone's Linked devices list. Never the hostname, which
/// would leak the machine's name to every linked phone.
const DEVICE_NAME: &str = "Warust";


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
    /// A comment or reaction on a contact's status, quoting it (sent to the
    /// poster's chat, as WhatsApp does).
    StatusReply { text: String, status: store::StatusPost },
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

    pub fn request_avatar(&self, jid: String) {
        let _ = self.avatars.send(jid);
    }

    /// Shows the message as queued right away; the send worker sends it later.
    pub fn enqueue(&self, id: &str, chat: String, body: Body, state: &Updater, store: &Mutex<Store>) {
        self.enqueue_as(id, chat, body, format!("local-{}", unix_nanos()), state, store);
    }

    /// Like `enqueue`, under an id the caller already handed out (the REST API).
    pub fn enqueue_as(&self, id: &str, chat: String, body: Body, local_id: String, state: &Updater, store: &Mutex<Store>) {
        let shown = match &body {
            Body::Text(text) => text.clone(),
            Body::File { path, caption } => media::label(path, caption),
            Body::Sticker(_) => "[Stiker]".to_owned(),
            Body::StatusReply { text, .. } => text.clone(),
        };
        let media = match &body {
            Body::Text(_) | Body::StatusReply { .. } => None,
            Body::File { path, .. } => Some(media::describe_sent(path)),
            Body::Sticker(path) => Some(media::describe_sent_sticker(path)),
        };
        let record = store::msg_record(&chat, &local_id, true, "", &shown, unix_secs(), &Delivery::Queued, false);
        let mut record = store::with_media(record, media);
        if let Body::StatusReply { status, .. } = &body {
            let text = if status.media.is_some() && status.caption().is_empty() { status.text.clone() } else { status.caption() };
            record = store::with_quote(record, &store::Quote { id: status.id.clone(), text, status: true });
        }
        state.record(id, store, &[record]);
        state.update(id, |v| v.queue += 1);
        let _ = self.queue.send(Outgoing { local_id, chat, body });
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
                Some("Kedaluwarsa: masih antre lebih dari 24 jam saat aplikasi ditutup")
            } else {
                match queued_body(&book, m) {
                    Some(body) => {
                        pending.push((m.ts, Outgoing { local_id: m.id.clone(), chat: chat.jid.clone(), body }));
                        None
                    }
                    None => Some("Tidak bisa dikirim ulang: file atau status aslinya sudah tidak ada"),
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

/// What a stored queued message sends: its text, file, sticker or status reply.
fn queued_body(book: &store::Book, m: &store::ChatMsg) -> Option<Body> {
    if let Some(quote) = m.quote.as_ref().filter(|q| q.status) {
        let status = book.statuses.iter().find(|s| s.id == quote.id)?.clone();
        return Some(Body::StatusReply { text: m.text.clone(), status });
    }
    let Some(media) = &m.media else {
        return Some(Body::Text(m.text.clone()));
    };
    let path = media.file.clone().filter(|p| p.is_file())?;
    if media.kind == "sticker" {
        return Some(Body::Sticker(path));
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
            state.update(id, |v| v.status = Status::Failed(format!("Kunci sesi gagal: {e}")));
            return None;
        }
    };
    let path = registry.db_path(id);
    let device = match SqliteStore::open(&path.to_string_lossy()).await {
        Ok(store) => store,
        Err(e) => {
            state.update(id, |v| v.status = Status::Failed(format!("Device store gagal dibuka: {e}")));
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
    let bot = Bot::builder()
        .with_backend(device)
        .with_device_props(
            DevicePropsOverride::new()
                .with_os(DEVICE_NAME)
                .with_platform_type(wa::device_props::PlatformType::DESKTOP),
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
            state.update(id, |v| v.status = Status::Failed(format!("Gagal memulai: {e}")));
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
    if base.contact_message.as_option().is_some() {
        return Some("[Kontak]".to_owned());
    }
    None
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
            super::notify::incoming(state, id, book, &chat, &store::bare(&source.sender.to_string()), &push_name, &text, source.is_group);
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
            v.status = Status::Failed("QR kedaluwarsa, tautkan ulang".to_owned());
        }),
        Event::PairingCodeError(e) => state.update(id, |v| {
            v.pair_code = None;
            v.status = Status::Failed(format!("Kode pairing gagal: {e:?}"));
        }),
        Event::Connected(_) => {
            fetch_group_names(state, id, chats, client.clone());
            let phone = client.pn().map(|jid| mask_phone(&jid.to_string()));
            let own_number = client.pn().map(|jid| store::bare(&jid.to_string()).split('@').next().unwrap_or_default().to_owned());
            state.update(id, |v| {
                v.status = Status::Connected;
                v.qr = None;
                v.pair_code = None;
                v.last_activity = Some(Instant::now());
                if phone.is_some() {
                    v.phone = phone;
                    v.own_number = own_number;
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
        Event::HistorySync(lazy) => {
            let compressed = lazy.compressed_bytes().clone();
            // Decompressing and decoding can take a while; keep it off the event loop.
            let media_dir = media::dir(&state.data_dir, id);
            let records = tokio::task::spawn_blocking(move || history_records(&compressed, &media_dir)).await;
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

/// Turns one history-sync chunk into chat records: LID mappings, push names,
/// then conversations (their own names win over push names).
fn history_records(compressed: &[u8], media_dir: &Path) -> Result<Vec<Value>, String> {
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
            let record = store::with_media(record, media::describe(message, media_dir, msg_id));
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
        let outcome: Result<String, String> = match message.chat.parse::<Jid>() {
            Err(e) => Err(format!("JID tidak valid: {e:?}")),
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

async fn prepare(client: &Client, chat: &Jid, body: Body) -> Result<wa::Message, String> {
    match body {
        Body::Text(text) => Ok(wa::Message::text(text)),
        Body::File { path, caption } => media::message(client, &path, &caption).await,
        Body::Sticker(path) => media::sticker_message(client, &path).await,
        Body::StatusReply { text, status } => {
            let poster = status.sender.parse::<Jid>().map_err(|e| format!("JID pengirim status tidak valid: {e:?}"))?;
            // remote_jid = status@broadcast marks the quote as a status, like WA Web.
            let context = build_quote_context_with_info(status.id.clone(), &poster, &Jid::status_broadcast(), chat, &quoted_status(&status));
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
                v.status = Status::Failed(format!("Kode pairing gagal: {e:?}"));
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
    use super::{Body, mask_phone, queued_body};
    use crate::backend::store::{self, Delivery, Quote, Store};

    /// A stored queued message comes back as what it was: text, file with
    /// its caption, a reply to a status; a file that is gone does not.
    #[test]
    fn queued_messages_rebuild_their_body() {
        let dir = std::env::temp_dir().join(format!("warust-queue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("nota.pdf");
        std::fs::write(&doc, b"%PDF-1.4").unwrap();
        let chat = "628111000001@s.whatsapp.net";
        let q = |id: &str, text: &str| store::msg_record(chat, id, true, "", text, 100, &Delivery::Queued, false);
        let status = store::msg_record(store::STATUS_CHAT, "ST1", false, chat, "Promo", 90, &Delivery::Received, false);
        let records = [
            status,
            q("a", "halo"),
            store::with_media(q("b", "[Dokumen] nota.pdf tolong dicek"), Some(super::media::describe_sent(&doc))),
            store::with_media(q("c", "[Dokumen] hilang.pdf"), Some(super::media::describe_sent(&dir.join("hilang.pdf")))),
            store::with_quote(q("d", "👍"), &Quote { id: "ST1".to_owned(), text: "Promo".to_owned(), status: true }),
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
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn masks_the_middle_of_a_number() {
        assert_eq!(mask_phone("6281212344410@s.whatsapp.net"), "+62 812-****-4410");
        assert_eq!(mask_phone("6285200001234:12@s.whatsapp.net"), "+62 852-****-1234");
        assert_eq!(mask_phone("12345"), "+12345");
    }
}
