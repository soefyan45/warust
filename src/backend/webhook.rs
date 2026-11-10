//! Webhook delivery. Events (`message.received`, `message.status`,
//! `session.status`) are POSTed as JSON to the session's URL (its override,
//! else the global one), signed with HMAC-SHA256 of the body, and retried with
//! backoff while the endpoint is down. The last deliveries show in the UI.

use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tokio::sync::mpsc::UnboundedReceiver;
use whatsapp_rust::serde_json::{Value, json};

use super::rules::{Incoming, render};
use super::session::Body;
use super::{Command, Updater};

/// Waits before each attempt; five attempts over about 13 minutes.
const BACKOFF: [u64; 5] = [0, 5, 30, 120, 600];
/// Triggers: the first try and three more, as the design says.
const RULE_BACKOFF: [u64; 4] = [0, 5, 30, 120];
const TIMEOUT: Duration = Duration::from_secs(10);
/// Deliveries kept for the UI.
pub const LOG_SIZE: usize = 100;

pub struct Event {
    pub session: String,
    pub event: &'static str,
    pub data: Value,
    /// Send even when this kind of event is turned off (the test button).
    pub forced: bool,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Pending,
    /// HTTP status of the last attempt.
    Done(u16),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Delivery {
    pub id: String,
    /// Unix seconds of the first attempt.
    pub at: i64,
    pub event: String,
    pub session: String,
    pub url: String,
    pub attempts: u32,
    pub outcome: Outcome,
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `sha256=<hex>`: receivers recompute it over the raw body with the secret.
pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes keys of any length");
    mac.update(body);
    let hex: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={hex}")
}

/// Statuses worth another try: timeouts, rate limits, server errors.
fn retryable(status: u16) -> bool {
    matches!(status, 408 | 429) || status >= 500
}

fn post(method: &str, url: &str, event: &str, delivery: &str, signature: &str, body: &[u8]) -> Result<u16, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let request = if method.eq_ignore_ascii_case("PUT") { agent.put(url) } else { agent.post(url) };
    request
        .header("Content-Type", "application/json")
        .header("User-Agent", "Warust-Webhook/1")
        .header("X-Warust-Event", event)
        .header("X-Warust-Delivery", delivery)
        .header("X-Warust-Signature", signature)
        .send(body)
        .map(|response| response.status().as_u16())
        .map_err(|e| e.to_string())
}

/// Turns events into deliveries until the app closes.
pub async fn dispatcher(mut events: UnboundedReceiver<Event>, state: Updater) {
    let mut counter: u64 = 0;
    while let Some(event) = events.recv().await {
        let settings = state.settings();
        let hook = &settings.webhook;
        if !event.forced && !hook.wants(event.event) {
            continue;
        }
        let Some(url) = hook.url_for(&event.session).map(str::to_owned) else {
            continue;
        };
        counter += 1;
        let at = unix_secs();
        let id = format!("d{at}-{counter}");
        let body = json!({
            "event": event.event,
            "session": event.session,
            "delivery_id": id,
            "timestamp": at,
            "data": event.data,
        })
        .to_string()
        .into_bytes();
        let signature = sign(&hook.secret, &body);
        let delivery = Delivery {
            id,
            at,
            event: event.event.to_owned(),
            session: event.session,
            url,
            attempts: 0,
            outcome: Outcome::Pending,
        };
        // Each delivery retries on its own, so one slow endpoint holds up nothing else.
        tokio::spawn(deliver(state.clone(), delivery, "POST", signature, body, &BACKOFF, None));
    }
}

/// Runs every enabled trigger that matches an incoming message.
pub fn fire_rules(state: &Updater, message: &Incoming) {
    let settings = state.settings();
    for rule in settings.rules.iter().filter(|r| r.matches(message)) {
        state.count_rule(&rule.id);
        let body = render(&rule.body, message, true).into_bytes();
        let at = unix_secs();
        let delivery = Delivery {
            id: format!("r{at}-{}-{}", rule.id, message.id),
            at,
            event: format!("pemicu: {}", rule.name),
            session: message.session.clone(),
            url: rule.url.clone(),
            attempts: 0,
            outcome: Outcome::Pending,
        };
        let signature = sign(&settings.webhook.secret, &body);
        // The reply goes out only once the endpoint took the message.
        let reply = (rule.reply_enabled && !rule.reply.trim().is_empty()).then(|| Command::Enqueue {
            id: message.session.clone(),
            chat: message.chat.clone(),
            body: Body::Text(render(&rule.reply, message, false)),
            local_id: format!("rule-{at}-{}", rule.id),
        });
        let method = if rule.method.eq_ignore_ascii_case("PUT") { "PUT" } else { "POST" };
        tokio::spawn(deliver(state.clone(), delivery, method, signature, body, &RULE_BACKOFF, reply));
    }
}

async fn deliver(
    state: Updater,
    mut delivery: Delivery,
    method: &'static str,
    signature: String,
    body: Vec<u8>,
    backoff: &'static [u64],
    mut on_success: Option<Command>,
) {
    state.log_delivery(&delivery);
    for wait in backoff {
        tokio::time::sleep(Duration::from_secs(*wait)).await;
        delivery.attempts += 1;
        let (url, event, id, signature, body) =
            (delivery.url.clone(), delivery.event.clone(), delivery.id.clone(), signature.clone(), body.clone());
        let result = tokio::task::spawn_blocking(move || post(method, &url, &event, &id, &signature, &body))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        if matches!(result, Ok(status) if (200..300).contains(&status)) {
            if let Some(command) = on_success.take() {
                state.send(command);
            }
        }
        let again = match &result {
            Ok(status) => {
                delivery.outcome = Outcome::Done(*status);
                retryable(*status)
            }
            Err(e) => {
                delivery.outcome = Outcome::Failed(e.clone());
                true
            }
        };
        state.log_delivery(&delivery);
        if !again {
            return;
        }
    }
    log::warn!("webhook {} gave up after {} attempts", delivery.event, delivery.attempts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_known_hmac() {
        // RFC 4231 test case 2.
        assert_eq!(
            sign("Jefe", b"what do ya want for nothing?"),
            "sha256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert!(retryable(503) && retryable(429) && !retryable(404) && !retryable(200));
    }

    /// A matching trigger POSTs the rendered, signed body to a real local
    /// server, and only after its 2xx queues the reply from the receiving number.
    #[tokio::test(flavor = "multi_thread")]
    async fn trigger_posts_then_replies() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::mpsc::unbounded_channel;

        let (seen_tx, mut seen) = unbounded_channel::<(String, String)>();
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
                let seen_tx = seen_tx.clone();
                async move {
                    let signature = headers.get("x-warust-signature").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                    let _ = seen_tx.send((signature, body));
                    "ok"
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });

        let mut settings = crate::backend::Settings::default();
        settings.rules.push(crate::backend::rules::Rule {
            id: "r1".to_owned(),
            value: "register".to_owned(),
            url: format!("http://{address}/hook"),
            reply_enabled: true,
            ..Default::default()
        });
        // A second rule that must not fire.
        settings.rules.push(crate::backend::rules::Rule {
            id: "r2".to_owned(),
            value: "komplain".to_owned(),
            url: format!("http://{address}/hook"),
            ..Default::default()
        });
        let secret = settings.webhook.secret.clone();
        let shared = Arc::new(Mutex::new(crate::backend::Shared { settings: Arc::new(settings), ..Default::default() }));
        let (hooks, _hook_rx) = unbounded_channel();
        let (commands, mut queued) = unbounded_channel();
        let state = Updater { shared: shared.clone(), ctx: egui::Context::default(), data_dir: std::env::temp_dir(), hooks, commands };

        fire_rules(&state, &Incoming::sample());
        let (signature, body) = tokio::time::timeout(Duration::from_secs(5), seen.recv()).await.unwrap().unwrap();
        assert_eq!(signature, sign(&secret, body.as_bytes()));
        let json: Value = whatsapp_rust::serde_json::from_str(&body).unwrap();
        assert_eq!(json["teks"], "REGISTER Budi Santoso");
        assert_eq!(json["pengirim"], "628123456789");
        match tokio::time::timeout(Duration::from_secs(5), queued.recv()).await.unwrap() {
            Some(Command::Enqueue { chat, body: Body::Text(text), .. }) => {
                assert_eq!(chat, "628123456789@s.whatsapp.net");
                assert_eq!(text, "Thank you Budi Santoso, we are processing your message.");
            }
            _ => panic!("expected the reply to be queued"),
        }
        assert!(tokio::time::timeout(Duration::from_millis(300), seen.recv()).await.is_err(), "only one rule fired");
        assert_eq!(shared.lock().unwrap().rule_hits.get("r1").map(|h| h.1), Some(1));
    }
}
