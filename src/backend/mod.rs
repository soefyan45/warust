//! WhatsApp sessions, running in the same process as the UI on their own
//! tokio thread. The UI sends [`Command`]s and reads a
//! [`Shared`] snapshot every frame; the backend asks egui to repaint when the
//! snapshot changes.

mod api;
mod avatars;
mod media;
mod notify;
mod registry;
pub mod rules;
mod session;
pub mod settings;
pub mod store;
pub mod webhook;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use registry::Registry;
use session::{Body, Running};
pub use media::{MAX_BYTES as MAX_FILE_BYTES, make_sticker};
pub use session::mask_phone;
pub use settings::Settings;
pub use store::{Book, Chat, ChatMsg, Delivery, Media, StatusPost};
use store::Store;
use webhook::Delivery as HookDelivery;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Starting,
    /// Not linked yet: showing a QR code or a pair code.
    Pairing,
    Connected,
    Reconnecting,
    /// Unlinked from the phone, or logged out here.
    LoggedOut,
    Banned(String),
    /// Another connection with the same keys took over.
    Replaced,
    /// Another process holds this session.
    Locked,
    Failed(String),
}

#[derive(Clone)]
pub struct SessionView {
    pub id: String,
    pub name: String,
    /// Masked, e.g. `+62 812-****-4410`.
    pub phone: Option<String>,
    /// The full number, digits only; for triggers, never shown in the UI.
    pub own_number: Option<String>,
    pub status: Status,
    /// Current QR payload and when it rotates.
    pub qr: Option<(String, Instant)>,
    pub pair_code: Option<String>,
    pub incoming: Vec<Instant>,
    /// Messages sent from this app, for "sent today".
    pub sent: Vec<Instant>,
    /// Messages waiting in the send queue.
    pub queue: usize,
    pub last_activity: Option<Instant>,
    /// Chats and messages; shared with the UI without copying.
    pub book: Arc<Book>,
}

impl SessionView {
    pub(crate) fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            phone: None,
            own_number: None,
            status: Status::Starting,
            qr: None,
            pair_code: None,
            incoming: Vec::new(),
            sent: Vec::new(),
            queue: 0,
            last_activity: None,
            book: Arc::new(Book::default()),
        }
    }

    /// Messages sent from this app in the last 24 hours.
    pub fn sent_today(&self) -> usize {
        let day = Duration::from_secs(24 * 3600);
        self.sent.iter().filter(|t| t.elapsed() < day).count()
    }

    /// Incoming messages in the last 24 hours.
    pub fn incoming_today(&self) -> usize {
        let day = Duration::from_secs(24 * 3600);
        self.incoming.iter().filter(|t| t.elapsed() < day).count()
    }
}

#[derive(Clone, Default)]
pub struct Shared {
    pub sessions: Vec<SessionView>,
    /// Profile pictures by normalized JID: a cached file, or `None` for no picture.
    pub avatars: Arc<HashMap<String, Option<PathBuf>>>,
    /// Downloads by message id: `None` while running, else why it failed.
    /// Finished downloads leave the map and show as the message's file.
    pub downloads: Arc<HashMap<String, Option<String>>>,
    pub settings: Arc<Settings>,
    /// Latest webhook deliveries, newest first.
    pub webhooks: Arc<Vec<HookDelivery>>,
    pub api: ApiState,
    /// Trigger id → (local day, times it fired that day).
    pub rule_hits: Arc<HashMap<String, (i64, u32)>>,
    /// The chat (session id, JID) on screen in a focused window: it gets no
    /// notification.
    pub viewing: Option<(String, String)>,
    /// A clicked notification asks the UI to open this chat.
    pub open_chat: Option<(String, String)>,
    /// Contacts typing now, by `session|chat` (normalized JID).
    pub typing: Arc<HashMap<String, Typing>>,
}

/// A contact typing (or recording a voice note) in a chat.
#[derive(Clone, Debug)]
pub struct Typing {
    /// Who, in a group (normalized JID); `None` in a private chat.
    pub who: Option<String>,
    pub recording: bool,
    pub since: Instant,
}

impl Typing {
    /// WhatsApp repeats "typing" while it goes on and says "paused" when it
    /// stops; this long without news counts as stopped.
    pub const STALE: Duration = Duration::from_secs(25);

    pub fn key(session: &str, chat: &str) -> String {
        format!("{session}|{chat}")
    }

    pub fn fresh(&self) -> bool {
        self.since.elapsed() < Self::STALE
    }
}

/// The local REST API server.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ApiState {
    #[default]
    Off,
    Listening(String),
    Failed(String),
}

impl Status {
    /// Stable name for the API and webhooks, with the detail when there is one.
    pub fn code(&self) -> (&'static str, Option<&str>) {
        match self {
            Status::Starting => ("starting", None),
            Status::Pairing => ("pairing", None),
            Status::Connected => ("connected", None),
            Status::Reconnecting => ("reconnecting", None),
            Status::LoggedOut => ("logged_out", None),
            Status::Banned(d) => ("banned", Some(d)),
            Status::Replaced => ("replaced", None),
            Status::Locked => ("locked", None),
            Status::Failed(d) => ("failed", Some(d)),
        }
    }
}

pub enum Command {
    Add { id: String, name: String },
    /// Ask for an 8-character code for this phone number instead of the QR.
    PairCode { id: String, phone: String },
    /// Forget the keys and pair again.
    Relink { id: String },
    /// Unlink the device from the phone, keep the entry.
    Logout { id: String },
    /// Unlink (if linked) and delete the session.
    Remove { id: String },
    /// Queue a text message. `chat` is a JID from the session's book.
    Send { id: String, chat: String, text: String },
    /// Queue a photo or document with an optional caption.
    SendFile { id: String, chat: String, path: PathBuf, caption: String },
    /// The user opened a chat: clear its unread count.
    Seen { id: String, chat: String },
    /// Ask to be told when this contact types or comes online (as WhatsApp
    /// Web does when a chat is opened).
    WatchPresence { id: String, chat: String },
    /// Show (or stop showing) "sedang mengetik" in the chat on the contact's side.
    Typing { id: String, chat: String, typing: bool },
    /// "Tandai selesai" (or, with `done: false`, back to "Perlu balas").
    MarkDone { id: String, chat: String, done: bool },
    /// Ask the phone for messages older than the oldest one we have in `chat`.
    LoadOlder { id: String, chat: String },
    /// Fetch (or load from cache) the profile picture of a chat shown on screen.
    Avatar { id: String, jid: String },
    /// Queue a sticker (any picture; converted to 512×512 WebP when needed).
    SendSticker { id: String, chat: String, path: PathBuf },
    /// Download a received photo, video, sticker, voice note or document.
    Download { id: String, msg_id: String },
    /// Send the "seen" receipt for a status update shown in the viewer.
    StatusSeen { id: String, msg_id: String },
    /// Comment or react (one emoji) on a contact's status, from the number
    /// that saw it; goes to the poster's chat through the paced queue.
    StatusReply { id: String, msg_id: String, text: String },
    /// Queue a message from the REST API under an id chosen by the caller.
    Enqueue { id: String, chat: String, body: session::Body, local_id: String },
    /// Save new settings; restarts the API server when its part changed.
    SaveSettings(Settings),
    /// Send a `webhook.test` event to the global webhook URL.
    TestWebhook,
}

#[derive(Clone)]
pub struct Backend {
    commands: UnboundedSender<Command>,
    shared: Arc<Mutex<Shared>>,
}

impl Backend {
    pub fn start(ctx: egui::Context, data_dir: PathBuf) -> Self {
        let (commands, receiver) = unbounded_channel();
        let settings = Settings::load(&data_dir);
        let shared = Arc::new(Mutex::new(Shared { settings: Arc::new(settings), ..Shared::default() }));
        let (hooks, hook_events) = unbounded_channel();
        let state = Updater { shared: shared.clone(), ctx, data_dir: data_dir.clone(), hooks, commands: commands.clone() };
        let api_commands = commands.clone();
        std::thread::Builder::new()
            .name("whatsapp".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                runtime.spawn(webhook::dispatcher(hook_events, state.clone()));
                runtime.block_on(run(data_dir, state, receiver, api_commands));
            })
            .expect("whatsapp thread");
        Self { commands, shared }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn snapshot(&self) -> Shared {
        self.shared.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Tells the notifier which chat the user is looking at, if any.
    pub fn set_viewing(&self, viewing: Option<(String, String)>) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.viewing = viewing;
        }
    }

    /// The chat a clicked notification wants opened, once.
    pub fn take_open_chat(&self) -> Option<(String, String)> {
        self.shared.lock().ok()?.open_chat.take()
    }

    /// Closes the notifications still waiting for a click (on quit).
    pub fn close_notifications(&self) {
        notify::close_all();
    }

    /// A new, unique session id like `s1762591234`.
    pub fn new_id() -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("s{secs}")
    }
}

/// Writes into the shared snapshot and repaints the UI.
#[derive(Clone)]
pub(crate) struct Updater {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    pub(crate) data_dir: PathBuf,
    hooks: UnboundedSender<webhook::Event>,
    commands: UnboundedSender<Command>,
}

impl Updater {
    pub(crate) fn update(&self, id: &str, change: impl FnOnce(&mut SessionView)) {
        let mut changed = None;
        if let Ok(mut shared) = self.shared.lock() {
            if let Some(view) = shared.sessions.iter_mut().find(|s| s.id == id) {
                let before = view.status.clone();
                change(view);
                if view.status != before {
                    changed = Some(view.status.clone());
                }
            }
        }
        if let Some(status) = changed {
            let (code, detail) = status.code();
            self.emit(id, "session.status", whatsapp_rust::serde_json::json!({ "status": code, "detail": detail }));
        }
        self.ctx.request_repaint();
    }

    /// Queues a command from inside the backend (trigger replies).
    pub(crate) fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub(crate) fn count_rule(&self, rule: &str) {
        let today = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64 / 86400).unwrap_or(0);
        if let Ok(mut shared) = self.shared.lock() {
            let hits = Arc::make_mut(&mut shared.rule_hits);
            let entry = hits.entry(rule.to_owned()).or_insert((today, 0));
            if entry.0 != today {
                *entry = (today, 0);
            }
            entry.1 += 1;
        }
        self.ctx.request_repaint();
    }

    pub(crate) fn settings(&self) -> Arc<Settings> {
        self.shared.lock().map(|s| s.settings.clone()).unwrap_or_default()
    }

    /// Hands an event to the webhook dispatcher.
    pub(crate) fn emit(&self, session: &str, event: &'static str, data: whatsapp_rust::serde_json::Value) {
        let _ = self.hooks.send(webhook::Event { session: session.to_owned(), event, data, forced: false });
    }

    fn log_delivery(&self, delivery: &HookDelivery) {
        if let Ok(mut shared) = self.shared.lock() {
            let log = Arc::make_mut(&mut shared.webhooks);
            match log.iter_mut().find(|d| d.id == delivery.id) {
                Some(entry) => *entry = delivery.clone(),
                None => {
                    log.insert(0, delivery.clone());
                    log.truncate(webhook::LOG_SIZE);
                }
            }
        }
        self.ctx.request_repaint();
    }

    fn set_api(&self, api: ApiState) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.api = api;
        }
        self.ctx.request_repaint();
    }

    pub(crate) fn set_avatar(&self, jid: &str, picture: Option<PathBuf>) {
        if let Ok(mut shared) = self.shared.lock() {
            Arc::make_mut(&mut shared.avatars).insert(jid.to_owned(), picture);
        }
        self.ctx.request_repaint();
    }

    pub(crate) fn set_download(&self, msg_id: &str, state: Option<Option<String>>) {
        if let Ok(mut shared) = self.shared.lock() {
            let downloads = Arc::make_mut(&mut shared.downloads);
            match state {
                Some(state) => downloads.insert(msg_id.to_owned(), state),
                None => downloads.remove(msg_id),
            };
        }
        self.ctx.request_repaint();
    }

    pub(crate) fn downloading(&self, msg_id: &str) -> bool {
        self.shared.lock().is_ok_and(|s| matches!(s.downloads.get(msg_id), Some(None)))
    }

    pub(crate) fn own_number(&self, id: &str) -> Option<String> {
        let shared = self.shared.lock().ok()?;
        shared.sessions.iter().find(|s| s.id == id)?.own_number.clone()
    }

    /// Records (or, with `None`, clears) a contact typing in a chat.
    pub(crate) fn set_typing(&self, session: &str, chat: &str, typing: Option<Typing>) {
        if let Ok(mut shared) = self.shared.lock() {
            let key = Typing::key(session, chat);
            let map = Arc::make_mut(&mut shared.typing);
            map.retain(|_, t| t.fresh());
            match typing {
                Some(t) => {
                    map.insert(key, t);
                }
                None => {
                    if map.remove(&key).is_none() {
                        return;
                    }
                }
            }
        }
        self.ctx.request_repaint();
    }

    pub(crate) fn viewing(&self) -> Option<(String, String)> {
        self.shared.lock().ok()?.viewing.clone()
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn request_open(&self, session: &str, jid: &str) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.open_chat = Some((session.to_owned(), jid.to_owned()));
        }
        self.ctx.request_repaint();
    }

    /// The session's name, and whether there are several numbers.
    pub(crate) fn session_name(&self, id: &str) -> Option<(String, bool)> {
        let shared = self.shared.lock().ok()?;
        let name = shared.sessions.iter().find(|s| s.id == id)?.name.clone();
        Some((name, shared.sessions.len() > 1))
    }

    pub(crate) fn book(&self, id: &str) -> Option<Arc<Book>> {
        let shared = self.shared.lock().ok()?;
        shared.sessions.iter().find(|s| s.id == id).map(|s| s.book.clone())
    }

    /// Writes chat records to the session's store and applies them to its book.
    pub(crate) fn record(&self, id: &str, store: &Mutex<Store>, records: &[whatsapp_rust::serde_json::Value]) {
        if records.is_empty() {
            return;
        }
        self.update(id, |v| {
            if let Ok(mut store) = store.lock() {
                store.write(Arc::make_mut(&mut v.book), records);
            }
        });
    }

    fn insert(&self, view: SessionView) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.sessions.retain(|s| s.id != view.id);
            shared.sessions.push(view);
        }
        self.ctx.request_repaint();
    }

    fn remove(&self, id: &str) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.sessions.retain(|s| s.id != id);
        }
        self.ctx.request_repaint();
    }
}

/// Opens the session's chat store and shows its saved chats right away.
fn open_store(data_dir: &std::path::Path, id: &str, name: &str, state: &Updater) -> Arc<Mutex<Store>> {
    let (store, book) = Store::open(&Store::path(data_dir, id));
    let mut view = SessionView::new(id, name);
    view.book = Arc::new(book);
    state.insert(view);
    Arc::new(Mutex::new(store))
}

async fn run(
    data_dir: PathBuf,
    state: Updater,
    mut commands: UnboundedReceiver<Command>,
    api_commands: UnboundedSender<Command>,
) {
    let api_context = api::Context { commands: api_commands, shared: state.shared.clone(), data_dir: data_dir.clone() };
    let mut api_server = api::start(&api_context, &state);
    let mut registry = Registry::load(&data_dir);
    let mut running: HashMap<String, Running> = HashMap::new();
    let mut stores: HashMap<String, Arc<Mutex<Store>>> = HashMap::new();
    // Dev aid: WARUST_OFFLINE=1 shows the saved chats without connecting.
    let offline = std::env::var_os("WARUST_OFFLINE").is_some();
    // Dev aid, offline only: WARUST_DEMO=6281200004410,6281900003307:down shows
    // the numbers, in order, as linked (or logged out) for screenshots.
    let demo: Vec<String> = std::env::var("WARUST_DEMO").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_default();
    for (i, entry) in registry.entries.clone().into_iter().enumerate() {
        let store = open_store(&data_dir, &entry.id, &entry.name, &state);
        stores.insert(entry.id.clone(), store.clone());
        if offline {
            if let Some(number) = demo.get(i) {
                let (number, down) = number.split_once(':').map_or((number.as_str(), false), |(n, _)| (n, true));
                let phone = session::mask_phone(number);
                state.update(&entry.id, |v| {
                    v.status = if down { Status::LoggedOut } else { Status::Connected };
                    v.phone = Some(phone);
                    v.own_number = Some(number.to_owned());
                });
            }
            continue;
        }
        if let Some(r) = session::start(&entry.id, &registry, &state, store).await {
            running.insert(entry.id.clone(), r);
        }
    }

    while let Some(command) = commands.recv().await {
        match command {
            Command::Add { id, name } => {
                registry.add(&id, &name);
                let store = open_store(&data_dir, &id, &name, &state);
                stores.insert(id.clone(), store.clone());
                if let Some(r) = session::start(&id, &registry, &state, store).await {
                    running.insert(id, r);
                }
            }
            Command::PairCode { id, phone } => {
                if let Some(r) = running.get(&id) {
                    session::request_pair_code(r, &id, phone, &state);
                }
            }
            Command::Relink { id } => {
                // Unlink first, so the phone does not keep a dead "Warust" device.
                if let Some(r) = running.remove(&id) {
                    if r.is_linked() {
                        r.logout().await;
                    } else {
                        r.stop().await;
                    }
                }
                registry.delete_store(&id);
                // Chats stay: re-linking the same number brings the history again,
                // and duplicates merge by message id.
                state.update(&id, |v| {
                    let book = v.book.clone();
                    *v = SessionView::new(&v.id, &v.name);
                    v.book = book;
                });
                if let Some(store) = stores.get(&id).cloned()
                    && let Some(r) = session::start(&id, &registry, &state, store).await
                {
                    running.insert(id, r);
                }
            }
            Command::Logout { id } => {
                if let Some(r) = running.remove(&id) {
                    r.logout().await;
                }
                state.update(&id, |v| {
                    v.status = Status::LoggedOut;
                    v.qr = None;
                    v.pair_code = None;
                });
            }
            Command::Remove { id } => {
                if let Some(r) = running.remove(&id) {
                    if r.is_linked() {
                        r.logout().await;
                    } else {
                        r.stop().await;
                    }
                }
                registry.remove(&id);
                stores.remove(&id);
                let _ = std::fs::remove_file(Store::path(&data_dir, &id));
                state.remove(&id);
            }
            Command::Send { id, chat, text } => match (running.get(&id), stores.get(&id)) {
                (Some(r), Some(store)) => r.enqueue(&id, chat, Body::Text(text), &state, store),
                _ => log::warn!("send: session {id} is not running"),
            },
            Command::SendFile { id, chat, path, caption } => match (running.get(&id), stores.get(&id)) {
                (Some(r), Some(store)) => r.enqueue(&id, chat, Body::File { path, caption }, &state, store),
                _ => log::warn!("send file: session {id} is not running"),
            },
            Command::LoadOlder { id, chat } => {
                let oldest = state.book(&id).and_then(|book| {
                    let chat = book.chats.iter().find(|c| c.jid == chat)?;
                    // Only messages the phone knows: not ones still queued here.
                    chat.messages
                        .iter()
                        .find(|m| !m.id.starts_with("local-"))
                        .map(|m| (chat.jid.clone(), m.id.clone(), m.from_me, m.ts))
                });
                match (running.get(&id), oldest) {
                    (Some(r), Some((jid, msg_id, from_me, ts))) => r.fetch_older(&id, &jid, &msg_id, from_me, ts),
                    _ => log::warn!("load older: nothing to anchor on in {chat}"),
                }
            }
            Command::Enqueue { id, chat, body, local_id } => match (running.get(&id), stores.get(&id)) {
                (Some(r), Some(store)) => r.enqueue_as(&id, chat, body, local_id, &state, store),
                _ => log::warn!("api send: session {id} is not running"),
            },
            Command::SaveSettings(settings) => {
                let old = state.settings();
                settings.save(&data_dir);
                let api_changed = old.api != settings.api;
                if let Ok(mut shared) = state.shared.lock() {
                    shared.settings = Arc::new(settings);
                }
                if api_changed {
                    if let Some(server) = api_server.take() {
                        server.abort();
                    }
                    api_server = api::start(&api_context, &state);
                }
                state.ctx.request_repaint();
            }
            Command::TestWebhook => {
                let _ = state.hooks.send(webhook::Event {
                    session: "test".to_owned(),
                    event: "webhook.test",
                    data: whatsapp_rust::serde_json::json!({ "message": "Tes webhook dari Warust" }),
                    forced: true,
                });
            }
            Command::SendSticker { id, chat, path } => match (running.get(&id), stores.get(&id)) {
                (Some(r), Some(store)) => r.enqueue(&id, chat, Body::Sticker(path), &state, store),
                _ => log::warn!("send sticker: session {id} is not running"),
            },
            Command::Download { msg_id, .. } if state.downloading(&msg_id) => {}
            Command::StatusSeen { id, msg_id } => {
                let post = state.book(&id).and_then(|book| {
                    let post = book.statuses.iter().find(|s| s.id == msg_id)?;
                    (!post.seen && !post.from_me).then(|| post.sender.clone())
                });
                if let (Some(r), Some(store), Some(sender)) = (running.get(&id), stores.get(&id), post) {
                    r.mark_status_seen(&id, &sender, msg_id, &state, store.clone());
                }
            }
            Command::StatusReply { id, msg_id, text } => {
                let post = state.book(&id).and_then(|book| book.statuses.iter().find(|s| s.id == msg_id && !s.from_me).cloned());
                match (running.get(&id), stores.get(&id), post) {
                    (Some(r), Some(store), Some(post)) => {
                        let chat = store::bare(&post.sender);
                        r.enqueue(&id, chat, Body::StatusReply { text, status: post }, &state, store);
                    }
                    _ => log::warn!("status reply: session {id} is not running or the status is gone"),
                }
            }
            Command::Download { id, msg_id } => {
                let media = state.book(&id).and_then(|book| book.media_of(&msg_id).map(|m| m.raw.clone()));
                match (running.get(&id), stores.get(&id), media) {
                    (Some(r), Some(store), Some(media)) => r.download(&id, msg_id, media, &state, store.clone()),
                    _ => log::warn!("download: nothing to download for a message in {id}"),
                }
            }
            Command::Avatar { id, jid } => {
                if let Some(r) = running.get(&id) {
                    r.request_avatar(jid);
                }
            }
            Command::WatchPresence { id, chat } => {
                if let Some(r) = running.get(&id) {
                    r.watch_presence(&id, &chat);
                }
            }
            Command::Typing { id, chat, typing } => {
                if state.settings().typing {
                    if let Some(r) = running.get(&id) {
                        r.chat_state(&id, &chat, typing);
                    }
                }
            }
            Command::Seen { id, chat } => {
                if let Some(store) = stores.get(&id) {
                    state.record(&id, store, &[store::seen_record(&chat)]);
                }
            }
            Command::MarkDone { id, chat, done } => {
                if let Some(store) = stores.get(&id) {
                    let at = if done { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(1) } else { 0 };
                    state.record(&id, store, &[store::done_record(&chat, at)]);
                }
            }
        }
    }
}
