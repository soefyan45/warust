//! Desktop notifications for incoming messages with the OS's own tools; no
//! extra crates. Linux: `notify-send` (libnotify) and `canberra-gtk-play`, one
//! notification per chat that a newer message replaces and a click opens.
//! macOS: `osascript` notifications and `afplay`. Windows: a PowerShell
//! balloon and the system sound. On macOS and Windows a click does not open the
//! chat.

use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::Updater;
use super::session::mask_phone;
use super::store::Book;

/// Longest message text shown in a notification.
const PREVIEW_CHARS: usize = 160;
/// At most one sound in this time, so a burst of messages plays it once.
const SOUND_GAP: Duration = Duration::from_secs(2);

/// Per chat: the notification id (to replace it) and the `notify-send` that
/// waits for its click.
fn shown() -> &'static Mutex<HashMap<String, (u32, Child)>> {
    static SHOWN: OnceLock<Mutex<HashMap<String, (u32, Child)>>> = OnceLock::new();
    SHOWN.get_or_init(Default::default)
}

/// Escapes text for the notification body, which servers read as markup.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn preview(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one_line.char_indices().nth(PREVIEW_CHARS) {
        Some((at, _)) => format!("{}…", &one_line[..at]),
        None => one_line,
    }
}

/// Title and body of a notification, as the settings want them.
pub(crate) fn content(book: &Book, chat: &str, sender: &str, push_name: &str, text: &str, group: bool, via: Option<&str>, show_text: bool, unread: u32) -> (String, String) {
    let person = |jid: &str| book.name_of(jid).map(str::to_owned).filter(|n| !n.is_empty()).unwrap_or_else(|| {
        if push_name.is_empty() { mask_phone(jid) } else { push_name.to_owned() }
    });
    let mut title = if group { book.name_of(chat).map(str::to_owned).unwrap_or_else(|| "Grup".to_owned()) } else { person(chat) };
    if let Some(via) = via {
        title = format!("{title} · via {via}");
    }
    let mut body = if show_text { preview(text) } else { "Pesan baru".to_owned() };
    if group {
        body = format!("{}: {body}", person(sender));
    }
    if unread > 1 {
        body = format!("{body}\n{} pesan belum dibaca", unread);
    }
    (title, body)
}

/// Shows (or updates) the chat's notification for a message that just came in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn incoming(state: &Updater, session: &str, book: &Book, chat: &str, sender: &str, push_name: &str, text: &str, group: bool, mentioned: bool) {
    let settings = state.settings();
    let notify = &settings.notify;
    // A mention gets through even when group notifications are off.
    if !notify.enabled || (group && !notify.groups && !mentioned) {
        return;
    }
    let jid = book.normalize(chat);
    // The chat is already on screen in a focused window.
    if state.viewing().is_some_and(|(s, c)| s == session && c == jid) {
        return;
    }
    let (via, several) = state.session_name(session).unwrap_or_default();
    let unread = book.chats.iter().find(|c| c.jid == jid).map(|c| c.unread).unwrap_or(1);
    let (title, mut body) = content(book, &jid, sender, push_name, text, group, several.then_some(via.as_str()), notify.preview, unread);
    if mentioned {
        body = format!("Menyebut kamu: {body}");
    }
    if notify.sound {
        play_sound();
    }
    #[cfg(target_os = "linux")]
    {
        let key = format!("{session}|{jid}");
        let (state, session, jid) = (state.clone(), session.to_owned(), jid);
        std::thread::spawn(move || show(&state, &key, &session, &jid, &title, &escape(&body)));
    }
    #[cfg(not(target_os = "linux"))]
    std::thread::spawn(move || show_simple(&title, &body));
}

/// macOS and Windows: a plain notification, without the click to open.
#[cfg(not(target_os = "linux"))]
fn show_simple(title: &str, body: &str) {
    use crate::platform::{applescript_quote, command, powershell_quote};
    let result = if cfg!(target_os = "macos") {
        let script = format!("display notification \"{}\" with title \"{}\"", applescript_quote(body), applescript_quote(title));
        command("osascript").args(["-e", &script]).status()
    } else {
        // A tray balloon: Windows shows it as a notification, then it is removed.
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; $n = New-Object System.Windows.Forms.NotifyIcon; \
             $n.Icon = [System.Drawing.SystemIcons]::Information; $n.BalloonTipTitle = '{}'; $n.BalloonTipText = '{}'; \
             $n.Visible = $true; $n.ShowBalloonTip(5000); Start-Sleep -Seconds 6; $n.Dispose()",
            powershell_quote(title),
            powershell_quote(body)
        );
        command("powershell").args(["-NoProfile", "-Command", &script]).status()
    };
    if let Err(e) = result {
        log::warn!("no desktop notification: {e}");
    }
}

fn play_sound() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else {
        return;
    };
    if last.is_some_and(|t| t.elapsed() < SOUND_GAP) {
        return;
    }
    *last = Some(Instant::now());
    let mut command = if cfg!(target_os = "macos") {
        let mut c = crate::platform::command("afplay");
        c.arg("/System/Library/Sounds/Glass.aiff");
        c
    } else if cfg!(windows) {
        let mut c = crate::platform::command("powershell");
        c.args(["-NoProfile", "-Command", "[System.Media.SystemSounds]::Asterisk.Play()"]);
        c
    } else {
        let mut c = crate::platform::command("canberra-gtk-play");
        c.args(["-i", "message-new-instant", "-d", "warust"]);
        c
    };
    let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Runs `notify-send` and waits (on this thread) for a click.
#[cfg(target_os = "linux")]
fn show(state: &Updater, key: &str, session: &str, jid: &str, title: &str, body: &str) {
    let replace = shown().lock().ok().and_then(|s| s.get(key).map(|(id, _)| *id));
    let build = |program: &str, prefix: &[&str]| {
        let mut command = crate::platform::command(program);
        command
            .args(prefix)
            .args(["--app-name=warust", "--icon=warust", "--category=im.received"])
            .args(["--hint=string:desktop-entry:warust", "--print-id", "--wait", "--action=default=Buka chat"]);
        if let Some(id) = replace {
            command.arg(format!("--replace-id={id}"));
        }
        command.arg(title).arg(body).stdout(Stdio::piped()).stderr(Stdio::null());
        command
    };
    // Piped, notify-send holds its output until it exits; stdbuf makes it print
    // the id at once, so the next message can replace the notification.
    let spawned = build("stdbuf", &["-oL", "notify-send"]).spawn().or_else(|_| build("notify-send", &[]).spawn());
    let Ok(mut child) = spawned else {
        log::warn!("notify-send is not available; no desktop notification");
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    // First the id, then the action when the notification is clicked.
    let Some(id) = lines.next().and_then(Result::ok).and_then(|l| l.trim().parse::<u32>().ok()) else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    if let Ok(mut shown) = shown().lock() {
        // The older waiter belongs to the replaced notification.
        if let Some((_, mut old)) = shown.insert(key.to_owned(), (id, child)) {
            let _ = old.kill();
            let _ = old.wait();
        }
    }
    if lines.next().and_then(Result::ok).is_some_and(|action| action.trim() == "default") {
        state.request_open(session, jid);
    }
    if let Ok(mut shown) = shown().lock() {
        if shown.get(key).is_some_and(|(current, _)| *current == id) {
            if let Some((_, mut child)) = shown.remove(key) {
                let _ = child.wait();
            }
        }
    }
}

/// Ends the waiting `notify-send`s, so none outlive the app.
pub(crate) fn close_all() {
    if let Ok(mut shown) = shown().lock() {
        for (_, (_, mut child)) in shown.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_follows_the_settings() {
        let book = Book::default();
        let chat = "6281234567890@s.whatsapp.net";
        let (title, body) = content(&book, chat, chat, "Budi", "Sudah   transfer,\nmohon dicek", false, Some("CS Utama"), true, 1);
        assert_eq!(title, "Budi · via CS Utama");
        assert_eq!(body, "Sudah transfer, mohon dicek");
        let (_, body) = content(&book, chat, chat, "", "rahasia", false, None, false, 3);
        assert_eq!(body, "Pesan baru\n3 pesan belum dibaca");
        let (title, body) = content(&book, "123@g.us", chat, "", "halo", true, None, true, 1);
        assert_eq!(title, "Grup");
        assert_eq!(body, "+62 812-****-7890: halo");
        assert_eq!(escape("a<b>&c"), "a&lt;b&gt;&amp;c");
        assert!(preview(&"x".repeat(300)).ends_with('…'));
    }
}
