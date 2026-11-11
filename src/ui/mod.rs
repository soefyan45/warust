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
    /// Session shown on the Pairing screen.
    pairing_id: Option<String>,
    /// The chat last reported to the notifier as on screen.
    viewing: Option<(String, String)>,
    /// Marks the chat on screen read as messages arrive in it.
    seen: seen::Watch,
    /// The unread count last shown on the taskbar icon and the title.
    shown_unread: Option<u32>,
    /// Numbers open in windows of their own (issue #28).
    windows: Vec<NumberWindow>,
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
        // Sessions live in the data folder (device stores in sessions/<id>.db).
        let backend = Backend::start(cc.egui_ctx.clone(), crate::platform::data_dir().to_path_buf());
        Self {
            screen,
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
            seen: seen::Watch::default(),
            lock: privacy::Lock::default(),
            windows: Vec::new(),
            shown_unread: None,
            state: State {
                contact_panel: true,
                // Dev aid: WARUST_DEVICES=hidden starts with the device panel hidden.
                devices_panel: std::env::var("WARUST_DEVICES").as_deref() != Ok("hidden"),
                // Dev aid: WARUST_SCREEN=pemicu opens the first trigger.
                integrations_tab: if std::env::var("WARUST_SCREEN").as_deref() == Ok("pemicu") { 2 } else { 0 },
                rule_selected: (std::env::var("WARUST_SCREEN").as_deref() == Ok("pemicu")).then_some(0),
                ..State::default()
            },
        }
    }

    fn sidebar(&mut self, ui: &mut Ui) {
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
                let total = self.sessions.len().to_string();
                let unread: u32 = self
                    .sessions
                    .iter()
                    .flat_map(|s| s.book.chats.iter())
                    .map(|c| c.unread)
                    .sum();
                let unread = if unread > 0 { unread.to_string() } else { String::new() };
                let unread = (!unread.is_empty()).then_some(unread.as_str());
                let unseen: usize = self.sessions.iter().map(|s| screens::status::unseen_posters(&s.book)).sum();
                let unseen = if unseen > 0 { unseen.to_string() } else { String::new() };
                let unseen = (!unseen.is_empty()).then_some(unseen.as_str());
                let items = [
                    (Screen::Ringkasan, "gauge", tr("Overview", "Ringkasan"), None),
                    (Screen::Nomor, "smartphone", tr("Numbers", "Nomor"), Some(total.as_str())),
                    (Screen::Inbox, "inbox", "Inbox", unread),
                    (Screen::Status, "circle-dashed", "Status", unseen),
                    (Screen::Log, "scroll-text", tr("Message log", "Log pesan"), None),
                ];
                for (screen, icon, label, count) in items {
                    let selected = self.screen == screen
                        || (screen == Screen::Nomor && self.screen == Screen::Pairing);
                    if widgets::nav_item(ui, icon, label, count, selected).clicked() {
                        self.screen = screen;
                    }
                }
                ui.add_space(20.0);
                widgets::section_label(ui, tr("Integrations", "Integrasi"));
                ui.add_space(4.0);
                let api_on = matches!(self.api, backend::ApiState::Listening(_)).then_some("API");
                let items = [
                    (Screen::Integrations, "webhook", "Webhook & API", api_on),
                    (Screen::Settings, "settings", tr("Settings", "Pengaturan"), None),
                ];
                for (screen, icon, label, badge) in items {
                    if widgets::nav_item(ui, icon, label, badge, self.screen == screen).clicked() {
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
                        self.privacy_pill(ui);
                        self.topbar_actions(ui);
                    });
                });
            });
    }

    /// "warust 0.1.6 is available" (#93): a slim bar under the top bar with
    /// the release notes, the installer for this OS, and "Later" (no banner
    /// for that version again).
    fn update_banner(&mut self, ui: &mut Ui) {
        let backend::update::UpdateState::Available(release) = &self.state.update else { return };
        if self.settings.updates.skipped == release.version {
            return;
        }
        let p = theme::palette(ui.ctx());
        let release = release.clone();
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
                        icons::show(ui, "download", 16.0, p.accent_text);
                        ui.add_space(4.0);
                        let text = crate::trf!(
                            "warust {} is available (you have {})",
                            "warust {} sudah tersedia (terpasang {})",
                            release.version,
                            backend::update::current()
                        );
                        ui.add(egui::Label::new(rich(&text, 13.0, Weight::SemiBold, p.accent_text)).truncate());
                    },
                    |ui| {
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
                ui.horizontal_centered(|ui| {
                    // Past the macOS window buttons (the bar's margin is 32).
                    ui.add_space((window::buttons_inset(ui.ctx()) - 32.0).max(0.0));
                    let (logo, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                    widgets::logo(ui, logo, p.primary, p.accent);
                    ui.label(rich("warust", 17.0, Weight::SemiBold, p.text));
                    ui.add_space(14.0);
                    let items = [
                        (Screen::Ringkasan, tr("Overview", "Ringkasan")),
                        (Screen::Nomor, tr("Numbers", "Nomor")),
                        (Screen::Inbox, "Inbox"),
                        (Screen::Status, "Status"),
                        (Screen::Log, tr("Message log", "Log pesan")),
                        (Screen::Integrations, "Webhook & API"),
                    ];
                    let current = if self.screen == Screen::Pairing { Screen::Nomor } else { self.screen };
                    let mut selected = items.iter().position(|(s, _)| *s == current).unwrap_or(usize::MAX);
                    let labels: Vec<&str> = items.iter().map(|(_, l)| *l).collect();
                    if widgets::pills(ui, &labels, &mut selected) {
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
                        api_pill(ui, &p, &self.api);
                        self.privacy_pill(ui);
                        self.topbar_actions(ui);
                    });
                });
            });
    }

    /// Right-to-left: the first button added ends up rightmost.
    fn topbar_actions(&mut self, ui: &mut Ui) {
        let p = theme::palette(ui.ctx());
        match self.screen {
            Screen::Nomor => {
                if widgets::button(ui, Some("plus"), tr("Add number", "Tambah nomor"), Kind::Primary).clicked() {
                    self.add_number();
                }
                if widgets::button(ui, Some("book-open"), tr("API docs", "Dokumentasi API"), Kind::Secondary).clicked() {
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
                if widgets::button(ui, Some("download"), tr("Export CSV", "Ekspor CSV"), Kind::Secondary).clicked() {
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

impl App {
    /// Saved, so the next start opens in the same theme.
    /// Changes the anti-peek options and saves them at once.
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
    fn privacy_pill(&mut self, ui: &mut Ui) {
        let p = theme::palette(ui.ctx());
        let on = self.settings.privacy.enabled;
        let label = if on { tr("Anti-peek on", "Anti-intip aktif") } else { tr("Anti-peek", "Anti-intip") };
        let (fill, fg, stroke) = if on { (p.primary, p.on_primary, Stroke::NONE) } else { (p.panel, p.text, Stroke::new(1.0, p.border)) };
        let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(13.0, Weight::Medium), fg);
        let size = egui::vec2(galley.size().x + 58.0, 36.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
        ui.painter().rect(rect, 18.0, if on { fill } else { fill.lerp_to_gamma(p.raised, hover) }, stroke, egui::StrokeKind::Inside);
        icons::paint(ui, egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), egui::vec2(15.0, 15.0)), "eye-off", 15.0, fg);
        ui.painter().galley(egui::pos2(rect.left() + 34.0, rect.center().y - galley.size().y / 2.0), galley, fg);
        icons::paint(ui, egui::Rect::from_center_size(egui::pos2(rect.right() - 14.0, rect.center().y), egui::vec2(12.0, 12.0)), "chevron-down", 12.0, fg);
        let response = response.on_hover_text(tr("Anti-peek mode (Ctrl+Shift+P)", "Mode anti-intip (Ctrl+Shift+P)")).on_hover_cursor(CursorIcon::PointingHand);
        let mut privacy = self.settings.privacy.clone();
        egui::Popup::menu(&response).show(|ui| {
            ui.set_width(330.0);
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
            widgets::toggle(ui, &mut privacy.blur_previews, tr("Blur chat list previews and notification text", "Buramkan pratinjau daftar chat dan isi notifikasi"));
            widgets::toggle(ui, &mut privacy.hide_names, tr("Hide names and numbers, for presentations and recordings", "Sembunyikan nama & nomor, untuk presentasi atau rekaman"));
            ui.horizontal(|ui| {
                ui.label(rich(tr("Lock when idle", "Kunci otomatis saat ditinggal"), 13.0, Weight::Regular, p.text));
                let options: [(u32, &str); 5] = [(0, tr("Never", "Tidak")), (1, "1 min"), (2, "2 min"), (5, "5 min"), (10, "10 min")];
                let shown = options.iter().find(|(m, _)| *m == privacy.lock_minutes).map(|(_, l)| *l).unwrap_or("2 min");
                egui::ComboBox::from_id_salt("privacy_lock").selected_text(shown).show_ui(ui, |ui| {
                    for (minutes, label) in options {
                        ui.selectable_value(&mut privacy.lock_minutes, minutes, label);
                    }
                });
            });
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
        let locked = self.lock.locked;
        for w in &mut self.windows {
            let Some(session) = self.sessions.iter().find(|s| s.id == w.id) else {
                closed.push(w.id.clone());
                continue;
            };
            w.state.settings = self.settings.clone();
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
                    lock_cover(ui.ctx());
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

/// The anti-peek lock: the whole window frosted, with how to come back.
fn lock_cover(ctx: &egui::Context) {
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
            widgets::paint_text(ui, title, tr("warust is locked", "warust terkunci"), fonts::ui(16.0, Weight::SemiBold), p.text, egui::Align::Center);
            let hint = egui::Rect::from_center_size(card.center() + egui::vec2(0.0, 34.0), egui::vec2(300.0, 18.0));
            widgets::paint_text(ui, hint, tr("Click or press any key to continue", "Klik atau tekan tombol apa saja untuk lanjut"), fonts::ui(12.5, Weight::Regular), p.muted, egui::Align::Center);
        });
}

/// "API berjalan v0.1.0" in a white pill (Forest top bar).
fn api_pill(ui: &mut Ui, p: &theme::Palette, api: &backend::ApiState) {
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
            let width = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(13.0, Weight::Medium), p.text).size().x;
            ui.allocate_ui_with_layout(egui::vec2(width + 74.0, 18.0), ltr, |ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, color);
                ui.label(rich(label, 13.0, Weight::Medium, p.text));
                ui.label(rich_mono(concat!("v", env!("CARGO_PKG_VERSION")), 11.0, p.muted));
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
            ui.label(rich_mono("v0.1.0", 12.0, p.muted));
        },
    );
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.backend.close_notifications();
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        // A clicked notification: bring the window up on that chat.
        if let Some(chat) = self.backend.take_open_chat() {
            self.state.open_chat_requested = Some(chat);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let snapshot = self.backend.snapshot();
        self.sessions = snapshot.sessions;
        self.avatars = snapshot.avatars;
        self.state.full_avatars = snapshot.full_avatars;
        self.downloads = snapshot.downloads;
        self.settings = snapshot.settings;
        self.state.settings = self.settings.clone();
        self.state.rule_hits = snapshot.rule_hits;
        self.state.typing_now = snapshot.typing;
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
        if ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P))) {
            self.set_privacy(|p| p.enabled = !p.enabled);
        }
        widgets::set_wallpaper(&self.settings.wallpaper, &self.settings.wallpaper_file, self.settings.wallpaper_dim);
        if self.applied_template.as_deref() != Some(self.settings.template.as_str()) {
            let template = theme::Template::from_key(&self.settings.template);
            // Forest opens on Ringkasan, like its design; Classic on Nomor.
            let first = self.applied_template.is_none() && std::env::var_os("WARUST_SCREEN").is_none();
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
                Screen::Ringkasan if compact => summary_action = screens::ringkasan::compact(ui, &self.sessions, &self.webhooks),
                Screen::Ringkasan => summary_action = screens::ringkasan::show(ui, &mut self.state, &self.sessions, &self.webhooks),
                Screen::Nomor if compact => action = screens::devices::compact(ui, &self.sessions),
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
        // Unread messages on the taskbar/Dock icon and in the window title.
        // Muted chats keep their own count but not the icon's, as on the phone.
        let now = time::now();
        let unread: u32 = self.sessions.iter().flat_map(|s| s.book.chats.iter()).filter(|c| !c.is_muted(now)).map(|c| c.unread).sum();
        if self.shown_unread != Some(unread) {
            crate::platform::set_unread_badge(unread);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(crate::platform::window_title(unread)));
            self.shown_unread = Some(unread);
        }
        // The open chat in a focused window gets no notification.
        let focused = ui.ctx().input(|i| i.viewport().focused).unwrap_or(true);
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
        if self.lock.locked {
            lock_cover(ui.ctx());
        }
        window::edges(ui);
    }
}
