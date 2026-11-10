//! Inbox screen (Pego `v4/Inbox`) on a real session: its chats (history sync
//! plus live messages), the conversation, the composer (sends through the
//! paced queue) and the contact panel.

use egui::{
    Align, CentralPanel, Color32, CursorIcon, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, Panel, Rect, ScrollArea,
    Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::backend::store::PinLength;
use crate::backend::{Book, Chat, ChatMsg, Command, Delivery, MAX_FILE_BYTES, Media, SessionView, Status};

/// Profile pictures by JID, as the backend shares them.
type Avatars = HashMap<String, Option<PathBuf>>;
/// Downloads by message id: `None` while running, else why it failed.
type Downloads = HashMap<String, Option<String>>;
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette, R_LG, R_MD};
use crate::ui::widgets::{self, Kind, rich, rich_mono};
use crate::ui::{ReplyTarget, State, UnreadAnchor, VoiceNote, VoiceStep, emoji, forward, icons, markup, mention, pdf, stickers, time};

/// A panel's id; a number's own window salts it with the number, so its
/// panel sizes and scroll positions are not the main window's.
fn panel_id(state: &State, name: &'static str) -> egui::Id {
    match &state.window {
        Some(id) => egui::Id::unique(name).with(id),
        None => egui::Id::unique(name),
    }
}

const ROW: f32 = 60.0;
/// Chat list tabs (Pego `v5/Inbox`): "Perlu balas", "Semua", "Selesai", "Diarsipkan".
pub const TAB_REPLY: usize = 0;
pub const TAB_ALL: usize = 1;
pub const TAB_DONE: usize = 2;
pub const TAB_ARCHIVED: usize = 3;
/// Older messages stay in the store; the view paints the latest ones.
const SHOWN: usize = 300;

pub fn show(
    ui: &mut Ui,
    state: &mut State,
    sessions: &[SessionView],
    avatars: &Avatars,
    downloads: &Downloads,
    auto_stickers: bool,
) -> Vec<Command> {
    let p = theme::palette(ui.ctx());
    let mut commands = Vec::new();
    if state.inbox_scope.as_ref().is_some_and(|id| !sessions.iter().any(|s| &s.id == id)) {
        state.inbox_scope = None;
    }
    // The number of the open chat: replies always go out from it. Without an
    // open chat: the number in scope, else the first connected, else the first.
    let valid = state.inbox_session.as_ref().is_some_and(|id| sessions.iter().any(|s| &s.id == id));
    let outside_scope = state.inbox_scope.is_some() && state.inbox_scope != state.inbox_session;
    if !valid || outside_scope {
        state.inbox_session = state
            .inbox_scope
            .clone()
            .or_else(|| sessions.iter().find(|s| s.status == Status::Connected).map(|s| s.id.clone()))
            .or_else(|| sessions.first().map(|s| s.id.clone()));
        state.chat_selected = None;
    }
    let Some(session) = state.inbox_session.as_ref().and_then(|id| sessions.iter().find(|s| &s.id == id)) else {
        CentralPanel::default()
            .frame(Frame::NONE.fill(p.bg))
            .show(ui, |ui| super::empty_state(ui, &p, "inbox", tr("No numbers yet", "Belum ada nomor"), tr("Link a number in the Numbers menu to start receiving messages.", "Tautkan nomor di menu Nomor untuk mulai menerima pesan.")));
        return commands;
    };
    let book = session.book.clone();
    // Dev aid: WARUST_CHAT=first opens the newest chat (for screenshots).
    if state.chat_selected.is_none() && std::env::var("WARUST_CHAT").as_deref() == Ok("first") {
        state.chat_selected = book.chats.first().map(|c| c.jid.clone());
    }
    let selected = state
        .chat_selected
        .as_ref()
        .and_then(|jid| book.chats.iter().find(|c| &c.jid == jid));

    let several = sessions.len() > 1;
    let forest = theme::template().top_nav();
    if state.devices_panel {
        Panel::left(panel_id(state, "devices"))
            .default_size(if forest { 250.0 } else { 230.0 })
            .size_range(210.0..=320.0)
            .resizable(true)
            .show_separator_line(!forest)
            .frame(theme::pane(ui.ctx(), p.panel, Margin { left: 10, right: 10, top: 0, bottom: 10 }))
            .show(ui, |ui| super::devices::show(ui, &p, state, sessions));
    }
    Panel::left(panel_id(state, "chat_list"))
        .default_size(if forest { 360.0 } else { 300.0 })
        .size_range(if forest { 330.0..=460.0 } else { 260.0..=420.0 })
        .resizable(true)
        .show_separator_line(!theme::template().top_nav())
        .frame(theme::pane(ui.ctx(), p.bg, Margin { left: 8, right: 8, top: 0, bottom: 8 }))
        .show(ui, |ui| {
            commands.extend(chat_list(ui, &p, state, sessions, avatars));
        });
    let Some(chat) = selected else {
        CentralPanel::default().frame(theme::pane(ui.ctx(), p.bg, Margin::ZERO)).show(ui, |ui| {
            let (title, desc) = if book.chats.is_empty() {
                (
                    tr("No chats yet", "Belum ada chat"),
                    tr(
                        "Incoming messages will show up here. WhatsApp only sends the old history during pairing.",
                        "Pesan masuk akan muncul di sini. Riwayat lama hanya dikirim WhatsApp saat pairing.",
                    ),
                )
            } else {
                (tr("Select a chat", "Pilih chat"), tr("Select a conversation on the left to read and reply.", "Pilih percakapan di sebelah kiri untuk membaca dan membalas."))
            };
            super::empty_state(ui, &p, "inbox", title, desc);
        });
        return commands;
    };

    // The contact panel only when the conversation keeps room to breathe.
    if state.contact_panel && ui.available_width() >= 980.0 {
        Panel::right(panel_id(state, "contact"))
            .default_size(300.0)
            .size_range(260.0..=400.0)
            .resizable(true)
            .show_separator_line(!theme::template().top_nav())
            .frame(theme::pane(ui.ctx(), p.panel, Margin::same(20)))
            .show(ui, |ui| contact(ui, &p, &book, chat, avatars, &super::webhook_label(state, &session.id)));
    }
    // Header, messages and composer: one card in Forest.
    theme::pane(ui.ctx(), p.bg, Margin::ZERO).show(ui, |ui| {
    ui.set_min_size(ui.available_size());
    let surface = theme::pane_bg(ui.ctx());
    let typing_now = typing_label(state, &session.id, &book, chat);
    Panel::top(panel_id(state, "conversation_header"))
        .exact_size(52.0)
        .resizable(false)
        .frame(Frame::NONE.fill(surface).corner_radius(theme::top_corners()).inner_margin(Margin { left: 24, right: 16, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            widgets::row(
                ui,
                52.0,
                |ui| {
                    ui.label(rich(&super::chat_title(&book, chat), 14.0, Weight::SemiBold, p.text));
                    if several {
                        // Which number the chat came in on, and so replies from.
                        ui.label(rich(tr("to", "masuk ke"), 12.0, Weight::Regular, p.muted));
                        let width = ui.painter().layout_no_wrap(super::devices::chip_label(session), fonts::ui(11.0, Weight::Medium), p.text_2).size().x + 24.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
                        number_chip(ui, &p, rect.min, session);
                    } else {
                        ui.label(rich_mono(&super::chat_subtitle(chat), 12.0, p.muted));
                    }
                    if let Some(typing) = &typing_now {
                        ui.label(rich(typing, 12.0, Weight::Medium, typing_color(&p)));
                    }
                },
                |ui| {
                    if widgets::icon_button(ui, "panel-right", 32.0, state.contact_panel).clicked() {
                        state.contact_panel = !state.contact_panel;
                    }
                    // A group only while a mention waits: marking it done clears the `@`.
                    if !chat.is_group() || chat.mention_pending() || chat.is_done() {
                        ui.add_space(4.0);
                        if let Some(done) = done_button(ui, chat) {
                            commands.push(Command::MarkDone { id: session.id.clone(), chat: chat.jid.clone(), done });
                        }
                        if chat.needs_reply() {
                            ui.add_space(4.0);
                            reply_badge(ui, &p);
                        }
                    }
                },
            );
        });
    Panel::bottom(panel_id(state, "composer"))
        .resizable(false)
        .frame(Frame::NONE.fill(surface).corner_radius(theme::bottom_corners()).inner_margin(Margin { left: 32, right: 32, top: 8, bottom: 20 }))
        .show_separator_line(false)
        .show(ui, |ui| {
            let to = super::chat_title(&book, chat);
            let (id, jid) = (session.id.clone(), chat.jid.clone());
            reply_bar(ui, &p, state, &id, &jid);
            match composer(ui, &p, state, &to, &jid, session, &book, several, false) {
                Some(Outgoing::Text(text, mentions)) => commands.push(text_command(state, id, jid, text, mentions)),
                Some(Outgoing::File(path, caption)) => commands.push(Command::SendFile { id, chat: jid, path, caption }),
                Some(Outgoing::Sticker(path)) => commands.push(Command::SendSticker { id, chat: jid, path }),
                Some(Outgoing::Voice { session, chat, path }) => commands.push(Command::SendVoice { id: session, chat, path }),
                None => {}
            }
        });
    CentralPanel::default()
        .frame(Frame::NONE)
        .show(ui, |ui| {
            widgets::wallpaper(ui, ui.max_rect());
            Frame::NONE.inner_margin(Margin { left: 32, right: 24, top: 0, bottom: 8 }).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                if several {
                    bound_banner(ui, &p, session);
                }
                if crate::ui::privacy::blur_chat() {
                    privacy_banner(ui, &p);
                }
                if older_bar(ui, &p, state, session, chat) {
                    state.older_asked = Some((chat.jid.clone(), std::time::Instant::now()));
                    commands.push(Command::LoadOlder { id: session.id.clone(), chat: chat.jid.clone() });
                }
                pinned_bar(ui, &p, chat);
                let mut want = Vec::new();
                let own = (session.own_number.as_deref(), session.own_lid.as_deref());
                let event = messages(ui, &p, &book, chat, downloads, avatars, &mut want, auto_stickers, typing_label(state, &session.id, &book, chat).as_deref(), own, &mut state.unread_anchor);
                ask_avatars(state, &mut commands, &session.id, want);
                if let Some(event) = event {
                    on_message_event(event, state, session, chat, &mut commands);
                }
            });
        });
    });
    commands
}

/// While typing, "sedang mengetik" is said again this often (the contact's
/// app hides it after a while).
const TYPING_RESEND: std::time::Duration = std::time::Duration::from_secs(8);
/// No key for this long counts as stopped.
const TYPING_IDLE: std::time::Duration = std::time::Duration::from_secs(5);

/// A group's members are asked again after this long without an answer.
const MEMBERS_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether to ask for a group's members now: none are known yet, and it was
/// never asked or the last ask is `MEMBERS_RETRY` old (the backend may have
/// dropped it).
fn members_due(state: &State, session: &str, chat: &str, now: std::time::Instant) -> bool {
    if state.members_now.contains_key(&crate::backend::Typing::key(session, chat)) {
        return false;
    }
    state
        .members_asked
        .get(&(session.to_owned(), chat.to_owned()))
        .is_none_or(|at| now.saturating_duration_since(*at) >= MEMBERS_RETRY)
}

/// Typing indicators for the chat on screen (`open`: session id, JID): starts
/// when the composer text changes, repeats while typing goes on, and stops
/// after a pause, when the text is cleared or sent, or when the chat is left.
pub fn typing_signal(ctx: &egui::Context, state: &mut State, open: Option<(String, String)>) -> Vec<Command> {
    let now = std::time::Instant::now();
    let mut commands = Vec::new();
    let changed = state.composer != state.typing_seen_text;
    if changed {
        state.typing_seen_text = state.composer.clone();
        state.typing_changed = Some(now);
    }
    // Opening a private chat asks to hear its typing, once per run; opening a
    // group asks for its members (the `@` picker), also once per run.
    if let Some((session, chat)) = &open {
        if chat.ends_with("@g.us") {
            if members_due(state, session, chat, now) {
                state.members_asked.insert((session.clone(), chat.clone()), now);
                commands.push(Command::GroupMembers { id: session.clone(), chat: chat.clone() });
            }
        } else if state.presence_watched.insert((session.clone(), chat.clone())) {
            commands.push(Command::WatchPresence { id: session.clone(), chat: chat.clone() });
        }
    }
    let empty = state.composer.trim().is_empty();
    if let Some((session, chat, _)) = state.typing.clone() {
        let here = open.as_ref().is_some_and(|(s, c)| *s == session && *c == chat);
        let idle = state.typing_changed.is_none_or(|t| now - t >= TYPING_IDLE);
        if !here || empty || idle {
            commands.push(Command::Typing { id: session, chat, typing: false });
            state.typing = None;
        }
    }
    if let Some((session, chat)) = open {
        if changed && !empty {
            let due = state.typing.as_ref().is_none_or(|(_, _, at)| now - *at >= TYPING_RESEND);
            if due {
                commands.push(Command::Typing { id: session.clone(), chat: chat.clone(), typing: true });
                state.typing = Some((session, chat, now));
            }
        }
    }
    // "Recording audio" for the chat a voice note is being recorded in, sent
    // again while it goes on, and stopped on pause, send or cancel.
    let recording = state.voice.as_ref().and_then(|v| match &v.step {
        VoiceStep::Recording(r) if !r.is_paused() && r.elapsed() > std::time::Duration::ZERO => Some((v.session.clone(), v.chat.clone())),
        _ => None,
    });
    match (&recording, state.recording_told.clone()) {
        (Some((session, chat)), told) => {
            let same = told.as_ref().is_some_and(|(s, c, _)| s == session && c == chat);
            if let Some((s, c, _)) = told.as_ref().filter(|_| !same) {
                commands.push(Command::RecordingAudio { id: s.clone(), chat: c.clone(), on: false });
            }
            if !same || told.is_some_and(|(_, _, at)| now - at >= TYPING_RESEND) {
                commands.push(Command::RecordingAudio { id: session.clone(), chat: chat.clone(), on: true });
                state.recording_told = Some((session.clone(), chat.clone(), now));
            }
        }
        (None, Some((session, chat, _))) => {
            commands.push(Command::RecordingAudio { id: session, chat, on: false });
            state.recording_told = None;
        }
        (None, None) => {}
    }
    if state.typing.is_some() {
        // Wakes up to notice the pause even when nothing else repaints.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
    commands
}

/// "mengetik…", "merekam suara…", or in a group "Andi mengetik…", while
/// the contact is at it.
fn typing_label(state: &State, session: &str, book: &Book, chat: &Chat) -> Option<String> {
    let typing = state.typing_now.get(&crate::backend::Typing::key(session, &chat.jid)).filter(|t| t.fresh())?;
    let what = if typing.recording { tr("recording voice…", "merekam suara…") } else { tr("typing…", "mengetik…") };
    Some(match &typing.who {
        Some(who) => format!("{} {what}", super::sender_label(book, who)),
        None => what.to_owned(),
    })
}

/// Green, like WhatsApp's "mengetik…".
fn typing_color(p: &Palette) -> Color32 {
    if theme::template().top_nav() { p.accent_text } else { p.green }
}

/// "Tandai selesai", or "Selesai" on a done chat (a click opens it again).
/// Returns the new state when clicked.
fn done_button(ui: &mut Ui, chat: &Chat) -> Option<bool> {
    let done = chat.is_done();
    let (icon, label, hint) = if done {
        ("check", tr("Done", "Selesai"), tr("Mark as needing a reply again", "Tandai perlu balas lagi"))
    } else {
        (
            "check",
            tr("Mark done", "Tandai selesai"),
            tr("Move to the Done tab until a new message arrives", "Pindahkan ke tab Selesai sampai ada pesan baru"),
        )
    };
    let kind = if done { Kind::Primary } else { Kind::Secondary };
    widgets::button(ui, Some(icon), label, kind).on_hover_text(hint).clicked().then_some(!done)
}

/// The soft orange "Belum dibalas" tag (Pego `v5/Inbox · Chat`).
fn reply_badge(ui: &mut Ui, p: &Palette) {
    let text = tr("Not replied", "Belum dibalas");
    let galley = ui.painter().layout_no_wrap(text.to_owned(), fonts::ui(12.0, Weight::SemiBold), theme::FOREST_ORANGE);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, 24.0), Sense::hover());
    let (fill, color) = if theme::template().top_nav() { (p.orange_soft, theme::FOREST_ORANGE) } else { (p.raised, p.amber) };
    ui.painter().rect_filled(rect, 12.0, fill);
    widgets::paint_text(ui, rect, text, fonts::ui(12.0, Weight::SemiBold), color, Align::Center);
}

/// "Load older messages from the phone". Returns true when clicked.
fn older_bar(ui: &mut Ui, p: &Palette, state: &State, session: &SessionView, chat: &Chat) -> bool {
    let asked = state
        .older_asked
        .as_ref()
        .is_some_and(|(jid, at)| jid == &chat.jid && at.elapsed().as_secs() < 30);
    let mut clicked = false;
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        if asked {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            ui.label(rich(
                tr(
                    "Asking the phone for history. The phone must be online; older messages show up as soon as they are sent.",
                    "Meminta riwayat ke HP. HP harus online; pesan lama muncul begitu dikirim.",
                ),
                12.0,
                Weight::Regular,
                p.muted,
            ));
        } else if session.status == Status::Connected {
            clicked = widgets::button(ui, Some("download"), tr("Load older messages from phone", "Muat pesan lama dari HP"), Kind::Secondary).clicked();
        }
    });
    clicked
}

/// A line of the chat list: a number's section header, or one chat.
enum Line<'a> {
    Header(&'a SessionView, usize),
    Chat(&'a SessionView, &'a Chat),
}

const HEADER: f32 = 42.0;
/// A chat row that also names the number it came in on.
const ROW_VIA: f32 = 82.0;

fn chat_list(
    ui: &mut Ui,
    p: &Palette,
    state: &mut State,
    sessions: &[SessionView],
    avatars: &Avatars,
) -> Vec<Command> {
    let mut commands = Vec::new();
    ui.spacing_mut().item_spacing.y = 2.0;
    let scoped: Vec<&SessionView> = match &state.inbox_scope {
        Some(id) => sessions.iter().filter(|s| &s.id == id).collect(),
        None => sessions.iter().collect(),
    };
    let several = sessions.len() > 1;
    let combined = state.inbox_scope.is_none() && several;
    let all_chats: Vec<(&SessionView, &Chat)> = scoped.iter().flat_map(|s| s.book.chats.iter().map(move |c| (*s, c))).collect();
    let archived = all_chats.iter().filter(|(_, c)| c.archived).count();
    // Archived chats stay out of the counts of the main tabs.
    let chats: Vec<(&SessionView, &Chat)> = all_chats.into_iter().filter(|(_, c)| !c.archived).collect();
    let unread = chats.iter().filter(|(_, c)| c.unread > 0).count();
    let title = match &state.inbox_scope {
        Some(_) => scoped.first().map(|s| s.name.clone()).unwrap_or_default(),
        None if state.inbox_grouped && several => tr("By number", "Per nomor").to_owned(),
        None => tr("All numbers", "Semua nomor").to_owned(),
    };
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(rich(&title, 17.0, Weight::SemiBold, p.text));
            let numbers = if combined { crate::trf!("{} numbers · ", "{} nomor · ", scoped.len()) } else { String::new() };
            ui.label(rich(
                &crate::trf!("{numbers}{} conversations · {unread} unread", "{numbers}{} percakapan · {unread} belum dibaca", chats.len()),
                12.0,
                Weight::Regular,
                p.muted,
            ));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.add_space(4.0);
            let hint = if state.devices_panel {
                tr("Hide the Devices panel", "Sembunyikan panel Perangkat")
            } else {
                tr("Show the Devices panel", "Tampilkan panel Perangkat")
            };
            if widgets::icon_button(ui, "panel-left", 32.0, state.devices_panel).on_hover_text(hint).clicked() {
                state.devices_panel = !state.devices_panel;
            }
        });
    });
    ui.add_space(6.0);
    if several {
        number_chips(ui, p, state, sessions);
        ui.add_space(6.0);
    }
    if combined {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let mut mode = usize::from(state.inbox_grouped);
            widgets::tabs(ui, &[tr("All numbers together", "Gabung semua nomor"), tr("Split by number", "Pisah per nomor")], &mut mode);
            state.inbox_grouped = mode == 1;
        });
        ui.add_space(4.0);
    }
    if state.chat_tab > TAB_ARCHIVED {
        state.chat_tab = TAB_REPLY;
    }
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let waiting = chats.iter().filter(|(_, c)| c.needs_reply()).count();
        let all = chats.len();
        let labels = [
            crate::trf!("Needs reply · {waiting}", "Perlu balas · {waiting}"),
            crate::trf!("All · {all}", "Semua · {all}"),
            tr("Done", "Selesai").to_owned(),
            crate::trf!("Archived · {archived}", "Diarsipkan · {archived}"),
        ];
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        widgets::tabs(ui, &labels, &mut state.chat_tab);
    });
    // Classic tabs sit on a rule; Forest's pills need none.
    if !theme::template().top_nav() {
        widgets::hline(ui, ui.max_rect(), ui.cursor().top() - 2.0, p.border);
    }
    ui.add_space(6.0);
    widgets::search(ui, &mut state.chat_search, tr("Search name or number", "Cari nama atau nomor"), ui.available_width(), false);
    ui.add_space(4.0);

    let lines = lines_for(state, &scoped, state.chat_tab, combined && state.inbox_grouped);
    // Rows name their number when every number is mixed, and in "Pisah per
    // nomor" when the same contact also chats with another number.
    let mixed = combined && !state.inbox_grouped;
    let shared = if combined { shared_jids(&scoped) } else { std::collections::HashSet::new() };
    let via = |chat: &Chat| mixed || shared.contains(chat.jid.as_str());
    let height = |line: &Line| match line {
        Line::Header(..) => HEADER,
        Line::Chat(_, chat) if via(chat) => ROW_VIA,
        Line::Chat(..) => ROW,
    };
    let total: f32 = lines.iter().map(|l| height(l) + 2.0).sum();
    if lines.is_empty() {
        ui.add_space(20.0);
        let empty = match state.chat_tab {
            TAB_REPLY => tr("All chats have been replied to.", "Semua chat sudah dibalas."),
            TAB_DONE => tr("No chats marked done yet.", "Belum ada chat yang ditandai selesai."),
            TAB_ARCHIVED => tr("No archived chats yet.", "Belum ada chat yang diarsipkan."),
            _ => tr("No chats here.", "Tidak ada chat di sini."),
        };
        ui.vertical_centered(|ui| ui.label(rich(empty, 13.0, Weight::Regular, p.muted)));
        return commands;
    }
    // Only the lines on screen are drawn, so hundreds of chats stay cheap.
    ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, viewport| {
        ui.set_height(total);
        let origin = ui.max_rect().min;
        let width = ui.available_width();
        let mut y = 0.0;
        let reveal = std::mem::take(&mut state.reveal_chat);
        // The section the top of the view is in, and where its next one starts.
        let mut current: Option<(&SessionView, usize)> = None;
        let mut next_header = f32::INFINITY;
        for line in &lines {
            let h = height(line);
            let visible = y + h >= viewport.min.y && y <= viewport.max.y;
            let rect = Rect::from_min_size(pos2(origin.x, origin.y + y), vec2(width, h));
            if let Line::Header(s, count) = line {
                if y <= viewport.min.y {
                    current = Some((s, *count));
                } else if next_header.is_infinite() {
                    next_header = y;
                }
            }
            if reveal
                && let Line::Chat(s, chat) = line
                && state.inbox_session.as_deref() == Some(s.id.as_str())
                && state.chat_selected.as_deref() == Some(chat.jid.as_str())
            {
                ui.scroll_to_rect(rect, Some(Align::Center));
            }
            y += h + 2.0;
            if !visible {
                continue;
            }
            match line {
                Line::Header(s, count) => {
                    let folded = state.folded.contains(&s.id);
                    header_action(state, s, section_header(ui, p, rect, s, *count, folded, ""));
                }
                Line::Chat(s, chat) => {
                    // Pictures are asked only for rows on screen; the backend paces them.
                    if !avatars.contains_key(&chat.jid) && state.avatar_asked.insert(chat.jid.clone()) {
                        commands.push(Command::Avatar { id: s.id.clone(), jid: chat.jid.clone() });
                    }
                    let picture = avatars.get(&chat.jid).and_then(|p| p.as_deref());
                    let selected = state.inbox_session.as_deref() == Some(s.id.as_str())
                        && state.chat_selected.as_deref() == Some(chat.jid.as_str());
                    let title = super::chat_title(&s.book, chat);
                    let via_number = via(chat).then_some(*s);
                    let row = chat_row(ui, p, &s.book, rect, &title, chat, picture, selected, &s.id, via_number, typing_label(state, &s.id, &s.book, chat).as_deref());
                    chat_menu(&row, s, chat, &mut commands);
                    if row.clicked() {
                        open_chat(state, &s.id, &chat.jid, Some(chat));
                        if chat.unread > 0 {
                            commands.push(Command::Seen { id: s.id.clone(), chat: chat.jid.clone() });
                        }
                    }
                }
            }
        }
        // "Pisah per nomor": the band of the section on screen stays at the top,
        // pushed up by the next one, so it is always clear whose chats these are.
        if let Some((s, count)) = current {
            let top = (origin.y + viewport.min.y).min(origin.y + next_header - HEADER - 2.0);
            let rect = Rect::from_min_size(pos2(origin.x, top), vec2(width, HEADER));
            ui.painter().rect_filled(rect, 0.0, p.bg);
            let folded = state.folded.contains(&s.id);
            header_action(state, s, section_header(ui, p, rect, s, count, folded, "sticky"));
        }
    });
    commands
}

/// The rows of the chat list: filtered by the search and `tab` (`TAB_*`),
/// newest first, or one section per number when `grouped`.
fn lines_for<'a>(state: &State, scoped: &[&'a SessionView], tab: usize, grouped: bool) -> Vec<Line<'a>> {
    let query = state.chat_search.trim().to_lowercase();
    let keep = |s: &SessionView, c: &Chat| {
        let tab = match tab {
            TAB_ARCHIVED => c.archived,
            _ if c.archived => false,
            TAB_REPLY => c.needs_reply(),
            TAB_DONE => c.is_done(),
            _ => true,
        };
        tab && (query.is_empty() || super::chat_title(&s.book, c).to_lowercase().contains(&query) || c.jid.contains(&query))
    };
    let mut lines: Vec<Line> = Vec::new();
    if grouped {
        // Connected numbers first, the ones that cannot reply last.
        let mut order: Vec<&SessionView> = scoped.to_vec();
        order.sort_by_key(|s| s.status != Status::Connected);
        for s in order {
            let mine: Vec<&Chat> = s.book.chats.iter().filter(|c| keep(s, c)).collect();
            lines.push(Line::Header(s, mine.len()));
            if !state.folded.contains(&s.id) {
                lines.extend(mine.into_iter().map(|c| Line::Chat(s, c)));
            }
        }
    } else {
        let mut mine: Vec<(&SessionView, &Chat)> =
            scoped.iter().flat_map(|s| s.book.chats.iter().map(move |c| (*s, c))).filter(|(s, c)| keep(s, c)).collect();
        mine.sort_by_key(|(_, c)| std::cmp::Reverse(c.last_ts()));
        lines.extend(mine.into_iter().map(|(s, c)| Line::Chat(s, c)));
    }
    lines
}

/// Opens a chat and remembers where its unread messages start; call before
/// `Seen` clears the count. Re-opening the chat already open keeps the anchor.
pub fn open_chat(state: &mut State, session: &str, jid: &str, chat: Option<&Chat>) {
    let already = state.inbox_session.as_deref() == Some(session) && state.chat_selected.as_deref() == Some(jid);
    state.inbox_session = Some(session.to_owned());
    state.chat_selected = Some(jid.to_owned());
    if already {
        return;
    }
    // A chat of another number than the filter: the filter follows it, so the
    // chat is not dropped, and its section opens so the row can be seen.
    if state.inbox_scope.as_deref().is_some_and(|scope| scope != session) {
        state.inbox_scope = Some(session.to_owned());
    }
    state.folded.remove(session);
    state.reveal_chat = true;
    state.unread_anchor = chat.and_then(|chat| {
        let incoming: Vec<&ChatMsg> = chat.messages.iter().filter(|m| !m.from_me).collect();
        let count = (chat.unread as usize).min(incoming.len());
        let first = incoming.get(incoming.len().checked_sub(count)?)?;
        (count > 0).then(|| UnreadAnchor { jid: jid.to_owned(), count, first_id: first.id.clone(), pending: true })
    });
}

/// Right-click menu of a chat row (wide and phone lists): read state and archive.
fn chat_menu(row: &egui::Response, session: &SessionView, chat: &Chat, commands: &mut Vec<Command>) {
    row.context_menu(|ui| {
        ui.set_min_width(180.0);
        let (id, jid) = (session.id.clone(), chat.jid.clone());
        if chat.unread > 0 {
            if ui.button(tr("Mark as read", "Tandai sudah dibaca")).clicked() {
                commands.push(Command::Seen { id: id.clone(), chat: jid.clone() });
            }
        } else if ui.button(tr("Mark as unread", "Tandai belum dibaca")).clicked() {
            commands.push(Command::MarkUnread { id: id.clone(), chat: jid.clone() });
        }
        if ui.button(if chat.archived { tr("Unarchive", "Keluarkan dari arsip") } else { tr("Archive", "Arsipkan") }).clicked() {
            commands.push(Command::Archive { id, chat: jid, archived: !chat.archived });
        }
    });
}

fn header_action(state: &mut State, session: &SessionView, click: HeaderClick) {
    match click {
        HeaderClick::Fold => {
            if !state.folded.remove(&session.id) {
                state.folded.insert(session.id.clone());
            }
        }
        HeaderClick::Window => state.open_window_requested = Some(session.id.clone()),
        HeaderClick::None => {}
    }
}

/// JIDs that chat with more than one of these numbers.
fn shared_jids<'a>(sessions: &[&'a SessionView]) -> std::collections::HashSet<&'a str> {
    let mut seen = std::collections::HashSet::new();
    let mut shared = std::collections::HashSet::new();
    for s in sessions {
        for c in &s.book.chats {
            if !seen.insert(c.jid.as_str()) {
                shared.insert(c.jid.as_str());
            }
        }
    }
    shared
}

/// The number filter above the chat list: "Semua · 144", "● Nomor 1 · 8222 ·
/// 135", ... always on screen, with or without the device panel. Right-click a
/// number for its own window.
fn number_chips(ui: &mut Ui, p: &Palette, state: &mut State, sessions: &[SessionView]) {
    ScrollArea::horizontal()
        .id_salt("number_chips")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.spacing_mut().item_spacing.x = 6.0;
                let total: usize = sessions.iter().map(|s| s.book.chats.iter().filter(|c| !c.archived).count()).sum();
                if number_chip_button(ui, p, &crate::trf!("All · {total}", "Semua · {total}"), None, state.inbox_scope.is_none()).clicked() {
                    state.inbox_scope = None;
                }
                for s in sessions {
                    let count = s.book.chats.iter().filter(|c| !c.archived).count();
                    let label = format!("{} · {count}", super::devices::chip_label(s));
                    let dot = super::devices::status_dot(p, &s.status);
                    let active = state.inbox_scope.as_deref() == Some(s.id.as_str());
                    let chip = number_chip_button(ui, p, &label, Some(dot), active);
                    if chip.clicked() {
                        state.inbox_scope = Some(s.id.clone());
                    }
                    chip.context_menu(|ui| {
                        if ui.button(tr("Open in new window", "Buka di jendela baru")).clicked() {
                            state.open_window_requested = Some(s.id.clone());
                        }
                    });
                }
                ui.add_space(6.0);
            });
        });
}

/// One chip of `number_chips`: a pill, filled when active, with the
/// number's status dot.
fn number_chip_button(ui: &mut Ui, p: &Palette, label: &str, dot: Option<Color32>, active: bool) -> egui::Response {
    let forest = theme::template().top_nav();
    let (fill, color) = match (active, forest) {
        (true, true) => (p.accent, p.primary),
        (true, false) => (p.primary, p.on_primary),
        (false, _) => (p.panel, p.text_2),
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(12.5, if active { Weight::SemiBold } else { Weight::Medium }), color);
    let lead = if dot.is_some() { 22.0 } else { 12.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + lead + 12.0, 28.0), Sense::click());
    ui.painter().rect_filled(rect, 14.0, if response.hovered() && !active { p.raised } else { fill });
    if !active {
        ui.painter().rect_stroke(rect, 14.0, Stroke::new(1.0, p.border), StrokeKind::Inside);
    }
    if let Some(dot) = dot {
        ui.painter().circle_filled(pos2(rect.left() + 13.0, rect.center().y), 3.5, dot);
    }
    ui.painter().galley(pos2(rect.left() + lead, rect.center().y - galley.size().y / 2.0), galley, color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// What a click on a section band asks for.
enum HeaderClick {
    None,
    Fold,
    Window,
}

/// "▾ ● CS Utama · 4410 ........ 3 chat ↗" in "Pisah per nomor", red when the
/// number cannot reply. A click folds or opens the section; ↗ opens the number
/// in a window of its own. `salt` tells the sticky copy apart from the band.
fn section_header(ui: &Ui, p: &Palette, rect: Rect, session: &SessionView, count: usize, folded: bool, salt: &str) -> HeaderClick {
    let band = rect.shrink2(vec2(0.0, 4.0));
    let down = session.status != Status::Connected;
    let fold = ui.interact(band, ui.scope_id().with(("fold_section", &session.id, salt)), Sense::click());
    let fill = match (down, fold.hovered()) {
        (true, _) => p.red_soft,
        (false, true) => p.selected,
        (false, false) => p.raised,
    };
    ui.painter().rect_filled(band, theme::r_inner(), fill);
    let chevron = Rect::from_center_size(pos2(band.left() + 14.0, band.center().y), vec2(14.0, 14.0));
    icons::paint(ui, chevron, if folded { "chevron-right" } else { "chevron-down" }, 14.0, p.muted);
    let dot = super::devices::status_dot(p, &session.status);
    ui.painter().circle_filled(pos2(band.left() + 30.0, band.center().y), 3.5, dot);
    let name = Rect::from_min_max(pos2(band.left() + 42.0, band.top()), pos2(band.right() - 120.0, band.bottom()));
    widgets::paint_text(ui, name, &super::devices::chip_label(session), fonts::ui(13.0, Weight::SemiBold), p.text, Align::Min);
    let right = Rect::from_min_max(pos2(band.right() - 120.0, band.top()), pos2(band.right() - 38.0, band.bottom()));
    let (label, color) = if down {
        let (_, _, label, _) = super::nomor::describe(p, &session.status);
        (label, p.red)
    } else {
        let label = if count == 1 { tr("1 chat", "1 chat").to_owned() } else { crate::trf!("{count} chats", "{count} chat") };
        (label, p.muted)
    };
    widgets::paint_text(ui, right, &label, fonts::ui(12.0, Weight::Medium), color, Align::Max);
    let button = Rect::from_center_size(pos2(band.right() - 20.0, band.center().y), vec2(24.0, 24.0));
    let window = ui.interact(button, ui.scope_id().with(("open_window", &session.id, salt)), Sense::click());
    if window.hovered() {
        ui.painter().rect_filled(button, theme::R_SM, p.panel);
    }
    icons::paint(ui, button, "arrow-up-right", 14.0, if window.hovered() { p.text } else { p.muted });
    if window.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tr("Open in new window", "Buka di jendela baru")).clicked() {
        HeaderClick::Window
    } else if fold.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        HeaderClick::Fold
    } else {
        HeaderClick::None
    }
}

/// Anti-peek's strip over the thread (Pego `v5/Privasi`): what is going on,
/// and the shortcut that turns it off.
fn privacy_banner(ui: &mut Ui, p: &Palette) {
    ui.add_space(8.0);
    Frame::NONE.fill(p.accent_soft).corner_radius(R_MD).inner_margin(Margin::symmetric(12, 7)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::show(ui, "shield", 14.0, p.accent_text);
            ui.label(rich(
                tr("Messages are blurred. Move the cursor over a message to read it.", "Isi pesan diburamkan. Arahkan kursor ke pesan untuk membacanya."),
                12.5,
                Weight::Medium,
                p.accent_text,
            ));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(rich_mono("Ctrl+Shift+P", 11.0, p.text_2));
            });
        });
    });
}

/// "Percakapan ini terikat ke CS Utama…" above the messages, with several numbers.
fn bound_banner(ui: &mut Ui, p: &Palette, session: &SessionView) {
    let phone = session.phone.clone().unwrap_or_else(|| tr("this number", "nomor ini").to_owned());
    let text = crate::trf!(
        "This conversation is bound to {}. All replies are sent from {phone}.",
        "Percakapan ini terikat ke {}. Semua balasan dikirim dari {phone}.",
        session.name
    );
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        Frame::NONE
            .fill(p.accent_soft)
            .corner_radius(12)
            .inner_margin(Margin::symmetric(12, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    icons::show(ui, "lock", 12.0, p.accent_text);
                    ui.label(rich(&text, 12.0, Weight::Regular, p.accent_text));
                });
            });
    });
}

/// A small "● CS Utama · 4410" chip; returns its width.
pub fn number_chip(ui: &Ui, p: &Palette, at: egui::Pos2, session: &SessionView) -> f32 {
    let label = super::devices::chip_label(session);
    let galley = ui.painter().layout_no_wrap(label, fonts::ui(11.0, Weight::Medium), p.text_2);
    let rect = Rect::from_min_size(at, vec2(galley.size().x + 24.0, 20.0));
    ui.painter().rect_filled(rect, 10.0, p.raised);
    ui.painter().circle_filled(pos2(rect.left() + 9.0, rect.center().y), 3.0, super::devices::status_dot(p, &session.status));
    ui.painter().galley(pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0), galley, p.text_2);
    rect.width()
}

#[allow(clippy::too_many_arguments)]
fn chat_row(
    ui: &Ui,
    p: &Palette,
    book: &Book,
    row: Rect,
    title: &str,
    chat: &Chat,
    picture: Option<&Path>,
    selected: bool,
    session: &str,
    via: Option<&SessionView>,
    typing: Option<&str>,
) -> egui::Response {
    // The same contact can chat with several numbers, so the id holds both.
    let resp = ui.interact(row, ui.scope_id().with(("chat_row", session, &chat.jid)), Sense::click());
    let picture = picture.filter(|_| !crate::ui::privacy::hide_names());
    if selected {
        ui.painter().rect_filled(row, theme::r_inner(), p.selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(row, theme::r_inner(), p.raised);
    }
    let tile = Rect::from_min_size(pos2(row.left() + 12.0, row.top() + 12.0), vec2(32.0, 32.0));
    let initials = if chat.is_group() { "GR".to_owned() } else { super::initials(title) };
    widgets::paint_avatar(ui, tile, &initials, picture, selected);

    let left = tile.right() + 12.0;
    let right = row.right() - 12.0;
    let top = Rect::from_min_max(pos2(left, row.top() + 10.0), pos2(right, row.top() + 30.0));
    let bottom = Rect::from_min_max(pos2(left, row.top() + 30.0), pos2(right, row.top() + 48.0));
    let when = time::short(chat.last_ts());
    let time_galley = ui
        .painter()
        .layout_no_wrap(when, fonts::ui(12.0, Weight::Regular), p.muted);
    let time_w = time_galley.size().x;
    ui.painter()
        .galley(pos2(right - time_w, top.center().y - time_galley.size().y / 2.0), time_galley, p.muted);
    let name = Rect::from_min_max(top.min, pos2(right - time_w - 8.0, top.bottom()));
    widgets::paint_text(ui, name, title, fonts::ui(14.0, Weight::SemiBold), p.text, Align::Min);
    let at = chat.mention_pending();
    let reserve = if chat.unread > 0 { 30.0 } else { 0.0 } + if at { 26.0 } else { 0.0 };
    let preview_rect = Rect::from_min_max(bottom.min, pos2(right - reserve, bottom.bottom()));
    paint_preview(ui, p, book, preview_rect, chat, 13.0, p.muted, typing);
    if at {
        // "@": a mention is waiting; sits before the unread count.
        let x = right - if chat.unread > 0 { 26.0 + 18.0 } else { 18.0 };
        let badge = Rect::from_min_size(pos2(x, bottom.center().y - 9.0), vec2(18.0, 18.0));
        ui.painter().rect_filled(badge, 9.0, p.primary);
        widgets::paint_text(ui, badge, "@", fonts::ui(11.0, Weight::SemiBold), p.on_primary, Align::Center);
    }
    if chat.unread > 0 {
        let badge = Rect::from_min_size(pos2(right - 22.0, bottom.center().y - 9.0), vec2(22.0, 18.0));
        ui.painter().rect_filled(badge, 9.0, p.primary);
        let count = if chat.unread > 99 { "99+".to_owned() } else { chat.unread.to_string() };
        widgets::paint_text(ui, badge, &count, fonts::ui(11.0, Weight::SemiBold), p.on_primary, Align::Center);
    }
    if let Some(session) = via {
        let label = ui.painter().layout_no_wrap(tr("to", "masuk ke").to_owned(), fonts::ui(11.0, Weight::Regular), p.muted);
        let label_w = label.size().x;
        let y = row.top() + 54.0;
        ui.painter().galley(pos2(left, y + 10.0 - label.size().y / 2.0), label, p.muted);
        number_chip(ui, p, pos2(left + label_w + 6.0, y), session);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// WhatsApp's blue "read" ticks.
const READ_BLUE: Color32 = Color32::from_rgb(0x53, 0xBD, 0xEB);

/// Asks for member pictures not asked yet this run (the backend paces them).
fn ask_avatars(state: &mut State, commands: &mut Vec<Command>, session: &str, jids: Vec<String>) {
    for jid in jids {
        if state.avatar_asked.insert(jid.clone()) {
            commands.push(Command::Avatar { id: session.to_owned(), jid });
        }
    }
}

/// Name colours for group members, readable on dark and on light bubbles.
const MEMBER_COLORS: [(u32, u32); 12] = [
    (0xFF8A80, 0xC62828),
    (0xFFB74D, 0xB35C00),
    (0xE6C84F, 0x8A6D00),
    (0x81C784, 0x2E7D32),
    (0x4DD0C4, 0x00796B),
    (0x64B5F6, 0x1565C0),
    (0x9FA8FF, 0x3F51B5),
    (0xCE93D8, 0x8E24AA),
    (0xF48FB1, 0xC2185B),
    (0xAED581, 0x558B2F),
    (0x4FC3F7, 0x0277BD),
    (0xFF9E80, 0xD84315),
];

/// A member's colour: the same for the same JID, every run and every start.
fn member_color(jid: &str, dark: bool) -> Color32 {
    // FNV-1a: stable across runs, unlike `DefaultHasher`.
    let hash = jid.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
    let (on_dark, on_light) = MEMBER_COLORS[hash as usize % MEMBER_COLORS.len()];
    let rgb = if dark { on_dark } else { on_light };
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// A member's picture beside their bubbles, else their initials in their colour.
fn paint_member_face(ui: &Ui, rect: Rect, initials: &str, picture: Option<&std::path::Path>, color: Color32) {
    let picture = picture.filter(|_| !crate::ui::privacy::hide_names());
    if picture.is_some() {
        widgets::paint_avatar(ui, rect, initials, picture, false);
        return;
    }
    ui.painter().rect_filled(rect, R_MD, color.gamma_multiply(0.18));
    widgets::paint_text(ui, rect, initials, fonts::ui(rect.height() * 0.38, Weight::SemiBold), color, Align::Center);
}

fn is_dark(c: Color32) -> bool {
    (c.r() as u32 * 299 + c.g() as u32 * 587 + c.b() as u32 * 114) / 1000 < 128
}

/// WhatsApp's delivery mark: a clock while queued or sending, ! when it
/// failed, one tick sent, two delivered, two blue read. `grey` is the colour
/// of the plain marks on whatever they sit on (`dark` = a dark bubble).
fn tick(p: &Palette, status: &Delivery, grey: Color32, dark: bool) -> Option<(&'static str, Color32)> {
    let failed = if dark { Color32::from_rgb(0xFF, 0x9B, 0x8F) } else { p.red_text };
    let blue = if dark || !is_dark(p.bg) { READ_BLUE } else { Color32::from_rgb(0x1E, 0x88, 0xE5) };
    match status {
        Delivery::Received => None,
        Delivery::Queued => Some(("clock", grey)),
        Delivery::Failed(_) => Some(("circle-alert", failed)),
        Delivery::Sent => Some(("check", grey)),
        Delivery::Delivered => Some(("check-check", grey)),
        Delivery::Read => Some(("check-check", blue)),
    }
}

/// The last message in a chat row; ours start with their tick, like WhatsApp.
#[allow(clippy::too_many_arguments)]
fn paint_preview(ui: &Ui, p: &Palette, book: &Book, rect: Rect, chat: &Chat, size: f32, color: Color32, typing: Option<&str>) {
    // Anti-peek: a bar of the same width for every chat, so nothing is read off it.
    if crate::ui::privacy::blur_previews() {
        crate::ui::privacy::frost_line(&ui.painter().with_clip_rect(rect), rect, 0.72, color.gamma_multiply(0.35));
        return;
    }
    if let Some(typing) = typing {
        widgets::paint_text(ui, rect, typing, fonts::ui(size, Weight::Medium), typing_color(p), Align::Min);
        return;
    }
    let Some(m) = chat.messages.last() else {
        return;
    };
    let mut rect = rect;
    if m.from_me {
        if let Some((icon, tint)) = tick(p, &m.status, p.muted, false) {
            let side = size + 1.0;
            icons::paint(ui, Rect::from_min_size(pos2(rect.left(), rect.center().y - side / 2.0), vec2(side, side)), icon, side, tint);
            rect.min.x += side + 4.0;
        }
    }
    let text = book.with_mention_names(&m.text, &m.mentions);
    widgets::paint_text(ui, rect, &text.replace('\n', " "), fonts::ui(size, Weight::Regular), color, Align::Min);
}

pub fn delivery_label(status: &Delivery) -> &'static str {
    match status {
        Delivery::Received => "",
        Delivery::Queued => tr("Queued", "Antre"),
        Delivery::Sent => tr("Sent", "Terkirim"),
        Delivery::Delivered => tr("Delivered", "Diterima"),
        Delivery::Read => tr("Read", "Dibaca"),
        Delivery::Failed(_) => tr("Failed", "Gagal"),
    }
}

/// The conversation with day dividers, sitting at the bottom and scrolling up.
/// How a message's media shows in its bubble.
enum MediaBox {
    /// A picture (the downloaded photo, else the inline thumbnail) of this size.
    Picture(PathBuf, egui::Vec2),
    /// A row with icon, name, size and the download state.
    Card,
    /// A sticker, drawn without a bubble; `None` until downloaded.
    Sticker(Option<PathBuf>),
    /// A document with a first-page preview above its card: the rendered PDF
    /// once downloaded, else the thumbnail that came with the message.
    Document { pdf: Option<PathBuf>, thumb: Option<PathBuf> },
    /// A voice note: play button, waveform and length (Pego `FM/VN/Bubble`).
    Voice(f32),
}

/// What a click on media asks for.
enum MediaClick {
    Download(String),
    /// A downloaded PDF and its title, for the in-app viewer.
    OpenPdf(PathBuf, String),
}

const PREVIEW_HEIGHT: f32 = 140.0;

const CARD: egui::Vec2 = egui::Vec2::new(260.0, 48.0);
const STICKER: egui::Vec2 = egui::Vec2::new(150.0, 150.0);

impl MediaBox {
    fn size(&self) -> egui::Vec2 {
        match self {
            MediaBox::Picture(_, size) => *size,
            MediaBox::Card => CARD,
            MediaBox::Sticker(_) => STICKER,
            MediaBox::Document { .. } => vec2(CARD.x, PREVIEW_HEIGHT + CARD.y),
            MediaBox::Voice(width) => vec2(*width, VOICE_HEIGHT),
        }
    }
}

fn media_box(media: &Media, wrap: f32) -> MediaBox {
    if media.kind == "sticker" {
        return MediaBox::Sticker(media.file.clone());
    }
    if media.kind == "audio" && media.voice {
        return MediaBox::Voice(wrap.clamp(200.0, 280.0));
    }
    if media.kind == "document" {
        let pdf = media.file.clone().filter(|f| pdf::is_pdf(f));
        return if pdf.is_some() || media.thumb.is_some() {
            MediaBox::Document { pdf, thumb: media.thumb.clone() }
        } else {
            MediaBox::Card
        };
    }
    let picture_file = media
        .file
        .clone()
        .filter(|f| media.kind == "image" && matches!(
            f.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
            Some("jpg" | "jpeg" | "png")
        ));
    let source = match media.kind.as_str() {
        "image" => picture_file.or_else(|| media.thumb.clone()),
        "video" => media.thumb.clone(),
        _ => None,
    };
    let Some(source) = source else {
        return MediaBox::Card;
    };
    let width = wrap.min(260.0);
    let height = match (media.width, media.height) {
        (Some(w), Some(h)) if w > 0 => width * h as f32 / w as f32,
        _ => width * 0.75,
    };
    MediaBox::Picture(source, vec2(width, height.clamp(80.0, 320.0)))
}

/// The bubble text without the `[Gambar]`-style label the media already shows.
fn caption(m: &ChatMsg) -> String {
    match m.media.as_ref().map(|media| media.kind.as_str()) {
        None => m.text.clone(),
        Some("image" | "video") if m.text.starts_with('[') => {
            m.text.split_once(']').map(|(_, rest)| rest.trim().to_owned()).unwrap_or_default()
        }
        Some("image" | "video") => m.text.clone(),
        Some(_) => String::new(),
    }
}

/// The mentions of a message as `@<number>` tokens with the name to show;
/// `own_number` and `own_lid` mark a mention of this account.
fn mention_refs(book: &Book, m: &ChatMsg, own_number: Option<&str>, own_lid: Option<&str>) -> Vec<markup::MentionRef> {
    m.mentions
        .iter()
        .map(|jid| {
            let token = mention::user_part(jid).to_owned();
            let me = m.mentions_me && (own_number == Some(token.as_str()) || own_lid == Some(token.as_str()));
            markup::MentionRef { name: super::sender_label(book, jid), token, me }
        })
        .collect()
}

fn media_title(media: &Media) -> String {
    if let Some(name) = media.name.as_ref().filter(|n| !n.is_empty()) {
        return name.clone();
    }
    match media.kind.as_str() {
        "image" => tr("Photo", "Foto"),
        "video" => "Video",
        "sticker" => tr("Sticker", "Stiker"),
        "audio" => tr("Voice message", "Pesan suara"),
        _ => tr("Document", "Dokumen"),
    }
    .to_owned()
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// What clicking the media does next.
#[derive(Clone, Copy, PartialEq)]
enum MediaState {
    Open,
    Downloading,
    Retry,
    Download,
    Unavailable,
}

impl MediaState {
    /// The short label shown on the media.
    fn label(self) -> &'static str {
        match self {
            MediaState::Open => tr("Open", "Buka"),
            MediaState::Downloading => tr("Downloading…", "Mengunduh…"),
            MediaState::Retry => tr("Failed, retry", "Gagal, ulangi"),
            MediaState::Download => tr("Download", "Unduh"),
            MediaState::Unavailable => tr("Unavailable", "Tidak tersedia"),
        }
    }
}

fn media_action(m: &ChatMsg, media: &Media, downloads: &Downloads) -> MediaState {
    if media.file.is_some() {
        return MediaState::Open;
    }
    match downloads.get(&m.id) {
        Some(None) => MediaState::Downloading,
        Some(Some(_)) => MediaState::Retry,
        None if media.downloadable() => MediaState::Download,
        None => MediaState::Unavailable,
    }
}

const VOICE_HEIGHT: f32 = 46.0;
/// Bars in a voice bubble: the waveform's peaks in this many equal slices,
/// so every bubble is as wide whatever its length.
const VOICE_BARS: usize = 34;

/// Length and waveform of voice notes read from their files, for notes
/// whose record does not carry them (sent before they were stored).
fn voice_file_info(ctx: &egui::Context, path: &Path) -> Option<(u32, Vec<u8>)> {
    use std::sync::{Mutex, OnceLock};
    static INFO: OnceLock<Mutex<HashMap<PathBuf, Option<(u32, Vec<u8>)>>>> = OnceLock::new();
    let info = INFO.get_or_init(Mutex::default);
    let mut map = info.lock().ok()?;
    if let Some(found) = map.get(path) {
        return found.clone();
    }
    map.insert(path.to_owned(), None);
    let (path, ctx) = (path.to_owned(), ctx.clone());
    // Decoding takes a moment for long notes: off the UI thread.
    std::thread::spawn(move || {
        let found = std::fs::read(&path).ok().and_then(|bytes| crate::backend::voice::inspect(&bytes).ok());
        if let Some(info) = INFO.get().and_then(|i| i.lock().ok()).as_mut() {
            info.insert(path, found);
        }
        ctx.request_repaint();
    });
    None
}

/// `VOICE_BARS` heights from 0 to 1: the peak of each slice of `wave`.
fn voice_bars(wave: &[u8]) -> Vec<f32> {
    if wave.is_empty() {
        return vec![0.15; VOICE_BARS];
    }
    (0..VOICE_BARS)
        .map(|bar| {
            let from = bar * wave.len() / VOICE_BARS;
            let to = ((bar + 1) * wave.len() / VOICE_BARS).max(from + 1).min(wave.len());
            f32::from(wave[from..to].iter().copied().max().unwrap_or(0)) / 100.0
        })
        .collect()
}

/// `m:ss`.
fn clock_of(seconds: f32) -> String {
    let s = seconds.max(0.0).round() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A voice note's bubble (Pego `FM/VN/Bubble/Out` and `/In`): ▶/⏸, the
/// waveform filling as it plays with a knob to seek, the length (or the time
/// played), and the speed chip while it is loaded. A note not downloaded yet
/// downloads on ▶.
fn paint_voice(ui: &Ui, p: &Palette, rect: Rect, m: &ChatMsg, media: &Media, downloads: &Downloads) -> Option<MediaClick> {
    use crate::backend::player;
    let forest = theme::template().top_nav();
    let (button, glyph, played, rest, sub) = if m.from_me {
        let button = if forest { p.accent } else { p.on_primary };
        (button, p.primary, button, p.on_primary_muted, p.on_primary_muted)
    } else {
        (p.primary, p.on_primary, p.primary, p.border_strong, p.muted)
    };
    let file = media.file.as_deref();
    let status = player::status(&m.id);
    let loading = player::is_loading(&m.id) || matches!(downloads.get(&m.id), Some(None));
    let from_file = if media.waveform.is_empty() || media.seconds.is_none() { file.and_then(|f| voice_file_info(ui.ctx(), f)) } else { None };
    let wave = if media.waveform.is_empty() { from_file.as_ref().map(|(_, w)| w.as_slice()).unwrap_or_default() } else { &media.waveform };
    let duration = status.map(|s| s.duration).or(media.seconds.map(|s| s as f32)).or(from_file.as_ref().map(|(s, _)| *s as f32));
    let fraction = status.map(|s| (s.position / s.duration.max(0.01)).clamp(0.0, 1.0)).unwrap_or(0.0);
    if status.is_some_and(|s| s.playing) || loading {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }

    // ▶ / ⏸
    let center = pos2(rect.left() + 20.0, rect.top() + 20.0);
    let play = Rect::from_center_size(center, vec2(38.0, 38.0));
    let play_response = ui.interact(play, ui.scope_id().with(("voice_play", &m.id)), Sense::click());
    let radius = if play_response.hovered() { 19.0 } else { 18.0 };
    ui.painter().circle_filled(center, radius, button);
    let icon = if loading {
        "loader-circle"
    } else if status.is_some_and(|s| s.playing) {
        "pause"
    } else if file.is_none() {
        "download"
    } else {
        "play"
    };
    icons::paint(ui, Rect::from_center_size(center, vec2(18.0, 18.0)), icon, 18.0, glyph);

    // Waveform; played bars full, the rest muted, the knob where it is.
    let bars_rect = Rect::from_min_max(pos2(rect.left() + 48.0, rect.top() + 6.0), pos2(rect.right() - 4.0, rect.top() + 30.0));
    let bars = voice_bars(wave);
    let step = bars_rect.width() / VOICE_BARS as f32;
    let width = (step * 0.55).clamp(2.0, 3.0);
    for (i, value) in bars.iter().enumerate() {
        let x = bars_rect.left() + step * (i as f32 + 0.5);
        let height = 3.0 + value.powf(0.7) * (bars_rect.height() - 3.0);
        let done = status.is_some() && (i as f32 + 0.5) / VOICE_BARS as f32 <= fraction;
        let bar = Rect::from_center_size(pos2(x, bars_rect.center().y), vec2(width, height));
        ui.painter().rect_filled(bar, width / 2.0, if done { played } else { rest });
    }
    let seek = ui.interact(bars_rect.expand2(vec2(0.0, 4.0)), ui.scope_id().with(("voice_seek", &m.id)), Sense::click_and_drag());
    if status.is_some() {
        let knob = pos2(bars_rect.left() + bars_rect.width() * fraction, bars_rect.center().y);
        ui.painter().circle_filled(knob, if seek.hovered() || seek.dragged() { 7.0 } else { 6.0 }, played);
    }

    // Length (or the time played), the mic on received notes, the speed chip.
    let row_y = rect.top() + 38.0;
    let mut x = bars_rect.left();
    if !m.from_me {
        icons::paint(ui, Rect::from_center_size(pos2(x + 6.0, row_y), vec2(12.0, 12.0)), "mic", 12.0, played);
        x += 16.0;
    }
    let shown = match (status, duration) {
        (Some(s), _) if s.playing || s.position > 0.0 => clock_of(s.position),
        (_, Some(d)) => clock_of(d),
        _ => match media.size {
            Some(size) => human_size(size),
            None => String::new(),
        },
    };
    let time = ui.painter().layout_no_wrap(shown, fonts::mono(11.0), sub);
    ui.painter().galley(pos2(x, row_y - time.size().y / 2.0), time, sub);
    let mut speed_clicked = false;
    if let Some(s) = status {
        let label = if s.speed == 1.0 { "1×".to_owned() } else { format!("{}×", s.speed).replace('.', if crate::i18n::indonesian() { "," } else { "." }) };
        let galley = ui.painter().layout_no_wrap(label, fonts::ui(11.0, Weight::SemiBold), if m.from_me { p.on_primary } else { p.text });
        let chip = Rect::from_center_size(pos2(bars_rect.center().x, row_y), vec2(galley.size().x + 14.0, 16.0));
        let chip_response = ui.interact(chip, ui.scope_id().with(("voice_speed", &m.id)), Sense::click());
        let fill = if m.from_me { p.on_primary_muted.gamma_multiply(0.35) } else { p.raised };
        ui.painter().rect_filled(chip, 8.0, fill);
        ui.painter().galley(chip.center() - galley.size() / 2.0, galley, if m.from_me { p.on_primary } else { p.text });
        speed_clicked = chip_response
            .on_hover_text(tr("Playback speed", "Kecepatan putar"))
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked();
        if speed_clicked {
            player::set_speed(player::next_speed(s.speed));
        }
    }

    let play_hint = match (file.is_some(), status.is_some_and(|s| s.playing)) {
        (false, _) => match downloads.get(&m.id) {
            Some(Some(reason)) => crate::trf!("Download failed: {reason}", "Gagal mengunduh: {reason}"),
            _ => tr("Download and play", "Unduh dan putar").to_owned(),
        },
        (true, true) => tr("Pause", "Jeda").to_owned(),
        (true, false) => tr("Play", "Putar").to_owned(),
    };
    let play_clicked = play_response.on_hover_text(play_hint).on_hover_cursor(CursorIcon::PointingHand).clicked();
    let Some(file) = file else {
        let wants = play_clicked && media.downloadable() && !loading;
        return wants.then(|| MediaClick::Download(m.id.clone()));
    };
    if play_clicked {
        player::toggle(&m.id, file);
    } else if !speed_clicked && (seek.clicked() || seek.dragged()) {
        if let Some(at) = seek.interact_pointer_pos() {
            player::seek(&m.id, file, (at.x - bars_rect.left()) / bars_rect.width());
        }
    }
    let _ = seek.on_hover_cursor(CursorIcon::PointingHand);
    None
}

/// Opens a downloaded file in the desktop's default app.
fn open_file(path: &Path) {
    crate::platform::open(path);
}

/// Paints the media part of a bubble at `rect`; returns true when the user
/// asked to download it.
/// The icon, name and detail row of a file in a bubble.
fn paint_card(ui: &Ui, p: &Palette, rect: Rect, m: &ChatMsg, media: &Media, detail: &str) {
    let (fg, sub) = if m.from_me { (p.on_primary, p.on_primary_muted) } else { (p.text, p.muted) };
    let back = if m.from_me { p.on_primary_muted.gamma_multiply(0.25) } else { p.panel };
    ui.painter().rect_filled(rect, R_MD, back);
    let icon = match media.kind.as_str() {
        "image" => "image",
        "video" => "video",
        "sticker" => "sticker",
        _ => "file-text",
    };
    let icon_rect = Rect::from_min_size(pos2(rect.left() + 10.0, rect.center().y - 10.0), vec2(20.0, 20.0));
    icons::paint(ui, icon_rect, icon, 20.0, fg);
    let text_left = rect.left() + 40.0;
    let name = Rect::from_min_max(pos2(text_left, rect.top() + 7.0), pos2(rect.right() - 10.0, rect.top() + 25.0));
    widgets::paint_text(ui, name, &media_title(media), fonts::ui(13.0, Weight::Medium), fg, Align::Min);
    let line = Rect::from_min_max(pos2(text_left, rect.top() + 25.0), pos2(rect.right() - 10.0, rect.bottom() - 5.0));
    widgets::paint_text(ui, line, detail, fonts::ui(12.0, Weight::Regular), sub, Align::Min);
}

/// The top of a document's first page, filling `rect` edge to edge.
fn paint_page_top(ui: &Ui, rect: Rect, picture: &Path, aspect: f32) {
    let Some(uri) = widgets::picture_uri(ui.ctx(), picture) else { return };
    let shown = (rect.height() / (rect.width() * aspect)).min(1.0);
    egui::Image::from_uri(uri)
        .uv(Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, shown)))
        .corner_radius(egui::CornerRadius { nw: R_MD, ne: R_MD, sw: 0, se: 0 })
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

fn paint_media(
    ui: &Ui,
    p: &Palette,
    rect: Rect,
    m: &ChatMsg,
    media: &Media,
    shown: &MediaBox,
    downloads: &Downloads,
    auto_stickers: bool,
) -> Option<MediaClick> {
    if matches!(shown, MediaBox::Voice(_)) {
        return paint_voice(ui, p, rect, m, media, downloads);
    }
    let state = media_action(m, media, downloads);
    let action = state.label();
    let response = ui.interact(rect, ui.scope_id().with(("media", &m.id)), Sense::click());
    match shown {
        MediaBox::Picture(source, _) => {
            if let Some(uri) = widgets::picture_uri(ui.ctx(), source) {
                egui::Image::from_uri(uri).corner_radius(R_MD).fit_to_exact_size(rect.size()).paint_at(ui, rect);
            } else {
                ui.painter().rect_filled(rect, R_MD, p.border);
            }
            if state != MediaState::Open {
                let label = if media.kind == "video" { format!("Video · {action}") } else { action.to_owned() };
                let galley = ui.painter().layout_no_wrap(label, fonts::ui(12.0, Weight::Medium), Color32::WHITE);
                let pill = Rect::from_min_size(
                    pos2(rect.left() + 8.0, rect.bottom() - 30.0),
                    vec2(galley.size().x + 34.0, 22.0),
                );
                ui.painter().rect_filled(pill, 11.0, Color32::from_black_alpha(160));
                icons::paint(ui, Rect::from_min_size(pill.min + vec2(8.0, 3.0), vec2(16.0, 16.0)), "download", 14.0, Color32::WHITE);
                ui.painter().galley(pos2(pill.left() + 26.0, pill.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
            }
        }
        MediaBox::Sticker(file) => {
            let uri = file.as_ref().and_then(|f| widgets::picture_uri(ui.ctx(), f));
            if let Some(uri) = uri {
                egui::Image::from_uri(uri).fit_to_exact_size(rect.size()).paint_at(ui, rect);
            } else {
                ui.painter().rect(rect, R_LG, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
                let icon = Rect::from_center_size(rect.center() - vec2(0.0, 12.0), vec2(28.0, 28.0));
                icons::paint(ui, icon, "sticker", 28.0, p.muted);
                let label = Rect::from_min_max(pos2(rect.left(), rect.center().y + 10.0), pos2(rect.right(), rect.center().y + 28.0));
                let text = if state == MediaState::Download { tr("Loading…", "Memuat…") } else { action };
                widgets::paint_text(ui, label, text, fonts::ui(12.0, Weight::Regular), p.muted, Align::Center);
            }
        }
        MediaBox::Card => {
            let detail = match media.size {
                Some(size) => format!("{} · {action}", human_size(size)),
                None => action.to_owned(),
            };
            paint_card(ui, p, rect, m, media, &detail);
        }
        MediaBox::Voice(_) => unreachable!("drawn by paint_voice above"),
        MediaBox::Document { pdf: file, thumb } => {
            let preview = Rect::from_min_size(rect.min, vec2(rect.width(), PREVIEW_HEIGHT));
            let card = Rect::from_min_max(pos2(rect.left(), preview.bottom()), rect.max);
            let info = file.as_ref().and_then(|f| pdf::info(ui.ctx(), f));
            if ui.is_rect_visible(preview) {
                ui.painter().rect_filled(preview, egui::CornerRadius { nw: R_MD, ne: R_MD, sw: 0, se: 0 }, Color32::WHITE);
                let ppp = ui.ctx().pixels_per_point();
                let page = file.as_ref().zip(info).and_then(|(f, i)| Some((pdf::page(ui.ctx(), f, 1, rect.width() * ppp)?, i.aspect)));
                let thumb = || {
                    let thumb = thumb.as_ref()?;
                    let (w, h) = image::image_dimensions(thumb).ok()?;
                    Some((thumb.clone(), h as f32 / w.max(1) as f32))
                };
                if let Some((picture, aspect)) = page.or_else(thumb) {
                    paint_page_top(ui, preview, &picture, aspect);
                }
            }
            let mut parts = Vec::new();
            if pdf::is_pdf(Path::new(media.name.as_deref().unwrap_or_default())) || file.is_some() {
                parts.push("PDF".to_owned());
            }
            if let Some(info) = info {
                parts.push(if info.pages == 1 { tr("1 page", "1 halaman").to_owned() } else { crate::trf!("{} pages", "{} halaman", info.pages) });
            }
            if let Some(size) = media.size {
                parts.push(human_size(size));
            }
            parts.push(if state == MediaState::Open && file.is_some() { tr("View", "Lihat").to_owned() } else { action.to_owned() });
            paint_card(ui, p, card, m, media, &parts.join(" · "));
        }
    }
    let hint = match downloads.get(&m.id) {
        Some(Some(reason)) => crate::trf!("Download failed: {reason}", "Gagal mengunduh: {reason}"),
        _ => match state {
            MediaState::Open if matches!(shown, MediaBox::Document { pdf: Some(_), .. }) => tr("View PDF", "Lihat PDF").to_owned(),
            MediaState::Open => tr("Open with the default app", "Buka dengan aplikasi bawaan").to_owned(),
            MediaState::Download | MediaState::Retry => tr("Download this file", "Unduh file ini").to_owned(),
            _ => action.to_owned(),
        },
    };
    // Stickers download by themselves once on screen, like in WhatsApp.
    let auto = auto_stickers && matches!(shown, MediaBox::Sticker(None)) && state == MediaState::Download && ui.is_rect_visible(rect);
    if auto {
        return Some(MediaClick::Download(m.id.clone()));
    }
    let clickable = matches!(state, MediaState::Open | MediaState::Download | MediaState::Retry);
    let response = response.on_hover_text(hint);
    let response = if clickable { response.on_hover_cursor(CursorIcon::PointingHand) } else { response };
    if !response.clicked() || !clickable {
        return None;
    }
    match (&media.file, shown) {
        (Some(_), MediaBox::Document { pdf: Some(file), .. }) => Some(MediaClick::OpenPdf(file.clone(), media_title(media))),
        (Some(file), _) => {
            open_file(file);
            None
        }
        (None, _) => Some(MediaClick::Download(m.id.clone())),
    }
}

/// An entry of the message context menu. To extend the menu add a variant,
/// a button in `message_menu` and a branch in `on_message_event` (or handle
/// it inline in `messages` when it needs no backend, like `Copy`).
enum MsgAction {
    Copy,
    Reply,
    Forward,
    Pin(PinLength),
    Unpin,
}

/// What the user did on a message in the open chat.
enum MsgEvent {
    Media(MediaClick),
    /// A context menu action on the message with this id.
    Menu(MsgAction, String),
}

/// Turns a message event into commands or UI state; shared by the wide and
/// compact layouts.
fn on_message_event(event: MsgEvent, state: &mut State, session: &SessionView, chat: &Chat, commands: &mut Vec<Command>) {
    match event {
        MsgEvent::Media(MediaClick::Download(msg_id)) => commands.push(Command::Download { id: session.id.clone(), msg_id }),
        MsgEvent::Media(MediaClick::OpenPdf(file, title)) => state.pdf_view = Some(pdf::Viewer::new(file, title)),
        // Handled inline where the menu is shown.
        MsgEvent::Menu(MsgAction::Copy, _) => {}
        MsgEvent::Menu(MsgAction::Forward, msg_id) => {
            if let Some(m) = chat.messages.iter().find(|m| m.id == msg_id).filter(|m| m.forwardable()) {
                state.forward = Some(forward::Dialog::new(session.id.clone(), m.text.clone()));
            }
        }
        MsgEvent::Menu(MsgAction::Reply, msg_id) => {
            let Some(m) = chat.messages.iter().find(|m| m.id == msg_id) else { return };
            let author = if m.from_me { tr("You", "Anda").to_owned() } else { super::sender_label(&session.book, &m.sender) };
            let snippet = m.text.split_whitespace().collect::<Vec<_>>().join(" ");
            state.reply = Some(ReplyTarget { session: session.id.clone(), chat: chat.jid.clone(), id: m.id.clone(), author, snippet });
        }
        MsgEvent::Menu(MsgAction::Pin(length), msg_id) => {
            commands.push(Command::Pin { id: session.id.clone(), chat: chat.jid.clone(), msg_id, pinned: true, length });
        }
        MsgEvent::Menu(MsgAction::Unpin, msg_id) => {
            commands.push(Command::Pin { id: session.id.clone(), chat: chat.jid.clone(), msg_id, pinned: false, length: PinLength::Days7 });
        }
    }
}

/// The text to send: a quoted reply when "Balas" is active in this chat.
fn text_command(state: &mut State, id: String, chat: String, text: String, mentions: Vec<String>) -> Command {
    match state.reply.take_if(|r| r.chat == chat && r.session == id) {
        Some(reply) => Command::Reply { id, chat, text, reply_to: reply.id, mentions },
        None => Command::Send { id, chat, text, mentions },
    }
}

/// "Membalas <nama>: <cuplikan>" above the composer; x or Esc cancels. A reply
/// started in another chat is dropped.
fn reply_bar(ui: &mut Ui, p: &Palette, state: &mut State, session: &str, chat: &str) {
    if state.reply.as_ref().is_some_and(|r| r.chat != chat || r.session != session) {
        state.reply = None;
    }
    let Some(reply) = &state.reply else { return };
    let mut cancel = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let snippet: String = reply.snippet.chars().take(120).collect();
    Frame::NONE
        .fill(p.raised)
        .corner_radius(R_MD)
        .inner_margin(Margin { left: 10, right: 4, top: 4, bottom: 4 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::show(ui, "message-circle", 16.0, p.text_2);
                let label = crate::trf!("Replying to {}: {snippet}", "Membalas {}: {snippet}", reply.author);
                ui.add(egui::Label::new(rich(&label, 13.0, Weight::Regular, p.text_2)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    cancel |= widgets::icon_button(ui, "x", 24.0, false).on_hover_text(tr("Cancel reply (Esc)", "Batalkan balasan (Esc)")).clicked();
                });
            });
        });
    ui.add_space(6.0);
    if cancel {
        state.reply = None;
    }
}

/// Where a click on the pinned bar asks `messages` to scroll to (temp memory,
/// so the bar and the list need no shared state).
fn jump_id() -> egui::Id {
    egui::Id::unique("pin_jump")
}

/// The bar above the messages: the latest pin that has not lapsed, with the
/// number of pins. A click scrolls to the message and flashes it.
fn pinned_bar(ui: &mut Ui, p: &Palette, chat: &Chat) {
    let pins = chat.active_pins(time::now());
    let Some((latest, m)) = pins.iter().rev().find_map(|pin| chat.messages.iter().find(|m| m.id == pin.id).map(|m| (*pin, m))) else {
        return;
    };
    let mut label = caption(m);
    if label.is_empty() {
        label = m.media.as_ref().map(media_title).unwrap_or_default();
    }
    let label = label.replace('\n', " ");
    ui.add_space(8.0);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tr("Open the pinned message", "Buka pesan yang disematkan"));
    ui.painter().rect(rect, R_MD, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
    icons::paint(ui, Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(14.0, 14.0)), "pin", 14.0, p.primary);
    let count = if pins.len() > 1 { crate::trf!("{} pins", "{} sematan", pins.len()) } else { String::new() };
    let right = if count.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(count.clone(), fonts::ui(11.0, Weight::Medium), p.muted).size().x + 12.0 };
    let text = Rect::from_min_max(pos2(rect.left() + 34.0, rect.top()), pos2(rect.right() - 12.0 - right, rect.bottom()));
    widgets::paint_text(ui, text, &label, fonts::ui(13.0, Weight::Regular), p.text, Align::Min);
    if !count.is_empty() {
        let tag = Rect::from_min_max(pos2(rect.right() - right, rect.top()), pos2(rect.right() - 12.0, rect.bottom()));
        widgets::paint_text(ui, tag, &count, fonts::ui(11.0, Weight::Medium), p.muted, Align::Max);
    }
    if response.clicked() {
        ui.data_mut(|d| d.insert_temp(jump_id(), latest.id.clone()));
        ui.ctx().request_repaint();
    }
    ui.add_space(4.0);
}

/// Right-click / long-press menu of a message bubble; returns the chosen action.
fn message_menu(ui: &mut Ui, m: &ChatMsg, pinned: bool) -> Option<MsgAction> {
    let mut action = None;
    // A message still in the send queue has no WhatsApp id to quote yet.
    if !m.id.starts_with("local-") && ui.button(tr("Reply", "Balas")).clicked() {
        action = Some(MsgAction::Reply);
    }
    // Text only for now; media forwarding is stage 2.
    if m.forwardable() && ui.button(tr("Forward", "Teruskan")).clicked() {
        action = Some(MsgAction::Forward);
    }
    // Media without a caption has nothing to copy.
    if !caption(m).is_empty() && ui.button(tr("Copy", "Salin")).clicked() {
        action = Some(MsgAction::Copy);
    }
    // A message still in the send queue has no WhatsApp id to pin yet.
    if !m.id.starts_with("local-") {
        if pinned {
            if ui.button(tr("Unpin", "Lepas sematan")).clicked() {
                action = Some(MsgAction::Unpin);
            }
        } else {
            ui.menu_button(tr("Pin", "Sematkan"), |ui| {
                for (label, length) in [(tr("24 hours", "24 jam"), PinLength::Hours24), (tr("7 days", "7 hari"), PinLength::Days7), (tr("30 days", "30 hari"), PinLength::Days30)] {
                    if ui.button(label).clicked() {
                        action = Some(MsgAction::Pin(length));
                    }
                }
            });
        }
    }
    if action.is_some() {
        ui.close();
    }
    action
}

/// Paints the open chat; returns a message whose media the user wants downloaded.
/// Draws the conversation. In a group, incoming runs from one member get the
/// member's picture beside the first bubble and their name, in their colour,
/// on top of it; `want_avatars` collects pictures to ask the backend for.
/// `own` is our number and LID, so mentions of us are highlighted.
#[allow(clippy::too_many_arguments)]
fn messages(
    ui: &mut Ui,
    p: &Palette,
    book: &Book,
    chat: &Chat,
    downloads: &Downloads,
    avatars: &Avatars,
    want_avatars: &mut Vec<String>,
    auto_stickers: bool,
    typing: Option<&str>,
    own: (Option<&str>, Option<&str>),
    anchor: &mut Option<UnreadAnchor>,
) -> Option<MsgEvent> {
    const GAP: f32 = 6.0;
    /// Extra space before a group run from another member.
    const RUN_GAP: f32 = 6.0;
    const DIVIDER: f32 = 36.0;
    /// Member picture beside incoming group bubbles, and the column it takes.
    const FACE: f32 = 28.0;
    const GUTTER: f32 = FACE + 8.0;
    let group = chat.is_group();
    let wrap = (ui.available_width() * 0.7).min(420.0);
    let dark = is_dark(p.bubble_in);
    let start = chat.messages.len().saturating_sub(SHOWN);
    let shown: &[ChatMsg] = &chat.messages[start..];
    let mut clicked = None;
    let now = time::now();
    // Set by the pinned bar: scroll to this message, then flash it for a moment.
    let jump: Option<String> = ui.data_mut(|d| d.remove_temp(jump_id()));
    let flash_id = egui::Id::unique("pin_flash");
    let flash: Option<(String, f64)> = ui.data(|d| d.get_temp(flash_id));
    let clock = ui.input(|i| i.time);
    let flash = flash.filter(|(_, until)| *until > clock).map(|(id, _)| id);
    if flash.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    // The unread divider goes before this message; the first frame scrolls to it once.
    let unread = anchor.as_ref().filter(|a| a.jid == chat.jid && shown.iter().any(|m| m.id == a.first_id)).map(|a| (a.first_id.clone(), a.count));
    let mut scroll_unread = unread.is_some() && anchor.as_mut().is_some_and(|a| std::mem::take(&mut a.pending));
    let memo_id = ui.scope_id().with(("chat_scroll_memo", &chat.jid));
    let bottom_id = ui.scope_id().with(("chat_scroll_bottom", &chat.jid));
    let to_bottom: bool = ui.data_mut(|d| d.remove_temp::<bool>(bottom_id)).unwrap_or(false);

    struct Bubble<'a> {
        m: &'a ChatMsg,
        media: Option<(&'a Media, MediaBox)>,
        text: Option<std::sync::Arc<egui::Galley>>,
        /// Delivery mark after the time on our messages.
        tick: Option<(&'static str, Color32)>,
        /// "Status · …": what the message replies to.
        quote: Option<std::sync::Arc<egui::Galley>>,
        meta: std::sync::Arc<egui::Galley>,
        /// First bubble of a group member's run: their name (in their colour),
        /// normalized JID and colour, for the name line and the picture.
        head: Option<(std::sync::Arc<egui::Galley>, String, Color32)>,
        /// Space above the bubble that starts a new run.
        lead: f32,
        size: egui::Vec2,
    }
    enum Item<'a> {
        Day(String),
        /// "N pesan belum dibaca" before the first unread message.
        Unread(usize),
        Bubble(Bubble<'a>),
        /// "••• sedang mengetik" after the last message (Pego `v5/Inbox · Chat`).
        Typing(String),
    }
    const TYPING_ROW: f32 = 36.0;
    let mut items = Vec::new();
    let mut last_day = None;
    // Who sent the bubble above: `None` for us, a day pill or the top.
    let mut prev: Option<Option<String>> = None;
    for m in shown {
        let day = time::day(m.ts);
        if last_day != Some(day) {
            items.push(Item::Day(time::day_label(m.ts)));
            last_day = Some(day);
            prev = None;
        }
        let who = (!m.from_me).then(|| book.normalize(&m.sender));
        let new_run = prev.as_ref() != Some(&who);
        let lead = if new_run && prev.is_some() && group { RUN_GAP } else { 0.0 };
        prev = Some(who.clone());
        // A failed message shows it with the red mark; the time keeps its colour.
        let (fg, meta_color, code_bg) = if m.from_me {
            (p.on_primary, p.on_primary_muted, p.on_primary_muted.gamma_multiply(0.35))
        } else {
            (p.text, p.muted, p.border)
        };
        if let Some((_, count)) = unread.as_ref().filter(|(first, _)| *first == m.id) {
            items.push(Item::Unread(*count));
        }
        let media = m.media.as_ref().map(|media| (media, media_box(media, wrap)));
        let media_size = media.as_ref().map(|(_, b)| b.size());
        let sticker = matches!(media, Some((_, MediaBox::Sticker(_))));
        let meta_color = if sticker { p.muted } else { meta_color };
        let caption = caption(m);
        let mentions = mention_refs(book, m, own.0, own.1);
        let accent = if m.from_me { fg } else { p.accent_text };
        let text = (!caption.is_empty()).then(|| ui.painter().layout_job(markup::job_with_mentions(&caption, 14.0, fg, code_bg, wrap, &mentions, accent)));
        let meta = time::clock(m.ts);
        let tick = if m.from_me { tick(p, &m.status, meta_color, !sticker && is_dark(p.primary)) } else { None };
        let meta = if m.forwarded { crate::trf!("Forwarded · {meta}", "Diteruskan · {meta}") } else { meta };
        let head = who.filter(|_| group && new_run).map(|jid| {
            let color = member_color(&jid, dark);
            let mut job = egui::text::LayoutJob::simple_singleline(super::sender_label(book, &m.sender), fonts::ui(12.5, Weight::SemiBold), color);
            job.wrap.max_width = wrap;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            (ui.painter().layout_job(job), jid, color)
        });
        let meta = ui.painter().layout_no_wrap(meta, fonts::ui(11.0, Weight::Regular), meta_color);
        let quote = m.quote.as_ref().map(|q| {
            let label = if q.status { format!("Status · {}", q.text) } else { q.text.clone() };
            let mut job = egui::text::LayoutJob::simple(label.replace('\n', " "), fonts::ui(12.0, Weight::Regular), meta_color, wrap - 20.0);
            job.wrap.max_rows = 2;
            ui.painter().layout_job(job)
        });
        let text_size = text.as_ref().map(|t| t.size() + vec2(0.0, 4.0)).unwrap_or_default();
        let media_space = media_size.map(|s| s + vec2(0.0, 8.0)).unwrap_or_default();
        let quote_space = quote.as_ref().map(|g| g.size() + vec2(20.0, 18.0)).unwrap_or_default();
        let head_space = head.as_ref().map(|(g, ..)| g.size() + vec2(0.0, 4.0)).unwrap_or_default();
        let size = vec2(
            text_size.x.max(meta.size().x + if tick.is_some() { 20.0 } else { 0.0 }).max(media_space.x).max(quote_space.x).max(head_space.x) + 28.0,
            head_space.y + quote_space.y + media_space.y + text_size.y + meta.size().y + 20.0,
        );
        items.push(Item::Bubble(Bubble { m, media, text, tick, quote, meta, head, lead, size }));
    }
    if let Some(label) = typing {
        let typing_word = tr("typing…", "mengetik…");
        items.push(Item::Typing(if label.ends_with(typing_word) {
            label.replace(typing_word, tr("typing", "sedang mengetik"))
        } else {
            label.to_owned()
        }));
    }
    let content: f32 = items
        .iter()
        .map(|i| match i {
            Item::Day(_) | Item::Unread(_) => DIVIDER,
            Item::Bubble(b) => b.lead + b.size.y + GAP,
            Item::Typing(_) => TYPING_ROW + GAP,
        })
        .sum();

    let output = ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.add_space(12.0);
            let height = content.max(ui.available_height());
            let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
            let painter = ui.painter().clone();
            let mut y = area.bottom() - content;
            if start > 0 {
                let note = Rect::from_min_size(pos2(area.left(), y - 24.0), vec2(area.width(), 16.0));
                widgets::paint_text(ui, note, &crate::trf!("{start} older messages not shown", "{start} pesan lebih lama tidak ditampilkan"), fonts::ui(12.0, Weight::Regular), p.muted, Align::Center);
            }
            for item in items {
                match item {
                    Item::Typing(label) => {
                        // In a group the row lines up with the bubbles, past the picture column.
                        let left = area.left() + if group { GUTTER } else { 0.0 };
                        let dots = Rect::from_min_size(pos2(left, y), vec2(52.0, TYPING_ROW));
                        painter.rect(dots, 18.0, p.bubble_in, Stroke::new(1.0, p.border), StrokeKind::Inside);
                        // Three dots, the middle one lifted a little in turn.
                        let t = ui.input(|i| i.time);
                        for k in 0..3 {
                            let phase = ((t * 3.0 - k as f64 * 0.6).sin() * 0.5 + 0.5) as f32;
                            let at = pos2(dots.left() + 16.0 + k as f32 * 10.0, dots.center().y - phase * 2.5);
                            painter.circle_filled(at, 3.2, p.muted.gamma_multiply(0.6 + 0.4 * phase));
                        }
                        let text = Rect::from_min_max(pos2(dots.right() + 10.0, y), pos2(area.right(), y + TYPING_ROW));
                        widgets::paint_text(ui, text, &label, fonts::ui(12.0, Weight::Regular), p.muted, Align::Min);
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                        y += TYPING_ROW + GAP;
                    }
                    Item::Unread(count) => {
                        let label = if count == 1 { tr("1 unread message", "1 pesan belum dibaca").to_owned() } else { crate::trf!("{count} unread messages", "{count} pesan belum dibaca") };
                        let galley = painter.layout_no_wrap(label, fonts::ui(12.0, Weight::Medium), p.accent_text);
                        let (w, cy, cx) = (galley.size().x, y + DIVIDER / 2.0, area.center().x);
                        let pill = Rect::from_center_size(pos2(cx, cy), vec2(w + 24.0, 24.0));
                        painter.rect(pill, 12.0, p.accent_soft, Stroke::new(1.0, p.border), StrokeKind::Inside);
                        painter.galley(pos2(cx - w / 2.0, cy - galley.size().y / 2.0), galley, p.accent_text);
                        if std::mem::take(&mut scroll_unread) {
                            ui.scroll_to_rect(Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), DIVIDER)), Some(Align::TOP));
                        }
                        y += DIVIDER;
                    }
                    Item::Day(label) => {
                        // A pill, so it reads on the wallpaper (Pego m5/Chat "Hari ini").
                        let galley = painter.layout_no_wrap(label, fonts::ui(12.0, Weight::Medium), p.text_2);
                        let (w, cy, cx) = (galley.size().x, y + DIVIDER / 2.0, area.center().x);
                        let pill = Rect::from_center_size(pos2(cx, cy), vec2(w + 24.0, 24.0));
                        painter.rect(pill, 12.0, p.bubble_in, Stroke::new(1.0, p.border), StrokeKind::Inside);
                        painter.galley(pos2(cx - w / 2.0, cy - galley.size().y / 2.0), galley, p.text_2);
                        y += DIVIDER;
                    }
                    Item::Bubble(Bubble { m, media, text, tick, quote, meta, head, lead, size }) => {
                        y += lead;
                        let (fill, stroke) = if m.from_me {
                            (p.primary, Stroke::NONE)
                        } else {
                            (p.bubble_in, Stroke::new(1.0, p.border))
                        };
                        let left = match (m.from_me, group) {
                            (true, _) => area.right() - size.x,
                            (false, true) => area.left() + GUTTER,
                            (false, false) => area.left(),
                        };
                        let rect = Rect::from_min_size(pos2(left, y), size);
                        if !matches!(media, Some((_, MediaBox::Sticker(_)))) {
                            painter.rect(rect, R_LG, fill, stroke, StrokeKind::Inside);
                        }
                        // Created before the media's own click area, so media keeps its primary click.
                        let bubble = ui.interact(rect, ui.scope_id().with(("bubble", &m.id)), Sense::click());
                        let mut action = None;
                        bubble.context_menu(|ui| action = message_menu(ui, m, chat.is_pinned(&m.id, now)));
                        if jump.as_deref() == Some(m.id.as_str()) {
                            ui.scroll_to_rect(rect, Some(Align::Center));
                            ui.data_mut(|d| d.insert_temp(flash_id, (m.id.clone(), clock + 1.8)));
                            ui.ctx().request_repaint();
                        }
                        if flash.as_deref() == Some(m.id.as_str()) {
                            painter.rect_stroke(rect.expand(2.0), R_LG, Stroke::new(2.0, p.primary), StrokeKind::Outside);
                        }
                        let mut at = rect.min + vec2(14.0, 10.0);
                        // Anti-peek: frosted until the cursor is on this bubble, then outlined.
                        let veiled = crate::ui::privacy::blur_chat() && !ui.rect_contains_pointer(rect);
                        if crate::ui::privacy::blur_chat() && !veiled {
                            painter.rect_stroke(rect.expand(1.0), R_LG, Stroke::new(1.5, p.primary), StrokeKind::Outside);
                        }
                        let frost = if m.from_me { p.on_primary_muted.gamma_multiply(0.55) } else { p.border_strong.gamma_multiply(0.45) };
                        if let Some((name, jid, color)) = head {
                            let face = Rect::from_min_size(pos2(area.left(), y), vec2(FACE, FACE));
                            match avatars.get(&jid) {
                                Some(picture) => paint_member_face(ui, face, &super::initials(&super::sender_label(book, &m.sender)), picture.as_deref(), color),
                                None => {
                                    want_avatars.push(jid);
                                    paint_member_face(ui, face, &super::initials(&super::sender_label(book, &m.sender)), None, color);
                                }
                            }
                            let height = name.size().y;
                            painter.galley(at, name, color);
                            at.y += height + 4.0;
                        }
                        if let Some(quote) = quote {
                            let boxed = Rect::from_min_size(at, vec2(size.x - 28.0, quote.size().y + 10.0));
                            let (fill, bar) = if m.from_me { (Color32::from_white_alpha(28), p.on_primary_muted) } else { (p.raised, p.primary) };
                            painter.rect_filled(boxed, R_MD, fill);
                            painter.rect_filled(Rect::from_min_size(boxed.min, vec2(3.0, boxed.height())), egui::CornerRadius { nw: R_MD, sw: R_MD, ne: 0, se: 0 }, bar);
                            if veiled {
                                crate::ui::privacy::frost_galley(&painter, boxed.min + vec2(10.0, 5.0), &quote, frost);
                            } else {
                                painter.galley(boxed.min + vec2(10.0, 5.0), quote, p.muted);
                            }
                            at.y += boxed.height() + 8.0;
                        }
                        if let Some((media, shown)) = &media {
                            let media_size = shown.size();
                            let media_rect = Rect::from_min_size(at, media_size);
                            if let Some(click) = paint_media(ui, p, media_rect, m, media, shown, downloads, auto_stickers) {
                                clicked = Some(MsgEvent::Media(click));
                            }
                            // Pictures, stickers and document pages go under a cover (voice notes show nothing).
                            if veiled && !matches!(shown, MediaBox::Voice(_)) {
                                painter.rect_filled(media_rect, R_MD, if m.from_me { p.primary } else { p.raised });
                                icons::paint(ui, Rect::from_center_size(media_rect.center(), vec2(22.0, 22.0)), "eye-off", 22.0, frost);
                            }
                            at.y += media_size.y + 8.0;
                        }
                        if let Some(text) = text.as_ref().filter(|_| veiled) {
                            crate::ui::privacy::frost_galley(&painter, at, text, frost);
                            at.y += text.size().y + 4.0;
                        } else if let Some(text) = text {
                            let height = text.size().y;
                            // Selectable label (drag to select, Cmd/Ctrl+C copies the selection). It sits above the
                            // bubble's click area, so it also needs its own context menu for right-click on text.
                            let link = link_at(ui, &text, at);
                            let label = ui.put(Rect::from_min_size(at, text.size()), egui::Label::new(text).selectable(true));
                            // A click (not a drag that selects text) on a link opens it.
                            if let Some(url) = link.filter(|_| label.hovered()) {
                                label.ctx.set_cursor_icon(CursorIcon::PointingHand);
                                if ui.input(|i| i.pointer.primary_clicked()) {
                                    crate::platform::open_url(&url);
                                }
                            }
                            label.context_menu(|ui| action = message_menu(ui, m, chat.is_pinned(&m.id, now)));
                            at.y += height + 4.0;
                        }
                        // "Salin" copies the whole caption; egui does not expose the label selection, Cmd/Ctrl+C copies a partial one.
                        match action {
                            Some(MsgAction::Copy) => ui.ctx().copy_text(caption(m)),
                            Some(action) => clicked = Some(MsgEvent::Menu(action, m.id.clone())),
                            None => {}
                        }
                        let (meta_w, meta_h) = (meta.size().x, meta.size().y);
                        painter.galley(at, meta, p.muted);
                        if let Some((icon, tint)) = tick {
                            let mark = Rect::from_min_size(pos2(at.x + meta_w + 4.0, at.y + meta_h / 2.0 - 7.5), vec2(15.0, 15.0));
                            icons::paint(ui, mark, icon, 15.0, tint);
                            ui.interact(mark, ui.scope_id().with(("tick", &m.id)), Sense::hover()).on_hover_text(delivery_label(&m.status));
                        }
                        if let Delivery::Failed(reason) = &m.status {
                            ui.interact(rect, ui.scope_id().with(("failed", &m.id)), Sense::hover())
                                .on_hover_text(crate::trf!("Failed to send: {reason}", "Gagal terkirim: {reason}"));
                        }
                        y += size.y + GAP;
                    }
                }
            }
            if to_bottom {
                ui.scroll_to_rect(Rect::from_min_size(pos2(area.left(), area.bottom() - 1.0), vec2(area.width(), 1.0)), Some(Align::BOTTOM));
            }
        });
    // Floating "down" button while scrolled up, with the incoming messages since leaving the bottom.
    let at_bottom = output.state.unclamped_offset().y + output.inner_rect.height() >= output.content_size.y - 8.0;
    if at_bottom {
        ui.data_mut(|d| d.remove_temp::<usize>(memo_id));
    } else {
        let seen = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(memo_id, || chat.messages.len()));
        let fresh = chat.messages.get(seen..).map_or(0, |new| new.iter().filter(|m| !m.from_me).count());
        let size = 40.0;
        let at = output.inner_rect.right_bottom() - vec2(size + 16.0, size + 16.0);
        egui::Area::new(ui.scope_id().with(("chat_to_bottom", &chat.jid)))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(size), Sense::click());
                let fill = if resp.hovered() { p.selected } else { p.raised };
                ui.painter().circle(rect.center(), size / 2.0, fill, Stroke::new(1.0, p.border_strong));
                icons::paint(ui, rect, "chevron-down", 20.0, p.text_2);
                if fresh > 0 {
                    let label = if fresh > 99 { "99+".to_owned() } else { fresh.to_string() };
                    let galley = ui.painter().layout_no_wrap(label, fonts::ui(11.0, Weight::Medium), p.on_primary);
                    let badge = Rect::from_center_size(rect.center_top() + vec2(0.0, 2.0), vec2((galley.size().x + 10.0).max(18.0), 18.0));
                    ui.painter().rect_filled(badge, 9.0, p.primary);
                    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, p.on_primary);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    ui.data_mut(|d| d.insert_temp(bottom_id, true));
                    ui.ctx().request_repaint();
                }
            });
    }
    clicked
}

/// Returns the text to send when Enter (or Kirim) is pressed. Shift+Enter
/// adds a new line, like WhatsApp Web.
/// Runs the desktop's file dialog (zenity) off the UI thread.
fn pick_file(ctx: &egui::Context) -> std::sync::mpsc::Receiver<Option<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let picked = crate::platform::pick_file(tr("Choose a photo, video, or document", "Pilih foto, video, atau dokumen"), false);
        let _ = tx.send(picked);
        ctx.request_repaint();
    });
    rx
}

/// The file a paste really is: Finder (and Linux file managers) put the
/// copied files' names (or paths) on the clipboard as text, one per line,
/// beside the files themselves. When every pasted line is one of `files`,
/// the paste is the first of them; any other text stays text.
fn copied_file(pasted: &str, files: &[PathBuf]) -> Option<PathBuf> {
    let lines: Vec<&str> = pasted.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect();
    let named = |line: &str, file: &PathBuf| {
        file.as_os_str() == line || file.file_name().is_some_and(|name| name == line)
    };
    let all_files = !lines.is_empty() && lines.iter().all(|line| files.iter().any(|f| named(line, f)));
    all_files.then(|| files.first().cloned()).flatten()
}

/// The files on the system clipboard, if it holds any.
fn clipboard_files() -> Vec<PathBuf> {
    arboard::Clipboard::new().and_then(|mut c| c.get().file_list()).unwrap_or_default()
}

/// Saves a pasted picture as `tempel-<time>.png` in `dir`; the sent bubble
/// shows it from there, so it is kept.
fn save_pasted_image(image: &egui::ColorImage, dir: &Path) -> Result<PathBuf, String> {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let picture = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("ukuran gambar tidak cocok")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stamp = time::stamp(time::now());
    let mut path = dir.join(format!("tempel-{stamp}.png"));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("tempel-{stamp}-{n}.png"));
    }
    picture.save(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The web link under the pointer in a bubble's text laid out at `at`.
fn link_at(ui: &Ui, text: &std::sync::Arc<egui::Galley>, at: egui::Pos2) -> Option<String> {
    let pointer = ui.input(|i| i.pointer.hover_pos())?;
    let rect = Rect::from_min_size(at, text.size());
    if !rect.contains(pointer) {
        return None;
    }
    let index: usize = text.cursor_from_pos(pointer - at).index.into();
    markup::links(text.text())
        .into_iter()
        .find(|(range, _)| range.start <= index && index < range.end)
        .map(|(_, url)| url)
}

/// The file waiting to be sent: icon, name, size and a remove button.
/// Returns false when the file cannot be sent (missing or too large).
fn attachment_chip(ui: &mut Ui, p: &Palette, attachment: &mut Option<PathBuf>) -> bool {
    let Some(path) = attachment.as_ref() else {
        return true;
    };
    let size = std::fs::metadata(path).map(|m| m.len()).ok();
    let (sendable, note) = match size {
        None => (false, tr("file not found", "file tidak ditemukan").to_owned()),
        Some(n) if n > MAX_FILE_BYTES => (false, crate::trf!("over {} MB", "lebih dari {} MB", MAX_FILE_BYTES / 1024 / 1024)),
        Some(n) if n >= 1024 * 1024 => (true, format!("{:.1} MB", n as f64 / 1024.0 / 1024.0)),
        Some(n) => (true, format!("{} KB", n.div_ceil(1024))),
    };
    let picture = matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("jpg" | "jpeg" | "png")
    );
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut remove = false;
    Frame::NONE
        .fill(p.raised)
        .corner_radius(R_MD)
        .inner_margin(Margin { left: 10, right: 4, top: 4, bottom: 4 })
        .show(ui, |ui| {
            // A picture (a pasted screenshot, say) shows what will be sent.
            if picture && size.is_some() && let Some(uri) = widgets::picture_uri(ui.ctx(), path) {
                ui.add_space(4.0);
                ui.add(egui::Image::from_uri(uri).max_size(vec2(260.0, 160.0)).corner_radius(R_MD));
                ui.add_space(2.0);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::show(ui, if picture { "image" } else { "file-text" }, 16.0, p.text_2);
                ui.label(rich(&name, 13.0, Weight::Medium, p.text));
                let color = if sendable { p.muted } else { p.red_text };
                ui.label(rich(&note, 12.0, Weight::Regular, color));
                remove = widgets::icon_button(ui, "x", 24.0, false).on_hover_text(tr("Remove attachment", "Batalkan lampiran")).clicked();
            });
        });
    if remove {
        *attachment = None;
        return true;
    }
    ui.add_space(8.0);
    sendable
}

/// What the composer asks to send.
enum Outgoing {
    /// Text on the wire (`@<number>` for mentions) and the JIDs it mentions.
    Text(String, Vec<String>),
    /// A file with its caption (may be empty).
    File(PathBuf, String),
    Sticker(PathBuf),
    /// A recorded voice note (Ogg Opus), for the chat it was recorded in.
    Voice { session: String, chat: String, path: PathBuf },
}

/// Enter alone sends. `consume_key` ignores extra Shift and Alt, so without
/// this check Shift+Enter would send too instead of reaching the text box as a
/// new line.
fn enter_sends(input: &mut egui::InputState) -> bool {
    !input.modifiers.shift && !input.modifiers.alt && input.consume_key(Modifiers::NONE, Key::Enter)
}

/// What the recording bar asks for.
enum Recording {
    Keep,
    Cancel,
    Send,
    Pause,
    Resume,
    /// Play or pause the paused note.
    Listen,
    /// The held mic slid up: keep recording without holding.
    Lock,
}

/// How long an error about a voice note stays under the composer.
const VOICE_ERROR_FOR: std::time::Duration = std::time::Duration::from_secs(8);

/// The player id of the note being recorded, heard back while paused.
const PREVIEW: &str = "voice-preview";
/// A new bar of the live wave every 60 ms (Pego "Perilaku wave").
const WAVE_EVERY: std::time::Duration = std::time::Duration::from_millis(60);
/// About the last second of bars is drawn full, older ones faded.
const WAVE_FRESH: usize = 16;

/// Height of a wave bar for a microphone level (0 to 1): log scale from 3 to
/// `max` px, so quiet speech still moves; under -50 dB it is a 3 px dot.
fn wave_height(level: f32, max: f32) -> f32 {
    let db = 20.0 * level.max(1e-5).log10();
    let v = ((db + 50.0) / 50.0).clamp(0.0, 1.0);
    3.0 + v * (max - 3.0)
}

/// The live wave: the newest bar enters at the right and the others slide
/// left, smoothly between samples; the last second is full colour.
fn paint_live_wave(ui: &Ui, rect: Rect, wave: &[f32], sampled: Option<std::time::Instant>, color: Color32) {
    const STEP: f32 = 5.0;
    let painter = ui.painter().with_clip_rect(rect);
    let slide = sampled.map(|t| (t.elapsed().as_secs_f32() / WAVE_EVERY.as_secs_f32()).min(1.0)).unwrap_or(0.0) * STEP;
    let fits = (rect.width() / STEP) as usize + 2;
    // A thin baseline, as the bars fill it from the right.
    painter.line_segment([pos2(rect.left(), rect.center().y), pos2(rect.right(), rect.center().y)], Stroke::new(1.0, color.gamma_multiply(0.2)));
    for (age, level) in wave.iter().rev().take(fits).enumerate() {
        let x = rect.right() - 2.0 - age as f32 * STEP - slide + STEP;
        let height = wave_height(*level, rect.height());
        let fill = if age < WAVE_FRESH { color } else { color.gamma_multiply(0.4) };
        painter.rect_filled(Rect::from_center_size(pos2(x, rect.center().y), vec2(3.0, height)), 1.5, fill);
    }
}

/// The whole note's wave while paused, 0..1 of it played.
fn paint_note_wave(ui: &Ui, rect: Rect, wave: &[f32], played: f32, color: Color32, rest: Color32) {
    let bars = ((rect.width() / 5.0) as usize).max(1);
    let step = rect.width() / bars as f32;
    for bar in 0..bars {
        let from = bar * wave.len() / bars;
        let to = ((bar + 1) * wave.len() / bars).max(from + 1).min(wave.len());
        let level = wave.get(from..to).and_then(|w| w.iter().copied().reduce(f32::max)).unwrap_or(0.0);
        let x = rect.left() + step * (bar as f32 + 0.5);
        let done = (bar as f32 + 0.5) / bars as f32 <= played;
        let height = wave_height(level, rect.height());
        ui.painter().rect_filled(Rect::from_center_size(pos2(x, rect.center().y), vec2(3.0, height)), 1.5, if done { color } else { rest });
    }
    if played > 0.0 {
        ui.painter().circle_filled(pos2(rect.left() + rect.width() * played, rect.center().y), 6.0, color);
    }
}

/// A round icon button for the recorder.
fn round_button(ui: &mut Ui, icon: &str, size: f32, fill: Color32, glyph: Color32, outline: Option<Color32>, hint: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let grow = if response.hovered() { 1.0 } else { 0.0 };
    ui.painter().circle_filled(rect.center(), size / 2.0 + grow, fill);
    if let Some(outline) = outline {
        ui.painter().circle_stroke(rect.center(), size / 2.0 - 0.5, Stroke::new(1.0, outline));
    }
    icons::paint(ui, Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), icon, 18.0, glyph);
    response.on_hover_text(hint).on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// A key hint chip: "Esc" beside "cancel".
fn key_hint(ui: &mut Ui, p: &Palette, key: &str, what: &str) {
    let galley = ui.painter().layout_no_wrap(key.to_owned(), fonts::mono(11.0), p.text_2);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(10.0, 6.0), Sense::hover());
    ui.painter().rect(rect, 4.0, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, p.text_2);
    ui.label(rich(what, 12.0, Weight::Regular, p.muted));
}

/// The composer while a voice note records (Pego `v5/VN · Rekam & putar`,
/// `m5/VN 2` and `m5/VN 3`):
/// - recording: 🗑, a red dot and the time, the live wave, ⏸ and send;
/// - paused: 🗑, ▶ to listen, the whole wave as it plays, the time, "Resume"
///   and send.
/// A strip above names the number it goes out from, with Esc and Enter hints.
/// While the microphone opens, or the note is saved, it says so instead.
fn recording_bar(ui: &mut Ui, p: &Palette, note: Option<&mut VoiceNote>, from: &str, compact: bool) -> Recording {
    let saving = note.as_ref().is_none_or(|n| !matches!(n.step, VoiceStep::Recording(_)));
    let (recorder, mut wave, mut sampled) = match note {
        Some(VoiceNote { step: VoiceStep::Recording(recorder), wave, sampled, .. }) => (Some(recorder), Some(wave), Some(sampled)),
        _ => (None, None, None),
    };
    let paused = recorder.as_ref().is_some_and(|r| r.is_paused());
    let recording = recorder.as_ref().is_some_and(|r| r.elapsed() > std::time::Duration::ZERO || r.is_paused());
    let elapsed = recorder.as_ref().map(|r| r.elapsed().as_secs_f32()).unwrap_or(0.0);
    if elapsed >= crate::backend::voice::MAX_SECONDS as f32 {
        return Recording::Send;
    }
    // A new bar every 60 ms while recording; the wave keeps sliding between.
    if let (Some(recorder), Some(wave), Some(sampled)) = (&recorder, wave.as_deref_mut(), sampled.as_deref_mut()) {
        if recording && !paused && sampled.is_none_or(|t| t.elapsed() >= WAVE_EVERY) {
            wave.push(recorder.level());
            *sampled = Some(std::time::Instant::now());
        }
    }
    let listen = player_status_preview();
    if (recording && !paused) || listen.is_some_and(|s| s.playing) || recorder.is_none() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    } else {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    let mut action = ui.input_mut(|i| {
        if recorder.is_some() && i.consume_key(Modifiers::NONE, Key::Escape) {
            Recording::Cancel
        } else if recording && enter_sends(i) {
            Recording::Send
        } else if recording && i.consume_key(Modifiers::NONE, Key::Space) {
            if paused { Recording::Listen } else { Recording::Pause }
        } else {
            Recording::Keep
        }
    });

    // The strip: what is happening, from which number, and the keys.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (icon, label) = if saving {
            ("loader-circle", tr("Saving voice message…", "Menyimpan pesan suara…").to_owned())
        } else if paused {
            ("pause", tr("Paused · listen before sending", "Dijeda · dengarkan dulu sebelum kirim").to_owned())
        } else {
            ("lock", crate::trf!("Recording voice message · sent from {from}", "Merekam voice note · dikirim dari {from}"))
        };
        icons::show(ui, icon, 13.0, p.text_2);
        ui.label(rich(&label, 12.0, Weight::Medium, p.text_2));
        if !saving && !compact {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                key_hint(ui, p, "Enter", tr("send", "kirim"));
                key_hint(ui, p, "Esc", tr("cancel", "batal"));
            });
        }
    });
    ui.add_space(6.0);

    Frame::NONE
        .fill(p.raised)
        .corner_radius(if compact { 24 } else { R_LG })
        .inner_margin(Margin { left: 6, right: 6, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.set_min_height(40.0);
                if saving {
                    ui.add_space(8.0);
                    ui.label(rich(tr("Saving voice message…", "Menyimpan pesan suara…"), 14.0, Weight::Regular, p.muted));
                    return;
                }
                if round_button(ui, "trash", 36.0, p.red_soft, p.red, None, tr("Delete recording (Esc)", "Hapus rekaman (Esc)")) {
                    action = Recording::Cancel;
                }
                if !recording {
                    ui.label(rich(tr("Getting the microphone ready…", "Menyiapkan mikrofon…"), 14.0, Weight::Regular, p.muted));
                    return;
                }
                let clock = format!("{}:{:02}", elapsed as u32 / 60, elapsed as u32 % 60);
                let right = if paused { 36.0 + 40.0 + 130.0 + 30.0 } else { 36.0 + 40.0 + 20.0 };
                if paused {
                    let listening = listen.is_some_and(|s| s.playing);
                    if round_button(ui, if listening { "pause" } else { "play" }, 36.0, p.primary, p.on_primary, None, tr("Listen (Space)", "Dengarkan (Spasi)")) {
                        action = Recording::Listen;
                    }
                    let played = listen.map(|s| s.position / s.duration.max(0.01)).unwrap_or(0.0);
                    let width = (ui.available_width() - right - 70.0).max(40.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
                    paint_note_wave(ui, rect, wave.as_deref().map(Vec::as_slice).unwrap_or(&[]), played, p.primary, p.border_strong);
                    let heard = listen.map(|s| format!("{}:{:02} / ", s.position as u32 / 60, s.position as u32 % 60)).unwrap_or_default();
                    ui.label(rich_mono(&format!("{heard}{clock}"), 13.0, p.text_2));
                } else {
                    // Blinks once a second while recording.
                    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    let on = (elapsed * 1000.0) as u64 % 1000 < 600;
                    ui.painter().circle_filled(dot.center(), 5.0, if on { p.red } else { p.red.gamma_multiply(0.35) });
                    ui.label(rich_mono(&clock, 14.0, p.text));
                    let width = (ui.available_width() - right).max(40.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
                    paint_live_wave(ui, rect, wave.as_deref().map(Vec::as_slice).unwrap_or(&[]), sampled.as_deref().copied().flatten(), p.primary);
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if round_button(ui, "send", 40.0, p.primary, p.on_primary, None, tr("Send voice message (Enter)", "Kirim pesan suara (Enter)")) {
                        action = Recording::Send;
                    }
                    if paused {
                        let resume = widgets::button(ui, Some("mic"), tr("Resume", "Lanjut rekam"), Kind::Secondary);
                        if resume.clicked() {
                            action = Recording::Resume;
                        }
                    } else if round_button(ui, "pause", 36.0, p.panel, p.text, Some(p.border_strong), tr("Pause (Space)", "Jeda (Spasi)")) {
                        action = Recording::Pause;
                    }
                });
            });
        });
    action
}

/// Playback of the paused note, when it is the one in the player.
fn player_status_preview() -> Option<crate::backend::player::Playback> {
    crate::backend::player::status(PREVIEW)
}


/// Starts recording a voice note for this chat; `held` for the phone
/// layout's hold-to-record.
fn start_voice(voice: &mut Option<VoiceNote>, session: &str, chat: &str, held: bool) {
    let recorder = crate::backend::voice::Recorder::start();
    let mut note = VoiceNote::new(session, chat, VoiceStep::Recording(recorder));
    note.held = held;
    *voice = Some(note);
}

/// The round button at the end of the phone composer (Pego `m5/VN 4`): send
/// when there is something to send, else the mic, which can also be held to
/// record. Returns true when Send was clicked.
#[allow(clippy::too_many_arguments)]
fn send_or_mic(ui: &mut Ui, p: &Palette, voice: &mut Option<VoiceNote>, session: &str, chat: &str, connected: bool, empty: bool, size: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), if connected { Sense::click_and_drag() } else { Sense::hover() });
    // The icon turns between send and mic with a short crossfade.
    let mic = ui.ctx().animate_bool_with_time(response.id.with("mic"), empty, 0.12);
    let fill = if connected { p.primary } else { p.border_strong };
    let grow = if response.hovered() && connected { 1.0 } else { 0.0 };
    ui.painter().circle_filled(rect.center(), size / 2.0 + grow, fill);
    if mic < 1.0 {
        let icon_rect = Rect::from_center_size(rect.center() + vec2(-1.0, 1.0), vec2(20.0, 20.0));
        icons::paint(ui, icon_rect, "send", 20.0, p.on_primary.gamma_multiply(1.0 - mic));
    }
    if mic > 0.0 {
        icons::paint(ui, Rect::from_center_size(rect.center(), vec2(20.0, 20.0)), "mic", 20.0, p.on_primary.gamma_multiply(mic));
    }
    if !connected {
        return false;
    }
    if !empty {
        return response.on_hover_text(tr("Send", "Kirim")).on_hover_cursor(CursorIcon::PointingHand).clicked();
    }
    // Held a moment: hold-to-record; a short click records hands-free.
    let held_long = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.press_start_time().is_some_and(|t| i.time - t > 0.25));
    if response.is_pointer_button_down_on() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }
    if held_long {
        start_voice(voice, session, chat, true);
    } else {
        let hint = tr("Record a voice message (hold to talk)", "Rekam pesan suara (tahan untuk bicara)");
        if response.on_hover_text(hint).on_hover_cursor(CursorIcon::PointingHand).clicked() {
            start_voice(voice, session, chat, false);
        }
    }
    false
}

/// The phone layout's hold-to-record (Pego `m5/VN 1`): a pill with the red
/// dot, the time and the live wave, the mic with a lime ring that grows with
/// the voice, a lock to slide up to, and "slide left to cancel".
fn held_bar(ui: &mut Ui, p: &Palette, note: &mut VoiceNote) -> Recording {
    let VoiceStep::Recording(recorder) = &note.step else { return Recording::Keep };
    let elapsed = recorder.elapsed().as_secs_f32();
    let level = recorder.level();
    if elapsed > 0.0 && note.sampled.is_none_or(|t| t.elapsed() >= WAVE_EVERY) {
        note.wave.push(level);
        note.sampled = Some(std::time::Instant::now());
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    // Where the pointer went since the press: left cancels, up locks.
    let (down, moved) = ui.input(|i| {
        let moved = i.pointer.press_origin().zip(i.pointer.latest_pos()).map(|(from, to)| to - from).unwrap_or_default();
        (i.pointer.primary_down(), moved)
    });
    let mut action = if !down {
        // Let go: a real note goes out, a slip of the finger does not.
        if elapsed >= 0.5 { Recording::Send } else { Recording::Cancel }
    } else if moved.x < -90.0 {
        Recording::Cancel
    } else if moved.y < -70.0 {
        Recording::Lock
    } else {
        Recording::Keep
    };
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        action = Recording::Cancel;
    }
    // The lock above the mic rises as the finger does.
    let rise = (-moved.y).clamp(0.0, 70.0) / 70.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let pill_w = ui.available_width() - 66.0;
        Frame::NONE.fill(p.raised).corner_radius(24).inner_margin(Margin { left: 14, right: 12, top: 8, bottom: 8 }).show(ui, |ui| {
            ui.set_width(pill_w - 26.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                let on = (elapsed * 1000.0) as u64 % 1000 < 600;
                ui.painter().circle_filled(dot.center(), 5.0, if on { p.red } else { p.red.gamma_multiply(0.35) });
                ui.label(rich_mono(&format!("{}:{:02}", elapsed as u32 / 60, elapsed as u32 % 60), 14.0, p.text));
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                paint_live_wave(ui, rect, &note.wave, note.sampled, p.primary);
            });
        });
        let (rect, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
        // The ring grows 1.0 to 1.35 times with the voice, smoothed over 120 ms.
        let loud = ((wave_height(level, 32.0) - 3.0) / 29.0).clamp(0.0, 1.0);
        let smooth = ui.ctx().animate_value_with_time(egui::Id::unique("voice_ring"), loud, 0.12);
        ui.painter().circle_filled(rect.center(), 26.0 * (1.0 + 0.35 * smooth), p.accent.gamma_multiply(0.85));
        ui.painter().circle_filled(rect.center(), 24.0, p.primary);
        icons::paint(ui, Rect::from_center_size(rect.center(), vec2(20.0, 20.0)), "mic", 20.0, p.on_primary);
        let lock = Rect::from_center_size(rect.center() - vec2(0.0, 64.0 + 20.0 * rise), vec2(34.0, 54.0));
        ui.painter().rect(lock, 17.0, p.panel, Stroke::new(1.0, p.border), StrokeKind::Inside);
        let lock_color = if rise > 0.9 { p.primary } else { p.text_2 };
        icons::paint(ui, Rect::from_center_size(lock.center() - vec2(0.0, 10.0), vec2(16.0, 16.0)), "lock", 16.0, lock_color);
        icons::paint(ui, Rect::from_center_size(lock.center() + vec2(0.0, 12.0), vec2(14.0, 14.0)), "chevron-up", 14.0, p.muted);
    });
    ui.vertical_centered(|ui| {
        let slide = (-moved.x).clamp(0.0, 90.0) / 90.0;
        let color = if slide > 0.6 { p.red_text } else { p.muted };
        ui.label(rich(tr("« Slide left to cancel", "« Geser ke kiri untuk batal"), 12.0, Weight::Regular, color));
    });
    action
}

/// Each frame, outside the composer: a voice note whose chat is no longer on
/// screen (back to the chat list, another page, its window closed) stops
/// recording, so the mic is released. One still saving is sent to its chat
/// once saved, or its file deleted when that number is offline.
pub fn voice_off_screen(ctx: &egui::Context, state: &mut State, sessions: &[SessionView], open: Option<(&str, &str)>) -> Vec<Command> {
    let Some(note) = state.voice.as_mut() else { return Vec::new() };
    if open == Some((note.session.as_str(), note.chat.as_str())) {
        return Vec::new();
    }
    let VoiceStep::Saving(saved) = &note.step else {
        state.voice = None;
        return Vec::new();
    };
    match saved.try_recv() {
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            Vec::new()
        }
        Ok(Ok(path)) => {
            let connected = sessions.iter().any(|s| s.id == note.session && s.status == Status::Connected);
            let VoiceNote { session, chat, .. } = state.voice.take().expect("checked above");
            if connected {
                vec![Command::SendVoice { id: session, chat, path }]
            } else {
                let _ = std::fs::remove_file(&path);
                Vec::new()
            }
        }
        _ => {
            state.voice = None;
            Vec::new()
        }
    }
}

/// Moves the composer's voice note along. `Some` holds what to send (to the
/// chat the note belongs to); `Err` hides the rest of the composer while the
/// note records or saves here.
fn voice_note(ui: &mut Ui, p: &Palette, state: &mut State, session: &SessionView, chat: &str, compact: bool) -> Result<Option<Outgoing>, ()> {
    let Some(note) = state.voice.as_mut() else { return Ok(None) };
    let here = note.session == session.id && note.chat == chat;
    let failed = |e: String| VoiceStep::Failed(e, std::time::Instant::now());
    match &mut note.step {
        // Saving goes on whichever chat is open, and is sent to its own.
        VoiceStep::Saving(saved) => match saved.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                if here {
                    recording_bar(ui, p, None, &session.name, compact);
                    return Err(());
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => note.step = failed(tr("The voice message could not be processed", "Pesan suara gagal diproses").to_owned()),
            Ok(Err(e)) => note.step = failed(e),
            Ok(Ok(path)) if here && session.status != Status::Connected => {
                // Not sent, so not kept.
                let _ = std::fs::remove_file(&path);
                note.step = failed(crate::trf!("{} is not connected, the voice message cannot be sent yet", "{} tidak terhubung, pesan suara belum bisa dikirim", session.name));
            }
            Ok(Ok(path)) => {
                let VoiceNote { session, chat, .. } = state.voice.take().expect("checked above");
                return Ok(Some(Outgoing::Voice { session, chat, path }));
            }
        },
        // Another chat opened: the recording (or its error) is left behind.
        _ if !here => state.voice = None,
        VoiceStep::Recording(recorder) => {
            if let crate::backend::voice::Mic::Failed(e) = recorder.poll() {
                note.step = failed(e);
                return Ok(None);
            }
            // The paused note arrived from the recording thread: play it.
            if let Some(pcm) = note.preview.as_ref().and_then(|r| r.try_recv().ok()) {
                note.preview = None;
                crate::backend::player::play_buffer(PREVIEW, pcm);
            }
            let asked = if note.held { held_bar(ui, p, note) } else { recording_bar(ui, p, Some(note), &session.name, compact) };
            match asked {
                Recording::Keep => {}
                Recording::Lock => {
                    if let Some(note) = state.voice.as_mut() {
                        note.held = false;
                    }
                }
                Recording::Cancel => {
                    crate::backend::player::forget(PREVIEW);
                    state.voice = None;
                }
                Recording::Pause => {
                    if let Some(VoiceNote { step: VoiceStep::Recording(recorder), .. }) = state.voice.as_mut() {
                        recorder.pause();
                    }
                }
                Recording::Resume => {
                    crate::backend::player::forget(PREVIEW);
                    if let Some(VoiceNote { step: VoiceStep::Recording(recorder), sampled, .. }) = state.voice.as_mut() {
                        recorder.resume();
                        *sampled = None;
                    }
                }
                Recording::Listen => {
                    if crate::backend::player::status(PREVIEW).is_some() {
                        crate::backend::player::toggle(PREVIEW, Path::new(""));
                    } else if let Some(note) = state.voice.as_mut() {
                        if let VoiceStep::Recording(recorder) = &note.step {
                            note.preview = Some(recorder.preview());
                        }
                    }
                }
                Recording::Send => {
                    crate::backend::player::forget(PREVIEW);
                    if let Some(note) = state.voice.as_mut() {
                        let VoiceStep::Recording(recorder) = std::mem::replace(&mut note.step, VoiceStep::Failed(String::new(), std::time::Instant::now())) else {
                            unreachable!("matched above")
                        };
                        note.step = VoiceStep::Saving(recorder.finish(crate::platform::data_dir().join("voice")));
                    }
                }
            }
            return Err(());
        }
        VoiceStep::Failed(_, since) => {
            let left = VOICE_ERROR_FOR.saturating_sub(since.elapsed());
            if left.is_zero() {
                state.voice = None;
            } else {
                ui.ctx().request_repaint_after(left);
            }
        }
    }
    Ok(None)
}

fn composer(
    ui: &mut Ui,
    p: &Palette,
    state: &mut State,
    to: &str,
    chat: &str,
    session: &SessionView,
    book: &Book,
    several: bool,
    compact: bool,
) -> Option<Outgoing> {
    let connected = session.status == Status::Connected;
    let id = ui.make_persistent_id("composer_text");
    match voice_note(ui, p, state, session, chat, compact) {
        Err(()) => return None,
        Ok(Some(voice)) => return Some(voice),
        Ok(None) => {}
    }
    let mut mention = mention_input(ui, state, book, session, chat, id);
    if let Some(rx) = &state.picking {
        match rx.try_recv() {
            Ok(picked) => {
                if picked.is_some() {
                    state.attachment = picked;
                }
                state.picking = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => state.picking = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    let (dropped, hovering) = ui.input(|i| {
        (i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| p.is_file()), !i.raw.hovered_files.is_empty())
    });
    if dropped.is_some() {
        state.attachment = dropped;
    }
    // Ctrl+V with a picture on the clipboard (a screenshot, a copied image)
    // attaches it like the paperclip does; text still pastes into the box.
    let pasted = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::PasteImage(image) => Some(image.clone()),
            _ => None,
        })
    });
    let focus = ui.memory(|m| m.focused());
    // Cmd+V on a file copied in Finder arrives as its name; attach the file
    // instead, as dropping it would, and keep the name out of the text box.
    let pasted_text = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(text) => Some(text.clone()),
            _ => None,
        })
    });
    if let Some(text) = pasted_text.filter(|_| focus.is_none() || focus == Some(id)) {
        if let Some(file) = copied_file(&text, &clipboard_files()).filter(|f| f.is_file()) {
            state.attachment = Some(file);
            ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(t) if *t == text)));
            ui.memory_mut(|m| m.request_focus(id));
        }
    }
    if let Some(image) = pasted.filter(|_| focus.is_none() || focus == Some(id)) {
        match save_pasted_image(&image, &crate::platform::data_dir().join("pasted")) {
            Ok(path) => {
                state.attachment = Some(path);
                ui.memory_mut(|m| m.request_focus(id));
            }
            Err(e) => log::warn!("cannot keep a pasted image: {e}"),
        }
    }
    // Enter belongs to the `@` picker while it is open; Shift+Enter is a new line.
    let mut send = connected && !mention.picked && ui.memory(|m| m.has_focus(id)) && ui.input_mut(enter_sends);
    let mut sendable = true;
    let mut sticker_sent = None;
    let mut edit_rect = None;
    let text = &mut state.composer;
    let border = if hovering { p.primary } else { p.border_strong };
    // Lays the typed text out like bubbles do, so emoji show in colour while typing.
    let text_color = p.text;
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = egui::text::LayoutJob::default();
        let format = egui::text::TextFormat::simple(fonts::ui(14.0, Weight::Regular), text_color);
        emoji::append(&mut job, buffer.as_str(), format.clone());
        if job.sections.is_empty() {
            // Keeps the line height (and caret) of an empty box.
            job.append("", 0.0, format);
        }
        job.wrap.max_width = wrap_width;
        ui.painter().fonts_mut(|f| f.layout_job(job))
    };
    if compact {
        // Phone (Pego m5/Chat): emoji, text and attachments in one pill, a round send button.
        sendable = attachment_chip(ui, p, &mut state.attachment);
        let hint = if state.attachment.is_some() { tr("Add a caption", "Tambahkan keterangan") } else { tr("Type a reply", "Ketik balasan") };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let pill_w = ui.available_width() - 56.0;
            let forest = theme::template().top_nav();
            let pill = Frame::NONE
                .fill(if forest { p.raised } else { p.panel })
                .stroke(if forest { Stroke::NONE } else { Stroke::new(1.0, border) })
                .corner_radius(24)
                .inner_margin(Margin { left: 8, right: 8, top: 6, bottom: 6 });
            pill.show(ui, |ui| {
                ui.set_width(pill_w - 16.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let smile = widgets::icon_button(ui, "smile", 32.0, false).on_hover_text("Emoji");
                    emoji::picker(ui, &smile, id, text);
                    let edit_w = (ui.available_width() - 104.0).max(60.0);
                    let edit = ui.add(
                        egui::TextEdit::multiline(text)
                            .id(id)
                            .layouter(&mut layouter)
                            .frame(Frame::NONE)
                            .hint_text(rich(hint, 15.0, Weight::Regular, p.muted))
                            .font(fonts::ui(14.0, Weight::Regular))
                            .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                            .event_filter(composer_filter(mention.picked))
                            .desired_rows(1)
                            .desired_width(edit_w),
                    );
                    edit_rect = Some(edit.rect);
                    let sticker = widgets::icon_button(ui, "sticker", 32.0, false).on_hover_text(tr("Sticker", "Stiker"));
                    sticker_sent = stickers::picker(ui, &sticker, book, connected);
                    let clip = widgets::icon_button(ui, "paperclip", 32.0, state.picking.is_some()).on_hover_text(tr("Attach a photo or document", "Lampirkan foto atau dokumen"));
                    if clip.clicked() && state.picking.is_none() {
                        state.picking = Some(pick_file(ui.ctx()));
                    }
                });
            });
            let empty = text.trim().is_empty() && state.attachment.is_none();
            send |= send_or_mic(ui, p, &mut state.voice, &session.id, chat, connected, empty, 48.0);
        });
        if !connected {
            ui.add_space(4.0);
            ui.label(rich(&crate::trf!("{} is not connected, messages cannot be sent yet", "{} tidak terhubung, pesan belum bisa dikirim", session.name), 12.0, Weight::Regular, p.red_text));
        }
    } else {
    Frame::NONE
        .fill(p.panel)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(R_LG)
        .inner_margin(Margin { left: 14, right: 12, top: 12, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            sendable = attachment_chip(ui, p, &mut state.attachment);
            let hint = if hovering {
                tr("Drop the file here to attach it", "Lepas file di sini untuk melampirkan").to_owned()
            } else if state.attachment.is_some() {
                tr("Add a caption (optional)", "Tambahkan keterangan (opsional)").to_owned()
            } else {
                crate::trf!("Reply to {to}", "Balas ke {to}")
            };
            let edit = ui.add(
                egui::TextEdit::multiline(text)
                    .id(id)
                    .layouter(&mut layouter)
                    .frame(Frame::NONE)
                    .hint_text(rich(&hint, 14.0, Weight::Regular, p.muted))
                    .font(fonts::ui(14.0, Weight::Regular))
                    .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                    .event_filter(composer_filter(mention.picked))
                    .desired_rows(1)
                    .desired_width(f32::INFINITY),
            );
            edit_rect = Some(edit.rect);
            ui.add_space(6.0);
            let empty = text.trim().is_empty() && state.attachment.is_none();
            widgets::row(
                ui,
                32.0,
                |ui| {
                    let clip = widgets::icon_button(ui, "paperclip", 28.0, state.picking.is_some())
                        .on_hover_text(tr("Attach a photo or document (or drag a file here)", "Lampirkan foto atau dokumen (atau seret file ke sini)"));
                    if clip.clicked() && state.picking.is_none() {
                        state.picking = Some(pick_file(ui.ctx()));
                    }
                    let smile = widgets::icon_button(ui, "smile", 28.0, false).on_hover_text("Emoji");
                    emoji::picker(ui, &smile, id, text);
                    let sticker = widgets::icon_button(ui, "sticker", 28.0, false).on_hover_text(tr("Sticker", "Stiker"));
                    sticker_sent = stickers::picker(ui, &sticker, book, connected);
                    ui.add_space(8.0);
                    let note = if connected && several {
                        crate::trf!(
                            "Replying from {} · locked to the recipient's number",
                            "Membalas dari {} · dikunci ke nomor penerima",
                            super::devices::chip_label(session)
                        )
                    } else if connected {
                        tr("Enter to send, Shift+Enter for a new line", "Enter kirim, Shift+Enter baris baru").to_owned()
                    } else {
                        crate::trf!("{} is not connected, messages cannot be sent yet", "{} tidak terhubung, pesan belum bisa dikirim", session.name)
                    };
                    let color = if connected { p.muted } else { p.red_text };
                    ui.add(egui::Label::new(rich(&note, 12.0, Weight::Regular, color)).truncate());
                },
                |ui| {
                    // Send turns into the mic while there is nothing to send.
                    if empty {
                        let button = ui.add_enabled_ui(connected, |ui| widgets::button(ui, Some("mic"), tr("Record", "Rekam"), Kind::Primary));
                        if connected && button.inner.on_hover_text(tr("Record a voice message", "Rekam pesan suara")).clicked() {
                            start_voice(&mut state.voice, &session.id, chat, false);
                        }
                    } else {
                        let button = ui.add_enabled_ui(connected, |ui| {
                            widgets::button(ui, Some("arrow-up"), tr("Send", "Kirim"), Kind::Primary)
                        });
                        send |= connected && button.inner.clicked();
                    }
                    ui.add_space(8.0);
                    let queue = if session.queue == 0 {
                        tr("Queued 0", "Antre 0").to_owned()
                    } else {
                        crate::trf!("Queued {} · ~4 s pause per message", "Antre {} · jeda ± 4 dtk per pesan", session.queue)
                    };
                    ui.label(rich(&queue, 12.0, Weight::Regular, p.muted));
                },
            );
        });
    }
    if let Some(rect) = edit_rect {
        mention_popup(ui, p, state, &mut mention, rect, id);
    }
    if let Some(VoiceNote { step: VoiceStep::Failed(error, _), .. }) = &state.voice {
        ui.add_space(4.0);
        ui.label(rich(error, 12.0, Weight::Regular, p.red_text));
    }
    if let Some(path) = sticker_sent {
        return Some(Outgoing::Sticker(path));
    }
    let message = state.composer.trim();
    if !send || !sendable || (message.is_empty() && state.attachment.is_none()) {
        return None;
    }
    let message = message.to_owned();
    state.composer.clear();
    let picks = std::mem::take(&mut state.mention_picks);
    ui.memory_mut(|m| m.request_focus(id));
    Some(match state.attachment.take() {
        Some(path) => Outgoing::File(path, message),
        None => {
            let (text, mentions) = wire_text(chat.ends_with("@g.us"), &message, &picks);
            Outgoing::Text(text, mentions)
        }
    })
}

/// The text to send and its mentioned JIDs. Only a group has mentions; a
/// private chat sends the text as typed, whatever picks are left over.
fn wire_text(group: bool, message: &str, picks: &[(String, String)]) -> (String, Vec<String>) {
    if group {
        mention::to_wire(message, picks)
    } else {
        (message.to_owned(), Vec::new())
    }
}

/// Keys the composer keeps for itself. egui moves focus on Tab and drops it on
/// Esc before our code runs, so while the picker is open it must hold both:
/// they are the picker's pick and close keys. Closed: TextEdit's own default.
fn composer_filter(picker_open: bool) -> egui::EventFilter {
    egui::EventFilter { tab: picker_open, horizontal_arrows: true, vertical_arrows: true, escape: picker_open }
}

/// The `@` picker for this frame: what it lists, and where the `@` sits.
#[derive(Default)]
struct MentionInput {
    /// Rows to show (empty while loading or closed).
    list: Vec<mention::Candidate>,
    /// Members not loaded yet: a "Memuat anggota…" row.
    loading: bool,
    /// The picker owns Enter (and Tab) this frame.
    picked: bool,
    /// Byte offset of the `@` and the cursor in characters.
    at: usize,
    cursor: usize,
}

/// Reads the cursor, decides whether the picker is open and handles its keys
/// (Up/Down, Enter/Tab, Esc) before the text box sees them.
fn mention_input(ui: &mut Ui, state: &mut State, book: &Book, session: &SessionView, chat_jid: &str, id: egui::Id) -> MentionInput {
    let mut out = MentionInput::default();
    let key = crate::backend::Typing::key(&session.id, chat_jid);
    if state.mention_chat != key {
        // Another chat (group or not): its picks do not apply.
        state.mention_picks.clear();
        state.mention_sel = 0;
        state.mention_dismissed = None;
        state.mention_query.clear();
        state.mention_chat = key.clone();
    }
    if !chat_jid.ends_with("@g.us") {
        return out;
    }
    if !ui.memory(|m| m.has_focus(id)) {
        return out;
    }
    let cursor = composer_cursor(ui, id, &state.composer);
    let Some((at, query)) = mention::active_query(&state.composer, cursor) else {
        state.mention_dismissed = None;
        return out;
    };
    if state.mention_dismissed == Some(at) {
        return out;
    }
    if query != state.mention_query {
        state.mention_sel = 0;
        state.mention_query = query.clone();
    }
    (out.at, out.cursor) = (at, cursor);
    match state.members_now.get(&key) {
        None => out.loading = true,
        Some(members) => out.list = mention::candidates(book, members, &query),
    }
    if out.list.is_empty() {
        return out;
    }
    out.picked = true;
    let len = out.list.len();
    state.mention_sel = state.mention_sel.min(len - 1);
    let (down, up, enter, tab, esc) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Tab),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    if down {
        state.mention_sel = (state.mention_sel + 1) % len;
    }
    if up {
        state.mention_sel = (state.mention_sel + len - 1) % len;
    }
    if esc {
        state.mention_dismissed = Some(at);
        out.list.clear();
        out.picked = false;
    } else if enter || tab {
        let chosen = state.mention_sel;
        apply_mention(ui, state, &out, chosen, id);
        out.list.clear();
    }
    out
}

/// The caret in characters, the end of the text when there is no caret yet.
fn composer_cursor(ui: &Ui, id: egui::Id, text: &str) -> usize {
    egui::TextEdit::load_state(ui.ctx(), id)
        .and_then(|st| st.cursor.char_range())
        .map_or(text.chars().count(), |r| r.primary.index.0)
}

/// Puts `@Name ` in place of the `@query`, remembers it, and moves the cursor
/// to just after it.
fn apply_mention(ui: &Ui, state: &mut State, input: &MentionInput, row: usize, id: egui::Id) {
    let Some(c) = input.list.get(row) else { return };
    // The text may have changed since the picker was computed: look again.
    let now = composer_cursor(ui, id, &state.composer);
    let Some((at, _)) = mention::active_query(&state.composer, now) else { return };
    let Some((text, cursor)) = mention::insert_pick(&state.composer, at, now, &c.name) else { return };
    state.composer = text;
    let pick = (format!("@{}", c.name), c.jid.clone());
    if !state.mention_picks.contains(&pick) {
        state.mention_picks.push(pick);
    }
    state.mention_dismissed = None;
    state.mention_query.clear();
    let mut edit = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    edit.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
    edit.store(ui.ctx(), id);
    ui.memory_mut(|m| m.request_focus(id));
    ui.ctx().request_repaint();
}

/// Draws the picker above the text box; a click on a row picks it.
fn mention_popup(ui: &mut Ui, p: &Palette, state: &mut State, input: &mut MentionInput, edit: Rect, id: egui::Id) {
    let rows = if !input.list.is_empty() {
        mention::Rows::List(&input.list)
    } else if input.loading {
        mention::Rows::Loading
    } else {
        return;
    };
    if let Some(row) = mention::popup(ui, p, edit.left_top(), edit.width(), rows, state.mention_sel) {
        apply_mention(ui, state, input, row, id);
    }
}

fn contact(ui: &mut Ui, p: &Palette, book: &Book, chat: &Chat, avatars: &Avatars, webhook: &str) {
    ui.spacing_mut().item_spacing.y = 20.0;
    let title = super::chat_title(book, chat);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        let initials = if chat.is_group() { "GR".to_owned() } else { super::initials(&title) };
        let picture = avatars.get(&chat.jid).and_then(|p| p.as_deref());
        widgets::avatar(ui, &initials, picture, 48.0, false);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(rich(&title, 16.0, Weight::SemiBold, p.text));
            ui.label(rich_mono(&super::chat_subtitle(chat), 12.0, p.muted));
        });
    });

    let total = chat.messages.len();
    let mine = chat.messages.iter().filter(|m| m.from_me).count();
    let since = chat.messages.first().map(|m| time::day_label(m.ts)).unwrap_or_else(|| "-".to_owned());
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
    ui.painter()
        .rect_stroke(strip, R_MD, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let cells = widgets::columns(strip, &[None; 3], 0.0);
    let values = [(total.to_string(), tr("messages", "pesan")), (mine.to_string(), tr("from you", "dari Anda")), (since, tr("since", "sejak"))];
    for (i, (cell, (value, key))) in cells.iter().zip(values).enumerate() {
        if i > 0 {
            ui.painter()
                .vline(cell.left(), strip.y_range().shrink(1.0), Stroke::new(1.0, p.border));
        }
        let inner = cell.shrink2(vec2(12.0, 8.0));
        let (top, bottom) = inner.split_top_bottom_at_fraction(0.55);
        widgets::paint_text(ui, top, &value, fonts::ui(15.0, Weight::SemiBold), p.text, Align::Min);
        widgets::paint_text(ui, bottom, key, fonts::ui(12.0, Weight::Regular), p.muted, Align::Min);
    }

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        widgets::section_label(ui, tr("Technical", "Teknis"));
        let kind = if chat.is_group() { tr("Group", "Grup") } else { tr("Private", "Pribadi") };
        let replied_first = chat.messages.iter().any(|m| !m.from_me);
        for (key, value) in [
            ("JID", super::short_jid(&chat.jid)),
            (tr("Type", "Tipe"), kind.to_owned()),
            (tr("Has messaged first", "Pernah chat duluan"), if replied_first { tr("Yes", "Ya").to_owned() } else { tr("Not yet", "Belum").to_owned() }),
            ("Webhook", webhook.to_owned()),
        ] {
            widgets::row(
                ui,
                18.0,
                |ui| {
                    ui.label(rich(key, 12.0, Weight::Regular, p.muted));
                },
                |ui| {
                    ui.label(rich_mono(&value, 12.0, p.text_2));
                },
            );
        }
    });
}

/// Inbox in the phone layout (Pego `m5/Inbox`, `m5/Chat`): the chat list, or
/// the open chat in its place with a back button.
pub fn compact(
    ui: &mut Ui,
    state: &mut State,
    sessions: &[SessionView],
    avatars: &Avatars,
    downloads: &Downloads,
    auto_stickers: bool,
) -> Vec<Command> {
    if state.inbox_scope.as_ref().is_some_and(|id| !sessions.iter().any(|s| &s.id == id)) {
        state.inbox_scope = None;
    }
    // Dev aid: WARUST_CHAT=first opens the newest chat once (for screenshots).
    static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var("WARUST_CHAT").as_deref() == Ok("first") && !FIRST.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some((s, c)) = sessions.iter().find_map(|s| s.book.chats.first().map(|c| (s, c))) {
            state.inbox_session = Some(s.id.clone());
            state.chat_selected = Some(c.jid.clone());
            FIRST.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let open = state
        .inbox_session
        .as_ref()
        .and_then(|id| sessions.iter().find(|s| &s.id == id))
        .and_then(|s| state.chat_selected.as_ref().and_then(|jid| s.book.chats.iter().find(|c| &c.jid == jid)).map(|c| (s, c)));
    match open {
        Some((session, chat)) => compact_chat(ui, state, session, chat, sessions.len() > 1, avatars, downloads, auto_stickers),
        None => {
            state.chat_selected = None;
            compact_list(ui, state, sessions, avatars)
        }
    }
}

const M_ROW: f32 = 72.0;
const M_ROW_VIA: f32 = 88.0;

fn compact_list(ui: &mut Ui, state: &mut State, sessions: &[SessionView], avatars: &Avatars) -> Vec<Command> {
    use crate::ui::compact::{self as phone, PAD};
    let p = theme::palette(ui.ctx());
    let forest = theme::template().top_nav();
    let mut commands = Vec::new();
    let several = sessions.len() > 1;
    let combined = state.inbox_scope.is_none() && several;
    Panel::top(panel_id(state, "m_inbox_head"))
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE.fill(p.bg).inner_margin(Margin { left: PAD as i8, right: PAD as i8, top: 12, bottom: 14 }))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            phone::page_title(ui, "Inbox", |ui| {
                if phone::round_button(ui, if state.m_search { "x" } else { "search" }, false).clicked() {
                    state.m_search = !state.m_search;
                    if !state.m_search {
                        state.chat_search.clear();
                    }
                }
            });
            if state.m_search {
                ui.add_space(10.0);
                widgets::search(ui, &mut state.chat_search, tr("Search name or number", "Cari nama atau nomor"), ui.available_width(), false);
            }
            if several {
                ui.add_space(14.0);
                // "Gabung semua" / "Per nomor" as two halves of one track.
                let (track, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
                ui.painter().rect_filled(track, 22.0, if forest { p.panel } else { p.raised });
                for (i, (icon, label)) in [("layers", tr("All together", "Gabung semua")), ("columns-2", tr("By number", "Per nomor"))].into_iter().enumerate() {
                    let half = Rect::from_min_size(pos2(track.left() + i as f32 * track.width() / 2.0, track.top()), vec2(track.width() / 2.0, 44.0));
                    let active = state.inbox_grouped == (i == 1);
                    let response = ui.interact(half, ui.scope_id().with(("m_mode", i)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                    if active {
                        ui.painter().rect_filled(half.shrink(4.0), 18.0, p.primary);
                    }
                    let color = if active { p.on_primary } else { p.text_2 };
                    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(15.0, Weight::SemiBold), color);
                    let total = galley.size().x + 24.0;
                    let left = half.center().x - total / 2.0;
                    icons::paint(ui, Rect::from_center_size(pos2(left + 8.0, half.center().y), vec2(16.0, 16.0)), icon, 16.0, color);
                    ui.painter().galley(pos2(left + 24.0, half.center().y - galley.size().y / 2.0), galley, color);
                    if response.clicked() {
                        state.inbox_grouped = i == 1;
                    }
                }
                ui.add_space(12.0);
                // Number filter: "Semua · 14", "CS Utama · 11", ...
                ScrollArea::horizontal().id_salt("m_scope").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let total: usize = sessions.iter().map(|s| s.book.chats.len()).sum();
                        if scope_chip(ui, &p, &crate::trf!("All · {total}", "Semua · {total}"), state.inbox_scope.is_none()).clicked() {
                            state.inbox_scope = None;
                        }
                        for s in sessions {
                            let label = format!("{} · {}", s.name, s.book.chats.len());
                            if scope_chip(ui, &p, &label, state.inbox_scope.as_deref() == Some(s.id.as_str())).clicked() {
                                state.inbox_scope = Some(s.id.clone());
                            }
                        }
                    });
                });
            }
        });
    let scoped: Vec<&SessionView> = match &state.inbox_scope {
        Some(id) => sessions.iter().filter(|s| &s.id == id).collect(),
        None => sessions.iter().collect(),
    };
    // The phone list has no tabs: "Diarsipkan" is a switch above the list.
    let archived_view = state.chat_tab == TAB_ARCHIVED;
    let archived = scoped.iter().map(|s| s.book.chats.iter().filter(|c| c.archived).count()).sum::<usize>();
    let lines = lines_for(state, &scoped, if archived_view { TAB_ARCHIVED } else { TAB_ALL }, combined && state.inbox_grouped);
    let via = combined && !state.inbox_grouped;
    CentralPanel::default().frame(Frame::NONE.fill(p.panel)).show(ui, |ui| {
        ui.painter().hline(ui.max_rect().x_range(), ui.max_rect().top(), Stroke::new(1.0, p.border));
        if archived_view || archived > 0 {
            let label = if archived_view {
                tr("← Back to all chats", "← Kembali ke semua chat").to_owned()
            } else {
                crate::trf!("Archived · {archived}", "Diarsipkan · {archived}")
            };
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                let link = ui.add(egui::Button::new(rich(&label, 13.0, Weight::Medium, p.accent_text)).frame(false));
                if link.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    state.chat_tab = if archived_view { TAB_ALL } else { TAB_ARCHIVED };
                }
            });
        }
        if lines.is_empty() {
            ui.add_space(32.0);
            let text = if sessions.is_empty() {
                tr("No numbers yet. Link a number in the Devices tab.", "Belum ada nomor. Tautkan nomor di tab Perangkat.")
            } else if archived_view {
                tr("No archived chats yet.", "Belum ada chat yang diarsipkan.")
            } else {
                tr("No chats here.", "Tidak ada chat di sini.")
            };
            ui.vertical_centered(|ui| ui.label(rich(text, 14.0, Weight::Regular, p.muted)));
            return;
        }
        let height = |line: &Line| match line {
            Line::Header(..) => HEADER,
            Line::Chat(..) if via => M_ROW_VIA,
            Line::Chat(..) => M_ROW,
        };
        let total: f32 = lines.iter().map(height).sum();
        ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let origin = ui.max_rect().min;
            let width = ui.available_width();
            let mut y = 0.0;
            for line in &lines {
                let h = height(line);
                let visible = y + h >= viewport.min.y && y <= viewport.max.y;
                let rect = Rect::from_min_size(pos2(origin.x, origin.y + y), vec2(width, h));
                y += h;
                if !visible {
                    continue;
                }
                match line {
                    Line::Header(s, count) => {
                        let folded = state.folded.contains(&s.id);
                        let clicked = section_header(ui, &p, rect.shrink2(vec2(PAD - 8.0, 0.0)), s, *count, folded, "");
                        header_action(state, s, clicked);
                    }
                    Line::Chat(s, chat) => {
                        if !avatars.contains_key(&chat.jid) && state.avatar_asked.insert(chat.jid.clone()) {
                            commands.push(Command::Avatar { id: s.id.clone(), jid: chat.jid.clone() });
                        }
                        let picture = avatars.get(&chat.jid).and_then(|p| p.as_deref());
                        let title = super::chat_title(&s.book, chat);
                        let row = compact_row(ui, &p, &s.book, rect, &title, chat, picture, &s.id, via.then_some(*s), typing_label(state, &s.id, &s.book, chat).as_deref());
                        chat_menu(&row, s, chat, &mut commands);
                        if row.clicked() {
                            open_chat(state, &s.id, &chat.jid, Some(chat));
                            if chat.unread > 0 {
                                commands.push(Command::Seen { id: s.id.clone(), chat: chat.jid.clone() });
                            }
                        }
                    }
                }
            }
        });
    });
    commands
}

fn scope_chip(ui: &mut Ui, p: &Palette, label: &str, active: bool) -> egui::Response {
    let forest = theme::template().top_nav();
    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(14.0, if active { Weight::SemiBold } else { Weight::Medium }), p.text);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 34.0), Sense::click());
    let (fill, color) = match (active, forest) {
        (true, true) => (p.accent, p.primary),
        (true, false) => (p.primary, p.on_primary),
        (false, _) => (p.panel, p.text),
    };
    ui.painter().rect_filled(rect, 17.0, fill);
    if !active && !forest {
        ui.painter().rect_stroke(rect, 17.0, Stroke::new(1.0, p.border), StrokeKind::Inside);
    }
    ui.painter().galley(pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0), galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A chat row of the phone list: round avatar, name, time, preview, unread
/// badge, and "ke CS Utama · 4410" when every number is combined.
#[allow(clippy::too_many_arguments)]
fn compact_row(ui: &Ui, p: &Palette, book: &Book, row: Rect, title: &str, chat: &Chat, picture: Option<&Path>, session: &str, via: Option<&SessionView>, typing: Option<&str>) -> egui::Response {
    use crate::ui::compact::{self as phone, PAD};
    let resp = ui.interact(row, ui.scope_id().with(("m_chat_row", session, &chat.jid)), Sense::click());
    let picture = picture.filter(|_| !crate::ui::privacy::hide_names());
    if resp.hovered() {
        ui.painter().rect_filled(row, 0.0, p.raised);
    }
    let tile = Rect::from_min_size(pos2(row.left() + PAD, row.top() + 14.0), vec2(44.0, 44.0));
    let initials = if chat.is_group() { "GR".to_owned() } else { super::initials(title) };
    phone::avatar(ui, tile, &initials, picture, chat.unread > 0);
    let left = tile.right() + 16.0;
    let right = row.right() - PAD;
    let when = ui.painter().layout_no_wrap(time::short(chat.last_ts()), fonts::ui(12.0, Weight::Regular), p.muted);
    let when_w = when.size().x;
    ui.painter().galley(pos2(right - when_w, row.top() + 16.0), when, p.muted);
    let name = Rect::from_min_max(pos2(left, row.top() + 12.0), pos2(right - when_w - 8.0, row.top() + 34.0));
    widgets::paint_text(ui, name, title, fonts::ui(16.0, Weight::SemiBold), p.text, Align::Min);
    let at = chat.mention_pending();
    let unread_w = if chat.unread > 9 { 28.0 } else { 20.0 };
    let reserve = if chat.unread > 0 { unread_w + 12.0 } else { 0.0 } + if at { 28.0 } else { 0.0 };
    let line = Rect::from_min_max(pos2(left, row.top() + 35.0), pos2(right - reserve, row.top() + 56.0));
    paint_preview(ui, p, book, line, chat, 14.0, p.text_2, typing);
    if at {
        let x = right - if chat.unread > 0 { unread_w + 6.0 + 20.0 } else { 20.0 };
        let badge = Rect::from_min_size(pos2(x, line.center().y - 10.0), vec2(20.0, 20.0));
        ui.painter().rect_filled(badge, 10.0, p.primary);
        widgets::paint_text(ui, badge, "@", fonts::ui(11.0, Weight::SemiBold), p.on_primary, Align::Center);
    }
    if chat.unread > 0 {
        let count = if chat.unread > 99 { "99+".to_owned() } else { chat.unread.to_string() };
        let w = if chat.unread > 9 { 28.0 } else { 20.0 };
        let badge = Rect::from_min_size(pos2(right - w, line.center().y - 10.0), vec2(w, 20.0));
        ui.painter().rect_filled(badge, 10.0, p.primary);
        widgets::paint_text(ui, badge, &count, fonts::ui(11.0, Weight::SemiBold), p.on_primary, Align::Center);
    }
    if let Some(session) = via {
        let y = row.top() + 58.0;
        let color = if theme::template().top_nav() { p.accent_text } else { p.text_2 };
        icons::paint(ui, Rect::from_min_size(pos2(left, y + 2.0), vec2(14.0, 14.0)), "smartphone", 13.0, color);
        let label = Rect::from_min_max(pos2(left + 18.0, y), pos2(right, y + 18.0));
        widgets::paint_text(ui, label, &crate::trf!("to {}", "ke {}", super::devices::chip_label(session)), fonts::ui(13.0, Weight::Medium), color, Align::Min);
    }
    ui.painter().hline((row.left() + PAD)..=(row.right() - PAD), row.bottom() - 0.5, Stroke::new(1.0, p.border));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[allow(clippy::too_many_arguments)]
fn compact_chat(
    ui: &mut Ui,
    state: &mut State,
    session: &SessionView,
    chat: &Chat,
    several: bool,
    avatars: &Avatars,
    downloads: &Downloads,
    auto_stickers: bool,
) -> Vec<Command> {
    use crate::ui::compact as phone;
    let p = theme::palette(ui.ctx());
    let mut commands = Vec::new();
    let book = &session.book;
    let title = super::chat_title(book, chat);
    let typing_now = typing_label(state, &session.id, book, chat);
    Panel::top(panel_id(state, "m_chat_header"))
        .exact_size(64.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE.fill(p.panel).inner_margin(Margin { left: 8, right: 8, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let bar = ui.max_rect().expand2(vec2(8.0, 0.0));
            ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.border));
            widgets::row(
                ui,
                64.0,
                |ui| {
                    if widgets::icon_button(ui, "chevron-left", 36.0, false).on_hover_text(tr("Back", "Kembali")).clicked() {
                        state.chat_selected = None;
                    }
                    let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                    let initials = if chat.is_group() { "GR".to_owned() } else { super::initials(&title) };
                    phone::avatar(ui, tile, &initials, avatars.get(&chat.jid).and_then(|p| p.as_deref()), true);
                    ui.add_space(4.0);
                    let (text, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                    let top = Rect::from_min_max(text.min, pos2(text.right(), text.top() + 22.0));
                    widgets::paint_text(ui, top, &title, fonts::ui(16.0, Weight::SemiBold), p.text, Align::Min);
                    let bottom = Rect::from_min_max(pos2(text.left(), text.top() + 23.0), text.max);
                    if several {
                        let color = if theme::template().top_nav() { p.accent_text } else { p.text_2 };
                        icons::paint(ui, Rect::from_min_size(pos2(bottom.left(), bottom.center().y - 6.0), vec2(12.0, 12.0)), "arrow-down-left", 12.0, color);
                        let label = Rect::from_min_max(pos2(bottom.left() + 16.0, bottom.top()), bottom.max);
                        widgets::paint_text(ui, label, &crate::trf!("Chat to {}", "Chat masuk ke {}", super::devices::chip_label(session)), fonts::ui(12.0, Weight::Medium), color, Align::Min);
                    } else if let Some(typing) = &typing_now {
                        widgets::paint_text(ui, bottom, typing, fonts::ui(12.0, Weight::Medium), typing_color(&p), Align::Min);
                    } else {
                        widgets::paint_text(ui, bottom, &super::chat_subtitle(chat), fonts::mono(12.0), p.muted, Align::Min);
                    }
                },
                |ui| {
                    let menu = widgets::icon_button(ui, "ellipsis-vertical", 36.0, false);
                    egui::Popup::menu(&menu).show(|ui| {
                        ui.set_min_width(200.0);
                        if ui.button(tr("Load older messages from phone", "Muat pesan lama dari HP")).clicked() {
                            state.older_asked = Some((chat.jid.clone(), std::time::Instant::now()));
                            commands.push(Command::LoadOlder { id: session.id.clone(), chat: chat.jid.clone() });
                        }
                        if ui.button(tr("Copy number", "Salin nomor")).clicked() {
                            ui.ctx().copy_text(chat.jid.split('@').next().unwrap_or_default().to_owned());
                        }
                        if !chat.is_group() || chat.mention_pending() || chat.is_done() {
                            let done = chat.is_done();
                            if ui.button(if done { tr("Mark needs reply", "Tandai perlu balas") } else { tr("Mark done", "Tandai selesai") }).clicked() {
                                commands.push(Command::MarkDone { id: session.id.clone(), chat: chat.jid.clone(), done: !done });
                            }
                        }
                    });
                },
            );
        });
    Panel::bottom(panel_id(state, "m_composer"))
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE.fill(p.panel).inner_margin(Margin { left: 12, right: 12, top: 10, bottom: 10 }))
        .show(ui, |ui| {
            let bar = ui.max_rect().expand2(vec2(12.0, 10.0));
            ui.painter().hline(bar.x_range(), bar.top() + 0.5, Stroke::new(1.0, p.border));
            if several {
                // "Membalas dari CS Utama · 4410": replies are bound to this number.
                ui.vertical_centered(|ui| {
                    let forest = theme::template().top_nav();
                    let (fill, color) = if forest { (p.accent_soft, p.accent_text) } else { (p.raised, p.text_2) };
                    Frame::NONE.fill(fill).corner_radius(16).inner_margin(Margin::symmetric(14, 6)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            icons::show(ui, "lock", 14.0, color);
                            ui.label(rich(&crate::trf!("Replying from {}", "Membalas dari {}", super::devices::chip_label(session)), 13.0, Weight::SemiBold, color));
                        });
                    });
                });
                ui.add_space(8.0);
            }
            let (id, jid) = (session.id.clone(), chat.jid.clone());
            reply_bar(ui, &p, state, &id, &jid);
            match composer(ui, &p, state, &title, &jid, session, book, several, true) {
                Some(Outgoing::Text(text, mentions)) => commands.push(text_command(state, id, jid, text, mentions)),
                Some(Outgoing::File(path, caption)) => commands.push(Command::SendFile { id, chat: jid, path, caption }),
                Some(Outgoing::Sticker(path)) => commands.push(Command::SendSticker { id, chat: jid, path }),
                Some(Outgoing::Voice { session, chat, path }) => commands.push(Command::SendVoice { id: session, chat, path }),
                None => {}
            }
        });
    CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
        widgets::wallpaper(ui, ui.max_rect());
        Frame::NONE.inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 6 }).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            if older_bar(ui, &p, state, session, chat) {
                state.older_asked = Some((chat.jid.clone(), std::time::Instant::now()));
                commands.push(Command::LoadOlder { id: session.id.clone(), chat: chat.jid.clone() });
            }
            pinned_bar(ui, &p, chat);
            let mut want = Vec::new();
            let own = (session.own_number.as_deref(), session.own_lid.as_deref());
            let event = messages(ui, &p, book, chat, downloads, avatars, &mut want, auto_stickers, typing_label(state, &session.id, book, chat).as_deref(), own, &mut state.unread_anchor);
            ask_avatars(state, &mut commands, &session.id, want);
            if let Some(event) = event {
                on_message_event(event, state, session, chat, &mut commands);
            }
        });
    });
    commands
}

#[cfg(test)]
mod wave_tests {
    use super::*;

    #[test]
    fn wave_heights_are_log_scaled() {
        assert_eq!(wave_height(0.0, 32.0), 3.0);
        assert_eq!(wave_height(1.0, 32.0), 32.0);
        // Quiet speech (about -30 dB) is already well above the dot.
        assert!(wave_height(0.03, 32.0) > 12.0);
        assert_eq!(voice_bars(&[]).len(), VOICE_BARS);
        let bars = voice_bars(&(0..64).map(|i| i as u8).collect::<Vec<_>>());
        assert_eq!(bars.len(), VOICE_BARS);
        assert!(bars.windows(2).all(|w| w[0] <= w[1]));
    }
}

#[cfg(test)]
mod finder_paste_tests {
    use super::*;

    #[test]
    fn finder_copy_is_a_file_not_its_name() {
        let photo = PathBuf::from("/Users/a/Desktop/foto liburan.png");
        let doc = PathBuf::from("/Users/a/nota.pdf");
        // Finder puts the names on the clipboard as text, one per file.
        assert_eq!(copied_file("foto liburan.png", std::slice::from_ref(&photo)), Some(photo.clone()));
        assert_eq!(copied_file("foto liburan.png\rnota.pdf", &[photo.clone(), doc.clone()]), Some(photo.clone()));
        // Linux file managers give the full path.
        assert_eq!(copied_file("/Users/a/nota.pdf\n", std::slice::from_ref(&doc)), Some(doc.clone()));
        // Text that is not those names stays text.
        assert_eq!(copied_file("lihat foto liburan.png ya", std::slice::from_ref(&photo)), None);
        assert_eq!(copied_file("foto liburan.png", &[]), None);
        assert_eq!(copied_file("", std::slice::from_ref(&photo)), None);
    }
}

#[cfg(test)]
mod paste_tests {
    use super::*;

    #[test]
    fn pasted_image_is_kept_as_png() {
        let dir = std::env::temp_dir().join(format!("warust-paste-{}", std::process::id()));
        let image = egui::ColorImage::new([3, 2], vec![egui::Color32::from_rgb(29, 90, 63); 6]);
        let first = save_pasted_image(&image, &dir).unwrap();
        let second = save_pasted_image(&image, &dir).unwrap();
        assert_ne!(first, second, "two pastes in the same second keep both");
        let back = image::open(&first).unwrap().to_rgba8();
        assert_eq!(back.dimensions(), (3, 2));
        assert_eq!(back.get_pixel(0, 0).0, [29, 90, 63, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod typing_tests {
    use super::*;

    fn kinds(commands: &[Command]) -> Vec<(String, bool)> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::Typing { chat, typing, .. } => Some((chat.clone(), *typing)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn contact_typing_label() {
        use crate::backend::{Book, Chat, Typing};
        use std::time::{Duration, Instant};
        let chat = Chat { jid: "62811@s.whatsapp.net".to_owned(), unread: 0, messages: Vec::new(), done_at: None, archived: false, pins: Vec::new() };
        let book = Book::default();
        let mut state = State::default();
        assert_eq!(typing_label(&state, "s1", &book, &chat), None);
        let mut map = std::collections::HashMap::new();
        map.insert(Typing::key("s1", &chat.jid), Typing { who: None, recording: false, since: Instant::now() });
        state.typing_now = std::sync::Arc::new(map.clone());
        assert_eq!(typing_label(&state, "s1", &book, &chat).as_deref(), Some("typing…"));
        assert_eq!(typing_label(&state, "s2", &book, &chat), None, "another number's chat");
        map.insert(Typing::key("s1", &chat.jid), Typing { who: None, recording: true, since: Instant::now() - Typing::STALE - Duration::from_secs(1) });
        state.typing_now = std::sync::Arc::new(map);
        assert_eq!(typing_label(&state, "s1", &book, &chat), None, "stale");
    }

    #[test]
    fn leftover_picks_do_not_leak_into_a_private_chat() {
        let picks = vec![("@Budi".to_owned(), "6281@s.whatsapp.net".to_owned())];
        let (text, men) = wire_text(false, "halo @Budi", &picks);
        assert_eq!(text, "halo @Budi");
        assert!(men.is_empty());
        let (text, men) = wire_text(true, "halo @Budi", &picks);
        assert_eq!(text, "halo @6281");
        assert_eq!(men, ["6281@s.whatsapp.net"]);
    }

    #[test]
    fn members_are_asked_again_after_a_pause_until_they_arrive() {
        use crate::backend::Typing;
        use std::time::{Duration, Instant};
        let mut state = State::default();
        let t0 = Instant::now();
        assert!(members_due(&state, "s1", "g@g.us", t0));
        state.members_asked.insert(("s1".to_owned(), "g@g.us".to_owned()), t0);
        assert!(!members_due(&state, "s1", "g@g.us", t0 + Duration::from_secs(5)));
        assert!(members_due(&state, "s1", "g@g.us", t0 + MEMBERS_RETRY));
        let mut map = std::collections::HashMap::new();
        map.insert(Typing::key("s1", "g@g.us"), Vec::new());
        state.members_now = std::sync::Arc::new(map);
        assert!(!members_due(&state, "s1", "g@g.us", t0 + MEMBERS_RETRY * 2));
    }

    #[test]
    fn typing_starts_repeats_and_stops() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        let open = || Some(("s1".to_owned(), "a@s.whatsapp.net".to_owned()));
        state.composer = "hal".to_owned();
        assert_eq!(kinds(&typing_signal(&ctx, &mut state, open())), [("a@s.whatsapp.net".to_owned(), true)]);
        // More keys right away: no second "typing" yet.
        state.composer = "halo".to_owned();
        assert!(kinds(&typing_signal(&ctx, &mut state, open())).is_empty());
        // A pause stops it.
        state.typing_changed = Some(std::time::Instant::now() - TYPING_IDLE);
        assert_eq!(kinds(&typing_signal(&ctx, &mut state, open())), [("a@s.whatsapp.net".to_owned(), false)]);
        // Typing again, then the text is sent (cleared): stops at once.
        state.composer = "halo kak".to_owned();
        assert_eq!(kinds(&typing_signal(&ctx, &mut state, open())), [("a@s.whatsapp.net".to_owned(), true)]);
        state.composer.clear();
        assert_eq!(kinds(&typing_signal(&ctx, &mut state, open())), [("a@s.whatsapp.net".to_owned(), false)]);
        // Leaving the chat while typing stops it too.
        state.composer = "x".to_owned();
        typing_signal(&ctx, &mut state, open());
        assert_eq!(kinds(&typing_signal(&ctx, &mut state, None)), [("a@s.whatsapp.net".to_owned(), false)]);
    }
}

#[cfg(test)]
mod member_color_tests {
    use super::member_color;

    #[test]
    fn same_member_same_colour_and_members_differ() {
        let a = "628111@s.whatsapp.net";
        assert_eq!(member_color(a, true), member_color(a, true));
        assert_ne!(member_color(a, true), member_color(a, false), "dark and light variants");
        let distinct: std::collections::HashSet<_> =
            (0..40).map(|n| member_color(&format!("62812{n:04}@s.whatsapp.net"), true)).collect();
        assert!(distinct.len() >= 8, "members spread over the palette: {}", distinct.len());
    }
}

#[cfg(test)]
mod enter_tests {
    use super::*;

    /// Whether one frame with Enter pressed under `modifiers` asks to send.
    fn sends(modifiers: Modifiers) -> bool {
        let ctx = egui::Context::default();
        let event = egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers };
        let input = egui::RawInput { events: vec![egui::Event::ModifiersChanged(modifiers), event], ..Default::default() };
        let mut sent = false;
        ctx.run_ui(input, |ui| sent = ui.input_mut(enter_sends)).textures_delta.clear();
        sent
    }

    #[test]
    fn only_plain_enter_sends() {
        assert!(sends(Modifiers::NONE));
        assert!(!sends(Modifiers::SHIFT), "Shift+Enter is a new line");
        assert!(!sends(Modifiers::ALT));
    }
}
