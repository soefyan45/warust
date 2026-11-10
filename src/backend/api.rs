//! Local REST API (see README, "REST API"), on 127.0.0.1 only. Every route but
//! `/healthz` needs `Authorization: Bearer <API key>`. Requests become the same
//! [`Command`]s the UI sends, so API sends go through the same anti-ban queue,
//! and answers are read from the shared snapshot. Errors are always
//! `{"error": {"code": "...", "message": "..."}}`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use whatsapp_rust::serde_json::{Value, json};

use super::session::Body;
use super::{ApiState, Backend, Book, Command, SessionView, Shared, Status, Updater, media};

#[derive(Clone)]
pub struct Context {
    pub commands: UnboundedSender<Command>,
    pub shared: Arc<Mutex<Shared>>,
    pub data_dir: PathBuf,
}

/// Starts the server when the settings turn it on; `None` when it is off.
pub fn start(context: &Context, state: &Updater) -> Option<JoinHandle<()>> {
    let api = state.settings().api.clone();
    if !api.enabled {
        state.set_api(ApiState::Off);
        return None;
    }
    let (context, state) = (context.clone(), state.clone());
    Some(tokio::spawn(async move {
        let address = format!("127.0.0.1:{}", api.port);
        let listener = match tokio::net::TcpListener::bind(&address).await {
            Ok(listener) => listener,
            Err(e) => {
                log::warn!("REST API cannot listen on {address}: {e}");
                state.set_api(ApiState::Failed(format!("Port {} tidak bisa dipakai: {e}", api.port)));
                return;
            }
        };
        log::info!("REST API listening on http://{address}");
        state.set_api(ApiState::Listening(format!("http://{address}")));
        if let Err(e) = axum::serve(listener, router(context)).await {
            state.set_api(ApiState::Failed(e.to_string()));
        }
    }))
}

fn router(context: Context) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", get(get_session).patch(rename_session).delete(delete_session))
        .route("/sessions/{id}/qr", get(get_qr))
        .route("/sessions/{id}/pair-code", post(pair_code))
        .route("/sessions/{id}/logout", post(logout))
        .route("/sessions/{id}/chats", get(list_chats))
        .route("/sessions/{id}/statuses", get(list_statuses))
        .route("/sessions/{id}/messages/text", post(send_text))
        .route("/sessions/{id}/messages/media", post(send_media))
        .route("/sessions/{id}/messages/{msg_id}", get(get_message))
        .route("/sessions/{id}/webhook", put(put_webhook))
        .fallback(|| async { error(StatusCode::NOT_FOUND, "not_found", "Endpoint tidak ada") })
        .with_state(context)
}

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": { "code": code, "message": message.into() } }))).into_response()
}

type Answer = Result<Response, Response>;

/// Proof that the request carried the API key.
struct Authed;

impl FromRequestParts<Context> for Authed {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, context: &Context) -> Result<Self, Self::Rejection> {
        let key = context.shared.lock().map(|s| s.settings.api.key.clone()).unwrap_or_default();
        let given = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or_default();
        // Compare every byte so the time taken says nothing about the key.
        let same = !key.is_empty()
            && given.len() == key.len()
            && given.bytes().zip(key.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
        if same {
            Ok(Authed)
        } else {
            Err(error(StatusCode::UNAUTHORIZED, "unauthorized", "API key salah atau tidak ada (Authorization: Bearer <key>)"))
        }
    }
}

fn session(context: &Context, id: &str) -> Result<SessionView, Response> {
    let shared = context.shared.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "state"))?;
    shared
        .sessions
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "session_not_found", format!("Sesi {id} tidak ada")))
}

fn session_json(view: &SessionView, shared_webhook: Option<&str>) -> Value {
    let (status, detail) = view.status.code();
    json!({
        "id": view.id,
        "name": view.name,
        "phone": view.phone,
        "status": status,
        "detail": detail,
        "has_qr": view.qr.is_some(),
        "pair_code": view.pair_code,
        "queue": view.queue,
        "incoming_today": view.incoming_today(),
        "sent_today": view.sent_today(),
        "webhook_url": shared_webhook,
    })
}

fn send(context: &Context, command: Command) -> Result<(), Response> {
    context
        .commands
        .send(command)
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "backend_stopped", "Backend berhenti"))
}

async fn healthz() -> Response {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })).into_response()
}

async fn list_sessions(_: Authed, State(context): State<Context>) -> Response {
    let shared = context.shared.lock().map(|s| s.clone()).unwrap_or_default();
    let list: Vec<Value> = shared
        .sessions
        .iter()
        .map(|v| session_json(v, shared.settings.webhook.url_for(&v.id)))
        .collect();
    Json(json!({ "sessions": list })).into_response()
}

async fn get_session(_: Authed, State(context): State<Context>, Path(id): Path<String>) -> Answer {
    let view = session(&context, &id)?;
    let settings = context.shared.lock().map(|s| s.settings.clone()).unwrap_or_default();
    Ok(Json(session_json(&view, settings.webhook.url_for(&id))).into_response())
}

#[derive(Deserialize, Default)]
struct CreateSession {
    name: Option<String>,
}

async fn create_session(_: Authed, State(context): State<Context>, body: Option<Json<CreateSession>>) -> Answer {
    let (count, mut id) = {
        let shared = context.shared.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "state"))?;
        (shared.sessions.len(), Backend::new_id())
    };
    // Ids are per second; never reuse one.
    while context.shared.lock().is_ok_and(|s| s.sessions.iter().any(|v| v.id == id)) {
        id.push('b');
    }
    let name = body
        .and_then(|Json(b)| b.name)
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("Nomor {}", count + 1));
    send(&context, Command::Add { id: id.clone(), name: name.clone() })?;
    let next = format!("GET /sessions/{id}/qr untuk QR, atau POST /sessions/{id}/pair-code");
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "status": "starting", "next": next }))).into_response())
}

async fn get_qr(_: Authed, State(context): State<Context>, Path(id): Path<String>) -> Answer {
    let view = session(&context, &id)?;
    let (status, _) = view.status.code();
    Ok(Json(json!({
        "status": status,
        "qr": view.qr.as_ref().map(|(qr, _)| qr),
        "pair_code": view.pair_code,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct PairCode {
    phone: String,
}

async fn pair_code(_: Authed, State(context): State<Context>, Path(id): Path<String>, Json(body): Json<PairCode>) -> Answer {
    let view = session(&context, &id)?;
    if view.status == Status::Connected {
        return Err(error(StatusCode::CONFLICT, "already_linked", "Nomor ini sudah tertaut"));
    }
    let phone: String = body.phone.chars().filter(char::is_ascii_digit).collect();
    if phone.len() < 8 || phone.starts_with('0') {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_phone", "Nomor harus dengan kode negara, mis. 628123456789"));
    }
    send(&context, Command::PairCode { id: id.clone(), phone })?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "requested", "next": format!("GET /sessions/{id}/qr → pair_code") })))
        .into_response())
}

async fn logout(_: Authed, State(context): State<Context>, Path(id): Path<String>) -> Answer {
    session(&context, &id)?;
    send(&context, Command::Logout { id })?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "logging_out" }))).into_response())
}

#[derive(Deserialize)]
struct RenameSession {
    name: String,
}

async fn rename_session(_: Authed, State(context): State<Context>, Path(id): Path<String>, Json(body): Json<RenameSession>) -> Answer {
    session(&context, &id)?;
    let name = super::registry::clean_name(&body.name).map_err(|why| error(StatusCode::BAD_REQUEST, "invalid_name", why))?;
    send(&context, Command::Rename { id: id.clone(), name: name.clone() })?;
    Ok(Json(json!({ "id": id, "name": name })).into_response())
}

async fn delete_session(_: Authed, State(context): State<Context>, Path(id): Path<String>) -> Answer {
    session(&context, &id)?;
    send(&context, Command::Remove { id })?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "removing" }))).into_response())
}

#[derive(Deserialize)]
struct ChatQuery {
    limit: Option<usize>,
}

async fn list_chats(
    _: Authed,
    State(context): State<Context>,
    Path(id): Path<String>,
    Query(query): Query<ChatQuery>,
) -> Answer {
    let view = session(&context, &id)?;
    let book = &view.book;
    let chats: Vec<Value> = book
        .chats
        .iter()
        .take(query.limit.unwrap_or(50).min(500))
        .map(|c| {
            let last = c.messages.last();
            json!({
                "jid": c.jid,
                "name": book.name_of(&c.jid),
                "is_group": c.is_group(),
                "unread": c.unread,
                "last_timestamp": c.last_ts(),
                "last_text": last.map(|m| &m.text),
            })
        })
        .collect();
    Ok(Json(json!({ "chats": chats })).into_response())
}

/// Status updates of the last 24 hours, by poster.
async fn list_statuses(_: Authed, State(context): State<Context>, Path(id): Path<String>) -> Answer {
    let view = session(&context, &id)?;
    let book = &view.book;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let feed: Vec<Value> = book
        .status_feed(now)
        .iter()
        .map(|(poster, posts)| {
            json!({
                "from": poster,
                "mine": poster.is_none(),
                "name": poster.as_ref().and_then(|p| book.name_of(p)),
                "updates": posts.iter().map(|s| json!({
                    "id": s.id,
                    "text": s.text,
                    "timestamp": s.ts,
                    "seen": s.seen,
                    "media": s.media.as_ref().map(|m| json!({ "kind": m.kind, "name": m.name, "size": m.size })),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Json(json!({ "statuses": feed })).into_response())
}

/// `628123456789`, `+62 812-3456-789` or a full JID → a JID.
fn to_jid(to: &str) -> Result<String, Response> {
    let to = to.trim();
    if to.contains('@') {
        return Ok(to.to_owned());
    }
    let digits: String = to.chars().filter(char::is_ascii_digit).collect();
    if digits.starts_with('0') || !(8..=15).contains(&digits.len()) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_to",
            "`to` harus nomor dengan kode negara (mis. 628123456789) atau JID lengkap",
        ));
    }
    Ok(format!("{digits}@s.whatsapp.net"))
}

/// Anti-ban: only chats that have written to this number, unless
/// the caller says `allow_cold: true`.
fn check_target(view: &SessionView, jid: &str, allow_cold: bool) -> Result<String, Response> {
    if view.status != Status::Connected {
        let (status, _) = view.status.code();
        return Err(error(StatusCode::CONFLICT, "not_connected", format!("Sesi tidak terhubung ({status})")));
    }
    let book: &Book = &view.book;
    let chat = book.normalize(jid);
    let known = book.chats.iter().any(|c| c.jid == chat && c.messages.iter().any(|m| !m.from_me));
    if !known && !allow_cold && !chat.ends_with("@g.us") {
        return Err(error(
            StatusCode::FORBIDDEN,
            "cold_contact",
            "Nomor ini belum pernah mengirim pesan ke nomor Anda. Kirim `allow_cold: true` bila memang disengaja.",
        ));
    }
    Ok(chat)
}

fn new_local_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("api-{nanos}")
}

fn queued(local_id: &str, id: &str) -> Response {
    (
        StatusCode::ACCEPTED,
        Json(json!({ "id": local_id, "status": "queued", "next": format!("GET /sessions/{id}/messages/{local_id}") })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct SendText {
    to: String,
    text: String,
    #[serde(default)]
    allow_cold: bool,
}

async fn send_text(_: Authed, State(context): State<Context>, Path(id): Path<String>, Json(body): Json<SendText>) -> Answer {
    let view = session(&context, &id)?;
    if body.text.trim().is_empty() || body.text.len() > 65_536 {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_text", "`text` kosong atau terlalu panjang"));
    }
    let chat = check_target(&view, &to_jid(&body.to)?, body.allow_cold)?;
    let local_id = new_local_id();
    send(&context, Command::Enqueue { id: id.clone(), chat, body: Body::Text(body.text), local_id: local_id.clone() })?;
    Ok(queued(&local_id, &id))
}

#[derive(Deserialize)]
struct SendMedia {
    to: String,
    /// A file on this computer.
    path: Option<String>,
    /// Or a file to fetch first (http/https).
    url: Option<String>,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    as_sticker: bool,
    #[serde(default)]
    allow_cold: bool,
}

/// Fetches a URL into `data/uploads/`, at most `media::MAX_BYTES`.
fn fetch(url: &str, dir: &std::path::Path) -> Result<PathBuf, String> {
    let mut response = ureq::get(url).call().map_err(|e| format!("Gagal mengambil URL: {e}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(media::MAX_BYTES)
        .read_to_vec()
        .map_err(|e| format!("Gagal membaca URL: {e}"))?;
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .filter(|n| !n.is_empty())
        .unwrap_or("file")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect::<String>();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}-{name}", new_local_id()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

async fn send_media(_: Authed, State(context): State<Context>, Path(id): Path<String>, Json(body): Json<SendMedia>) -> Answer {
    let view = session(&context, &id)?;
    let chat = check_target(&view, &to_jid(&body.to)?, body.allow_cold)?;
    let path = match (body.path, body.url) {
        (Some(path), _) => {
            let path = PathBuf::from(path);
            if !path.is_file() {
                return Err(error(StatusCode::BAD_REQUEST, "file_not_found", "`path` bukan file yang ada"));
            }
            path
        }
        (None, Some(url)) if url.starts_with("http://") || url.starts_with("https://") => {
            let dir = context.data_dir.join("uploads");
            tokio::task::spawn_blocking(move || fetch(&url, &dir))
                .await
                .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
                .map_err(|e| error(StatusCode::BAD_GATEWAY, "fetch_failed", e))?
        }
        _ => return Err(error(StatusCode::BAD_REQUEST, "no_file", "Isi `path` (file lokal) atau `url` (http/https)")),
    };
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > media::MAX_BYTES {
        return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "too_large", format!("File lebih dari {} MB", media::MAX_BYTES / 1024 / 1024)));
    }
    let message = if body.as_sticker { Body::Sticker(path) } else { Body::File { path, caption: body.caption } };
    let local_id = new_local_id();
    send(&context, Command::Enqueue { id: id.clone(), chat, body: message, local_id: local_id.clone() })?;
    Ok(queued(&local_id, &id))
}

async fn get_message(_: Authed, State(context): State<Context>, Path((id, msg_id)): Path<(String, String)>) -> Answer {
    let view = session(&context, &id)?;
    let book = &view.book;
    let real = book.aliases.get(&msg_id).cloned().unwrap_or_else(|| msg_id.clone());
    let found = book
        .chats
        .iter()
        .find_map(|c| c.messages.iter().find(|m| m.id == real || m.id == msg_id).map(|m| (c, m)));
    let Some((chat, m)) = found else {
        return Err(error(StatusCode::NOT_FOUND, "message_not_found", "Pesan tidak ditemukan"));
    };
    let code = m.status.code();
    let (status, failure) = match code.split_once(':') {
        Some(("failed", reason)) => ("failed".to_owned(), Some(reason.to_owned())),
        _ => (code, None),
    };
    Ok(Json(json!({
        "id": m.id,
        "local_id": (msg_id != m.id).then_some(&msg_id),
        "chat": chat.jid,
        "from_me": m.from_me,
        "text": m.text,
        "timestamp": m.ts,
        "status": status,
        "error": failure,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct PutWebhook {
    /// Empty or null: use the global webhook again.
    url: Option<String>,
}

async fn put_webhook(_: Authed, State(context): State<Context>, Path(id): Path<String>, Json(body): Json<PutWebhook>) -> Answer {
    session(&context, &id)?;
    let url = body.url.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty());
    if let Some(url) = &url {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_url", "URL harus http:// atau https://"));
        }
    }
    let mut settings = context.shared.lock().map(|s| (*s.settings).clone()).unwrap_or_default();
    match &url {
        Some(url) => settings.webhook.overrides.insert(id.clone(), url.clone()),
        None => settings.webhook.overrides.remove(&id),
    };
    let effective = settings.webhook.url_for(&id).map(str::to_owned);
    send(&context, Command::SaveSettings(settings))?;
    Ok(Json(json!({ "session": id, "override": url, "webhook_url": effective })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(to_jid("+62 812-3456-7890").ok(), Some("6281234567890@s.whatsapp.net".to_owned()));
        assert_eq!(to_jid("120363@g.us").ok(), Some("120363@g.us".to_owned()));
        assert!(to_jid("08123456789").is_err());
        assert!(to_jid("123").is_err());
    }

    fn call(method: &str, url: &str, key: Option<&str>, body: Option<&str>) -> (u16, Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let auth = key.map(|k| format!("Bearer {k}"));
        let response = match method {
            "GET" => {
                let mut r = agent.get(url);
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                r.call()
            }
            _ => {
                let mut r = agent.post(url).header("Content-Type", "application/json");
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                r.send(body.unwrap_or("{}"))
            }
        };
        let mut response = response.unwrap();
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        (status, whatsapp_rust::serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// The real router on a free port: auth, listing, and the anti-ban checks.
    #[tokio::test(flavor = "multi_thread")]
    async fn serves_requests() {
        let mut shared = Shared::default();
        let key = shared.settings.api.key.clone();
        let mut connected = SessionView::new("s1", "Nomor 1");
        connected.status = Status::Connected;
        shared.sessions.push(connected);
        let (commands, mut sent) = tokio::sync::mpsc::unbounded_channel();
        let context = Context { commands, shared: Arc::new(Mutex::new(shared)), data_dir: std::env::temp_dir() };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router(context)).await });

        let key2 = key.clone();
        let results = tokio::task::spawn_blocking(move || {
            vec![
                call("GET", &format!("{base}/healthz"), None, None),
                call("GET", &format!("{base}/sessions"), None, None),
                call("GET", &format!("{base}/sessions"), Some("wr_wrong"), None),
                call("GET", &format!("{base}/sessions"), Some(&key2), None),
                call("GET", &format!("{base}/sessions/nope"), Some(&key2), None),
                call("POST", &format!("{base}/sessions/s1/messages/text"), Some(&key2), Some(r#"{"to":"6281234567890","text":"hai"}"#)),
                call("POST", &format!("{base}/sessions/s1/messages/text"), Some(&key2), Some(r#"{"to":"6281234567890","text":"hai","allow_cold":true}"#)),
            ]
        })
        .await
        .unwrap();
        assert_eq!(results[0].0, 200);
        assert_eq!(results[1].0, 401);
        assert_eq!(results[2].0, 401);
        assert_eq!(results[3].0, 200);
        assert_eq!(results[3].1["sessions"][0]["status"], "connected");
        assert_eq!(results[4].0, 404);
        assert_eq!(results[5].0, 403);
        assert_eq!(results[5].1["error"]["code"], "cold_contact");
        assert_eq!(results[6].0, 202);
        let id = results[6].1["id"].as_str().unwrap().to_owned();
        match sent.try_recv() {
            Ok(Command::Enqueue { chat, local_id, .. }) => {
                assert_eq!(chat, "6281234567890@s.whatsapp.net");
                assert_eq!(local_id, id);
            }
            _ => panic!("expected an Enqueue command"),
        }
    }
}
