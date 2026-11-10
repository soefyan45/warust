//! Desktop UI (egui), following the Pego design: v4 layout, v4 dark palette
//! and v3 light palette. Every screen shows the real WhatsApp sessions.

mod compact;
mod emoji;
mod fonts;
mod icons;
mod markup;
mod pdf;
mod qr;
mod screens;
mod stickers;
mod theme;
mod time;
mod widgets;
mod window;

use egui::{CursorIcon, Frame, Margin, Panel, Stroke, Theme, ThemePreference, Ui};

use crate::backend::{self, Backend, Command, SessionView, Status};
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
    /// File waiting in the composer, sent with the text as its caption.
    pub attachment: Option<std::path::PathBuf>,
    /// PDF open in the in-app viewer.
    pub pdf_view: Option<pdf::Viewer>,
    /// The file dialog running on its own thread; `None` inside when cancelled.
    pub picking: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
    /// Result of the last CSV export, shown for a few seconds.
    pub export_note: Option<(String, std::time::Instant)>,
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
    pub qr: Option<qr::Qr>,
    /// Session waiting for the user to confirm its removal.
    pub confirm_remove: Option<String>,
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
}

pub struct App {
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
                widgets::row(
                    ui,
                    32.0,
                    |ui| {
                        ui.add_space(6.0);
                        let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::hover());
                        ui.painter().rect_filled(mark, theme::R_SM, p.primary);
                        widgets::paint_text(
                            ui,
                            mark,
                            "w",
                            fonts::ui(13.0, Weight::SemiBold),
                            p.on_primary,
                            egui::Align::Center,
                        );
                        ui.label(rich("warust", 14.0, Weight::SemiBold, p.text));
                    },
                    |ui| {
                        ui.add_space(6.0);
                        icons::show(ui, "chevrons-up-down", 16.0, p.muted);
                    },
                );
                ui.add_space(14.0);
                widgets::search(ui, &mut self.search, "Cari", ui.available_width(), true);
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
                    (Screen::Ringkasan, "gauge", "Ringkasan", None),
                    (Screen::Nomor, "smartphone", "Nomor", Some(total.as_str())),
                    (Screen::Inbox, "inbox", "Inbox", unread),
                    (Screen::Status, "circle-dashed", "Status", unseen),
                    (Screen::Log, "scroll-text", "Log pesan", None),
                ];
                for (screen, icon, label, count) in items {
                    let selected = self.screen == screen
                        || (screen == Screen::Nomor && self.screen == Screen::Pairing);
                    if widgets::nav_item(ui, icon, label, count, selected).clicked() {
                        self.screen = screen;
                    }
                }
                ui.add_space(20.0);
                widgets::section_label(ui, "Integrasi");
                ui.add_space(4.0);
                let api_on = matches!(self.api, backend::ApiState::Listening(_)).then_some("API");
                let items = [
                    (Screen::Integrations, "webhook", "Webhook & API", api_on),
                    (Screen::Settings, "settings", "Pengaturan", None),
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
            Screen::Ringkasan => ("Gateway", "Ringkasan"),
            Screen::Nomor => ("Gateway", "Nomor"),
            Screen::Pairing => ("Nomor", "Tautkan nomor baru"),
            Screen::Inbox => ("Gateway", "Inbox"),
            Screen::Status => ("Gateway", "Status"),
            Screen::More => ("Gateway", "Lainnya"),
            Screen::Log => ("Gateway", "Log pesan"),
            Screen::Integrations => ("Integrasi", "Webhook & API"),
            Screen::Settings => ("Integrasi", "Pengaturan"),
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
                        ui.add_space(6.0);
                        let (sep, _) = ui.allocate_exact_size(egui::vec2(1.0, 20.0), egui::Sense::hover());
                        ui.painter().rect_filled(sep, 0.0, p.border);
                        ui.add_space(6.0);
                        let dark = ui.ctx().theme() == Theme::Dark;
                        let toggle = widgets::icon_button(ui, if dark { "sun" } else { "moon" }, 32.0, false)
                            .on_hover_text(if dark { "Tema terang" } else { "Tema gelap" });
                        if toggle.clicked() {
                            self.toggle_theme(dark);
                        }
                        self.topbar_actions(ui);
                    });
                });
            });
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
                    let (logo, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                    ui.painter().circle_filled(logo.center(), 17.0, p.primary);
                    widgets::paint_text(ui, logo, "w", fonts::ui(16.0, Weight::SemiBold), p.accent, egui::Align::Center);
                    ui.label(rich("warust", 17.0, Weight::SemiBold, p.text));
                    ui.add_space(14.0);
                    let items = [
                        (Screen::Ringkasan, "Ringkasan"),
                        (Screen::Nomor, "Nomor"),
                        (Screen::Inbox, "Inbox"),
                        (Screen::Status, "Status"),
                        (Screen::Log, "Log pesan"),
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
                            .on_hover_text("Pengaturan");
                        if settings.clicked() {
                            self.screen = Screen::Settings;
                        }
                        api_pill(ui, &p, &self.api);
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
                if widgets::button(ui, Some("plus"), "Tambah nomor", Kind::Primary).clicked() {
                    self.add_number();
                }
                if widgets::button(ui, Some("book-open"), "Dokumentasi API", Kind::Secondary).clicked() {
                    self.screen = Screen::Integrations;
                    self.state.integrations_tab = 0;
                }
            }
            Screen::Pairing => {
                if widgets::button(ui, Some("x"), "Batal", Kind::Secondary).clicked() {
                    self.cancel_pairing();
                }
            }
            Screen::Ringkasan | Screen::Inbox | Screen::Status | Screen::Integrations | Screen::Settings | Screen::More => {}
            Screen::Log => {
                if widgets::button(ui, Some("download"), "Ekspor CSV", Kind::Secondary).clicked() {
                    let note = match screens::log::export_csv(&self.sessions) {
                        Ok((path, rows)) => format!("{rows} pesan diekspor ke {}", path.display()),
                        Err(e) => format!("Ekspor gagal: {e}"),
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
        let name = format!("Nomor {}", self.sessions.len() + 1);
        self.backend.send(Command::Add { id: id.clone(), name });
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
            &format!("Hapus {}?", session.name),
            "Tautan di HP dilepas dan kunci sesi di komputer ini dihapus. Tidak bisa dibatalkan; untuk memakai nomor ini lagi harus pairing ulang.",
            "x",
            "Hapus",
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
            &format!("Tautkan ulang {}?", session.name),
            "Tautan di HP dilepas dan kunci sesi di komputer ini dihapus, lalu QR baru ditampilkan. Nomor ini terputus sampai dipindai lagi dari HP. Chat tetap tersimpan.",
            "rotate-cw",
            "Tautkan ulang",
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
                    if widgets::button(ui, None, "Batal", Kind::Secondary).clicked() {
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

/// "API berjalan v0.1.0" in a white pill (Forest top bar).
fn api_pill(ui: &mut Ui, p: &theme::Palette, api: &backend::ApiState) {
    let (color, label) = match api {
        backend::ApiState::Listening(_) => (p.green, "API berjalan"),
        backend::ApiState::Failed(_) => (p.red, "API gagal"),
        backend::ApiState::Off => (p.gray, "API mati"),
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
            let text = format!("{connected} dari {} terhubung", sessions.len());
            ui.label(rich(&text, 12.0, Weight::Medium, p.text));
        },
        |_| {},
    );
    widgets::row(
        ui,
        18.0,
        |ui| {
            ui.label(rich("REST API belum aktif", 12.0, Weight::Regular, p.muted));
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

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // A clicked notification: bring the window up on that chat.
        if let Some(chat) = self.backend.take_open_chat() {
            self.state.open_chat_requested = Some(chat);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let snapshot = self.backend.snapshot();
        self.sessions = snapshot.sessions;
        self.avatars = snapshot.avatars;
        self.downloads = snapshot.downloads;
        self.settings = snapshot.settings;
        self.state.settings = self.settings.clone();
        self.state.rule_hits = snapshot.rule_hits;
        self.state.typing_now = snapshot.typing;
        if self.state.typing_now.values().any(backend::Typing::fresh) {
            // So a stale "mengetik…" disappears even when nothing else repaints.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        }
        self.webhooks = snapshot.webhooks;
        self.api = snapshot.api;
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
                Screen::Pairing => Some(("Tautkan nomor", Screen::Nomor)),
                Screen::Log => Some(("Log pesan", Screen::More)),
                Screen::Integrations => Some(("Webhook & API", Screen::More)),
                Screen::Settings => Some(("Pengaturan", Screen::More)),
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
            self.state.inbox_session = Some(session.clone());
            self.state.chat_selected = Some(jid.clone());
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
                self.state.inbox_session = Some(session.clone());
                self.state.chat_selected = Some(jid.clone());
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
            None => {}
        }
        if let (Some(id), Some(a)) = (self.pairing_id.clone(), pairing_action) {
            match a {
                screens::pairing::Action::RequestCode(phone) => {
                    self.backend.send(Command::PairCode { id, phone })
                }
                screens::pairing::Action::Retry => self.backend.send(Command::Relink { id }),
            }
        }
        self.confirm_remove(ui.ctx());
        self.confirm_relink(ui.ctx());
        pdf::show(ui.ctx(), &mut self.state.pdf_view);
        // "Sedang mengetik" for the chat on screen while its reply is typed.
        let open_chat = if self.screen == Screen::Inbox {
            self.state.inbox_session.clone().zip(self.state.chat_selected.clone())
        } else {
            None
        };
        for command in screens::inbox::typing_signal(ui.ctx(), &mut self.state, open_chat) {
            self.backend.send(command);
        }
        // The open chat in a focused window gets no notification.
        let focused = ui.ctx().input(|i| i.viewport().focused).unwrap_or(true);
        let viewing = if focused && self.screen == Screen::Inbox {
            self.state.inbox_session.clone().zip(self.state.chat_selected.clone())
        } else {
            None
        };
        if viewing != self.viewing {
            self.backend.set_viewing(viewing.clone());
            self.viewing = viewing;
        }
        // Last, so the resize edges sit above the panels.
        window::edges(ui);
    }
}
