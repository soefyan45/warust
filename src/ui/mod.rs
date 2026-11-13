//! Desktop UI (egui), following the Pego design: v4 layout, v4 dark palette
//! and v3 light palette. Every screen shows the real WhatsApp sessions.

mod call;
mod compact;
mod emoji;
mod fonts;
mod forward;
mod icons;
mod lists;
mod mention;
mod markup;
mod movie_view;
mod pdf;
mod photo_view;
pub mod privacy;
mod qr;
mod screens;
mod seen;
mod stickers;
mod theme;
mod time;
mod view;
mod widgets;
mod window;

use egui::{CursorIcon, Frame, Margin, Panel, Stroke, Theme, ThemePreference, Ui};

use crate::backend::{self, Backend, Command, SessionView, Status};
use crate::i18n::tr;
use fonts::Weight;
use widgets::{Kind, rich, rich_mono};

#[derive(Clone, Copy, PartialEq)]
pub enum Screen {
    Ringkasan,
    Nomor,
    Pairing,
    Inbox,
    Status,
    Log,
    Integrations,
    Settings,
    /// Phone layout only: the "Lainnya" menu.
    More,
}

/// The message the composer is replying to (Balas), for one chat.
pub struct ReplyTarget {
    /// Number (session) the message belongs to; the same chat JID can exist under two numbers.
    pub session: String,
    /// JID of the chat; the reply is dropped when another chat opens.
    pub chat: String,
    /// Id of the quoted message.
    pub id: String,
    /// "Anda" or the sender's name.
    pub author: String,
    /// Start of its text, for the bar above the composer.
    pub snippet: String,
}

/// A voice note in the composer, for the chat (and number) it was started in:
/// it is sent there even if another chat is open by the time it is saved.
pub struct VoiceNote {
    pub session: String,
    pub chat: String,
    pub step: VoiceStep,
    /// Microphone level every 60 ms while recording: the live wave, and the
    /// whole note's wave while it is paused.
    pub wave: Vec<f32>,
    /// When `wave` last got a bar.
    pub sampled: Option<std::time::Instant>,
    /// What was recorded so far, on its way from the recording thread to be
    /// heard while paused.
    pub preview: Option<std::sync::mpsc::Receiver<Vec<f32>>>,
    /// Phone layout: the mic is held down (Pego `m5/VN 1`). Letting go sends,
    /// sliding left cancels, sliding up locks it into the normal recorder.
    pub held: bool,
}

impl VoiceNote {
    pub fn new(session: &str, chat: &str, step: VoiceStep) -> Self {
        Self { session: session.to_owned(), chat: chat.to_owned(), step, wave: Vec::new(), sampled: None, preview: None, held: false }
    }
}

pub enum VoiceStep {
    Recording(crate::backend::voice::Recorder),
    /// Finished on the recording thread; the file's path arrives here.
    Saving(std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>),
    /// Why it could not be recorded or sent, shown under the composer for a
    /// few seconds.
    Failed(String, std::time::Instant),
}

/// A sidebar page: where it goes, its icon, its label, its count or badge.
type SideItem = (Screen, &'static str, &'static str, Option<String>);

/// The "Chat wallpaper" dialog of a number (#60): which number, and its
/// choice as edited (key `app` = the app's wallpaper).
pub struct NumberWallpaperEdit {
    pub id: String,
    pub name: String,
    pub wall: backend::settings::NumberWallpaper,
}

impl NumberWallpaperEdit {
    pub fn new(settings: &backend::Settings, session: &SessionView) -> Self {
        let wall = settings.number_wallpapers.get(&session.id).cloned().unwrap_or_else(|| backend::settings::NumberWallpaper { key: "app".to_owned(), file: String::new(), dim: 30 });
        Self { id: session.id.clone(), name: session.name.clone(), wall }
    }
}

/// UI state that survives between frames.
#[derive(Default)]
pub struct State {
    pub nomor_tab: usize,
    pub nomor_search: String,
    /// Session whose inbox is shown.
    pub inbox_session: Option<String>,
    pub chat_tab: usize,
    pub chat_search: String,
    /// Phone layout: the Inbox search field is open.
    pub m_search: bool,
    /// JID of the open chat.
    pub chat_selected: Option<String>,
    pub composer: String,
    /// The chat told "sedang mengetik" (session id, JID) and when it was last
    /// told; the composer text as last seen, and when it last changed.
    pub typing: Option<(String, String, std::time::Instant)>,
    pub typing_seen_text: String,
    pub typing_changed: Option<std::time::Instant>,
    /// Contacts typing now (from the backend), and chats whose presence was
    /// already asked for this run.
    pub typing_now: std::sync::Arc<std::collections::HashMap<String, backend::Typing>>,
    pub presence_watched: std::collections::HashSet<(String, String)>,
    /// Contacts' online state (from the backend), keyed like `typing_now`.
    pub presence_now: std::sync::Arc<std::collections::HashMap<String, backend::Presence>>,
    /// Group members by `session|group` (from the backend), and the groups
    /// whose members were already asked for this run.
    pub members_now: std::sync::Arc<std::collections::HashMap<String, Vec<backend::Member>>>,
    /// Groups' details for the contact panel (#91), keyed like `members_now`.
    pub groups_now: std::sync::Arc<std::collections::HashMap<String, backend::GroupInfo>>,
    pub members_asked: std::collections::HashMap<(String, String), std::time::Instant>,
    /// `@Name` picked in the composer with the member's JID, the chat they
    /// belong to (`session|chat`), the highlighted picker row, the `@` of a
    /// picker closed with Esc, and the query the highlight belongs to.
    pub mention_picks: Vec<(String, String)>,
    pub mention_chat: String,
    pub mention_sel: usize,
    pub mention_dismissed: Option<usize>,
    pub mention_query: String,
    /// Message the next text goes out as a reply to.
    pub reply: Option<ReplyTarget>,
    /// Our own message the next text replaces (#73); its snippet is the old text.
    pub editing: Option<ReplyTarget>,
    /// File waiting in the composer, sent with the text as its caption.
    pub attachment: Option<std::path::PathBuf>,
    /// PDF open in the in-app viewer.
    pub pdf_view: Option<pdf::Viewer>,
    /// Chat video open in the in-app viewer (#84).
    pub video_view: Option<movie_view::Viewer>,
    /// Profile photo open in the pop-up viewer (#92), and the full-size ones fetched.
    pub photo_view: Option<photo_view::Viewer>,
    pub full_avatars: std::sync::Arc<std::collections::HashMap<String, Option<std::path::PathBuf>>>,
    /// "Teruskan ke…" dialog for a message of the open chat.
    pub forward: Option<forward::Dialog>,
    /// "Delete message?" asked for a message of the open chat (#72).
    pub delete_ask: Option<screens::inbox::DeleteAsk>,
    /// The file dialog running on its own thread; `None` inside when cancelled.
    pub picking: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
    /// A photo being taken with the webcam for the composer (#71).
    pub camera: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    /// Why the last attachment could not be made, shown for a few seconds.
    pub attach_note: Option<(String, std::time::Instant)>,
    /// "Share a contact" dialog for the open chat (#71).
    pub contact_pick: Option<screens::inbox::ContactPick>,
    /// The voice note in the composer, tied to the chat it was started in.
    pub voice: Option<VoiceNote>,
    /// Result of the last CSV export, shown for a few seconds.
    pub export_note: Option<(String, std::time::Instant)>,
    /// Settings: the picture being chosen (and copied) for the chat wallpaper.
    pub wallpaper_picking: Option<std::sync::mpsc::Receiver<Option<Result<std::path::PathBuf, String>>>>,
    /// The voice call from the backend (#39), and whether the phone layout
    /// shows it as a banner.
    pub call_now: Option<backend::call::CallView>,
    /// The latest release check (#93), for the banner and Settings.
    pub update: backend::update::UpdateState,
    pub call_minimized: bool,
    /// The call's video pictures as textures (#55).
    pub call_textures: call::Textures,
    /// "Recording audio" last told to this chat, and when.
    pub recording_told: Option<(String, String, std::time::Instant)>,
    /// Chats whose profile picture was already asked for.
    pub avatar_asked: std::collections::HashSet<String>,
    /// Chat whose older messages were just requested, and when.
    pub older_asked: Option<(String, std::time::Instant)>,
    pub contact_panel: bool,
    /// The details were asked for (the chat's name or the panel button), so
    /// they may take the chat's place as a page when there is no room beside
    /// it; a chat just opened shows the chat (#91).
    pub contact_page: bool,
    /// The Inbox's device panel (Perangkat) is shown.
    pub devices_panel: bool,
    pub log_tab: usize,
    /// (session id, message id)
    pub log_selected: Option<(String, String)>,
    /// Pairing screen: false = QR, true = phone code.
    pub pair_with_code: bool,
    pub pair_phone: String,
    /// The pairing screen's session details being edited (#82).
    pub pair_edit: Option<screens::pairing::PairEdit>,
    pub qr: Option<qr::Qr>,
    /// Session waiting for the user to confirm its removal.
    pub confirm_remove: Option<String>,
    /// "Ganti nama" dialog: the number and the name being typed.
    pub rename: Option<(String, String)>,
    /// "Chat wallpaper" dialog of a number (#60).
    pub number_wallpaper: Option<NumberWallpaperEdit>,
    /// Asked from a menu: make this the main number, or have none (#138).
    pub set_main: Option<Option<String>>,
    /// The "Colour and icon" dialog of this number is open (#142).
    pub number_look: Option<String>,
    /// "Tautkan ulang" waiting for a yes: it deletes the number's keys.
    pub confirm_relink: Option<String>,
    /// The saved settings, for screens that show them (webhook per number).
    pub settings: std::sync::Arc<backend::Settings>,
    /// Settings being edited on the Pengaturan and Webhook & API screens.
    pub settings_form: screens::settings::Form,
    /// Inbox: the number whose chats are listed (`None` = every number),
    /// and whether "every number" is split into one section per number.
    pub inbox_scope: Option<String>,
    pub inbox_grouped: bool,
    /// The number this state belongs to when it is a number's own window.
    pub window: Option<String>,
    /// A number to open in a window of its own.
    pub open_window_requested: Option<String>,
    /// "Pisah per nomor": numbers whose section is folded.
    pub folded: std::collections::HashSet<String>,
    /// The chat list scrolls to the open chat once (it was opened elsewhere).
    pub reveal_chat: bool,
    /// A chat was just opened: its text box takes the keyboard, so typing
    /// needs no click first.
    pub focus_composer: bool,
    /// "Tautkan nomor" was clicked in the Inbox's device panel.
    pub add_number_requested: bool,
    /// Ringkasan "Pesan terbaru": 0 = belum dibalas, 1 = semua.
    pub summary_tab: usize,
    /// Webhook & API: 0 = REST API, 1 = Webhook, 2 = Pemicu.
    pub integrations_tab: usize,
    /// Pemicu: the rule open in the editor, and the editor's test message.
    pub rule_selected: Option<usize>,
    pub rule_test: String,
    /// Times each trigger fired today (from the backend).
    pub rule_hits: std::sync::Arc<std::collections::HashMap<String, (i64, u32)>>,
    /// Inbox Status tab: the poster open in the viewer (`Some(None)` = this
    /// number), which of their updates, and updates already marked seen.
    pub status_selected: Option<Option<String>>,
    pub status_index: usize,
    pub status_marked: std::collections::HashSet<String>,
    /// Status page: the number whose view of the poster is open, the number
    /// filter, the search, the comment being typed, and the auto-advance timer
    /// (update shown, since when) with its pause.
    pub status_session: Option<String>,
    pub status_scope: Option<String>,
    pub status_search: String,
    pub status_comment: String,
    pub status_shown: Option<(String, std::time::Instant)>,
    pub status_paused: bool,
    /// The Status list's filter (#101): 0 all (the default), 1 not muted,
    /// 2 muted only.
    pub status_filter: usize,
    /// A screen asked to open this chat (session id, JID) in the Inbox.
    pub open_chat_requested: Option<(String, String)>,
    /// Unread messages of the chat that was just opened, captured before `Seen` clears them.
    pub unread_anchor: Option<UnreadAnchor>,
}

/// Where the unread messages of an opened chat start; `pending` is the
/// one-shot "scroll to the divider" flag.
#[derive(Clone, Debug)]
pub struct UnreadAnchor {
    pub jid: String,
    pub count: usize,
    pub first_id: String,
    pub pending: bool,
}

pub struct App {
    /// Anti-peek mode's auto-lock (#57).
    lock: privacy::Lock,
    screen: Screen,
    search: String,
    state: State,
    backend: Backend,
    /// Snapshot of the sessions for this frame.
    sessions: Vec<SessionView>,
    avatars: std::sync::Arc<std::collections::HashMap<String, Option<std::path::PathBuf>>>,
    downloads: std::sync::Arc<std::collections::HashMap<String, Option<String>>>,
    settings: std::sync::Arc<backend::Settings>,
    webhooks: std::sync::Arc<Vec<backend::webhook::Delivery>>,
    api: backend::ApiState,
    /// Theme taken from the settings last; `None` until the first frame.
    applied_theme: Option<String>,
    applied_template: Option<String>,
    /// Keeps the screen and the Inbox's number and tab for the next start (#128).
    view: view::Keeper,
    /// The start opened on the screen the last run ended on.
    screen_restored: bool,
    /// Session shown on the Pairing screen.
    pairing_id: Option<String>,
    /// The chat last reported to the notifier as on screen.
    viewing: Option<(String, String)>,
    /// Whether a warust window was in front when last told to the backend.
    active: Option<bool>,
    /// The backend's `presence_epoch` last acted on.
    presence_epoch: u64,
    /// Marks the chat on screen read as messages arrive in it.
    seen: seen::Watch,
    /// The unread count last shown on the taskbar icon and the title.
    shown_unread: Option<u32>,
    /// The Inbox was put on the main number at start, or nothing waits for it (#138).
    main_applied: bool,
    /// A ready update was asked to install (once per download, #93).
    update_asked: bool,
    /// Numbers open in windows of their own (issue #28).
    windows: Vec<NumberWindow>,
    /// The tray icon while "close to tray" is on (#134). `None` when it is
    /// off or this desktop has no tray: closing the window then quits.
    tray: Option<crate::tray::Tray>,
    /// The setting and the language `tray` was last made for.
    tray_made_for: Option<(bool, String)>,
    /// What was picked on the tray icon, for `logic` to act on.
    tray_asked: std::sync::Arc<TrayAsked>,
    /// The main window was closed to the tray and nothing showed it since.
    hidden: bool,
    /// The app is ending for real (the tray's Quit, a restart into an
    /// update), so this close is not turned into hiding.
    quitting: bool,
}

/// What the tray icon asked for since `logic` last looked. Its callbacks run
/// outside the UI's frame, on Linux on another thread.
#[derive(Default)]
struct TrayAsked {
    open: std::sync::atomic::AtomicBool,
    quit: std::sync::atomic::AtomicBool,
}

/// The app icon at the size of a tray icon.
fn tray_icon() -> crate::tray::Icon {
    let image = image::load_from_memory(include_bytes!("../../assets/app/warust-256.png"))
        .expect("the bundled icon is a valid PNG")
        .resize_exact(64, 64, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = image.dimensions();
    crate::tray::Icon { rgba: image.into_raw(), width, height }
}

/// One number's inbox in its own OS window, with its own chat, composer and
/// scroll positions. It shares the connection with the main window.
struct NumberWindow {
    id: String,
    state: State,
}

impl NumberWindow {
    fn new(id: String) -> Self {
        let state = State {
            inbox_scope: Some(id.clone()),
            inbox_session: Some(id.clone()),
            window: Some(id.clone()),
            contact_panel: true,
            ..State::default()
        };
        Self { id, state }
    }

    fn viewport(&self) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("number_window", &self.id))
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        fonts::install(&cc.egui_ctx);
        theme::install(&cc.egui_ctx);
        // Dev aid: WARUST_SCREEN=nomor|pairing|inbox|status|log|webhook|settings|lainnya,
        // WARUST_THEME=light|dark (overrides the saved theme).
        let env_theme = std::env::var("WARUST_THEME").ok();
        match env_theme.as_deref() {
            Some("light") => cc.egui_ctx.set_theme(ThemePreference::Light),
            Some("dark") => cc.egui_ctx.set_theme(ThemePreference::Dark),
            _ => {}
        }
        let screen = match std::env::var("WARUST_SCREEN").as_deref() {
            Ok("ringkasan") => Screen::Ringkasan,
            Ok("pairing") => Screen::Pairing,
            Ok("inbox") => Screen::Inbox,
            Ok("status") => Screen::Status,
            Ok("lainnya") => Screen::More,
            Ok("log") => Screen::Log,
            Ok("webhook" | "pemicu") => Screen::Integrations,
            Ok("settings") => Screen::Settings,
            _ => Screen::Nomor,
        };
        let mut state = State {
            contact_panel: true,
            // Dev aid: WARUST_SCREEN=pemicu opens the first trigger.
            integrations_tab: if std::env::var("WARUST_SCREEN").as_deref() == Ok("pemicu") { 2 } else { 0 },
            rule_selected: (std::env::var("WARUST_SCREEN").as_deref() == Ok("pemicu")).then_some(0),
            ..State::default()
        };
        // Where the last run ended (#128). The dev aids still win over it.
        let (mut view, last_screen) = view::Keeper::open(crate::platform::data_dir(), &mut state);
        // A dev override must not become the next normal start's view.
        if std::env::var_os("WARUST_SCREEN").is_some() || std::env::var_os("WARUST_DEVICES").is_some() {
            view.read_only();
        }
        // Dev aid: WARUST_DEVICES=hidden starts with the device panel hidden.
        if std::env::var("WARUST_DEVICES").as_deref() == Ok("hidden") {
            state.devices_panel = false;
        }
        let last_screen = last_screen.filter(|_| std::env::var_os("WARUST_SCREEN").is_none());
        let screen_restored = last_screen.is_some();
        let screen = last_screen.unwrap_or(screen);
        // Sessions live in the data folder (device stores in sessions/<id>.db).
        let backend = Backend::start(cc.egui_ctx.clone(), crate::platform::data_dir().to_path_buf());
        Self {
            screen,
            view,
            screen_restored,
            search: String::new(),
            backend,
            sessions: Vec::new(),
            avatars: Default::default(),
            downloads: Default::default(),
            settings: Default::default(),
            webhooks: Default::default(),
            api: backend::ApiState::Off,
            applied_theme: env_theme,
            applied_template: None,
            // Dev aid: WARUST_PAIR=<session id> opens its pairing screen.
            pairing_id: std::env::var("WARUST_PAIR").ok(),
            viewing: None,
            active: None,
            presence_epoch: 0,
            seen: seen::Watch::default(),
            lock: privacy::Lock::default(),
            windows: Vec::new(),
            shown_unread: None,
            main_applied: false,
            update_asked: false,
            tray: None,
            tray_made_for: None,
            tray_asked: Default::default(),
            hidden: false,
            quitting: false,
            state,
        }
    }

    /// Unread messages on the taskbar/Dock icon and in the window title.
    /// Muted chats keep their own count but not the icon's, as on the phone.
    fn show_unread(&mut self, ctx: &egui::Context) {
        let now = time::now();
        let unread: u32 = self.sessions.iter().flat_map(|s| s.book.chats.iter()).filter(|c| !c.is_muted(now)).map(|c| c.unread).sum();
        if self.shown_unread != Some(unread) {
            crate::platform::set_unread_badge(unread);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(crate::platform::window_title(unread)));
            self.shown_unread = Some(unread);
        }
    }

    /// Keeps the tray icon in step with the "close to tray" setting and the
    /// language of its menu (#134).
    fn sync_tray(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        use std::sync::atomic::Ordering;
        let want = (self.settings.close_to_tray, self.settings.language.clone());
        if self.tray_made_for.as_ref() == Some(&want) {
            return;
        }
        // The old icon goes before a new one is made: its menu is in the old
        // language.
        self.tray = None;
        // Wayland can neither hide a window nor bring one back (winit says so
        // by not knowing whether it is visible), so there the icon would lead
        // nowhere: no icon, and closing the window quits.
        let can_hide = cfg!(target_os = "macos") || frame.winit_window().is_some_and(|w| w.is_visible().is_some());
        if !can_hide {
            log::info!("this desktop cannot hide a window: closing it quits warust");
        }
        if want.0 && can_hide {
            let labels = crate::tray::Labels {
                tooltip: "warust".to_owned(),
                open: tr("Open warust", "Buka warust").to_owned(),
                quit: tr("Quit", "Keluar").to_owned(),
            };
            let (asked, ctx) = (self.tray_asked.clone(), ctx.clone());
            let handler = move |event| {
                match event {
                    crate::tray::Event::Open => asked.open.store(true, Ordering::Relaxed),
                    crate::tray::Event::Quit => asked.quit.store(true, Ordering::Relaxed),
                }
                // Runs `logic`, also while the window is hidden.
                ctx.request_repaint();
            };
            self.tray = crate::tray::Tray::new(&tray_icon(), &labels, std::sync::Arc::new(handler));
            if self.tray.is_none() {
                log::info!("no tray icon on this desktop: closing the window quits warust");
            }
        }
        // Also when there is no tray here, so it is not tried on every frame.
        self.tray_made_for = Some(want);
    }

    /// Installs a downloaded update once nothing is going on (no call, empty
    /// queues, no recording, nothing typed), and quits for the restart. Runs
    /// from the banner and, while warust waits in the tray where `ui` does
    /// not run, from `logic` (#93, #134).
    fn install_when_idle(&mut self, ctx: &egui::Context) {
        use backend::update::UpdateState;
        match &self.state.update {
            UpdateState::Restarting => {
                self.update_asked = false;
                self.quit(ctx);
            }
            UpdateState::Ready(_) => {
                let idle = self.state.call_now.is_none()
                    && self.sessions.iter().all(|s| s.queue == 0)
                    && self.state.voice.is_none()
                    && self.state.composer.trim().is_empty();
                if idle && !self.update_asked {
                    self.update_asked = true;
                    self.backend.send(Command::InstallUpdate);
                }
                // Looks again for an idle moment even when nothing repaints.
                ctx.request_repaint_after(std::time::Duration::from_secs(5));
            }
            _ => self.update_asked = false,
        }
    }

    /// Closes the main window to the tray (#134): warust keeps running.
    fn hide_window(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        // No window is in front now: the numbers go offline (#131), as `ui`,
        // which would notice, does not run while hidden.
        if self.active != Some(false) {
            self.active = Some(false);
            self.backend.send(Command::Active { active: false });
        }
        // No chat is on screen any more, so the notifier holds none back.
        self.backend.set_viewing(None);
        self.viewing = None;
        if !crate::platform::hide_app() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    /// Brings the main window back from the tray, or up from behind.
    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        crate::platform::unhide_app();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    /// Ends the app for real: `logic` lets this close through.
    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        if self.settings.nav_icons_only {
            self.sidebar_rail(ui);
            return;
        }
        let p = theme::palette(ui.ctx());
        Panel::left("sidebar")
            .default_size(232.0)
            .size_range(200.0..=320.0)
            .resizable(true)
            .frame(
                Frame::NONE
                    .fill(p.panel)
                    .inner_margin(Margin::symmetric(12, 14)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let panel = ui.max_rect();
                // The logo row continues the title bar across the sidebar.
                let title = egui::Rect::from_min_size(panel.min - egui::vec2(12.0, 14.0), egui::vec2(panel.width() + 12.0, 52.0));
                window::drag_area(ui, title, "sidebar");
                // macOS: the window buttons get the title row to themselves and
                // the logo row sits under it.
                if window::buttons_inset(ui.ctx()) > 0.0 {
                    ui.add_space(title.bottom() - panel.top());
                }
                widgets::row(
                    ui,
                    32.0,
                    |ui| {
                        ui.add_space(6.0);
                        let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::hover());
                        widgets::logo(ui, mark, p.primary, p.on_primary);
                        ui.label(rich("warust", 14.0, Weight::SemiBold, p.text));
                    },
                    |ui| {
                        ui.add_space(6.0);
                        icons::show(ui, "chevrons-up-down", 16.0, p.muted);
                    },
                );
                ui.add_space(14.0);
                widgets::search(ui, &mut self.search, tr("Search", "Cari"), ui.available_width(), true);
                ui.add_space(20.0);

                widgets::section_label(ui, "Gateway");
                ui.add_space(4.0);
                let (gateway, integrations) = self.side_items();
                for (screen, icon, label, count) in gateway {
                    if widgets::nav_item(ui, icon, label, count.as_deref(), self.side_selected(screen)).clicked() {
                        self.screen = screen;
                    }
                }
                ui.add_space(20.0);
                widgets::section_label(ui, tr("Integrations", "Integrasi"));
                ui.add_space(4.0);
                for (screen, icon, label, badge) in integrations {
                    if widgets::nav_item(ui, icon, label, badge.as_deref(), self.side_selected(screen)).clicked() {
                        self.screen = screen;
                    }
                }

                // Pinned to the bottom of the sidebar.
                let status = egui::Rect::from_min_max(
                    egui::pos2(panel.left(), panel.bottom() - 60.0),
                    panel.max,
                );
                gateway_status(
                    &mut widgets::child(ui, status, egui::Layout::top_down(egui::Align::Min)),
                    &self.sessions,
                );
            });
    }

    /// The sidebar's pages with their counts: Gateway, then Integrations.
    fn side_items(&self) -> ([SideItem; 5], [SideItem; 2]) {
        let shown = |n: usize| (n > 0).then(|| n.to_string());
        let unread: u32 = self.sessions.iter().flat_map(|s| s.book.chats.iter()).map(|c| c.unread).sum();
        let unseen: usize = self.sessions.iter().map(|s| screens::status::unseen_posters(&s.book)).sum();
        let api_on = matches!(self.api, backend::ApiState::Listening(_)).then(|| "API".to_owned());
        (
            [
                (Screen::Ringkasan, "gauge", tr("Overview", "Ringkasan"), None),
                (Screen::Nomor, "smartphone", tr("Numbers", "Nomor"), Some(self.sessions.len().to_string())),
                (Screen::Inbox, "inbox", "Inbox", shown(unread as usize)),
                (Screen::Status, "circle-dashed", "Status", shown(unseen)),
                (Screen::Log, "scroll-text", tr("Message log", "Log pesan"), None),
            ],
            [
                (Screen::Integrations, "webhook", "Webhook & API", api_on),
                (Screen::Settings, "settings", tr("Settings", "Pengaturan"), None),
            ],
        )
    }

    fn side_selected(&self, screen: Screen) -> bool {
        self.screen == screen || (screen == Screen::Nomor && self.screen == Screen::Pairing)
    }

    /// The Classic sidebar as a rail of icons (#102): Settings → Appearance →
    /// Navigation → Icons only. Labels move to the hints, counts to badges.
    fn sidebar_rail(&mut self, ui: &mut Ui) {
        let p = theme::palette(ui.ctx());
        Panel::left("sidebar_rail")
            .exact_size(64.0)
            .resizable(false)
            .frame(Frame::NONE.fill(p.panel).inner_margin(Margin::symmetric(8, 14)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let panel = ui.max_rect();
                let title = egui::Rect::from_min_size(panel.min - egui::vec2(8.0, 14.0), egui::vec2(panel.width() + 16.0, 52.0));
                window::drag_area(ui, title, "sidebar");
                if window::buttons_inset(ui.ctx()) > 0.0 {
                    ui.add_space(title.bottom() - panel.top());
                }
                let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::hover());
                widgets::logo(ui, egui::Rect::from_center_size(row.center(), egui::Vec2::splat(26.0)), p.primary, p.on_primary);
                ui.add_space(22.0);
                let (gateway, integrations) = self.side_items();
                for (screen, icon, label, count) in gateway {
                    if widgets::nav_icon(ui, icon, label, count.as_deref(), self.side_selected(screen)).clicked() {
                        self.screen = screen;
                    }
                }
                ui.add_space(10.0);
                let (line, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
                ui.painter().hline(line.x_range().shrink(8.0), line.center().y, Stroke::new(1.0, p.border));
                ui.add_space(10.0);
                for (screen, icon, label, badge) in integrations {
                    if widgets::nav_icon(ui, icon, label, badge.as_deref(), self.side_selected(screen)).clicked() {
                        self.screen = screen;
                    }
                }
                // The gateway's state as a dot at the bottom, its words in the hint.
                let connected = self.sessions.iter().filter(|s| s.status == Status::Connected).count();
                let spot = egui::Rect::from_center_size(egui::pos2(panel.center().x, panel.bottom() - 12.0), egui::vec2(panel.width(), 24.0));
                let response = ui.interact(spot, ui.scope_id().with("rail_status"), egui::Sense::hover());
                ui.painter().circle_filled(spot.center(), 4.0, if connected > 0 { p.green } else { p.gray });
                let api = if matches!(self.api, backend::ApiState::Listening(_)) { tr("REST API running", "REST API berjalan") } else { tr("REST API not active", "REST API belum aktif") };
                response.on_hover_text(format!(
                    "{}\n{api}\nv{}",
                    crate::trf!("{connected} of {} connected", "{connected} dari {} terhubung", self.sessions.len()),
                    env!("CARGO_PKG_VERSION")
                ));
            });
    }

    fn topbar(&mut self, ui: &mut Ui) {
        let p = theme::palette(ui.ctx());
        let (parent, current) = match self.screen {
            Screen::Ringkasan => ("Gateway", tr("Overview", "Ringkasan")),
            Screen::Nomor => ("Gateway", tr("Numbers", "Nomor")),
            Screen::Pairing => (tr("Numbers", "Nomor"), tr("Link new number", "Tautkan nomor baru")),
            Screen::Inbox => ("Gateway", "Inbox"),
            Screen::Status => ("Gateway", "Status"),
            Screen::More => ("Gateway", tr("More", "Lainnya")),
            Screen::Log => ("Gateway", tr("Message log", "Log pesan")),
            Screen::Integrations => (tr("Integrations", "Integrasi"), "Webhook & API"),
            Screen::Settings => (tr("Integrations", "Integrasi"), tr("Settings", "Pengaturan")),
        };
        Panel::top("topbar")
            .exact_size(52.0)
            .resizable(false)
            .frame(Frame::NONE.fill(p.bg).inner_margin(Margin {
                left: 24,
                right: 10,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| {
                // The topbar is also the window's title bar.
                window::drag_area(ui, ui.max_rect().expand2(egui::vec2(10.0, 0.0)), "topbar");
                ui.horizontal_centered(|ui| {
                    let parent_label = ui
                        .label(rich(parent, 13.0, Weight::Regular, p.muted))
                        .interact(egui::Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    if parent_label.clicked() && self.screen == Screen::Pairing {
                        self.screen = Screen::Nomor;
                    }
                    ui.label(rich("/", 13.0, Weight::Regular, p.muted));
                    ui.label(rich(current, 13.0, Weight::Medium, p.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        window::controls(ui);
                        // The window buttons sit tight; everything else gets normal spacing.
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if !window::NATIVE {
                            ui.add_space(6.0);
                            let (sep, _) = ui.allocate_exact_size(egui::vec2(1.0, 20.0), egui::Sense::hover());
                            ui.painter().rect_filled(sep, 0.0, p.border);
                            ui.add_space(6.0);
                        }
                        let dark = ui.ctx().theme() == Theme::Dark;
                        let toggle = widgets::icon_button(ui, if dark { "sun" } else { "moon" }, 32.0, false)
                            .on_hover_text(if dark { tr("Light theme", "Tema terang") } else { tr("Dark theme", "Tema gelap") });
                        if toggle.clicked() {
                            self.toggle_theme(dark);
                        }
                        self.privacy_pill(ui, false);
                        self.topbar_actions(ui, false);
                    });
                });
            });
    }

    /// The update bar under the top bar (#93): a new release to download
    /// (What's new, Download, Later), or, installing by itself, its download,
    /// "installs when idle" and the install. Once nothing is going on (no
    /// call, empty queues, no recording, nothing typed) a ready update
    /// installs, and the window closes for warust to start again.
    fn update_banner(&mut self, ui: &mut Ui) {
        use backend::update::UpdateState;
        self.install_when_idle(ui.ctx());
        let update = self.state.update.clone();
        let (text, release, buttons) = match &update {
            UpdateState::Restarting => return,
            UpdateState::Available(r) if self.settings.updates.skipped != r.version => (
                crate::trf!("warust {} is available (you have {})", "warust {} sudah tersedia (terpasang {})", r.version, backend::update::current()),
                r.clone(),
                true,
            ),
            UpdateState::Downloading(r) => (crate::trf!("Downloading warust {}…", "Mengunduh warust {}…", r.version), r.clone(), false),
            UpdateState::Installing(r) => (crate::trf!("Installing warust {}; warust restarts by itself", "Memasang warust {}; warust akan mulai ulang sendiri", r.version), r.clone(), false),
            UpdateState::Ready(r) => {
                (crate::trf!("warust {} is ready and installs once warust is idle", "warust {} siap dan dipasang saat warust tidak dipakai", r.version), r.clone(), false)
            }
            _ => return,
        };
        let p = theme::palette(ui.ctx());
        let mut later = false;
        Panel::top("update_banner")
            .exact_size(44.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::NONE.fill(p.accent_soft).inner_margin(Margin::symmetric(16, 0)))
            .show(ui, |ui| {
                widgets::row(
                    ui,
                    44.0,
                    |ui| {
                        icons::show(ui, if buttons { "download" } else { "loader-circle" }, 16.0, p.accent_text);
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(rich(&text, 13.0, Weight::SemiBold, p.accent_text)).truncate());
                    },
                    |ui| {
                        if !buttons {
                            return;
                        }
                        if widgets::button(ui, None, tr("Later", "Nanti"), Kind::Secondary).clicked() {
                            later = true;
                        }
                        if widgets::button(ui, None, tr("What's new", "Apa yang baru"), Kind::Secondary).clicked() {
                            crate::platform::open_url(&release.page);
                        }
                        if widgets::button(ui, Some("download"), tr("Download", "Unduh"), Kind::Primary).clicked() {
                            crate::platform::open_url(&release.download);
                        }
                    },
                );
            });
        if later {
            let mut settings = (*self.settings).clone();
            settings.updates.skipped = release.version.clone();
            self.backend.send(Command::SaveSettings(settings));
        }
    }

    /// Forest: brand, pill navigation, API state and the window buttons in one
    /// bar, which is also the title bar.
    fn topnav(&mut self, ui: &mut Ui) {
        let p = theme::palette(ui.ctx());
        Panel::top("topnav")
            .exact_size(72.0)
            .resizable(false)
            .frame(Frame::NONE.fill(p.bg).inner_margin(Margin { left: 32, right: 10, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                window::drag_area(ui, ui.max_rect().expand2(egui::vec2(32.0, 0.0)), "topnav");
                // Short of room, the buttons on the right shrink to icons (and
                // then the wordmark goes) so they never run into the navigation.
                let room = ui.available_width();
                let compact = room < TOPNAV_FULL;
                let tight = room < TOPNAV_TIGHT;
                ui.horizontal_centered(|ui| {
                    // Past the macOS window buttons (the bar's margin is 32).
                    ui.add_space((window::buttons_inset(ui.ctx()) - 32.0).max(0.0));
                    let (logo, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                    widgets::logo(ui, logo, p.primary, p.accent);
                    if !tight {
                        ui.label(rich("warust", 17.0, Weight::SemiBold, p.text));
                    }
                    ui.add_space(14.0);
                    // The same icons as the classic sidebar.
                    let items = [
                        (Screen::Ringkasan, "gauge", tr("Overview", "Ringkasan")),
                        (Screen::Nomor, "smartphone", tr("Numbers", "Nomor")),
                        (Screen::Inbox, "inbox", "Inbox"),
                        (Screen::Status, "circle-dashed", "Status"),
                        (Screen::Log, "scroll-text", tr("Message log", "Log pesan")),
                        (Screen::Integrations, "webhook", "Webhook & API"),
                    ];
                    let current = if self.screen == Screen::Pairing { Screen::Nomor } else { self.screen };
                    let mut selected = items.iter().position(|(s, _, _)| *s == current).unwrap_or(usize::MAX);
                    let pills: Vec<(&str, &str)> = items.iter().map(|(_, icon, label)| (*icon, *label)).collect();
                    // Icons only when chosen in Settings, or when the bar is tight.
                    if widgets::icon_pills(ui, &pills, &mut selected, tight || self.settings.nav_icons_only) {
                        self.screen = items[selected].0;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        window::controls(ui);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add_space(6.0);
                        let settings = widgets::icon_button(ui, "settings", 36.0, self.screen == Screen::Settings)
                            .on_hover_text(tr("Settings", "Pengaturan"));
                        if settings.clicked() {
                            self.screen = Screen::Settings;
                        }
                        api_pill(ui, &p, &self.api, compact);
                        self.privacy_pill(ui, compact);
                        self.topbar_actions(ui, compact);
                    });
                });
            });
    }

    /// Right-to-left: the first button added ends up rightmost.
    /// The page's own buttons at the right of the bar; `compact` keeps only
    /// their icons, with the label as a hint.
    fn topbar_actions(&mut self, ui: &mut Ui, compact: bool) {
        let p = theme::palette(ui.ctx());
        match self.screen {
            Screen::Nomor => {
                if action_button(ui, "plus", tr("Add number", "Tambah nomor"), Kind::Primary, compact).clicked() {
                    self.add_number();
                }
                if action_button(ui, "book-open", tr("API docs", "Dokumentasi API"), Kind::Secondary, compact).clicked() {
                    self.screen = Screen::Integrations;
                    self.state.integrations_tab = 0;
                }
            }
            Screen::Pairing => {
                if widgets::button(ui, Some("x"), tr("Cancel", "Batal"), Kind::Secondary).clicked() {
                    self.cancel_pairing();
                }
            }
            Screen::Ringkasan | Screen::Inbox | Screen::Status | Screen::Integrations | Screen::Settings | Screen::More => {}
            Screen::Log => {
                if action_button(ui, "download", tr("Export CSV", "Ekspor CSV"), Kind::Secondary, compact).clicked() {
                    let note = match screens::log::export_csv(&self.sessions) {
                        Ok((path, rows)) => crate::trf!("{rows} messages exported to {}", "{rows} pesan diekspor ke {}", path.display()),
                        Err(e) => crate::trf!("Export failed: {e}", "Ekspor gagal: {e}"),
                    };
                    self.state.export_note = Some((note, std::time::Instant::now()));
                }
                ui.add_space(8.0);
                // Right to left: the label first, then the dot ends up before it.
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(rich("Live", 12.0, Weight::Medium, p.text_2));
                let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, p.green);
            }
        }
    }
}

/// "Lock when idle" choices in minutes; 0 is never.
const LOCK_MINUTES: [u32; 5] = [0, 1, 2, 5, 10];

/// The choice shown for a stored idle time: its own, or 2 minutes (the
/// default) for a value that is none of the choices.
fn lock_choice(minutes: u32) -> usize {
    LOCK_MINUTES.iter().position(|m| *m == minutes).unwrap_or(2)
}

impl App {
    /// Saved, so the next start opens in the same theme.
    /// Changes the anti-peek options and saves them at once.
    /// "Colour and icon" of a number (#142): the colours and icons to pick
    /// from, each choice saved at once and shown everywhere the number is.
    fn number_look_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.state.number_look.clone().or_else(|| self.windows.iter_mut().find_map(|w| w.state.number_look.take())) else {
            return;
        };
        let p = theme::palette(ctx);
        let name = self.sessions.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();
        let (color, icon) = (widgets::number_color_index(&id), widgets::number_look(&id).1);
        let mut picked: Option<backend::settings::NumberLook> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("number_look"))
            .frame(Frame::NONE.fill(p.panel).stroke(Stroke::new(1.0, p.border_strong)).corner_radius(theme::R_LG).inner_margin(Margin::same(24)))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.label(rich(&crate::trf!("Colour and icon · {}", "Warna dan ikon · {}", name), 16.0, Weight::SemiBold, p.text));
                ui.label(rich(tr("Colour", "Warna"), 13.0, Weight::Medium, p.text_2));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                    for (i, swatch) in widgets::NUMBER_COLORS.iter().enumerate() {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
                        ui.painter().circle_filled(rect.center(), 15.0, *swatch);
                        if i == color {
                            ui.painter().circle_stroke(rect.center(), 17.5, Stroke::new(2.0, p.text));
                            icons::paint(ui, rect.shrink(8.0), "check", 14.0, egui::Color32::WHITE);
                        }
                        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            picked = Some(backend::settings::NumberLook { color: i as u8, icon: icon.to_owned() });
                        }
                    }
                });
                ui.label(rich(tr("Icon", "Ikon"), 13.0, Weight::Medium, p.text_2));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                    for name in widgets::NUMBER_ICONS {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::click());
                        let tint = widgets::NUMBER_COLORS[color];
                        let fill = if name == icon { tint.gamma_multiply(0.22) } else if response.hovered() { p.raised } else { p.panel };
                        ui.painter().rect(rect, theme::R_MD, fill, Stroke::new(1.0, if name == icon { tint } else { p.border }), egui::StrokeKind::Inside);
                        icons::paint(ui, rect.shrink(9.0), name, 18.0, if name == icon { tint } else { p.text_2 });
                        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            picked = Some(backend::settings::NumberLook { color: color as u8, icon: name.to_owned() });
                        }
                    }
                });
                widgets::row(
                    ui,
                    32.0,
                    |_| {},
                    |ui| {
                        close = widgets::button(ui, Some("check"), tr("Done", "Selesai"), Kind::Primary).clicked();
                    },
                );
            });
        if let Some(look) = picked {
            let mut settings = (*self.settings).clone();
            settings.number_looks.insert(id.clone(), look.clone());
            if let Some(draft) = self.state.settings_form.draft.as_mut() {
                draft.number_looks.insert(id.clone(), look);
            }
            widgets::set_number_looks(&settings);
            self.settings = std::sync::Arc::new(settings.clone());
            self.backend.send(Command::SaveSettings(settings));
        }
        self.state.number_look = if close || modal.should_close() { None } else { Some(id) };
    }

    /// Makes `id` the main number, or (`None`) leaves none, and saves it (#138).
    fn set_main(&mut self, id: Option<String>) {
        let mut settings = (*self.settings).clone();
        settings.main_number = id.clone();
        if let Some(draft) = self.state.settings_form.draft.as_mut() {
            draft.main_number = id;
        }
        self.settings = std::sync::Arc::new(settings.clone());
        self.backend.send(Command::SaveSettings(settings));
    }

    /// Puts the Inbox on the main number once, as soon as its session has
    /// loaded (#138). A number picked by hand before that wins, and an
    /// unlinked main number leaves the Inbox on all numbers.
    fn open_on_main(&mut self) {
        if self.main_applied {
            return;
        }
        let Some(main) = self.settings.main_number.clone() else {
            self.main_applied = true;
            return;
        };
        if self.state.inbox_scope.is_some() {
            self.main_applied = true;
            return;
        }
        if let Some(session) = self.sessions.iter().find(|s| s.id == main) {
            if !matches!(session.status, Status::LoggedOut | Status::Banned(_)) {
                self.state.inbox_scope = Some(main);
            }
            self.main_applied = true;
        }
    }

    fn set_privacy(&mut self, change: impl FnOnce(&mut backend::settings::Privacy)) {
        let mut settings = (*self.settings).clone();
        change(&mut settings.privacy);
        if let Some(draft) = self.state.settings_form.draft.as_mut() {
            draft.privacy = settings.privacy.clone();
        }
        privacy::set(&settings.privacy);
        self.settings = std::sync::Arc::new(settings.clone());
        self.backend.send(Command::SaveSettings(settings));
    }

    /// "Anti-peek" in the top bar (Pego `v5/Privasi · Mode anti-intip`): a pill
    /// that is forest while on, opening the mode's options.
    fn privacy_pill(&mut self, ui: &mut Ui, compact: bool) {
        let p = theme::palette(ui.ctx());
        let on = self.settings.privacy.enabled;
        let label = if on { tr("Anti-peek on", "Anti-intip aktif") } else { tr("Anti-peek", "Anti-intip") };
        let (fill, fg, stroke) = if on { (p.primary, p.on_primary, Stroke::NONE) } else { (p.panel, p.text, Stroke::new(1.0, p.border)) };
        // Compact: the eye and the chevron only; the label moves to the hint.
        let galley = ui.painter().layout_no_wrap(if compact { String::new() } else { label.to_owned() }, fonts::ui(13.0, Weight::Medium), fg);
        let size = egui::vec2(if compact { 56.0 } else { galley.size().x + 58.0 }, 36.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
        ui.painter().rect(rect, 18.0, if on { fill } else { fill.lerp_to_gamma(p.raised, hover) }, stroke, egui::StrokeKind::Inside);
        icons::paint(ui, egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), egui::vec2(15.0, 15.0)), "eye-off", 15.0, fg);
        ui.painter().galley(egui::pos2(rect.left() + 34.0, rect.center().y - galley.size().y / 2.0), galley, fg);
        icons::paint(ui, egui::Rect::from_center_size(egui::pos2(rect.right() - 14.0, rect.center().y), egui::vec2(12.0, 12.0)), "chevron-down", 12.0, fg);
        let response = response.on_hover_text(tr("Anti-peek mode (Ctrl+Shift+P)", "Mode anti-intip (Ctrl+Shift+P)")).on_hover_cursor(CursorIcon::PointingHand);
        let mut privacy = self.settings.privacy.clone();
        // A menu closes on any click inside it; this one holds settings, so it
        // stays open until a click outside or Esc.
        egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(350.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(rich(tr("Anti-peek mode", "Mode anti-intip"), 14.0, Weight::SemiBold, p.text));
            ui.add(egui::Label::new(rich(
                tr("Keeps customer chats from people behind you when you work in busy places.", "Lindungi chat pelanggan dari orang di belakang Anda saat bekerja di tempat ramai."),
                12.0,
                Weight::Regular,
                p.muted,
            )).wrap());
            widgets::toggle(ui, &mut privacy.enabled, tr("On (Ctrl+Shift+P)", "Aktif (Ctrl+Shift+P)"));
            ui.separator();
            widgets::toggle(ui, &mut privacy.blur_chat, tr("Blur chat content: clear while the cursor is on a message", "Buramkan isi chat: terbaca saat kursor di atas pesan"));
            widgets::toggle(ui, &mut privacy.blur_previews, tr("Blur chat list previews (clear while the cursor is on the chat) and notification text", "Buramkan pratinjau daftar chat (terbaca saat kursor di chat itu) dan isi notifikasi"));
            widgets::toggle(ui, &mut privacy.hide_names, tr("Hide names and numbers, for presentations and recordings", "Sembunyikan nama & nomor, untuk presentasi atau rekaman"));
            watch_toggle(ui, &p, &mut privacy);
            // The choices sit in the popup itself: a drop-down would be a
            // popup inside this one, and a click on it counts as outside.
            ui.label(rich(tr("Lock when idle", "Kunci otomatis saat ditinggal"), 13.0, Weight::Regular, p.text));
            let labels = [tr("Never", "Tidak"), "1 min", "2 min", "5 min", "10 min"];
            let shown = lock_choice(privacy.lock_minutes);
            let mut choice = shown;
            widgets::tabs(ui, &labels, &mut choice);
            if choice != shown {
                privacy.lock_minutes = LOCK_MINUTES[choice];
            }
        });
        if privacy != self.settings.privacy {
            self.set_privacy(|p| *p = privacy);
        }
    }

    fn toggle_theme(&mut self, dark: bool) {
        let theme = if dark { "light" } else { "dark" };
        let mut settings = (*self.settings).clone();
        settings.theme = theme.to_owned();
        if let Some(draft) = self.state.settings_form.draft.as_mut() {
            draft.theme = theme.to_owned();
        }
        self.backend.send(Command::SaveSettings(settings));
    }

    fn add_number(&mut self) {
        let id = Backend::new_id();
        let name = crate::trf!("Number {}", "Nomor {}", self.sessions.len() + 1);
        self.backend.send(Command::Add { id: id.clone(), name, device: crate::backend::Device::default() });
        self.open_pairing(id);
    }

    fn open_pairing(&mut self, id: String) {
        self.pairing_id = Some(id);
        self.state.pair_with_code = false;
        self.state.pair_phone.clear();
        self.state.qr = None;
        self.screen = Screen::Pairing;
    }

    /// Leaving pairing before the phone confirmed drops the half-made session.
    fn cancel_pairing(&mut self) {
        if let Some(id) = self.pairing_id.take() {
            let linked = self
                .sessions
                .iter()
                .any(|s| s.id == id && s.status == Status::Connected);
            if !linked {
                self.backend.send(Command::Remove { id });
            }
        }
        self.screen = Screen::Nomor;
    }

    /// Opens a number's window, or brings it up when it is already open.
    fn open_window(&mut self, ctx: &egui::Context, id: String) {
        if let Some(open) = self.windows.iter().find(|w| w.id == id) {
            ctx.send_viewport_cmd_to(open.viewport(), egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(open.viewport(), egui::ViewportCommand::Focus);
        } else if self.sessions.iter().any(|s| s.id == id) {
            self.windows.push(NumberWindow::new(id));
        }
    }

    /// Draws the numbers' windows; returns the open chat of the one in focus.
    fn number_windows(&mut self, ctx: &egui::Context) -> Option<(String, String)> {
        let mut closed: Vec<String> = Vec::new();
        let mut focused_chat = None;
        let (locked, peeked) = (self.lock.locked, self.lock.peeked);
        for w in &mut self.windows {
            let Some(session) = self.sessions.iter().find(|s| s.id == w.id) else {
                closed.push(w.id.clone());
                continue;
            };
            w.state.settings = self.settings.clone();
            w.state.presence_now = self.state.presence_now.clone();
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("{} · warust", session.name))
                .with_app_id("warust")
                .with_inner_size([1000.0, 720.0])
                .with_min_inner_size([380.0, 480.0]);
            let one = std::slice::from_ref(session);
            let mut commands = Vec::new();
            ctx.show_viewport_immediate(w.viewport(), builder, |ui, _| {
                if ui.input(|i| i.viewport().close_requested()) {
                    closed.push(w.id.clone());
                }
                let p = theme::palette(ui.ctx());
                let phone = compact::is_compact(ui.ctx());
                egui::CentralPanel::default().frame(Frame::NONE.fill(p.bg)).show(ui, |ui| {
                    commands = if phone {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        screens::inbox::compact(ui, &mut w.state, one, &self.avatars, &self.downloads, self.settings.auto_stickers)
                    } else {
                        screens::inbox::show(ui, &mut w.state, one, &self.avatars, &self.downloads, self.settings.auto_stickers)
                    };
                });
                pdf::show(ui.ctx(), &mut w.state.pdf_view);
                movie_view::show(ui.ctx(), &mut w.state.video_view);
                photo_view::show(ui.ctx(), &mut w.state.photo_view, &w.state.full_avatars);
                if locked {
                    lock_cover(ui.ctx(), peeked);
                }
                commands.extend(forward::show(ui.ctx(), &mut w.state.forward, &self.sessions));
                let open = w.state.inbox_session.clone().zip(w.state.chat_selected.clone());
                let shown = open.as_ref().map(|(session, chat)| (session.as_str(), chat.as_str()));
                commands.extend(screens::inbox::voice_off_screen(ui.ctx(), &mut w.state, one, shown));
                commands.extend(screens::inbox::typing_signal(ui.ctx(), &mut w.state, open.clone()));
                if ui.input(|i| i.viewport().focused).unwrap_or(false) {
                    focused_chat = open;
                }
            });
            for command in commands {
                self.backend.send(command);
            }
        }
        self.windows.retain(|w| !closed.contains(&w.id));
        focused_chat
    }

    /// "Ganti nama": a new name for a number, used everywhere it shows.
    fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, mut draft)) = self.state.rename.clone() else {
            return;
        };
        let p = theme::palette(ctx);
        let checked = backend::clean_name(&draft);
        let mut close = false;
        let mut save = false;
        let modal = egui::Modal::new(egui::Id::unique("rename_number"))
            .frame(
                Frame::NONE
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(theme::R_LG)
                    .inner_margin(Margin::same(24)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.label(rich(tr("Rename number", "Ganti nama nomor"), 16.0, Weight::SemiBold, p.text));
                ui.add(
                    egui::Label::new(rich(
                        tr(
                            "Used in the Numbers list, Inbox, notifications, Overview and Log. The device name shown on the phone does not change.",
                            "Dipakai di daftar Nomor, Inbox, notifikasi, Ringkasan dan Log. Nama perangkat yang terlihat di HP tidak berubah.",
                        ),
                        13.0,
                        Weight::Regular,
                        p.text_2,
                    ))
                    .wrap(),
                );
                let field = ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .desired_width(f32::INFINITY)
                        .char_limit(backend::NAME_MAX + 10)
                        .hint_text(tr("e.g. Main support", "mis. CS Utama")),
                );
                if !field.has_focus() && !field.lost_focus() {
                    field.request_focus();
                }
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                if let Err(why) = &checked {
                    ui.label(rich(why, 12.0, Weight::Regular, p.red_text));
                }
                widgets::row(
                    ui,
                    32.0,
                    |_| {},
                    |ui| {
                        let ok = ui.add_enabled_ui(checked.is_ok(), |ui| widgets::button(ui, Some("check"), tr("Save", "Simpan"), Kind::Primary));
                        save |= ok.inner.clicked();
                        if widgets::button(ui, None, tr("Cancel", "Batal"), Kind::Secondary).clicked() {
                            close = true;
                        }
                    },
                );
            });
        if modal.should_close() {
            close = true;
        }
        if save {
            if let Ok(name) = backend::clean_name(&draft) {
                self.backend.send(Command::Rename { id: id.clone(), name });
                close = true;
            }
        }
        self.state.rename = if close { None } else { Some((id, draft)) };
    }

    /// A number's own chat wallpaper (#60): the Settings picker, plus
    /// "Same as the app".
    fn number_wallpaper_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.state.number_wallpaper.take() else {
            return;
        };
        let p = theme::palette(ctx);
        let dark = ctx.theme() == egui::Theme::Dark;
        let (mut close, mut save) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("number_wallpaper"))
            .frame(Frame::NONE.fill(p.panel).stroke(Stroke::new(1.0, p.border_strong)).corner_radius(theme::R_LG).inner_margin(Margin::same(24)))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 80.0).clamp(300.0, 640.0));
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.label(rich(&crate::trf!("Chat wallpaper · {}", "Wallpaper chat · {}", edit.name), 16.0, Weight::SemiBold, p.text));
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 220.0).max(200.0)).show(ui, |ui| {
                    let wall = screens::settings::WallDraft { key: &mut edit.wall.key, file: &mut edit.wall.file, dim: &mut edit.wall.dim };
                    screens::settings::wallpaper_picker(ui, &p, wall, &mut self.state.wallpaper_picking, dark, Some(&self.settings));
                });
                widgets::row(
                    ui,
                    32.0,
                    |_| {},
                    |ui| {
                        save = widgets::button(ui, Some("check"), tr("Save", "Simpan"), Kind::Primary).clicked();
                        if widgets::button(ui, None, tr("Cancel", "Batal"), Kind::Secondary).clicked() {
                            close = true;
                        }
                    },
                );
            });
        if modal.should_close() {
            close = true;
        }
        if save {
            let mut settings = (*self.settings).clone();
            let saved = settings.number_wallpapers.get(&edit.id).map(|w| w.file.clone()).unwrap_or_default();
            if edit.wall.key == "app" {
                // Nothing of its own left: its picture goes too.
                settings.number_wallpapers.remove(&edit.id);
                screens::settings::forget_custom(&edit.wall.file);
            } else {
                settings.number_wallpapers.insert(edit.id.clone(), edit.wall.clone());
            }
            if saved != edit.wall.file {
                screens::settings::forget_custom(&saved);
            }
            self.backend.send(Command::SaveSettings(settings));
            close = true;
        } else if close {
            // A picture chosen and then not saved is not kept.
            let saved = self.settings.number_wallpapers.get(&edit.id).map(|w| w.file.as_str()).unwrap_or_default();
            if edit.wall.file != saved {
                screens::settings::forget_custom(&edit.wall.file);
            }
        }
        if !close {
            self.state.number_wallpaper = Some(edit);
        }
    }

    fn relink(&mut self, id: String) {
        self.backend.send(Command::Relink { id: id.clone() });
        self.open_pairing(id);
    }

    fn confirm_remove(&mut self, ctx: &egui::Context) {
        let Some(id) = self.state.confirm_remove.clone() else {
            return;
        };
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            self.state.confirm_remove = None;
            return;
        };
        let answer = confirm(
            ctx,
            "confirm_remove",
            &crate::trf!("Remove {}?", "Hapus {}?", session.name),
            tr(
                "The link on the phone is removed and the session key on this computer is deleted. This cannot be undone; to use this number again you must pair it again.",
                "Tautan di HP dilepas dan kunci sesi di komputer ini dihapus. Tidak bisa dibatalkan; untuk memakai nomor ini lagi harus pairing ulang.",
            ),
            "x",
            tr("Remove", "Hapus"),
        );
        if answer == Some(true) {
            self.backend.send(Command::Remove { id });
        }
        if answer.is_some() {
            self.state.confirm_remove = None;
        }
    }

    fn confirm_relink(&mut self, ctx: &egui::Context) {
        let Some(id) = self.state.confirm_relink.clone() else {
            return;
        };
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            self.state.confirm_relink = None;
            return;
        };
        let answer = confirm(
            ctx,
            "confirm_relink",
            &crate::trf!("Relink {}?", "Tautkan ulang {}?", session.name),
            tr(
                "The link on the phone is removed and the session key on this computer is deleted, then a new QR is shown. This number stays disconnected until it is scanned again from the phone. Chats stay saved.",
                "Tautan di HP dilepas dan kunci sesi di komputer ini dihapus, lalu QR baru ditampilkan. Nomor ini terputus sampai dipindai lagi dari HP. Chat tetap tersimpan.",
            ),
            "rotate-cw",
            tr("Relink", "Tautkan ulang"),
        );
        if answer == Some(true) {
            self.relink(id);
        }
        if answer.is_some() {
            self.state.confirm_relink = None;
        }
    }
}

/// A yes/no dialog for a step that cannot be undone: `Some(true)` when
/// confirmed, `Some(false)` when cancelled, `None` while it is open.
fn confirm(ctx: &egui::Context, salt: &str, title: &str, text: &str, icon: &str, label: &str) -> Option<bool> {
    let p = theme::palette(ctx);
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique(salt))
        .frame(
            Frame::NONE
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(theme::R_LG)
                .inner_margin(Margin::same(24)),
        )
        .show(ctx, |ui| {
            ui.set_width(380.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(rich(title, 16.0, Weight::SemiBold, p.text));
            ui.add(egui::Label::new(rich(text, 13.0, Weight::Regular, p.text_2)).wrap());
            ui.add_space(4.0);
            widgets::row(
                ui,
                32.0,
                |_| {},
                |ui| {
                    if widgets::button(ui, Some(icon), label, Kind::Primary).clicked() {
                        answer = Some(true);
                    }
                    if widgets::button(ui, None, tr("Cancel", "Batal"), Kind::Secondary).clicked() {
                        answer = Some(false);
                    }
                },
            );
        });
    if answer.is_none() && modal.should_close() {
        answer = Some(false);
    }
    answer
}

/// "Detect someone behind you" (#57) and what the camera sees. Without
/// ffmpeg it cannot be turned on.
fn watch_toggle(ui: &mut Ui, p: &theme::Palette, privacy: &mut backend::settings::Privacy) {
    let camera = backend::video::ffmpeg().is_some();
    let label = tr("Detect someone behind you: the camera looks for a second face, on this computer only", "Deteksi orang di belakang Anda: kamera mencari wajah kedua, hanya di komputer ini");
    ui.add_enabled_ui(camera, |ui| widgets::toggle(ui, &mut privacy.watch_behind, label))
        .response
        .on_disabled_hover_text(tr("Needs ffmpeg installed", "Butuh ffmpeg terpasang"));
    if !privacy.watch_behind() {
        return;
    }
    let (text, color) = match backend::guard::status().0 {
        backend::guard::Watch::Off | backend::guard::Watch::Starting => (tr("Starting the camera…", "Menyalakan kamera…").to_owned(), p.muted),
        backend::guard::Watch::Paused => (tr("Paused while a call or photo uses the camera", "Jeda selama panggilan atau foto memakai kamera").to_owned(), p.muted),
        backend::guard::Watch::Faces(n) => (crate::trf!("Camera on · {n} face(s) seen", "Kamera aktif · {n} wajah terlihat"), p.text_2),
        backend::guard::Watch::Failed(why) => (why, p.red_text),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::show(ui, "camera", 13.0, color);
        ui.add(egui::Label::new(rich(&text, 12.0, Weight::Regular, color)).wrap());
    });
}

/// The anti-peek lock: the whole window frosted, with why and how to come back.
fn lock_cover(ctx: &egui::Context, peeked: bool) {
    let p = theme::palette(ctx);
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("privacy_lock"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(rect, 0.0, p.bg);
            let card = egui::Rect::from_center_size(rect.center(), egui::vec2(320.0, 150.0));
            ui.painter().rect(card, theme::R_LG, p.panel, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
            icons::paint(ui, egui::Rect::from_center_size(card.center() - egui::vec2(0.0, 36.0), egui::vec2(26.0, 26.0)), "lock", 26.0, p.primary);
            let title = egui::Rect::from_center_size(card.center() + egui::vec2(0.0, 6.0), egui::vec2(300.0, 22.0));
            let title_text = if peeked { tr("Someone may be looking at your screen", "Ada yang mungkin melihat layar Anda") } else { tr("warust is locked", "warust terkunci") };
            widgets::paint_text(ui, title, title_text, fonts::ui(16.0, Weight::SemiBold), p.text, egui::Align::Center);
            let hint = egui::Rect::from_center_size(card.center() + egui::vec2(0.0, 34.0), egui::vec2(300.0, 18.0));
            widgets::paint_text(ui, hint, tr("Click or press any key to continue", "Klik atau tekan tombol apa saja untuk lanjut"), fonts::ui(12.5, Weight::Regular), p.muted, egui::Align::Center);
        });
}

/// "API berjalan v0.1.0" in a white pill (Forest top bar).
/// The bar's buttons, full or (compact) as an icon with the label as a hint.
fn action_button(ui: &mut Ui, icon: &str, label: &str, kind: Kind, compact: bool) -> egui::Response {
    if compact {
        widgets::icon_button(ui, icon, 36.0, kind == Kind::Primary).on_hover_text(label)
    } else {
        widgets::button(ui, Some(icon), label, kind)
    }
}

/// Below this bar width the buttons on the right show only their icons;
/// below the second the wordmark goes too.
const TOPNAV_FULL: f32 = 1720.0;
const TOPNAV_TIGHT: f32 = 1380.0;

fn api_pill(ui: &mut Ui, p: &theme::Palette, api: &backend::ApiState, compact: bool) {
    let (color, label) = match api {
        backend::ApiState::Listening(_) => (p.green, tr("API running", "API berjalan")),
        backend::ApiState::Failed(_) => (p.red, tr("API failed", "API gagal")),
        backend::ApiState::Off => (p.gray, tr("API off", "API mati")),
    };
    Frame::NONE
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(18)
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            let ltr = egui::Layout::left_to_right(egui::Align::Center);
            // Compact: the dot and "API", the state and version in the hint.
            let shown = if compact { "API" } else { label };
            let width = ui.painter().layout_no_wrap(shown.to_owned(), fonts::ui(13.0, Weight::Medium), p.text).size().x;
            let extra = if compact { 18.0 } else { 74.0 };
            ui.allocate_ui_with_layout(egui::vec2(width + extra, 18.0), ltr, |ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, color);
                ui.label(rich(shown, 13.0, Weight::Medium, p.text));
                if !compact {
                    ui.label(rich_mono(concat!("v", env!("CARGO_PKG_VERSION")), 11.0, p.muted));
                }
            });
        });
}

fn gateway_status(ui: &mut Ui, sessions: &[SessionView]) {
    let p = theme::palette(ui.ctx());
    Frame::NONE
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::R_MD)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| gateway_rows(ui, &p, sessions));
}

fn gateway_rows(ui: &mut Ui, p: &theme::Palette, sessions: &[SessionView]) {
    let connected = sessions.iter().filter(|s| s.status == Status::Connected).count();
    ui.set_width(ui.available_width());
    widgets::row(
        ui,
        18.0,
        |ui| {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
            let color = if connected > 0 { p.green } else { p.gray };
            ui.painter().circle_filled(dot.center(), 3.5, color);
            let text = crate::trf!("{connected} of {} connected", "{connected} dari {} terhubung", sessions.len());
            ui.label(rich(&text, 12.0, Weight::Medium, p.text));
        },
        |_| {},
    );
    widgets::row(
        ui,
        18.0,
        |ui| {
            ui.label(rich(tr("REST API not active", "REST API belum aktif"), 12.0, Weight::Regular, p.muted));
        },
        |ui| {
            ui.label(rich_mono(concat!("v", env!("CARGO_PKG_VERSION")), 12.0, p.muted));
        },
    );
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // A change made in the very last frame is kept too.
        self.view.follow(self.screen, &mut self.state, |_| false);
        self.backend.close_notifications();
    }

    /// Runs before every `ui`, and on its own while the main window is hidden
    /// in the tray (#134): whatever has to work without a window is here.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        use std::sync::atomic::Ordering;
        // A clicked notification: bring the window up on that chat.
        let chat = self.backend.take_open_chat();
        let open = self.tray_asked.open.swap(false, Ordering::Relaxed);
        if chat.is_some() || open {
            self.show_window(ctx);
        }
        if let Some(chat) = chat {
            self.state.open_chat_requested = Some(chat);
        }
        if self.tray_asked.quit.swap(false, Ordering::Relaxed) {
            self.quit(ctx);
        }
        // Closing the main window hides it while the tray icon is there to
        // bring it back; without one, closing quits as before.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx);
        }
        if self.hidden {
            // `ui` does not run now, and the Dock or taskbar icon is where
            // new messages show.
            let snapshot = self.backend.snapshot();
            self.sessions = snapshot.sessions;
            self.state.call_now = snapshot.call;
            self.state.update = snapshot.update;
            self.show_unread(ctx);
            // A downloaded update still installs while warust is in the tray.
            self.install_when_idle(ctx);
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        // Drawn again, so on screen again: also after a click on the Dock
        // icon, which shows the window without going through `show_window`.
        // Not in the frame that closes it: `logic` has just hidden it.
        if !ui.ctx().input(|i| i.viewport().close_requested()) {
            self.hidden = false;
        }
        let snapshot = self.backend.snapshot();
        self.sessions = snapshot.sessions;
        // The main number first in every list (#138).
        if let Some(main) = snapshot.settings.main_number.as_deref() {
            self.sessions.sort_by_key(|s| s.id != main);
        }
        // With a main number the Inbox opens on it, not on the remembered
        // number (#138); the rest of the remembered view still comes back.
        let main_set = snapshot.settings.main_number.is_some();
        let sessions = &self.sessions;
        self.view.follow(self.screen, &mut self.state, |id| !main_set && sessions.iter().any(|s| s.id == id));
        self.avatars = snapshot.avatars;
        self.state.full_avatars = snapshot.full_avatars;
        self.downloads = snapshot.downloads;
        self.settings = snapshot.settings;
        self.open_on_main();
        let asked = self.state.set_main.take().or_else(|| self.windows.iter_mut().find_map(|w| w.state.set_main.take()));
        if let Some(id) = asked {
            self.set_main(id);
        }
        self.state.settings = self.settings.clone();
        self.state.rule_hits = snapshot.rule_hits;
        self.state.typing_now = snapshot.typing;
        self.state.presence_now = snapshot.presence;
        if self.presence_epoch != snapshot.presence_epoch {
            // A number went online (a window came to the front, the setting
            // was turned on, it reconnected). WhatsApp says who is online
            // only from then on: ask again for the open chats.
            self.presence_epoch = snapshot.presence_epoch;
            self.state.presence_watched.clear();
            for w in &mut self.windows {
                w.state.presence_watched.clear();
            }
        }
        self.state.members_now = snapshot.members;
        self.state.groups_now = snapshot.groups;
        self.state.call_now = snapshot.call;
        self.state.update = snapshot.update;
        if self.state.typing_now.values().any(backend::Typing::fresh) {
            // So a stale "mengetik…" disappears even when nothing else repaints.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        }
        self.webhooks = snapshot.webhooks;
        self.api = snapshot.api;
        crate::i18n::set(&self.settings.language);
        privacy::set(&self.settings.privacy);
        self.sync_tray(ui.ctx(), frame);
        if ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P))) {
            self.set_privacy(|p| p.enabled = !p.enabled);
        }
        widgets::set_wallpapers(&self.settings);
        widgets::set_number_looks(&self.settings);
        if self.applied_template.as_deref() != Some(self.settings.template.as_str()) {
            let template = theme::Template::from_key(&self.settings.template);
            // Forest opens on Ringkasan, like its design; Classic on Nomor.
            // Not when the start opened on the last run's screen (#128).
            let first = self.applied_template.is_none() && std::env::var_os("WARUST_SCREEN").is_none() && !self.screen_restored;
            if first && template == theme::Template::Forest && self.screen == Screen::Nomor {
                self.screen = Screen::Ringkasan;
            }
            theme::set_template(ui.ctx(), template);
            self.applied_template = Some(self.settings.template.clone());
            // The theme is applied again on top of the new template.
            self.applied_theme = None;
        }
        if theme::template().has_dark() && self.applied_theme.as_deref() != Some(self.settings.theme.as_str()) {
            ui.ctx().set_theme(if self.settings.theme == "light" {
                ThemePreference::Light
            } else {
                ThemePreference::Dark
            });
            self.applied_theme = Some(self.settings.theme.clone());
        }
        if self.screen == Screen::Pairing {
            let session = self.pairing_id.as_ref().and_then(|id| self.sessions.iter().find(|s| &s.id == id));
            match session {
                // Linked: back to the list, like the design's "pindah sendiri".
                Some(s) if s.status == Status::Connected => {
                    self.pairing_id = None;
                    self.screen = Screen::Nomor;
                }
                Some(_) => ui.ctx().request_repaint_after(std::time::Duration::from_secs(1)),
                None if self.pairing_id.is_none() => self.screen = Screen::Nomor,
                None => {}
            }
        }
        let compact = compact::is_compact(ui.ctx());
        if !compact && self.screen == Screen::More {
            self.screen = Screen::Ringkasan;
        }
        if compact {
            compact::title_bar(ui);
            let unread: u32 = self.sessions.iter().flat_map(|s| s.book.chats.iter()).map(|c| c.unread).sum();
            let unseen: usize = self.sessions.iter().map(|s| screens::status::unseen_posters(&s.book)).sum();
            // An open chat or status takes the whole window, like the phone.
            let immersive = match self.screen {
                Screen::Inbox => self.state.chat_selected.is_some(),
                Screen::Status => self.state.status_selected.is_some(),
                _ => false,
            };
            if !immersive {
                if let Some(screen) = compact::tab_bar(ui, self.screen, unread, unseen) {
                    self.screen = screen;
                }
            }
            let back = match self.screen {
                Screen::Pairing => Some((tr("Link number", "Tautkan nomor"), Screen::Nomor)),
                Screen::Log => Some((tr("Message log", "Log pesan"), Screen::More)),
                Screen::Integrations => Some(("Webhook & API", Screen::More)),
                Screen::Settings => Some((tr("Settings", "Pengaturan"), Screen::More)),
                _ => None,
            };
            if let Some((title, to)) = back {
                // Back leaves a pairing as it is; it can be reopened from Perangkat.
                if compact::back_bar(ui, title) {
                    self.screen = to;
                }
            }
        } else if theme::template().top_nav() {
            self.topnav(ui);
        } else {
            self.sidebar(ui);
            self.topbar(ui);
        }
        self.update_banner(ui);
        // Heights of compact::title_bar, topnav and the sidebar/topbar row.
        let bar = if compact { 40.0 } else if theme::template().top_nav() { 72.0 } else { 52.0 };
        // In full screen the title bar lives in AppKit's own toolbar window.
        if !window::is_fullscreen(ui.ctx()) {
            window::place_buttons(frame, bar);
        }
        let p = theme::palette(ui.ctx());
        let mut action = None;
        let mut pairing_action = None;
        let mut summary_action = None;
        let mut more_action = None;
        let api_on = matches!(self.api, backend::ApiState::Listening(_));
        let failed_hooks = self
            .webhooks
            .iter()
            .filter(|d| match &d.outcome {
                backend::webhook::Outcome::Failed(_) => true,
                backend::webhook::Outcome::Done(code) => !(200..300).contains(code),
                backend::webhook::Outcome::Pending => false,
            })
            .count();
        let margin = if compact { Margin::ZERO } else { theme::root_margin() };
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(p.bg).inner_margin(margin))
            .show(ui, |ui| {
                // Desktop pages in a phone window: text wraps instead of
                // widening the page past the window.
                if compact {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                }
                match self.screen {
                Screen::Ringkasan if compact => summary_action = screens::ringkasan::compact(ui, &self.sessions, &self.webhooks, self.settings.main_number.as_deref()),
                Screen::Ringkasan => summary_action = screens::ringkasan::show(ui, &mut self.state, &self.sessions, &self.webhooks),
                Screen::Nomor if compact => action = screens::devices::compact(ui, &self.sessions, self.settings.main_number.as_deref()),
                Screen::Nomor => action = screens::nomor::show(ui, &mut self.state, &self.sessions),
                Screen::More => more_action = compact::more(ui, api_on, failed_hooks),
                Screen::Status => {
                    let shown = if compact {
                        screens::status::compact(ui, &mut self.state, &self.sessions, &self.avatars, &self.downloads)
                    } else {
                        screens::status::page(ui, &mut self.state, &self.sessions, &self.avatars, &self.downloads)
                    };
                    for command in shown {
                        self.backend.send(command);
                    }
                }
                Screen::Inbox if compact => {
                    for command in screens::inbox::compact(ui, &mut self.state, &self.sessions, &self.avatars, &self.downloads, self.settings.auto_stickers) {
                        self.backend.send(command);
                    }
                }
                Screen::Pairing => {
                    let session = self
                        .pairing_id
                        .as_ref()
                        .and_then(|id| self.sessions.iter().find(|s| &s.id == id));
                    pairing_action = screens::pairing::show(ui, &mut self.state, session);
                }
                Screen::Inbox => {
                    for command in screens::inbox::show(ui, &mut self.state, &self.sessions, &self.avatars, &self.downloads, self.settings.auto_stickers) {
                        self.backend.send(command);
                    }
                }
                Screen::Log => screens::log::show(ui, &mut self.state, &self.sessions),
                Screen::Integrations => {
                    let shown = screens::integrations::show(ui, &mut self.state, &self.sessions, &self.settings, &self.api, &self.webhooks);
                    for command in shown {
                        self.backend.send(command);
                    }
                }
                Screen::Settings => {
                    for command in screens::settings::show(ui, &mut self.state, &self.settings) {
                        self.backend.send(command);
                    }
                }
                }
            });
        match more_action {
            Some(compact::More::Open(screen)) => self.screen = screen,
            Some(compact::More::ToggleTheme) => self.toggle_theme(ui.ctx().theme() == Theme::Dark),
            None => {}
        }
        if let Some((session, jid)) = self.state.open_chat_requested.take() {
            let chat = self.sessions.iter().find(|s| s.id == session).and_then(|s| s.book.chats.iter().find(|c| c.jid == jid));
            screens::inbox::open_chat(&mut self.state, &session, &jid, chat);
            self.state.chat_tab = screens::inbox::TAB_ALL;
            self.backend.send(Command::Seen { id: session, chat: jid });
            self.screen = Screen::Inbox;
        }
        if std::mem::take(&mut self.state.add_number_requested) {
            self.add_number();
        }
        match summary_action {
            Some(screens::ringkasan::Action::Nomor(a)) => action = Some(a),
            Some(screens::ringkasan::Action::OpenChat { session, jid }) => {
                let chat = self.sessions.iter().find(|s| s.id == session).and_then(|s| s.book.chats.iter().find(|c| c.jid == jid));
                screens::inbox::open_chat(&mut self.state, &session, &jid, chat);
                self.state.chat_tab = screens::inbox::TAB_ALL;
                self.backend.send(Command::Seen { id: session, chat: jid });
                self.screen = Screen::Inbox;
            }
            Some(screens::ringkasan::Action::OpenInbox) => self.screen = Screen::Inbox,
            Some(screens::ringkasan::Action::Devices) => self.screen = Screen::Nomor,
            None => {}
        }
        match action {
            Some(screens::nomor::Action::Add) => self.add_number(),
            Some(screens::nomor::Action::Open(id)) => self.open_pairing(id),
            Some(screens::nomor::Action::Relink(id)) => {
                // A linked number loses its keys, so that asks first.
                if self.sessions.iter().any(|s| s.id == id && s.phone.is_some()) {
                    self.state.confirm_relink = Some(id);
                } else {
                    self.relink(id);
                }
            }
            Some(screens::nomor::Action::Logout(id)) => self.backend.send(Command::Logout { id }),
            Some(screens::nomor::Action::Remove(id)) => self.state.confirm_remove = Some(id),
            Some(screens::nomor::Action::Window(id)) => self.state.open_window_requested = Some(id),
            Some(screens::nomor::Action::Rename(id)) => {
                let name = self.sessions.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();
                self.state.rename = Some((id, name));
            }
            Some(screens::nomor::Action::Main(id)) => self.state.set_main = Some(id),
            Some(screens::nomor::Action::Look(id)) => self.state.number_look = Some(id),
            Some(screens::nomor::Action::Wallpaper(id)) => {
                if let Some(session) = self.sessions.iter().find(|s| s.id == id) {
                    self.state.number_wallpaper = Some(NumberWallpaperEdit::new(&self.settings, session));
                }
            }
            None => {}
        }
        if let (Some(id), Some(a)) = (self.pairing_id.clone(), pairing_action) {
            match a {
                screens::pairing::Action::RequestCode(phone) => {
                    self.backend.send(Command::PairCode { id, phone })
                }
                screens::pairing::Action::Retry => self.backend.send(Command::Relink { id }),
                screens::pairing::Action::Save { name, device } => {
                    if let Some(name) = name {
                        self.backend.send(Command::Rename { id: id.clone(), name });
                    }
                    if let Some(device) = device {
                        self.backend.send(Command::SetDevice { id, device });
                    }
                }
            }
        }
        self.confirm_remove(ui.ctx());
        self.rename_dialog(ui.ctx());
        self.number_wallpaper_dialog(ui.ctx());
        self.number_look_dialog(ui.ctx());
        self.confirm_relink(ui.ctx());
        pdf::show(ui.ctx(), &mut self.state.pdf_view);
        movie_view::show(ui.ctx(), &mut self.state.video_view);
        photo_view::show(ui.ctx(), &mut self.state.photo_view, &self.state.full_avatars);
        // "Sedang mengetik" for the chat on screen while its reply is typed.
        let open_chat = if self.screen == Screen::Inbox {
            self.state.inbox_session.clone().zip(self.state.chat_selected.clone())
        } else {
            None
        };
        let open = open_chat.as_ref().map(|(session, chat)| (session.as_str(), chat.as_str()));
        for command in screens::inbox::voice_off_screen(ui.ctx(), &mut self.state, &self.sessions, open) {
            self.backend.send(command);
        }
        for command in screens::inbox::typing_signal(ui.ctx(), &mut self.state, open_chat) {
            self.backend.send(command);
        }
        for command in forward::show(ui.ctx(), &mut self.state.forward, &self.sessions) {
            self.backend.send(command);
        }
        if let Some(id) = self.state.open_window_requested.take() {
            self.open_window(ui.ctx(), id);
        }
        let in_window = self.number_windows(ui.ctx());
        self.show_unread(ui.ctx());
        // The numbers are online while one of warust's windows is in front and
        // not behind the lock cover (the backend checks the setting).
        let active = !self.lock.locked && ui.ctx().input(|i| i.raw.viewports.values().any(|v| v.focused == Some(true)));
        if self.active != Some(active) {
            self.active = Some(active);
            self.backend.send(Command::Active { active });
        }
        // The open chat in a focused window gets no notification.
        // In the tray no chat is in front of the user, so its messages notify.
        let focused = !self.hidden && ui.ctx().input(|i| i.viewport().focused).unwrap_or(true);
        let viewing = if focused && self.screen == Screen::Inbox {
            self.state.inbox_session.clone().zip(self.state.chat_selected.clone())
        } else {
            in_window
        };
        // A message that arrives in the chat in front of the user is read.
        // Behind the lock cover nothing is in front of them.
        let read = viewing.as_ref().filter(|_| !self.lock.locked);
        let open = read.and_then(|(session, jid)| self.sessions.iter().find(|s| &s.id == session)?.book.chats.iter().find(|c| &c.jid == jid));
        let newest = open.and_then(|c| c.messages.iter().rev().find(|m| !m.from_me)).map(|m| m.id.as_str());
        if self.seen.check(read.filter(|_| open.is_some()), newest, open.map_or(0, |c| c.unread))
            && let Some((session, jid)) = read
        {
            self.backend.send(Command::Seen { id: session.clone(), chat: jid.clone() });
        }
        if viewing != self.viewing {
            self.backend.set_viewing(viewing.clone());
            self.viewing = viewing;
        }
        // Last, so the resize edges sit above the panels.
        // A voice call ringing or in progress (#39).
        if let Some(call) = self.state.call_now.clone() {
            let compact = compact::is_compact(ui.ctx());
            for command in call::show(ui.ctx(), &mut self.state, &call, &self.sessions, compact) {
                self.backend.send(command);
            }
        }
        // Anti-peek auto-lock: covers everything until a click or a key.
        if let Some(wake) = self.lock.update(ui.ctx(), &self.settings.privacy) {
            ui.ctx().request_repaint_after(wake);
        }
        // Someone else in front of the camera locks too (#57).
        backend::guard::want(self.settings.privacy.watch_behind(), ui.ctx());
        self.lock.watch(self.settings.privacy.watch_behind() && backend::guard::status().1);
        if self.lock.locked {
            lock_cover(ui.ctx(), self.lock.peeked);
        }
        window::edges(ui);
    }
}

#[cfg(test)]
mod lock_choice_tests {
    use super::*;

    #[test]
    fn each_idle_time_has_its_choice() {
        for (choice, minutes) in LOCK_MINUTES.iter().enumerate() {
            assert_eq!(lock_choice(*minutes), choice);
        }
        // A value from an edited settings file shows as the default.
        assert_eq!(LOCK_MINUTES[lock_choice(3)], 2);
    }
}
