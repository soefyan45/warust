//! Links that open a chat with a number (#193): `whatsapp://send?phone=…`,
//! which a website's "Chat on WhatsApp" button ends in, and the `wa.me` and
//! `api.whatsapp.com/send` links it starts from. A warust started with one
//! hands it to the warust already running on the same data and quits.

use std::sync::Mutex;

/// A chat to open: the number's digits with its country code, and the text
/// the link suggests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLink {
    pub number: String,
    pub text: Option<String>,
}

/// The chat a WhatsApp link opens, or `None` for any other link.
pub fn parse(link: &str) -> Option<ChatLink> {
    let link = link.trim();
    let lower = link.to_ascii_lowercase();
    let scheme = ["whatsapp://", "https://", "http://"].into_iter().find(|s| lower.starts_with(s))?;
    let rest = &link[scheme.len()..];
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = path.trim_end_matches('/').to_ascii_lowercase();
    let param = |key: &str| {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            k.eq_ignore_ascii_case(key).then(|| decode(v))
        })
    };
    let number = if scheme == "whatsapp://" {
        (path == "send").then(|| param("phone")).flatten()?
    } else if matches!(path.as_str(), "api.whatsapp.com/send" | "web.whatsapp.com/send" | "wa.me/send" | "www.whatsapp.com/send") {
        param("phone")?
    } else {
        path.strip_prefix("wa.me/").or_else(|| path.strip_prefix("www.wa.me/"))?.to_owned()
    };
    // Digits, maybe written with "+", spaces or dashes; a wa.me/message/…
    // short link or anything else is not a number.
    if number.chars().any(|c| !(c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '.' | '(' | ')'))) {
        return None;
    }
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    if !(7..=15).contains(&digits.len()) {
        return None;
    }
    let text = param("text").filter(|t| !t.trim().is_empty());
    Some(ChatLink { number: digits, text })
}

/// A query value: `%XX` escapes and `+` for a space.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Links that came in and wait for the window to open them.
static PENDING: Mutex<Vec<ChatLink>> = Mutex::new(Vec::new());

pub fn push(link: ChatLink) {
    if let Ok(mut pending) = PENDING.lock() {
        pending.push(link);
    }
}

/// The oldest link waiting, if any.
pub fn take() -> Option<ChatLink> {
    let mut pending = PENDING.lock().ok()?;
    (!pending.is_empty()).then(|| pending.remove(0))
}

/// Where the running warust listens: one socket per data folder, in the
/// user's own runtime folder.
#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir);
    let data = crate::platform::data_dir();
    let data = data.canonicalize().unwrap_or_else(|_| data.to_path_buf());
    // FNV-1a: the same name in every build, unlike std's hasher.
    let key = data.to_string_lossy().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    base.join(format!("warust-{key:016x}.sock"))
}

/// Gives `link` to the warust already running on this data; true when one
/// took it, so this start can end.
#[cfg(unix)]
pub fn hand_over(link: &str) -> bool {
    use std::io::Write;
    match std::os::unix::net::UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(format!("{}\n", link.trim()).as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Takes the links later starts hand over while this one runs; `wake`
/// makes the app look at them.
#[cfg(unix)]
pub fn listen(wake: impl Fn() + Send + 'static) {
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::net::{UnixListener, UnixStream};
    let path = socket_path();
    // Another warust on this data already listens: leave its socket.
    if UnixStream::connect(&path).is_ok() {
        return;
    }
    // A socket left by a warust that did not end cleanly.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            log::warn!("chat links: cannot listen on {}: {e}", path.display());
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream.take(8192)).read_line(&mut line).is_ok()
                && let Some(link) = parse(&line)
            {
                push(link);
                wake();
            }
        }
    });
}

#[cfg(not(unix))]
pub fn hand_over(_: &str) -> bool {
    false
}

#[cfg(not(unix))]
pub fn listen(_: impl Fn() + Send + 'static) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(number: &str, text: Option<&str>) -> Option<ChatLink> {
        Some(ChatLink { number: number.to_owned(), text: text.map(str::to_owned) })
    }

    #[test]
    fn whatsapp_links_give_the_number_and_text() {
        assert_eq!(parse("whatsapp://send?phone=6285271995583&text=Halo%20kak%2C+tanya"), link("6285271995583", Some("Halo kak, tanya")));
        assert_eq!(parse("whatsapp://send/?phone=%2B62%20852-7199-5583"), link("6285271995583", None));
        assert_eq!(parse("https://wa.me/6285271995583"), link("6285271995583", None));
        assert_eq!(parse("https://wa.me/6285271995583/?text=hi"), link("6285271995583", Some("hi")));
        assert_eq!(parse("HTTPS://API.WhatsApp.com/send?phone=6285271995583&text="), link("6285271995583", None));
        assert_eq!(parse("https://web.whatsapp.com/send?text=x&phone=628111222333"), link("628111222333", Some("x")));
    }

    #[test]
    fn other_links_are_not_chats() {
        assert_eq!(parse("https://wa.me/message/ABCDEF123"), None);
        assert_eq!(parse("https://www.youtube.com/live/DsncuGWEH5Q"), None);
        assert_eq!(parse("https://wa.me/123"), None);
        assert_eq!(parse("whatsapp://chat?code=abc"), None);
        assert_eq!(parse("https://evil.example/wa.me/6285271995583"), None);
        assert_eq!(parse("whatsapp://send?phone=62852&text=x"), None);
    }

    #[test]
    fn a_bad_escape_stays_as_written() {
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("a%zzb"), "a%zzb");
        assert_eq!(decode("%E2%9C%93+ok"), "✓ ok");
    }
}
