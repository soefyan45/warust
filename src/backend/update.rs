//! Updates (#93). Stage 1 asks GitHub for the latest warust release and
//! tells the UI when it is newer than this build. Stage 2 installs it by
//! itself: the installer for this OS is downloaded in the background and
//! checked against the release's `SHA256SUMS`, installed once the app is
//! idle, and warust starts again. Only the requests themselves leave the
//! computer; a failure falls back to the banner's Download.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use sha2::{Digest, Sha256};

use whatsapp_rust::serde_json::{self, Value};

const LATEST: &str = "https://api.github.com/repos/soefyan45/warust/releases/latest";
const TIMEOUT: Duration = Duration::from_secs(15);
/// How often the app looks again while it runs.
pub const EVERY: Duration = Duration::from_secs(24 * 3600);

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Without the `v`, e.g. `0.1.6`.
    pub version: String,
    /// The release page (its notes).
    pub page: String,
    /// The installer for this OS, else the release page.
    pub download: String,
    /// The installer's file name, and the release's checksum list.
    pub asset: Option<String>,
    pub sums: Option<String>,
}

/// Where the last check stands, for the banner and Settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    /// Stage 2: being fetched and checked, then waiting for an idle moment.
    Downloading(Release),
    Ready(Release),
    Installing(Release),
    /// Installed (or handed to the helper): the window closes and warust
    /// starts again.
    Restarting,
    Failed(String),
}

/// This build's version. Dev aid: a debug build takes `WARUST_VERSION`
/// instead, to see the banner against the release that is out.
pub fn current() -> &'static str {
    static PRETEND: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let pretend = PRETEND.get_or_init(|| if cfg!(debug_assertions) { std::env::var("WARUST_VERSION").ok().filter(|v| !v.is_empty()) } else { None });
    pretend.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Asks GitHub for the latest release (blocking; run it off the UI thread).
pub fn check() -> UpdateState {
    match latest() {
        Ok(Some(release)) if newer(&release.version, current()) => UpdateState::Available(release),
        Ok(_) => UpdateState::UpToDate,
        Err(e) => {
            log::info!("update check failed: {e}");
            UpdateState::Failed(e)
        }
    }
}

fn latest() -> Result<Option<Release>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into();
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", &format!("warust/{}", current()))
        .call()
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    // No release published yet.
    if status == 404 {
        return Ok(None);
    }
    if status != 200 {
        return Err(format!("GitHub answered {status}"));
    }
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(release_from(&value))
}

/// The release in GitHub's answer; drafts and pre-releases are not offered.
fn release_from(value: &Value) -> Option<Release> {
    if value.get("draft").and_then(Value::as_bool).unwrap_or(false) || value.get("prerelease").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let tag = value.get("tag_name")?.as_str()?;
    let page = value.get("html_url").and_then(Value::as_str).unwrap_or("https://github.com/soefyan45/warust/releases/latest").to_owned();
    let assets: Vec<(String, String)> = value
        .get("assets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| Some((a.get("name")?.as_str()?.to_owned(), a.get("browser_download_url")?.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let installer = asset_for(std::env::consts::OS, &assets);
    let download = installer.as_ref().map(|(_, url)| url.clone()).unwrap_or_else(|| page.clone());
    let sums = assets.iter().find(|(name, _)| name == "SHA256SUMS").map(|(_, url)| url.clone());
    Some(Release { version: tag.trim_start_matches('v').to_owned(), page, download, asset: installer.map(|(name, _)| name), sums })
}

/// The installer the release workflow builds for `os`: the `.deb` on Linux,
/// the Windows `.zip`, the macOS `.dmg`.
fn asset_for(os: &str, assets: &[(String, String)]) -> Option<(String, String)> {
    let wanted = |name: &str| match os {
        "linux" => name.ends_with(".deb"),
        "windows" => name.ends_with(".zip") && name.contains("windows"),
        "macos" => name.ends_with(".dmg"),
        _ => false,
    };
    assets.iter().find(|(name, _)| wanted(name)).cloned()
}

/// Where downloaded updates wait to be installed.
fn dir() -> PathBuf {
    crate::platform::data_dir().join("updates")
}

/// The installer downloaded and checked, waiting for an idle moment.
fn ready() -> &'static Mutex<Option<PathBuf>> {
    static READY: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    READY.get_or_init(Default::default)
}

/// What runs once the window has closed: warust again, or the helper that
/// swaps its files first.
fn after() -> &'static Mutex<Option<std::process::Command>> {
    static AFTER: OnceLock<Mutex<Option<std::process::Command>>> = OnceLock::new();
    AFTER.get_or_init(Default::default)
}

/// This executable, as it was before an install replaced it.
fn exe() -> Option<PathBuf> {
    static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();
    EXE.get_or_init(|| {
        let path = std::env::current_exe().ok()?;
        // Linux names a replaced binary "… (deleted)".
        let text = path.to_string_lossy();
        Some(PathBuf::from(text.trim_end_matches(" (deleted)")))
    })
    .clone()
}

/// Whether this copy of warust may install updates by itself: never a
/// development build run from `target/` (dev aid: `WARUST_UPDATE_TEST=1`
/// in a debug build lets one try it).
pub fn installable() -> bool {
    if cfg!(debug_assertions) && std::env::var("WARUST_UPDATE_TEST").is_ok_and(|v| v == "1") {
        return true;
    }
    exe().is_some_and(|path| !path.components().any(|c| c.as_os_str() == "target"))
}

/// Fetches the installer and keeps it when its SHA-256 matches the
/// release's `SHA256SUMS` (blocking).
pub fn download(release: &Release) -> Result<(), String> {
    let (Some(asset), Some(sums)) = (&release.asset, &release.sums) else {
        return Err("this release has no installer or checksums for this computer".to_owned());
    };
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into();
    let get = |url: &str, limit: u64| -> Result<Vec<u8>, String> {
        let mut response = agent.get(url).header("User-Agent", &format!("warust/{}", current())).call().map_err(|e| e.to_string())?;
        response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())
    };
    let list = String::from_utf8(get(sums, 64 * 1024)?).map_err(|e| e.to_string())?;
    let expected = checksum_for(&list, asset).ok_or("the installer is not in SHA256SUMS")?;
    let bytes = get(&release.download, 400 * 1024 * 1024)?;
    let actual: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        return Err("the download does not match its checksum".to_owned());
    }
    let dir = dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(asset);
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    if let Ok(mut ready) = ready().lock() {
        *ready = Some(file);
    }
    Ok(())
}

/// The hash `sha256sum` wrote for `name` ("<hash>  <name>" per line).
fn checksum_for(list: &str, name: &str) -> Option<String> {
    list.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.trim().to_ascii_lowercase())
    })
}

/// Installs the downloaded update (blocking): on Linux the package itself,
/// elsewhere the files are readied and swapped once warust has exited.
pub fn install() -> Result<(), String> {
    let file = ready().lock().ok().and_then(|r| r.clone()).ok_or("no update downloaded")?;
    let exe = exe().ok_or("cannot tell where warust runs from")?;
    let pid = std::process::id();
    let restart = if cfg!(target_os = "linux") {
        // The package lives in system folders: the system asks for the password.
        let status = crate::platform::command("pkexec").arg("dpkg").arg("-i").arg(&file).status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("the package was not installed".to_owned());
        }
        crate::platform::command(&exe)
    } else if cfg!(windows) {
        let unpacked = dir().join("unpacked");
        let _ = std::fs::remove_dir_all(&unpacked);
        let expand = format!("Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force", ps(&file), ps(&unpacked));
        let status = crate::platform::command("powershell").args(["-NoProfile", "-Command", &expand]).status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("the archive could not be unpacked".to_owned());
        }
        // The zip holds one folder with warust.exe and its notes.
        let source = std::fs::read_dir(&unpacked).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.is_dir()).unwrap_or(unpacked.clone());
        let target = exe.parent().ok_or("no folder for warust.exe")?;
        let script = dir().join("install.ps1");
        let body = format!(
            "Wait-Process -Id {pid} -ErrorAction SilentlyContinue\nStart-Sleep -Seconds 1\nCopy-Item -Path '{}\\*' -Destination '{}' -Recurse -Force\nStart-Process -FilePath '{}'\n",
            ps(&source),
            ps(target),
            ps(&exe)
        );
        std::fs::write(&script, body).map_err(|e| e.to_string())?;
        let mut helper = crate::platform::command("powershell");
        helper.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"]).arg(&script);
        helper
    } else if cfg!(target_os = "macos") {
        let app = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).ok_or("warust is not running from warust.app")?;
        let script = dir().join("install.sh");
        let body = format!(
            "while kill -0 {pid} 2>/dev/null; do sleep 0.5; done\nmnt=$(mktemp -d)\nhdiutil attach -nobrowse -readonly -mountpoint \"$mnt\" '{dmg}' >/dev/null || exit 1\nsrc=$(ls -d \"$mnt\"/*.app | head -1)\nrm -rf '{app}.new' && cp -R \"$src\" '{app}.new' && rm -rf '{app}' && mv '{app}.new' '{app}'\nhdiutil detach \"$mnt\" >/dev/null\nxattr -dr com.apple.quarantine '{app}' 2>/dev/null\nopen '{app}'\n",
            dmg = sh(&file),
            app = sh(app)
        );
        std::fs::write(&script, body).map_err(|e| e.to_string())?;
        let mut helper = crate::platform::command("/bin/sh");
        helper.arg(&script);
        helper
    } else {
        return Err("updates install by themselves only on Linux, Windows and macOS".to_owned());
    };
    if let Ok(mut after) = after().lock() {
        *after = Some(restart);
    }
    Ok(())
}

/// A path inside a PowerShell single-quoted string.
fn ps(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

/// A path inside a shell single-quoted string.
fn sh(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}

/// Run once the window has closed: starts warust again, or the helper that
/// swaps its files and then starts it.
pub fn after_exit() {
    let Some(mut command) = after().lock().ok().and_then(|mut a| a.take()) else { return };
    if let Err(e) = command.spawn() {
        log::error!("update: could not start warust again: {e}");
    }
}

/// Whether version `a` comes after `b`, number by number (`0.1.10` > `0.1.9`).
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.trim_start_matches('v').split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect() };
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_number_by_number() {
        assert!(newer("0.1.6", "0.1.5"));
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(newer("0.2.0", "0.1.99"));
        assert!(!newer("0.1.5", "0.1.5"));
        assert!(!newer("0.1.4", "0.1.5"));
    }

    #[test]
    fn the_installer_for_this_os_is_picked() {
        let assets = [
            ("warust-v0.1.5-macos-universal.dmg".to_owned(), "m".to_owned()),
            ("warust-v0.1.5-windows-x86_64.zip".to_owned(), "w".to_owned()),
            ("warust_0.1.5_amd64.deb".to_owned(), "l".to_owned()),
        ];
        let url = |os| asset_for(os, &assets).map(|(_, url)| url);
        assert_eq!(url("linux").as_deref(), Some("l"));
        assert_eq!(url("windows").as_deref(), Some("w"));
        assert_eq!(url("macos").as_deref(), Some("m"));
        assert_eq!(url("freebsd"), None);
    }

    #[test]
    fn checksums_are_read_from_sha256sums() {
        let list = "ab12  warust_0.1.6_amd64.deb\nCD34 *warust-v0.1.6-windows-x86_64.zip\n";
        assert_eq!(checksum_for(list, "warust_0.1.6_amd64.deb").as_deref(), Some("ab12"));
        assert_eq!(checksum_for(list, "warust-v0.1.6-windows-x86_64.zip").as_deref(), Some("cd34"));
        assert_eq!(checksum_for(list, "other.dmg"), None);
    }

    #[test]
    fn shell_quotes_are_escaped() {
        assert_eq!(sh(Path::new("/Applications/it's.app")), "/Applications/it'\\''s.app");
        assert_eq!(ps(Path::new("C:/it's")), "C:/it''s");
    }

    #[test]
    fn github_answers_become_releases() {
        let value: Value = serde_json::from_str(
            r#"{"tag_name":"v0.1.6","html_url":"https://x/r","draft":false,"prerelease":false,
               "assets":[{"name":"warust_0.1.6_amd64.deb","browser_download_url":"https://x/d"}]}"#,
        )
        .unwrap();
        let release = release_from(&value).unwrap();
        assert_eq!(release.version, "0.1.6");
        assert_eq!(release.page, "https://x/r");
        let pre: Value = serde_json::from_str(r#"{"tag_name":"v0.2.0","prerelease":true}"#).unwrap();
        assert_eq!(release_from(&pre), None);
    }
}
