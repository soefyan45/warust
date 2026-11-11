//! Where the user left the app (#128): the screen, and the Inbox's number,
//! tab and Devices panel, in `data/view.json`. Read once on start so the app
//! opens there again, and written when one of them changes. The open chat is
//! not kept: opening a chat marks it read, which a start must not do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use whatsapp_rust::serde_json;

use super::screens::inbox::{TAB_ALL, TAB_ARCHIVED, TAB_DONE, TAB_REPLY};
use super::{Screen, State};

/// The file's content. Missing fields take their defaults, so older files
/// still load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// `summary`, `numbers`, `inbox`, `status`, `log`, `integrations` or
    /// `settings`; empty when no screen was kept yet.
    pub screen: String,
    /// The Inbox tab: `reply`, `all`, `done` or `archived`.
    pub chat_tab: String,
    /// The number (session id) whose chats are listed; `None` for every number.
    pub inbox_scope: Option<String>,
    /// "Split by number" rather than "All numbers together".
    pub inbox_grouped: bool,
    pub devices_panel: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            screen: String::new(),
            chat_tab: tab_key(TAB_REPLY).to_owned(),
            inbox_scope: None,
            inbox_grouped: false,
            devices_panel: true,
        }
    }
}

impl View {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("view.json")
    }

    /// The saved view; `None` without a file or with one that does not parse.
    pub fn load(data_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(data_dir)).ok()?;
        serde_json::from_str(&text)
            .map_err(|e| log::warn!("view.json unreadable ({e}); starting on the default view"))
            .ok()
    }

    fn save(&self, data_dir: &Path) {
        let path = Self::path(data_dir);
        let _ = std::fs::create_dir_all(data_dir);
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        // Write then rename, so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)) {
            log::warn!("cannot save the view: {e}");
        }
    }
}

fn tab_key(tab: usize) -> &'static str {
    match tab {
        TAB_ALL => "all",
        TAB_DONE => "done",
        TAB_ARCHIVED => "archived",
        _ => "reply",
    }
}

fn tab_from(key: &str) -> usize {
    match key {
        "all" => TAB_ALL,
        "done" => TAB_DONE,
        "archived" => TAB_ARCHIVED,
        _ => TAB_REPLY,
    }
}

/// The name a screen is kept under. Pairing belongs to one linking in
/// progress and "More" to the phone layout only, so neither is kept.
fn screen_key(screen: Screen) -> Option<&'static str> {
    Some(match screen {
        Screen::Ringkasan => "summary",
        Screen::Nomor => "numbers",
        Screen::Inbox => "inbox",
        Screen::Status => "status",
        Screen::Log => "log",
        Screen::Integrations => "integrations",
        Screen::Settings => "settings",
        Screen::Pairing | Screen::More => return None,
    })
}

fn screen_from(key: &str) -> Option<Screen> {
    Some(match key {
        "summary" => Screen::Ringkasan,
        "numbers" => Screen::Nomor,
        "inbox" => Screen::Inbox,
        "status" => Screen::Status,
        "log" => Screen::Log,
        "integrations" => Screen::Integrations,
        "settings" => Screen::Settings,
        _ => return None,
    })
}

/// Puts the saved view back on start, then saves it whenever it changes.
pub struct Keeper {
    data_dir: PathBuf,
    /// What the file holds.
    saved: View,
    /// The saved number, until its session is loaded. Sessions come in one at
    /// a time after a start, and the Inbox drops a number it does not know.
    waiting_scope: Option<String>,
}

impl Keeper {
    /// Reads the file and puts its Inbox tab, mode and panel on `state`.
    /// Returns the saved screen, for the caller to open or not.
    pub fn open(data_dir: &Path, state: &mut State) -> (Self, Option<Screen>) {
        let found = View::load(data_dir);
        let screen = found.as_ref().and_then(|view| screen_from(&view.screen));
        let saved = found.unwrap_or_default();
        state.chat_tab = tab_from(&saved.chat_tab);
        state.inbox_grouped = saved.inbox_grouped;
        state.devices_panel = saved.devices_panel;
        let waiting_scope = saved.inbox_scope.clone();
        (Self { data_dir: data_dir.to_owned(), saved, waiting_scope }, screen)
    }

    /// Once a frame, before the screens draw: selects the saved number when
    /// its session has loaded, and saves the view when it changed.
    pub fn follow(&mut self, screen: Screen, state: &mut State, loaded: impl Fn(&str) -> bool) {
        if state.inbox_scope.is_some() {
            // The user chose a number before the saved one came in.
            self.waiting_scope = None;
        } else if self.waiting_scope.as_deref().is_some_and(loaded) {
            state.inbox_scope = self.waiting_scope.take();
        }
        let now = View {
            screen: screen_key(screen).map_or_else(|| self.saved.screen.clone(), str::to_owned),
            chat_tab: tab_key(state.chat_tab).to_owned(),
            // A number still waited for stays in the file.
            inbox_scope: state.inbox_scope.clone().or_else(|| self.waiting_scope.clone()),
            inbox_grouped: state.inbox_grouped,
            devices_panel: state.devices_panel,
        };
        if now != self.saved {
            now.save(&self.data_dir);
            self.saved = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty data folder of this test's own.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("warust-view-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("makes the folder");
        dir
    }

    /// A start: what `App::new` does with the file.
    fn start(dir: &Path) -> (Keeper, State, Option<Screen>) {
        let mut state = State::default();
        let (keeper, screen) = Keeper::open(dir, &mut state);
        (keeper, state, screen)
    }

    const NONE: fn(&str) -> bool = |_| false;
    const ALL: fn(&str) -> bool = |_| true;

    #[test]
    fn a_first_start_keeps_the_defaults() {
        let dir = folder("first");
        let (_, state, screen) = start(&dir);
        assert!(screen.is_none());
        assert_eq!(state.chat_tab, TAB_REPLY);
        assert!(state.devices_panel && !state.inbox_grouped && state.inbox_scope.is_none());
    }

    #[test]
    fn the_next_start_opens_where_the_last_one_ended() {
        let dir = folder("again");
        let (mut keeper, mut state, _) = start(&dir);
        state.chat_tab = TAB_DONE;
        state.inbox_scope = Some("work".to_owned());
        state.inbox_grouped = true;
        state.devices_panel = false;
        keeper.follow(Screen::Inbox, &mut state, ALL);

        let (mut keeper, mut state, screen) = start(&dir);
        assert!(screen == Some(Screen::Inbox));
        assert_eq!(state.chat_tab, TAB_DONE);
        assert!(state.inbox_grouped && !state.devices_panel);
        // The number comes back once its session is there.
        assert!(state.inbox_scope.is_none());
        keeper.follow(Screen::Inbox, &mut state, NONE);
        assert!(state.inbox_scope.is_none());
        keeper.follow(Screen::Inbox, &mut state, |id| id == "work");
        assert_eq!(state.inbox_scope.as_deref(), Some("work"));
    }

    #[test]
    fn a_number_still_loading_is_not_forgotten() {
        let dir = folder("loading");
        let (mut keeper, mut state, _) = start(&dir);
        state.inbox_scope = Some("work".to_owned());
        keeper.follow(Screen::Inbox, &mut state, ALL);

        // A start that ends before the number's session came in.
        let (mut keeper, mut state, _) = start(&dir);
        state.chat_tab = TAB_ALL;
        keeper.follow(Screen::Inbox, &mut state, NONE);
        let saved = View::load(&dir).expect("saved");
        assert_eq!(saved.inbox_scope.as_deref(), Some("work"));
        assert_eq!(saved.chat_tab, "all");
    }

    #[test]
    fn a_number_chosen_meanwhile_wins_over_the_saved_one() {
        let dir = folder("chosen");
        let (mut keeper, mut state, _) = start(&dir);
        state.inbox_scope = Some("work".to_owned());
        keeper.follow(Screen::Inbox, &mut state, ALL);

        let (mut keeper, mut state, _) = start(&dir);
        state.inbox_scope = Some("personal".to_owned());
        keeper.follow(Screen::Inbox, &mut state, |id| id == "personal");
        keeper.follow(Screen::Inbox, &mut state, ALL);
        assert_eq!(state.inbox_scope.as_deref(), Some("personal"));
        assert_eq!(View::load(&dir).expect("saved").inbox_scope.as_deref(), Some("personal"));
    }

    #[test]
    fn pairing_and_more_do_not_replace_the_kept_screen() {
        let dir = folder("pairing");
        let (mut keeper, mut state, _) = start(&dir);
        keeper.follow(Screen::Status, &mut state, NONE);
        keeper.follow(Screen::Pairing, &mut state, NONE);
        keeper.follow(Screen::More, &mut state, NONE);
        assert!(start(&dir).2 == Some(Screen::Status));
    }

    #[test]
    fn the_file_is_written_only_on_a_change() {
        let dir = folder("unchanged");
        let (mut keeper, mut state, _) = start(&dir);
        keeper.follow(Screen::Inbox, &mut state, NONE);
        std::fs::remove_file(View::path(&dir)).expect("was written");
        keeper.follow(Screen::Inbox, &mut state, NONE);
        assert!(!View::path(&dir).exists());
        state.chat_tab = TAB_ARCHIVED;
        keeper.follow(Screen::Inbox, &mut state, NONE);
        assert_eq!(View::load(&dir).expect("written again").chat_tab, "archived");
    }

    #[test]
    fn an_older_or_broken_file_starts_on_the_defaults() {
        let dir = folder("broken");
        std::fs::write(View::path(&dir), r#"{"chat_tab":"all","screen":"gone","tab_of_tomorrow":1}"#).expect("writes");
        let (_, state, screen) = start(&dir);
        assert!(screen.is_none(), "a screen this version does not know");
        assert_eq!(state.chat_tab, TAB_ALL);
        assert!(state.devices_panel, "a missing field takes its default");

        std::fs::write(View::path(&dir), "not json").expect("writes");
        let (_, state, screen) = start(&dir);
        assert!(screen.is_none());
        assert_eq!(state.chat_tab, TAB_REPLY);
        assert!(state.devices_panel);
    }

    #[test]
    fn every_kept_screen_and_tab_comes_back_as_itself() {
        for screen in [Screen::Ringkasan, Screen::Nomor, Screen::Inbox, Screen::Status, Screen::Log, Screen::Integrations, Screen::Settings] {
            assert!(screen_key(screen).and_then(screen_from) == Some(screen));
        }
        for tab in [TAB_REPLY, TAB_ALL, TAB_DONE, TAB_ARCHIVED] {
            assert_eq!(tab_from(tab_key(tab)), tab);
        }
    }
}
