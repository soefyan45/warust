//! Webhook triggers (Pego `v5/Webhook & API`, "Pemicu webhook"): when an
//! incoming message matches a rule, its fields go to a REST endpoint as JSON
//! built from a template, and on a 2xx answer an optional reply goes back to
//! the sender from the number that received the message.

use crate::i18n::tr;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use whatsapp_rust::serde_json::{self, Value};

/// How the message text is compared with `Rule::value`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    #[default]
    StartsWith,
    Contains,
    Equals,
    Regex,
    /// Every message (of the chosen kind).
    Any,
}

impl Op {
    pub const ALL: [Op; 5] = [Op::StartsWith, Op::Contains, Op::Equals, Op::Regex, Op::Any];

    pub fn label(self) -> &'static str {
        match self {
            Op::StartsWith => tr("starts with", "diawali dengan"),
            Op::Contains => tr("contains", "berisi"),
            Op::Equals => tr("equals", "sama dengan"),
            Op::Regex => tr("matches regex", "cocok regex"),
            Op::Any => tr("anything", "apa saja"),
        }
    }
}

/// Which messages a rule looks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Any,
    Text,
    Image,
    Document,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Any, Kind::Text, Kind::Image, Kind::Document];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Any => tr("All messages", "Semua pesan"),
            Kind::Text => tr("Message text", "Teks pesan"),
            Kind::Image => tr("Image + caption", "Gambar + keterangan"),
            Kind::Document => tr("Document", "Dokumen"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub kind: Kind,
    pub op: Op,
    pub value: String,
    pub ignore_case: bool,
    /// Skip group chats.
    pub private_only: bool,
    /// Session ids; empty means every number, including ones linked later.
    pub sessions: Vec<String>,
    /// `POST` or `PUT`.
    pub method: String,
    pub url: String,
    /// JSON with `{{placeholders}}`; see `placeholders()`.
    pub body: String,
    pub reply_enabled: bool,
    /// Plain text with the same placeholders.
    pub reply: String,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: tr("New rule", "Aturan baru").to_owned(),
            enabled: true,
            kind: Kind::Text,
            op: Op::StartsWith,
            value: String::new(),
            ignore_case: true,
            private_only: true,
            sessions: Vec::new(),
            method: "POST".to_owned(),
            url: String::new(),
            body: "{\n  \"teks\": \"{{teks}}\",\n  \"pengirim\": \"{{pengirim.nomor}}\",\n  \"nama\": \"{{pengirim.nama}}\",\n  \"masuk_ke\": \"{{perangkat.nomor}}\"\n}".to_owned(),
            reply_enabled: false,
            reply: tr("Thank you {{nama}}, we are processing your message.", "Terima kasih {{nama}}, pesanmu sedang kami proses.").to_owned(),
        }
    }
}

pub fn placeholders() -> [(&'static str, &'static str); 10] {
    [
        ("{{teks}}", tr("message text (caption for images/documents)", "isi pesan (keterangan untuk gambar/dokumen)")),
        ("{{teks.setelah_kata}}", tr("message text without the first word", "isi pesan tanpa kata pertama")),
        ("{{pengirim.nomor}}", tr("sender number, e.g. 628123456789", "nomor pengirim, mis. 628123456789")),
        ("{{pengirim.jid}}", tr("sender JID", "JID pengirim")),
        ("{{pengirim.nama}}", tr("contact name or profile name; {{nama}} is the same", "nama kontak atau nama profil; {{nama}} sama")),
        ("{{perangkat.nomor}}", tr("the warust number that received it", "nomor warust yang menerima")),
        ("{{perangkat.id}}", tr("session id of that number", "id sesi nomor itu")),
        ("{{pesan.id}}", tr("WhatsApp message id", "id pesan WhatsApp")),
        ("{{pesan.jenis}}", tr("text, image, document, ...", "teks, gambar, dokumen, ...")),
        ("{{waktu}}", tr("Unix seconds", "Unix detik")),
    ]
}

/// An incoming message, as rules see it.
#[derive(Clone, Debug, Default)]
pub struct Incoming {
    pub session: String,
    /// The receiving number, digits only (may be empty before it is known).
    pub own_number: String,
    pub id: String,
    pub chat: String,
    pub sender_jid: String,
    pub sender_name: String,
    /// The text, or a photo's/document's caption.
    pub text: String,
    /// `text`, `image`, `video`, `document`, `audio`, `sticker`.
    pub kind: String,
    pub is_group: bool,
    pub timestamp: i64,
}

impl Rule {
    /// Whether this rule fires for the message. A bad regex never matches.
    pub fn matches(&self, m: &Incoming) -> bool {
        if !self.enabled || (self.private_only && m.is_group) {
            return false;
        }
        if !self.sessions.is_empty() && !self.sessions.contains(&m.session) {
            return false;
        }
        let kind_ok = match self.kind {
            Kind::Any => true,
            Kind::Text => m.kind == "text",
            Kind::Image => m.kind == "image",
            Kind::Document => m.kind == "document",
        };
        if !kind_ok {
            return false;
        }
        let value = self.value.trim();
        let (text, value) = if self.ignore_case && self.op != Op::Regex {
            (m.text.trim().to_lowercase(), value.to_lowercase())
        } else {
            (m.text.trim().to_owned(), value.to_owned())
        };
        match self.op {
            Op::Any => true,
            Op::StartsWith => !value.is_empty() && text.starts_with(&value),
            Op::Contains => !value.is_empty() && text.contains(&value),
            Op::Equals => text == value,
            Op::Regex => RegexBuilder::new(&value)
                .case_insensitive(self.ignore_case)
                .size_limit(1 << 20)
                .build()
                .is_ok_and(|re| re.is_match(&text)),
        }
    }

    /// Why the rule cannot be saved, if it cannot.
    pub fn problem(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some(tr("The rule name is empty", "Nama aturan kosong").to_owned());
        }
        if !(self.url.starts_with("http://") || self.url.starts_with("https://")) {
            return Some(crate::trf!("{}: the target URL must be http:// or https://", "{}: URL tujuan harus http:// atau https://", self.name));
        }
        if self.op != Op::Any && self.op != Op::Equals && self.value.trim().is_empty() {
            return Some(crate::trf!("{}: fill in the condition word/pattern", "{}: isi kata/pola kondisi", self.name));
        }
        if self.op == Op::Regex {
            if let Err(e) = RegexBuilder::new(self.value.trim()).size_limit(1 << 20).build() {
                return Some(crate::trf!("{}: invalid regex ({e})", "{}: regex tidak valid ({e})", self.name));
            }
        }
        if let Err(e) = serde_json::from_str::<Value>(&render(&self.body, &Incoming::sample(), true)) {
            return Some(crate::trf!("{}: the body is not valid JSON ({e})", "{}: body bukan JSON yang valid ({e})", self.name));
        }
        None
    }

    /// One line for the rule list: `teks diawali "REGISTER"`.
    pub fn summary(&self) -> String {
        let what = match self.kind {
            Kind::Any => tr("message", "pesan"),
            Kind::Text => tr("text", "teks"),
            Kind::Image => tr("image", "gambar"),
            Kind::Document => tr("document", "dokumen"),
        };
        match self.op {
            Op::Any => crate::trf!("every incoming {what}", "setiap {what} masuk"),
            op => format!("{what} {} \"{}\"", op.label(), self.value.trim()),
        }
    }
}

impl Incoming {
    /// The message the editor's test uses when the user gives no text.
    pub fn sample() -> Self {
        Self {
            session: "contoh".to_owned(),
            own_number: "628120000000".to_owned(),
            id: "3EB0CONTOH".to_owned(),
            chat: "628123456789@s.whatsapp.net".to_owned(),
            sender_jid: "628123456789@s.whatsapp.net".to_owned(),
            sender_name: "Budi Santoso".to_owned(),
            text: "REGISTER Budi Santoso".to_owned(),
            kind: "text".to_owned(),
            is_group: false,
            timestamp: 1_762_591_234,
        }
    }

    fn sender_number(&self) -> String {
        let user = self.sender_jid.split('@').next().unwrap_or_default();
        user.split(':').next().unwrap_or(user).to_owned()
    }

    fn kind_label(&self) -> &str {
        match self.kind.as_str() {
            "text" => "teks",
            "image" => "gambar",
            "document" => "dokumen",
            "video" => "video",
            "audio" => "suara",
            "sticker" => "stiker",
            other => other,
        }
    }
}

/// Fills the placeholders; with `json`, values are escaped for a JSON string.
pub fn render(template: &str, m: &Incoming, json: bool) -> String {
    let after_first = m.text.trim().split_once(char::is_whitespace).map(|(_, rest)| rest.trim()).unwrap_or("");
    let values: [(&str, String); 11] = [
        ("{{teks.setelah_kata}}", after_first.to_owned()),
        ("{{teks}}", m.text.clone()),
        ("{{pengirim.nomor}}", m.sender_number()),
        ("{{pengirim.jid}}", m.sender_jid.clone()),
        ("{{pengirim.nama}}", m.sender_name.clone()),
        ("{{nama}}", m.sender_name.clone()),
        ("{{perangkat.nomor}}", m.own_number.clone()),
        ("{{perangkat.id}}", m.session.clone()),
        ("{{pesan.id}}", m.id.clone()),
        ("{{pesan.jenis}}", m.kind_label().to_owned()),
        ("{{waktu}}", m.timestamp.to_string()),
    ];
    let mut out = template.to_owned();
    for (key, value) in values {
        let value = if json {
            // The string's content without its quotes.
            let quoted = Value::String(value).to_string();
            quoted[1..quoted.len() - 1].to_owned()
        } else {
            value
        };
        out = out.replace(key, &value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(op: Op, value: &str) -> Rule {
        Rule { op, value: value.to_owned(), url: "https://crm.test/hook".to_owned(), ..Rule::default() }
    }

    #[test]
    fn conditions() {
        let m = Incoming::sample();
        assert!(rule(Op::StartsWith, "register").matches(&m));
        assert!(!Rule { ignore_case: false, ..rule(Op::StartsWith, "register") }.matches(&m));
        assert!(rule(Op::Contains, "budi").matches(&m));
        assert!(!rule(Op::Equals, "register").matches(&m));
        assert!(rule(Op::Regex, r"^register\s+\w+").matches(&m));
        assert!(!rule(Op::Regex, "(").matches(&m), "a bad regex never matches");
        assert!(!rule(Op::StartsWith, "").matches(&m), "an empty prefix never matches");
        let group = Incoming { is_group: true, ..m.clone() };
        assert!(!rule(Op::Any, "").matches(&group));
        let other = Incoming { session: "s9".to_owned(), ..m.clone() };
        assert!(!Rule { sessions: vec!["s1".to_owned()], ..rule(Op::Any, "") }.matches(&other));
        let photo = Incoming { kind: "image".to_owned(), ..m };
        assert!(!rule(Op::Any, "").matches(&photo), "the default kind is text");
        assert!(Rule { kind: Kind::Image, ..rule(Op::Any, "") }.matches(&photo));
    }

    #[test]
    fn templates_escape_json() {
        let m = Incoming { text: "REGISTER Budi \"B\"\nSantoso".to_owned(), ..Incoming::sample() };
        let body = render(&Rule::default().body, &m, true);
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["teks"], "REGISTER Budi \"B\"\nSantoso");
        assert_eq!(parsed["pengirim"], "628123456789");
        assert_eq!(render("{{teks.setelah_kata}}|{{nama}}", &m, false), "Budi \"B\"\nSantoso|Budi Santoso");
        assert!(rule(Op::StartsWith, "REGISTER").problem().is_none());
        assert!(Rule { body: "{ \"a\": {{teks}} }".to_owned(), ..rule(Op::Any, "") }.problem().is_some());
    }
}
