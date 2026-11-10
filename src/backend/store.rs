//! Chats and messages of one session, kept in memory as a [`Book`] and on disk
//! as JSON Lines (`data/chats/<session>.jsonl`, one record per line, replayed
//! at start). WhatsApp sends the history only once, at pairing, so it has to
//! be kept here.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use whatsapp_rust::serde_json::{self, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    /// Incoming message.
    Received,
    /// Waiting in the send queue.
    Queued,
    Sent,
    Delivered,
    Read,
    Failed(String),
}

impl Delivery {
    pub fn code(&self) -> String {
        match self {
            Self::Received => "received".into(),
            Self::Queued => "queued".into(),
            Self::Sent => "sent".into(),
            Self::Delivered => "delivered".into(),
            Self::Read => "read".into(),
            Self::Failed(reason) => format!("failed:{reason}"),
        }
    }

    fn parse(code: &str) -> Self {
        match code {
            "received" => Self::Received,
            // Still queued after a restart: the session sends it again (or
            // marks it failed) once it connects; see `session::restore_queue`.
            "queued" => Self::Queued,
            "sent" => Self::Sent,
            "delivered" => Self::Delivered,
            "read" => Self::Read,
            other => Self::Failed(other.strip_prefix("failed:").unwrap_or(other).to_owned()),
        }
    }

    /// Receipts only move forward: a late "delivered" never hides "read".
    fn rank(&self) -> u8 {
        match self {
            Self::Received => 0,
            Self::Queued => 1,
            Self::Failed(_) => 2,
            Self::Sent => 3,
            Self::Delivered => 4,
            Self::Read => 5,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatMsg {
    pub id: String,
    pub from_me: bool,
    /// Sender JID (group member, or the chat itself in a 1:1 chat).
    pub sender: String,
    pub text: String,
    /// Unix seconds.
    pub ts: i64,
    pub status: Delivery,
    pub media: Option<Media>,
    /// What this message replies to; for now only our replies to a status.
    pub quote: Option<Quote>,
    /// JIDs it mentions, as WhatsApp sent them (`…@s.whatsapp.net` or
    /// `…@lid`); the text holds `@<user part>` for each. Record key `men`.
    pub mentions: Vec<String>,
    /// An incoming group message that mentions this number. Record key `mme`.
    pub mentions_me: bool,
    /// Shown as "Diteruskan": it came in forwarded, or we forwarded it.
    pub forwarded: bool,
}

impl ChatMsg {
    /// Stage 1 of forwarding: plain text only. Media, locations and
    /// contacts are only a label here, not something to send again.
    pub fn forwardable(&self) -> bool {
        self.media.is_none() && !self.text.trim().is_empty() && !matches!(self.text.as_str(), "[Lokasi]" | "[Kontak]")
    }
}

/// The message a reply quotes.
#[derive(Clone, Debug)]
pub struct Quote {
    pub id: String,
    /// Its text or caption, for the quote box.
    pub text: String,
    /// It is a status update (a comment or reaction on someone's status).
    pub status: bool,
}

/// A photo, video, sticker, voice note or document in a message.
#[derive(Clone, Debug)]
pub struct Media {
    /// `image`, `video`, `sticker`, `audio` or `document`.
    pub kind: String,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Small inline preview that came with the message.
    pub thumb: Option<PathBuf>,
    /// The downloaded file, or the original for files sent from here.
    pub file: Option<PathBuf>,
    /// The stored record, which `media::download` reads.
    pub raw: Value,
}

impl Media {
    fn parse(raw: &Value) -> Option<Self> {
        let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| raw.get(key).and_then(Value::as_u64);
        Some(Self {
            kind: text("k")?,
            name: text("name"),
            size: number("size"),
            width: number("w").map(|n| n as u32),
            height: number("h").map(|n| n as u32),
            thumb: text("thumb").map(PathBuf::from),
            file: text("file").map(PathBuf::from),
            raw: raw.clone(),
        })
    }

    /// Whether it can be downloaded from WhatsApp's servers.
    pub fn downloadable(&self) -> bool {
        self.raw.get("dl").is_some_and(|d| !d.is_null())
    }
}

#[derive(Clone, Debug)]
pub struct Chat {
    /// Normalized JID: phone-number form when the LID mapping is known.
    pub jid: String,
    pub unread: u32,
    pub messages: Vec<ChatMsg>,
    /// When the chat was marked done (Unix seconds); a later incoming message
    /// makes it need a reply again.
    pub done_at: Option<i64>,
    /// Archived (here or on the phone): hidden from the main list. New
    /// messages do not bring it back, as in WhatsApp.
    pub archived: bool,
    /// Pinned messages, oldest pin first; may include expired ones.
    pub pins: Vec<Pin>,
}

/// How long a pinned message stays pinned (the choices WhatsApp offers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinLength {
    Hours24,
    Days7,
    Days30,
}

impl PinLength {
    pub fn secs(self) -> i64 {
        match self {
            Self::Hours24 => 86_400,
            Self::Days7 => 604_800,
            Self::Days30 => 2_592_000,
        }
    }

    /// The API name: `24h`, `7d` or `30d`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Hours24 => "24h",
            Self::Days7 => "7d",
            Self::Days30 => "30d",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "24h" => Some(Self::Hours24),
            "7d" => Some(Self::Days7),
            "30d" => Some(Self::Days30),
            _ => None,
        }
    }

    /// Nearest choice for a duration that came in a pin message from the phone.
    pub fn from_secs(secs: i64) -> Self {
        match secs {
            s if s <= 0 => Self::Days7,
            s if s <= Self::Hours24.secs() => Self::Hours24,
            s if s <= Self::Days7.secs() => Self::Days7,
            _ => Self::Days30,
        }
    }
}

/// A pinned message.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub id: String,
    /// When it was pinned (Unix seconds).
    pub ts: i64,
    /// When the pin lapses (Unix seconds).
    pub expires: i64,
}

impl Chat {
    /// Pins that have not lapsed at `now`, oldest pin first (the last is the latest).
    pub fn active_pins(&self, now: i64) -> Vec<&Pin> {
        self.pins.iter().filter(|p| p.expires > now).collect()
    }

    pub fn is_pinned(&self, msg_id: &str, now: i64) -> bool {
        self.pins.iter().any(|p| p.id == msg_id && p.expires > now)
    }

    /// Time of the contact's latest message, 0 when there is none.
    pub fn last_incoming_ts(&self) -> i64 {
        self.messages.iter().rev().find(|m| !m.from_me).map(|m| m.ts).unwrap_or(0)
    }

    /// Marked done, and nothing came in since.
    pub fn is_done(&self) -> bool {
        self.done_at.is_some_and(|at| at >= self.last_incoming_ts())
    }

    /// A private chat whose last message is from the contact and that is not
    /// marked done ("Perlu balas"); a group only while `mention_pending`.
    pub fn needs_reply(&self) -> bool {
        if self.is_group() {
            return self.mention_pending();
        }
        self.messages.last().is_some_and(|m| !m.from_me) && !self.is_done()
    }

    /// A group message mentioning this number came after our last message in
    /// the chat and after it was marked done (the `@` badge).
    pub fn mention_pending(&self) -> bool {
        if !self.is_group() {
            return false;
        }
        // Newest first: stop at our own message or at anything already done.
        let done_at = self.done_at.unwrap_or(0);
        for m in self.messages.iter().rev() {
            if m.from_me || m.ts <= done_at {
                return false;
            }
            if m.mentions_me {
                return true;
            }
        }
        false
    }

    pub fn last_ts(&self) -> i64 {
        self.messages.last().map(|m| m.ts).unwrap_or(0)
    }

    pub fn is_group(&self) -> bool {
        self.jid.ends_with("@g.us")
    }
}

/// The chat id WhatsApp uses for everyone's status updates.
pub const STATUS_CHAT: &str = "status@broadcast";

/// Status updates last a day.
pub const STATUS_LIFETIME: i64 = 24 * 3600;

/// One status update (text, photo or video) from a contact or from this number.
#[derive(Clone, Debug)]
pub struct StatusPost {
    pub id: String,
    /// Poster JID as WhatsApp sent it (needed for the "seen" receipt).
    pub sender: String,
    pub from_me: bool,
    pub text: String,
    pub ts: i64,
    pub media: Option<Media>,
    /// ARGB of a text status's background and text.
    pub background: Option<u32>,
    pub text_color: Option<u32>,
    /// A "seen" receipt went out for it.
    pub seen: bool,
}

impl StatusPost {
    /// The text without the `[Gambar]`-style label a picture already shows.
    pub fn caption(&self) -> String {
        if self.media.is_some() && self.text.starts_with('[') {
            self.text.split_once(']').map(|(_, rest)| rest.trim().to_owned()).unwrap_or_default()
        } else {
            self.text.clone()
        }
    }
}

/// All chats of a session, newest first.
#[derive(Clone, Default)]
pub struct Book {
    /// Chats with at least one message.
    pub chats: Vec<Chat>,
    /// Contact names from the phone and group subjects.
    names: HashMap<String, String>,
    /// Names people set for themselves; used when there is no contact name.
    push_names: HashMap<String, String>,
    /// LID user → phone-number JID, from history sync and incoming messages.
    lid_to_pn: HashMap<String, String>,
    /// Queued id → the id WhatsApp gave the message once sent.
    pub aliases: HashMap<String, String>,
    /// Status updates, oldest first; kept apart from the chats.
    pub statuses: Vec<StatusPost>,
    /// Archived chats (normalized JIDs), also those without messages yet, so
    /// an archive record that comes before the history still counts.
    archived: HashSet<String>,
}

impl Book {
    /// Prefer the phone-number JID so a contact seen as LID and as number is one chat.
    pub fn normalize(&self, jid: &str) -> String {
        let bare = bare(jid);
        if bare.ends_with("@lid") {
            if let Some(pn) = self.lid_to_pn.get(&bare) {
                return pn.clone();
            }
        }
        bare
    }

    /// Contact name or group subject, else the person's own push name.
    pub fn name_of(&self, jid: &str) -> Option<&str> {
        let jid = self.normalize(jid);
        self.names
            .get(&jid)
            .or_else(|| self.push_names.get(&jid))
            .map(String::as_str)
    }

    /// `text` with each `@<user part>` of `mentions` shown as `@<name>`. A
    /// token only counts at a word boundary on both sides (`@6281` is not in
    /// `@62812`); a mention with no known name shows the masked number.
    pub fn with_mention_names(&self, text: &str, mentions: &[String]) -> String {
        if mentions.is_empty() || !text.contains('@') {
            return text.to_owned();
        }
        let user = |jid: &str| {
            let u = jid.split('@').next().unwrap_or(jid);
            u.split(':').next().unwrap_or(u).to_owned()
        };
        let named: Vec<(String, String)> = mentions
            .iter()
            .map(|jid| (user(jid), self.name_of(jid).filter(|n| !n.is_empty()).map(str::to_owned).unwrap_or_else(|| short_jid(jid))))
            .filter(|(token, _)| !token.is_empty())
            .collect();
        let mut out = String::with_capacity(text.len());
        let mut prev: Option<char> = None;
        let mut rest = text;
        while let Some(c) = rest.chars().next() {
            let hit = (c == '@' && prev.is_none_or(|p| !p.is_alphanumeric()))
                .then(|| {
                    named
                        .iter()
                        .filter(|(token, _)| {
                            rest[1..].strip_prefix(token.as_str()).is_some_and(|after| after.chars().next().is_none_or(|n| !n.is_alphanumeric()))
                        })
                        .max_by_key(|(token, _)| token.len())
                })
                .flatten();
            match hit {
                Some((token, name)) => {
                    out.push('@');
                    out.push_str(name);
                    prev = name.chars().next_back();
                    rest = &rest[1 + token.len()..];
                }
                None => {
                    out.push(c);
                    prev = Some(c);
                    rest = &rest[c.len_utf8()..];
                }
            }
        }
        out
    }

    /// Status updates of the last 24 hours by poster (normalized JID, or
    /// `None` for this number), newest poster first; posts oldest first.
    pub fn status_feed(&self, now: i64) -> Vec<(Option<String>, Vec<&StatusPost>)> {
        let mut feed: Vec<(Option<String>, Vec<&StatusPost>)> = Vec::new();
        for post in self.statuses.iter().filter(|s| now - s.ts < STATUS_LIFETIME) {
            let poster = (!post.from_me).then(|| self.normalize(&post.sender));
            match feed.iter_mut().find(|(p, _)| *p == poster) {
                Some((_, posts)) => posts.push(post),
                None => feed.push((poster, vec![post])),
            }
        }
        feed.sort_by_key(|(poster, posts)| {
            (poster.is_some(), std::cmp::Reverse(posts.last().map(|p| p.ts).unwrap_or(0)))
        });
        feed
    }

    /// The media of a message or a status update.
    pub fn media_of(&self, msg_id: &str) -> Option<&Media> {
        self.chats
            .iter()
            .flat_map(|c| &c.messages)
            .find(|m| m.id == msg_id)
            .and_then(|m| m.media.as_ref())
            .or_else(|| self.statuses.iter().find(|s| s.id == msg_id).and_then(|s| s.media.as_ref()))
    }

    fn chat_mut(&mut self, jid: &str) -> &mut Chat {
        if let Some(i) = self.chats.iter().position(|c| c.jid == jid) {
            return &mut self.chats[i];
        }
        self.chats.push(Chat {
            jid: jid.to_owned(),
            unread: 0,
            messages: Vec::new(),
            done_at: None,
            archived: self.archived.contains(jid),
            pins: Vec::new(),
        });
        self.chats.last_mut().expect("just pushed")
    }

    fn sort(&mut self) {
        self.chats.sort_by_key(|c| std::cmp::Reverse(c.last_ts()));
    }

    fn apply(&mut self, record: &Value) {
        let text = |key: &str| record.get(key).and_then(Value::as_str).map(str::to_owned);
        match record.get("t").and_then(Value::as_str) {
            Some("lid") => {
                if let (Some(lid), Some(pn)) = (text("lid"), text("pn")) {
                    let (lid, pn) = (bare(&lid), bare(&pn));
                    // An archive record may have come before this mapping.
                    if self.archived.remove(&lid) {
                        self.archived.insert(pn.clone());
                        if let Some(c) = self.chats.iter_mut().find(|c| c.jid == pn) {
                            c.archived = true;
                        }
                    }
                    self.lid_to_pn.insert(lid, pn);
                }
            }
            Some("name") => {
                if let (Some(chat), Some(name)) = (text("chat"), text("name")) {
                    self.names.insert(self.normalize(&chat), name);
                }
            }
            Some("push") => {
                if let (Some(jid), Some(name)) = (text("jid"), text("name")) {
                    self.push_names.insert(self.normalize(&jid), name);
                }
            }
            Some("msg") => {
                let (Some(chat), Some(id)) = (text("chat"), text("id")) else {
                    return;
                };
                if chat == STATUS_CHAT {
                    let post = StatusPost {
                        id,
                        sender: text("from").unwrap_or_default(),
                        from_me: record.get("me").and_then(Value::as_bool).unwrap_or(false),
                        text: text("text").unwrap_or_default(),
                        ts: record.get("ts").and_then(Value::as_i64).unwrap_or(0),
                        media: record.get("media").and_then(Media::parse),
                        background: record.get("bg").and_then(Value::as_u64).map(|n| n as u32),
                        text_color: record.get("fg").and_then(Value::as_u64).map(|n| n as u32),
                        seen: false,
                    };
                    if !self.statuses.iter().any(|s| s.id == post.id) {
                        let at = self.statuses.partition_point(|s| s.ts <= post.ts);
                        self.statuses.insert(at, post);
                    }
                    return;
                }
                let jid = self.normalize(&chat);
                let message = ChatMsg {
                    id,
                    from_me: record.get("me").and_then(Value::as_bool).unwrap_or(false),
                    sender: text("from").unwrap_or_default(),
                    text: text("text").unwrap_or_default(),
                    ts: record.get("ts").and_then(Value::as_i64).unwrap_or(0),
                    status: Delivery::parse(&text("s").unwrap_or_default()),
                    media: record.get("media").and_then(Media::parse),
                    quote: record.get("q").and_then(|q| {
                        Some(Quote {
                            id: q.get("id")?.as_str()?.to_owned(),
                            text: q.get("text").and_then(Value::as_str).unwrap_or_default().to_owned(),
                            status: q.get("st").and_then(Value::as_bool).unwrap_or(false),
                        })
                    }),
                    mentions: record
                        .get("men")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|j| j.as_str().map(str::to_owned)).collect())
                        .unwrap_or_default(),
                    mentions_me: record.get("mme").and_then(Value::as_bool).unwrap_or(false),
                    forwarded: record.get("fw").and_then(Value::as_bool).unwrap_or(false),
                };
                let unread = record.get("unread").and_then(Value::as_bool).unwrap_or(false);
                let chat = self.chat_mut(&jid);
                if let Some(existing) = chat.messages.iter_mut().find(|m| m.id == message.id) {
                    // History may repeat a message we already have.
                    if message.status.rank() > existing.status.rank() {
                        existing.status = message.status;
                    }
                    if existing.media.is_none() {
                        existing.media = message.media;
                    }
                    return;
                }
                if unread {
                    chat.unread += 1;
                }
                let at = chat.messages.partition_point(|m| m.ts <= message.ts);
                chat.messages.insert(at, message);
            }
            Some("status") => {
                if let (Some(id), Some(code)) = (text("id"), text("s")) {
                    let status = Delivery::parse(&code);
                    for chat in &mut self.chats {
                        if let Some(m) = chat.messages.iter_mut().find(|m| m.id == id) {
                            // Failures may replace "queued"; receipts only move forward.
                            if status.rank() > m.status.rank() || matches!(status, Delivery::Failed(_)) {
                                m.status = status;
                            }
                            return;
                        }
                    }
                }
            }
            Some("rekey") => {
                // A queued message got its real id from the server.
                if let (Some(old), Some(new)) = (text("old"), text("new")) {
                    self.aliases.insert(old.clone(), new.clone());
                    for chat in &mut self.chats {
                        if let Some(m) = chat.messages.iter_mut().find(|m| m.id == old) {
                            m.id = new;
                            return;
                        }
                    }
                }
            }
            Some("file") => {
                // A received file was downloaded.
                if let (Some(id), Some(path)) = (text("id"), text("path")) {
                    if let Some(media) = self.statuses.iter_mut().find(|s| s.id == id).and_then(|s| s.media.as_mut()) {
                        media.file = Some(PathBuf::from(path));
                        return;
                    }
                    for chat in &mut self.chats {
                        if let Some(media) = chat.messages.iter_mut().find(|m| m.id == id).and_then(|m| m.media.as_mut()) {
                            media.file = Some(PathBuf::from(path));
                            return;
                        }
                    }
                }
            }
            Some("done") => {
                // ts 0 takes the mark off again.
                if let Some(chat) = text("chat") {
                    let jid = self.normalize(&chat);
                    let at = record.get("ts").and_then(Value::as_i64).unwrap_or(0);
                    if let Some(c) = self.chats.iter_mut().find(|c| c.jid == jid) {
                        c.done_at = (at > 0).then_some(at);
                    }
                }
            }
            Some("archive") => {
                // A missing `v` means archived; `false` takes it out again.
                if let Some(chat) = text("chat") {
                    let jid = self.normalize(&chat);
                    let on = record.get("v").and_then(Value::as_bool).unwrap_or(true);
                    if on {
                        self.archived.insert(jid.clone());
                    } else {
                        self.archived.remove(&jid);
                    }
                    if let Some(c) = self.chats.iter_mut().find(|c| c.jid == jid) {
                        c.archived = on;
                    }
                }
            }
            Some("unread") => {
                // Local "Tandai belum dibaca": at least one unread until opened.
                if let Some(chat) = text("chat") {
                    let jid = self.normalize(&chat);
                    if let Some(c) = self.chats.iter_mut().find(|c| c.jid == jid) {
                        c.unread = c.unread.max(1);
                    }
                }
            }
            Some("pin") => {
                // Only for chats we already have; a pin never creates an empty chat.
                if let (Some(chat), Some(id)) = (text("chat"), text("id")) {
                    let jid = self.normalize(&chat);
                    let ts = record.get("ts").and_then(Value::as_i64).unwrap_or(0);
                    let expires = record.get("exp").and_then(Value::as_i64).unwrap_or(ts + PinLength::Days7.secs());
                    if let Some(c) = self.chats.iter_mut().find(|c| c.jid == jid) {
                        c.pins.retain(|p| p.id != id);
                        c.pins.push(Pin { id, ts, expires });
                        c.pins.sort_by_key(|p| p.ts);
                    }
                }
            }
            Some("unpin") => {
                if let (Some(chat), Some(id)) = (text("chat"), text("id")) {
                    let jid = self.normalize(&chat);
                    if let Some(c) = self.chats.iter_mut().find(|c| c.jid == jid) {
                        c.pins.retain(|p| p.id != id);
                    }
                }
            }
            Some("stseen") => {
                if let Some(id) = text("id") {
                    if let Some(post) = self.statuses.iter_mut().find(|s| s.id == id) {
                        post.seen = true;
                    }
                }
            }
            Some("seen") => {
                if let Some(chat) = text("chat") {
                    let jid = self.normalize(&chat);
                    if let Some(c) = self.chats.iter_mut().find(|c| c.jid == jid) {
                        c.unread = 0;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Short, masked form of a JID for small labels; never a full number.
pub fn short_jid(jid: &str) -> String {
    let jid = bare(jid);
    let jid = jid.as_str();
    let user = jid.split('@').next().unwrap_or(jid);
    let tail: String = user.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    if jid.ends_with("@s.whatsapp.net") {
        super::session::mask_phone(jid)
    } else if jid.ends_with("@g.us") {
        format!("grup …{tail}")
    } else {
        format!("lid …{tail}")
    }
}

/// `user:device@server` → `user@server`.
pub fn bare(jid: &str) -> String {
    match jid.split_once('@') {
        Some((user, server)) => format!("{}@{server}", user.split(':').next().unwrap_or(user)),
        None => jid.to_owned(),
    }
}

/// Appends records to the session's JSON Lines file and applies them to the book.
pub struct Store {
    file: Option<File>,
}

impl Store {
    pub fn path(data_dir: &Path, session: &str) -> PathBuf {
        data_dir.join("chats").join(format!("{session}.jsonl"))
    }

    /// Opens (or creates) the file and replays it into a new book.
    pub fn open(path: &Path) -> (Self, Book) {
        let mut book = Book::default();
        if let Ok(file) = File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if let Ok(record) = serde_json::from_str::<Value>(&line) {
                    book.apply(&record);
                }
            }
        }
        book.sort();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| log::error!("cannot open {}: {e}", path.display()))
            .ok();
        (Self { file }, book)
    }

    pub fn write(&mut self, book: &mut Book, records: &[Value]) {
        let mut lines = String::new();
        for record in records {
            book.apply(record);
            lines.push_str(&record.to_string());
            lines.push('\n');
        }
        book.sort();
        if let Some(file) = &mut self.file {
            if let Err(e) = file.write_all(lines.as_bytes()) {
                log::error!("cannot write chat store: {e}");
            }
        }
    }
}

pub fn msg_record(chat: &str, id: &str, from_me: bool, from: &str, text: &str, ts: i64, status: &Delivery, unread: bool) -> Value {
    json!({ "t": "msg", "chat": chat, "id": id, "me": from_me, "from": from, "text": text, "ts": ts, "s": status.code(), "unread": unread })
}

/// Adds the media part (from `media::describe`) to a msg record.
pub fn with_media(mut record: Value, media: Option<Value>) -> Value {
    if let (Some(media), Some(object)) = (media, record.as_object_mut()) {
        object.insert("media".to_owned(), media);
    }
    record
}

/// Adds who the message mentions (`men`) and whether it mentions this number
/// (`mme`); keys are left out when empty/false so old files stay the same.
pub fn with_mentions(mut record: Value, mentions: &[String], mentions_me: bool) -> Value {
    if let Some(object) = record.as_object_mut() {
        if !mentions.is_empty() {
            object.insert("men".to_owned(), json!(mentions));
        }
        if mentions_me {
            object.insert("mme".to_owned(), true.into());
        }
    }
    record
}

/// Marks a msg record as forwarded (absent means not).
pub fn with_forwarded(mut record: Value, forwarded: bool) -> Value {
    if let (true, Some(object)) = (forwarded, record.as_object_mut()) {
        object.insert("fw".to_owned(), true.into());
    }
    record
}

/// Adds what the message quotes to its msg record.
pub fn with_quote(mut record: Value, quote: &Quote) -> Value {
    if let Some(object) = record.as_object_mut() {
        object.insert("q".to_owned(), json!({ "id": quote.id, "text": quote.text, "st": quote.status }));
    }
    record
}

/// Adds a text status's colours to its msg record.
pub fn with_colors(mut record: Value, background: Option<u32>, text: Option<u32>) -> Value {
    if let Some(object) = record.as_object_mut() {
        if let Some(bg) = background {
            object.insert("bg".to_owned(), bg.into());
        }
        if let Some(fg) = text {
            object.insert("fg".to_owned(), fg.into());
        }
    }
    record
}

pub fn status_seen_record(id: &str) -> Value {
    json!({ "t": "stseen", "id": id })
}

pub fn file_record(id: &str, path: &Path) -> Value {
    json!({ "t": "file", "id": id, "path": path.to_string_lossy() })
}

pub fn status_record(id: &str, status: &Delivery) -> Value {
    json!({ "t": "status", "id": id, "s": status.code() })
}

pub fn name_record(chat: &str, name: &str) -> Value {
    json!({ "t": "name", "chat": chat, "name": name })
}

pub fn push_record(jid: &str, name: &str) -> Value {
    json!({ "t": "push", "jid": jid, "name": name })
}

pub fn lid_record(lid: &str, pn: &str) -> Value {
    json!({ "t": "lid", "lid": lid, "pn": pn })
}

pub fn rekey_record(old: &str, new: &str) -> Value {
    json!({ "t": "rekey", "old": old, "new": new })
}

/// Marks a chat done at `ts`, or (with 0) needing a reply again.
pub fn done_record(chat: &str, ts: i64) -> Value {
    json!({ "t": "done", "chat": chat, "ts": ts })
}

/// Pins message `id` at `ts` for `secs` seconds.
pub fn pin_record(chat: &str, id: &str, ts: i64, secs: i64) -> Value {
    json!({ "t": "pin", "chat": chat, "id": id, "ts": ts, "exp": ts + secs })
}

pub fn unpin_record(chat: &str, id: &str) -> Value {
    json!({ "t": "unpin", "chat": chat, "id": id })
}

pub fn seen_record(chat: &str) -> Value {
    json!({ "t": "seen", "chat": chat })
}

/// Archives a chat, or (with `false`) takes it out of the archive.
pub fn archive_record(chat: &str, archived: bool) -> Value {
    json!({ "t": "archive", "chat": chat, "v": archived })
}

/// Marks a chat unread again (at least one unread message).
pub fn unread_record(chat: &str) -> Value {
    json!({ "t": "unread", "chat": chat })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replays records through a real file, as a restart does.
    fn reopen(records: &[Value]) -> Book {
        let dir = std::env::temp_dir().join(format!("warust-store-{}-{:?}", std::process::id(), std::thread::current().id()));
        let path = dir.join("s.jsonl");
        let _ = fs::remove_dir_all(&dir);
        {
            let (mut store, mut book) = Store::open(&path);
            store.write(&mut book, records);
        }
        let (_, book) = Store::open(&path);
        let _ = fs::remove_dir_all(&dir);
        book
    }

    #[test]
    fn archive_state_round_trips_and_survives_new_messages() {
        let chat = "628111000002@s.whatsapp.net";
        let msg = |id: &str, ts: i64| msg_record(chat, id, false, chat, "x", ts, &Delivery::Received, true);
        // Old files have no archive records: not archived.
        assert!(!reopen(&[msg("a", 1)]).chats[0].archived);
        let b = reopen(&[msg("a", 1), archive_record(chat, true), msg("b", 2)]);
        assert!(b.chats[0].archived, "a new message keeps it archived");
        let b = reopen(&[msg("a", 1), archive_record(chat, true), archive_record(chat, false)]);
        assert!(!b.chats[0].archived);
        // The phone may archive before the history brings the chat.
        let b = reopen(&[archive_record(chat, true), msg("a", 1)]);
        assert!(b.chats[0].archived);
    }

    #[test]
    fn archive_before_lid_mapping_moves_to_the_number() {
        let (lid, pn) = ("111222@lid", "628111000003@s.whatsapp.net");
        let msg = msg_record(pn, "a", false, pn, "x", 1, &Delivery::Received, true);
        let b = reopen(&[archive_record(lid, true), lid_record(lid, pn), msg]);
        assert_eq!(b.chats.len(), 1);
        assert!(b.chats[0].archived);
    }

    #[test]
    fn unread_marker_stays_until_seen() {
        let chat = "628111000003@s.whatsapp.net";
        let read = msg_record(chat, "a", false, chat, "x", 1, &Delivery::Received, false);
        let b = reopen(&[read.clone(), unread_record(chat)]);
        assert_eq!(b.chats[0].unread, 1);
        let b = reopen(&[read.clone(), unread_record(chat), seen_record(chat)]);
        assert_eq!(b.chats[0].unread, 0);
        let more = msg_record(chat, "b", false, chat, "y", 2, &Delivery::Received, true);
        let b = reopen(&[read, more, unread_record(chat)]);
        assert_eq!(b.chats[0].unread, 1, "the marker never lowers a real count");
    }

    #[test]
    fn status_reply_keeps_its_quote() {
        let quote = Quote { id: "ST1".to_owned(), text: "Pesanan katering".to_owned(), status: true };
        let record = with_quote(msg_record("628111@s.whatsapp.net", "R1", true, "", "👍", 10, &Delivery::Queued, false), &quote);
        let b = book(&[record]);
        let m = &b.chats[0].messages[0];
        let q = m.quote.as_ref().expect("quote kept");
        assert_eq!((q.id.as_str(), q.text.as_str(), q.status), ("ST1", "Pesanan katering", true));
    }

    #[test]
    fn forwarded_flag_round_trips_and_old_records_read_false() {
        let chat = "628111@s.whatsapp.net";
        let plain = msg_record(chat, "a", false, chat, "halo", 10, &Delivery::Received, false);
        assert!(plain.get("fw").is_none(), "absent when not forwarded");
        let fw = with_forwarded(msg_record(chat, "b", false, chat, "info", 20, &Delivery::Received, false), true);
        let b = book(&[plain, fw]);
        let flags: Vec<bool> = b.chats[0].messages.iter().map(|m| m.forwarded).collect();
        assert_eq!(flags, [false, true]);
        assert!(b.chats[0].messages[1].forwardable());
    }

    #[test]
    fn done_until_the_contact_writes_again() {
        let chat = "628111000001@s.whatsapp.net";
        let msg = |id: &str, me: bool, ts: i64| msg_record(chat, id, me, chat, "x", ts, &Delivery::Received, false);
        let b = book(&[msg("a", false, 10)]);
        assert!(b.chats[0].needs_reply() && !b.chats[0].is_done());
        let b = book(&[msg("a", false, 10), done_record(chat, 20)]);
        assert!(!b.chats[0].needs_reply() && b.chats[0].is_done());
        let b = book(&[msg("a", false, 10), done_record(chat, 20), msg("b", false, 30)]);
        assert!(b.chats[0].needs_reply() && !b.chats[0].is_done(), "a new message reopens it");
        let b = book(&[msg("a", false, 10), msg("b", true, 15)]);
        assert!(!b.chats[0].needs_reply(), "answered");
        let b = book(&[msg("a", false, 10), done_record(chat, 20), done_record(chat, 0)]);
        assert!(b.chats[0].needs_reply(), "unmarked");
    }

    #[test]
    fn mentions_round_trip_and_old_records_load() {
        let chat = "123@g.us";
        let men = vec!["628111@s.whatsapp.net".to_owned(), "99@lid".to_owned()];
        let rec = with_mentions(msg_record(chat, "a", false, "5@lid", "@628111 @99", 10, &Delivery::Received, false), &men, true);
        let b = book(&[rec, msg_record(chat, "b", false, "5@lid", "plain", 11, &Delivery::Received, false)]);
        let (a, plain) = (&b.chats[0].messages[0], &b.chats[0].messages[1]);
        assert_eq!(a.mentions, men);
        assert!(a.mentions_me);
        assert!(plain.mentions.is_empty() && !plain.mentions_me);
        let bare_rec = with_mentions(msg_record(chat, "c", true, "", "x", 1, &Delivery::Sent, false), &[], false);
        assert!(bare_rec.get("men").is_none() && bare_rec.get("mme").is_none());
    }

    #[test]
    fn mention_pending_until_we_reply_or_mark_done() {
        let chat = "123@g.us";
        let m = |id: &str, ts: i64| with_mentions(msg_record(chat, id, false, "5@lid", "@me", ts, &Delivery::Received, false), &[], true);
        let other = |id: &str, ts: i64| msg_record(chat, id, false, "5@lid", "obrolan", ts, &Delivery::Received, false);
        let mine = |id: &str, ts: i64| msg_record(chat, id, true, "", "ok", ts, &Delivery::Sent, false);
        let b = book(&[other("o", 5), m("a", 10), other("o2", 12)]);
        assert!(b.chats[0].mention_pending() && b.chats[0].needs_reply());
        let b = book(&[other("o", 5)]);
        assert!(!b.chats[0].mention_pending() && !b.chats[0].needs_reply(), "plain group chatter is not a reply");
        let b = book(&[m("a", 10), mine("r", 15)]);
        assert!(!b.chats[0].mention_pending(), "we replied");
        let b = book(&[m("a", 10), done_record(chat, 10)]);
        assert!(!b.chats[0].mention_pending(), "done at the mention's time");
        let b = book(&[m("a", 10), done_record(chat, 20), m("b", 30)]);
        assert!(b.chats[0].mention_pending(), "a newer mention reopens it");
        let b = book(&[m("a", 10), mine("r", 15), m("b", 20)]);
        assert!(b.chats[0].mention_pending());
        // A mention that mentions_me only counts when incoming.
        let own = with_mentions(msg_record(chat, "x", true, "", "@me", 40, &Delivery::Sent, false), &[], true);
        assert!(!book(&[own]).chats[0].mention_pending());
    }

    #[test]
    fn private_chat_ignores_mentions() {
        let chat = "628111000001@s.whatsapp.net";
        let rec = with_mentions(msg_record(chat, "a", false, chat, "x", 10, &Delivery::Received, false), &[], true);
        let b = book(&[rec]);
        assert!(!b.chats[0].mention_pending());
        assert!(b.chats[0].needs_reply(), "private rule unchanged");
    }

    #[test]
    fn pins_expire_and_unpin() {
        let chat = "628111@s.whatsapp.net";
        let msg = |id: &str, ts: i64| msg_record(chat, id, false, chat, "x", ts, &Delivery::Received, false);
        let b = book(&[
            msg("a", 1),
            msg("b", 2),
            pin_record(chat, "a", 100, PinLength::Hours24.secs()),
            pin_record(chat, "b", 200, PinLength::Days7.secs()),
        ]);
        let c = &b.chats[0];
        let ids = |now| c.active_pins(now).iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(300), vec!["a", "b"]);
        assert_eq!(ids(100 + 86_400), vec!["b"], "24h pin lapsed");
        assert!(ids(200 + 604_800).is_empty());
        assert!(c.is_pinned("b", 300) && !c.is_pinned("zzz", 300));
        // Re-pin replaces rather than duplicates, and a pin without a chat is ignored.
        let b = book(&[
            msg("a", 1),
            pin_record(chat, "a", 100, 1000),
            pin_record(chat, "a", 150, 5000),
            pin_record("999@s.whatsapp.net", "q", 1, 10),
        ]);
        assert_eq!(b.chats.len(), 1);
        assert_eq!(b.chats[0].pins, vec![Pin { id: "a".into(), ts: 150, expires: 5150 }]);
        let b = book(&[msg("a", 1), pin_record(chat, "a", 100, 1000), unpin_record(chat, "a")]);
        assert!(b.chats[0].pins.is_empty());
    }

    #[test]
    fn pin_lengths_round_trip() {
        for l in [PinLength::Hours24, PinLength::Days7, PinLength::Days30] {
            assert_eq!(PinLength::parse(l.code()), Some(l));
            assert_eq!(PinLength::from_secs(l.secs()), l);
        }
        assert_eq!(PinLength::parse("1h"), None);
    }

    #[test]
    fn old_records_without_pins_still_load() {
        let b = book(&[msg_record("1@s.whatsapp.net", "x", true, "", "t", 1, &Delivery::Sent, false)]);
        assert!(b.chats[0].pins.is_empty());
    }

    fn book(records: &[Value]) -> Book {
        let mut book = Book::default();
        for r in records {
            book.apply(r);
        }
        book.sort();
        book
    }

    #[test]
    fn lid_and_number_are_one_chat() {
        let b = book(&[
            lid_record("111@lid", "628100@s.whatsapp.net"),
            msg_record("111:5@lid", "a", false, "111@lid", "halo", 10, &Delivery::Received, true),
            msg_record("628100@s.whatsapp.net", "b", true, "", "hai", 20, &Delivery::Sent, false),
        ]);
        assert_eq!(b.chats.len(), 1);
        assert_eq!(b.chats[0].jid, "628100@s.whatsapp.net");
        assert_eq!(b.chats[0].messages.len(), 2);
        assert_eq!(b.chats[0].unread, 1);
    }

    #[test]
    fn names_do_not_create_chats_and_contact_names_win() {
        let b = book(&[
            push_record("9@s.whatsapp.net", "Budi HP"),
            name_record("7@s.whatsapp.net", "Tanpa pesan"),
            msg_record("9@s.whatsapp.net", "m", false, "9@s.whatsapp.net", "hai", 1, &Delivery::Received, false),
            name_record("9@s.whatsapp.net", "Budi Kantor"),
        ]);
        assert_eq!(b.chats.len(), 1);
        assert_eq!(b.name_of("9@s.whatsapp.net"), Some("Budi Kantor"));
        assert_eq!(b.name_of("9:2@s.whatsapp.net"), Some("Budi Kantor"));
    }

    #[test]
    fn mention_tokens_become_names() {
        let b = book(&[push_record("6281@s.whatsapp.net", "Budi"), push_record("6282@s.whatsapp.net", "Ani")]);
        let men = vec!["6281@s.whatsapp.net".to_owned()];
        assert_eq!(b.with_mention_names("halo @6281 apa", &men), "halo @Budi apa");
        assert_eq!(b.with_mention_names("@6281", &men), "@Budi");
        let two = vec!["6281@s.whatsapp.net".to_owned(), "6282@s.whatsapp.net".to_owned()];
        assert_eq!(b.with_mention_names("@6281 @6282 cek", &two), "@Budi @Ani cek");
    }

    #[test]
    fn mention_names_respect_word_boundaries() {
        let b = book(&[push_record("6281@s.whatsapp.net", "Budi")]);
        let men = vec!["6281@s.whatsapp.net".to_owned()];
        assert_eq!(b.with_mention_names("@62812 dan a@6281", &men), "@62812 dan a@6281");
        assert_eq!(b.with_mention_names("@6281, ok", &men), "@Budi, ok");
    }

    #[test]
    fn unknown_mention_name_is_masked() {
        let b = book(&[push_record("6281@s.whatsapp.net", "Budi")]);
        let men = vec!["628121234410@s.whatsapp.net".to_owned(), "6281@s.whatsapp.net".to_owned()];
        let out = b.with_mention_names("@628121234410 @6281", &men);
        assert_eq!(out, "@+62 812-****-4410 @Budi");
        assert!(!out.contains("628121234410"));
        assert_eq!(b.with_mention_names("tanpa mention", &men), "tanpa mention");
    }

    #[test]
    fn receipts_only_move_forward_and_duplicates_merge() {
        let b = book(&[
            msg_record("1@s.whatsapp.net", "x", true, "", "t", 1, &Delivery::Sent, false),
            status_record("x", &Delivery::Read),
            status_record("x", &Delivery::Delivered),
            msg_record("1@s.whatsapp.net", "x", true, "", "t", 1, &Delivery::Sent, false),
        ]);
        assert_eq!(b.chats[0].messages.len(), 1);
        assert_eq!(b.chats[0].messages[0].status, Delivery::Read);
    }

    #[test]
    fn queued_becomes_real_id_and_newest_chat_first() {
        let b = book(&[
            msg_record("1@s.whatsapp.net", "old", true, "", "a", 5, &Delivery::Sent, false),
            msg_record("2@s.whatsapp.net", "local-1", true, "", "b", 9, &Delivery::Queued, false),
            rekey_record("local-1", "3EB0"),
            status_record("3EB0", &Delivery::Sent),
        ]);
        assert_eq!(b.chats[0].jid, "2@s.whatsapp.net");
        assert_eq!(b.chats[0].messages[0].id, "3EB0");
        assert_eq!(b.chats[0].messages[0].status, Delivery::Sent);
        // A message only queued so far stays queued, also after a restart.
        let b = book(&[msg_record("2@s.whatsapp.net", "local-2", true, "", "c", 9, &Delivery::Queued, false)]);
        assert_eq!(b.chats[0].messages[0].status, Delivery::Queued);
    }

    #[test]
    fn statuses_stay_out_of_chats() {
        let mut b = Book::default();
        let now = 100_000;
        let records = [
            with_colors(msg_record(STATUS_CHAT, "s1", false, "111@lid", "pagi", now - 60, &Delivery::Received, false), Some(0xff_25_d3_66), None),
            msg_record(STATUS_CHAT, "s2", false, "222@s.whatsapp.net", "", now - 30, &Delivery::Received, false),
            msg_record(STATUS_CHAT, "old", false, "222@s.whatsapp.net", "", now - STATUS_LIFETIME - 1, &Delivery::Received, false),
            msg_record(STATUS_CHAT, "mine", true, "", "aku", now - 10, &Delivery::Sent, false),
            status_seen_record("s1"),
        ];
        for r in &records {
            b.apply(r);
        }
        assert!(b.chats.is_empty());
        let feed = b.status_feed(now);
        // Own updates first, then the poster with the newest update.
        assert_eq!(feed.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec![
            None,
            Some("222@s.whatsapp.net".to_owned()),
            Some("111@lid".to_owned()),
        ]);
        assert_eq!(feed[1].1.len(), 1, "expired updates are left out");
        assert!(feed[2].1[0].seen);
        assert_eq!(feed[2].1[0].background, Some(0xff_25_d3_66));
    }
}
