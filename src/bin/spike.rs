//! Phase 0 spike: pair one number, log incoming messages and receipts, and
//! send a text. A test tool from before the gateway; `cargo run --bin spike`.
//!
//! Usage:
//!   cargo run --bin spike                       # QR pairing
//!   cargo run --bin spike -- --phone 628xxxxxxxxx  # pair code (and QR concurrently)
//!   cargo run --bin spike -- --send-to 628xxxxxxxxx --text "hello"  # send once after connecting
//!
//! Env: DEVICE_NAME (linked-device name, default "Warust"), RUST_LOG.
//! A message "!ping" received from another chat is answered with "pong".

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use log::{error, info, warn};
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::store::DevicePropsOverride;

const DEFAULT_DB: &str = "data/sessions/spike.db";
const DEFAULT_DEVICE_NAME: &str = "Warust";

struct Args {
    db: PathBuf,
    phone: Option<String>,
    send_to: Option<String>,
    text: String,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    Args {
        db: arg(&args, "--db").map_or_else(|| PathBuf::from(DEFAULT_DB), PathBuf::from),
        phone: arg(&args, "--phone"),
        send_to: arg(&args, "--send-to"),
        text: arg(&args, "--text").unwrap_or_else(|| "warust spike test".to_owned()),
    }
}

/// Accepts a bare phone number (digits only) or a full JID.
fn to_jid(target: &str) -> Result<Jid, String> {
    let target = target.trim().trim_start_matches('+');
    if target.contains('@') {
        target.parse().map_err(|e| format!("invalid JID {target}: {e:?}"))
    } else if !target.is_empty() && target.chars().all(|c| c.is_ascii_digit()) {
        format!("{target}@s.whatsapp.net")
            .parse()
            .map_err(|e| format!("invalid number {target}: {e:?}"))
    } else {
        Err(format!("expected digits or a JID, got {target}"))
    }
}

/// Masks the middle of a JID user part so logs never hold a full number.
fn mask(jid: &Jid) -> String {
    let s = jid.to_string();
    let (user, server) = s.split_once('@').unwrap_or((&s, ""));
    // Drop the device suffix (`user:device`) so one person always masks the same way.
    let user = user.split(':').next().unwrap_or(user);
    if user.len() <= 6 {
        return s.clone();
    }
    format!("{}***{}@{server}", &user[..4], &user[user.len() - 2..])
}

/// Draws the QR in the terminal with the system `qrencode`, if installed.
fn render_qr(code: &str) -> Option<String> {
    let output = std::process::Command::new("qrencode")
        .args(["-t", "ansiutf8", "-m", "2", code])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,whatsapp_rust=warn,wacore=warn"))
        .init();

    let args = parse_args();
    let send_target = match args.send_to.as_deref().map(to_jid).transpose() {
        Ok(target) => target,
        Err(e) => {
            error!("{e}");
            return;
        }
    };

    if let Some(parent) = args.db.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error!("cannot create {}: {e}", parent.display());
            return;
        }
    }
    // Same lock as the desktop app (data/sessions/<id>.lock): two connections
    // with the same keys kick each other off (StreamReplaced).
    let lock_path = args.db.with_extension("lock");
    let _lock = match std::fs::File::create(&lock_path).map(|f| (f.try_lock().is_ok(), f)) {
        Ok((true, file)) => file,
        Ok((false, _)) => {
            error!("session {} is in use by another process (the desktop app?)", args.db.display());
            return;
        }
        Err(e) => {
            error!("cannot create {}: {e}", lock_path.display());
            return;
        }
    };
    let store = match SqliteStore::open(args.db.to_string_lossy().as_ref()).await {
        Ok(store) => store,
        Err(e) => {
            error!("cannot open device store: {e}");
            return;
        }
    };

    // Neutral, stable device identity: never the server hostname.
    let device_name =
        std::env::var("DEVICE_NAME").unwrap_or_else(|_| DEFAULT_DEVICE_NAME.to_owned());

    let sent_once = Arc::new(AtomicBool::new(false));
    let text = args.text.clone();

    let mut builder = Bot::builder()
        .with_backend(store)
        .with_device_props(
            DevicePropsOverride::new()
                .with_os(device_name)
                .with_platform_type(wa::device_props::PlatformType::DESKTOP),
        )
        .on_qr_code(|code, timeout| async move {
            info!("QR (valid {} s), scan it from WhatsApp > Linked devices", timeout.as_secs());
            match render_qr(&code) {
                Some(art) => println!("\n{art}"),
                None => {
                    warn!("qrencode not found; install it or use --phone for a pair code");
                    println!("\n{code}\n");
                }
            }
        })
        .on_pair_code(|code, timeout| async move {
            info!("PAIR CODE (valid {} s): {code}", timeout.as_secs());
            info!("Phone: WhatsApp > Linked devices > Link a device > Link with phone number");
        })
        .on_connected(move |client| {
            let send_target = send_target.clone();
            let sent_once = sent_once.clone();
            let text = text.clone();
            async move {
                info!("connected");
                let Some(to) = send_target else { return };
                // on_connected fires again after every reconnect; send only once.
                if sent_once.swap(true, Ordering::SeqCst) {
                    return;
                }
                match client.send_message(&to, wa::Message::text(text)).await {
                    Ok(result) => info!("sent id={} to={}", result.message_id, mask(&result.to)),
                    Err(e) => error!("send failed: {e}"),
                }
            }
        })
        .on_logged_out(|info| async move {
            error!("logged out: {info:?}");
        })
        .on_message(|ctx| async move {
            let src = &ctx.info.source;
            let text = ctx.message.text_content();
            info!(
                "message id={} chat={} sender={} from_me={} group={} text={:?}",
                ctx.info.id,
                mask(&src.chat),
                mask(&src.sender),
                src.is_from_me,
                src.is_group,
                text.map(|t| t.chars().take(40).collect::<String>()),
            );
            if !src.is_from_me && text == Some("!ping") {
                if let Err(e) = ctx.reply("pong").await {
                    error!("reply failed: {e}");
                }
            }
        })
        .on_event(|event, _client| async move {
            match event.as_ref() {
                Event::Receipt(r) => info!(
                    "receipt {:?} ids={:?} chat={} offline={}",
                    r.r#type,
                    r.message_ids,
                    mask(&r.source.chat),
                    r.offline
                ),
                Event::Disconnected(_) => warn!("disconnected"),
                Event::StreamReplaced(_) => warn!("stream replaced (another client took over)"),
                Event::TemporaryBan(b) => error!("TEMPORARY BAN: {b:?}"),
                Event::ConnectFailure(f) => error!("connect failure: {f:?}"),
                _ => {}
            }
        });

    if let Some(phone) = args.phone {
        builder = builder.with_pair_code(PairCodeOptions {
            phone_number: phone,
            ..Default::default()
        });
    }

    let bot = match builder.build().await {
        Ok(bot) => bot,
        Err(e) => {
            error!("cannot build bot: {e}");
            return;
        }
    };

    let mut handle = bot.spawn();
    tokio::select! {
        _ = &mut handle => {}
        _ = shutdown_signal() => {
            info!("shutting down");
            handle.shutdown().await;
        }
    }
}
