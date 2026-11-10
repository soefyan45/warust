//! What differs between Linux, macOS and Windows: where the data lives, how a
//! file is opened or picked, the UTC offset, and OS randomness. Only tools
//! that ship with each OS are used (zenity/xdg-open on Linux, osascript and
//! `open` on macOS, PowerShell on Windows); no extra crates.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The data folder: `WARUST_DATA` if set; `./data` when it already holds
/// warust's files (running from a checkout); else the OS's per-user folder
/// (`~/.local/share/warust`, `~/Library/Application Support/warust`,
/// `%APPDATA%\warust`).
pub fn data_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = if let Some(dir) = std::env::var_os("WARUST_DATA") {
            PathBuf::from(dir)
        } else if ["sessions.json", "settings.json"].iter().any(|f| Path::new("data").join(f).is_file()) {
            PathBuf::from("data")
        } else {
            user_data_dir().unwrap_or_else(|| PathBuf::from("data"))
        };
        let _ = std::fs::create_dir_all(&dir);
        std::fs::canonicalize(&dir).unwrap_or(dir)
    })
}

fn user_data_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")?
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library").join("Application Support")
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local").join("share")))?
    };
    Some(base.join("warust"))
}

/// A command that, on Windows, opens no console window.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Opens a file, folder or URL in the desktop's default app.
pub fn open(target: impl AsRef<OsStr>) {
    let target = target.as_ref();
    let spawned = if cfg!(windows) {
        // `start` treats its first quoted argument as the window title.
        command("cmd").args([OsStr::new("/C"), OsStr::new("start"), OsStr::new(""), target]).spawn()
    } else if cfg!(target_os = "macos") {
        command("open").arg(target).spawn()
    } else {
        command("xdg-open").arg(target).spawn()
    };
    if let Err(e) = spawned {
        log::warn!("cannot open {}: {e}", target.to_string_lossy());
    }
}

/// Asks for one file with the OS's dialog; blocks, so call it off the UI
/// thread. `None` when cancelled or when no dialog is available.
pub fn pick_file(title: &str, images_only: bool) -> Option<PathBuf> {
    let output = if cfg!(windows) {
        let filter = if images_only { "$d.Filter = 'Gambar|*.png;*.jpg;*.jpeg;*.webp';" } else { "" };
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; \
             $d.Title = '{}'; {filter} if ($d.ShowDialog() -eq 'OK') {{ [Console]::OutputEncoding = [Text.Encoding]::UTF8; $d.FileName }}",
            powershell_quote(title)
        );
        command("powershell").args(["-NoProfile", "-STA", "-Command", &script]).output()
    } else if cfg!(target_os = "macos") {
        let kind = if images_only { " of type {\"public.image\"}" } else { "" };
        let script = format!("POSIX path of (choose file with prompt \"{}\"{kind})", applescript_quote(title));
        command("osascript").args(["-e", &script]).output()
    } else {
        let mut zenity = command("zenity");
        zenity.args(["--file-selection", &format!("--title={title}")]);
        if images_only {
            zenity.arg("--file-filter=Gambar | *.png *.jpg *.jpeg *.webp");
        }
        zenity.output()
    };
    let output = output.inspect_err(|e| log::warn!("file dialog failed: {e}")).ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim_end_matches(['\r', '\n']));
    path.is_file().then_some(path)
}

/// The local UTC offset in seconds, asked once of the OS.
pub fn utc_offset() -> Option<i64> {
    if cfg!(windows) {
        let out = command("powershell")
            .args(["-NoProfile", "-Command", "[int][TimeZoneInfo]::Local.GetUtcOffset([DateTime]::Now).TotalSeconds"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else {
        let out = command("date").arg("+%z").output().ok()?;
        parse_offset(String::from_utf8_lossy(&out.stdout).trim())
    }
}

/// `+0700` → 25200 seconds.
pub fn parse_offset(text: &str) -> Option<i64> {
    let (sign, digits) = match text.as_bytes().first()? {
        b'+' => (1, &text[1..]),
        b'-' => (-1, &text[1..]),
        _ => return None,
    };
    if digits.len() != 4 {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = digits[2..].parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Shows `count` unread messages on the app's taskbar or Dock icon; 0 hides
/// it. Linux: the Unity LauncherEntry signal (Ubuntu Dock, Dash to Dock, Dash
/// to Panel, Plasma), sent with `gdbus` for the `warust.desktop` entry.
/// macOS: the Dock badge. Windows: nothing yet; the window title has it.
pub fn set_unread_badge(count: u32) {
    #[cfg(target_os = "linux")]
    {
        let props = format!("{{'count': <int64 {count}>, 'count-visible': <{}>}}", count > 0);
        let mut emit = command("gdbus");
        emit.args([
            "emit",
            "--session",
            "--object-path",
            "/com/canonical/unity/launcherentry/warust",
            "--signal",
            "com.canonical.Unity.LauncherEntry.Update",
            "'application://warust.desktop'",
            &props,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        // Waited for on its own thread, so no zombie is left behind.
        std::thread::spawn(move || {
            if let Err(e) = emit.status() {
                log::debug!("unread badge not sent: {e}");
            }
        });
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSString;
        // AppKit only from the main thread, where the UI runs.
        if let Some(mtm) = MainThreadMarker::new() {
            let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
            NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
        }
    }
    #[cfg(windows)]
    let _ = count;
}

/// The window title with the unread count in front, as messaging apps do.
pub fn window_title(count: u32) -> String {
    if count == 0 { "warust".to_owned() } else { format!("({count}) warust") }
}

/// Fills `bytes` from the OS's secure random source.
pub fn random_bytes(bytes: &mut [u8]) -> bool {
    getrandom::fill(bytes).is_ok()
}

/// Text inside a single-quoted PowerShell string.
pub fn powershell_quote(text: &str) -> String {
    text.replace('\'', "''")
}

/// Text inside a double-quoted AppleScript string.
pub fn applescript_quote(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(parse_offset("+0700"), Some(25200));
        assert_eq!(parse_offset("-0230"), Some(-9000));
        assert_eq!(parse_offset("WIB"), None);
        assert_eq!(powershell_quote("it's"), "it''s");
        assert_eq!(applescript_quote(r#"a "b" \c"#), r#"a \"b\" \\c"#);
        assert_eq!(window_title(0), "warust");
        assert_eq!(window_title(12), "(12) warust");
        let mut a = [0u8; 16];
        let mut b = [0u8; 16];
        assert!(random_bytes(&mut a) && random_bytes(&mut b) && a != b);
    }
}
