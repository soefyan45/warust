//! Chats and messages of one session, kept in memory as a [`Book`] and on disk
//! as JSON Lines (`data/chats/<session>.jsonl`, one record per line, replayed
//! at start). WhatsApp sends the history only once, at pairing, so it has to
//! be kept here.

use std::collections::HashMap;
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
}

impl Chat {
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
                    self.lid_to_pn.insert(bare(&lid), bare(&pn));
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

pub fn seen_record(chat: &str) -> Value {
    json!({ "t": "seen", "chat": chat })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reply_keeps_its_quote() {
        let quote = Quote { id: "ST1".to_owned(), text: "Pesanan katering".to_owned(), status: true };
        let record = with_quote(msg_record("628111@s.whatsapp.net", "R1", true, "", "👍", 10, &Delivery::Queued, false), &quote);
        let b = book(&[record]);
        let m = &b.chats[0].messages[0];
        let q = m.quote.as_ref().expect("quote kept");
        assert_eq!((q.id.as_str(), q.text.as_str(), q.status), ("ST1", "Pesanan katering", true));
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
