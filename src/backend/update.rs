//! Update check (#93, stage 1): asks GitHub for the latest warust release
//! and tells the UI when it is newer than this build. Only the request
//! itself leaves the computer; a failure is reported to "Check now" and
//! otherwise stays quiet.

use std::time::Duration;

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
}

/// Where the last check stands, for the banner and Settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
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
    let download = asset_for(std::env::consts::OS, &assets).unwrap_or_else(|| page.clone());
    Some(Release { version: tag.trim_start_matches('v').to_owned(), page, download })
}

/// The installer the release workflow builds for `os`: the `.deb` on Linux,
/// the Windows `.zip`, the macOS `.dmg`.
fn asset_for(os: &str, assets: &[(String, String)]) -> Option<String> {
    let wanted = |name: &str| match os {
        "linux" => name.ends_with(".deb"),
        "windows" => name.ends_with(".zip") && name.contains("windows"),
        "macos" => name.ends_with(".dmg"),
        _ => false,
    };
    assets.iter().find(|(name, _)| wanted(name)).map(|(_, url)| url.clone())
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
        assert_eq!(asset_for("linux", &assets).as_deref(), Some("l"));
        assert_eq!(asset_for("windows", &assets).as_deref(), Some("w"));
        assert_eq!(asset_for("macos", &assets).as_deref(), Some("m"));
        assert_eq!(asset_for("freebsd", &assets), None);
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
