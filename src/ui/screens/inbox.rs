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
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette, R_LG, R_MD};
use crate::ui::widgets::{self, Kind, rich, rich_mono};
use crate::ui::{ReplyTarget, State, emoji, forward, icons, markup, pdf, stickers, time};

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
            .show(ui, |ui| super::empty_state(ui, &p, "inbox", "Belum ada nomor", "Tautkan nomor di menu Nomor untuk mulai menerima pesan."));
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
        Panel::left("devices")
            .default_size(if forest { 250.0 } else { 230.0 })
            .size_range(210.0..=320.0)
            .resizable(true)
            .show_separator_line(!forest)
            .frame(theme::pane(ui.ctx(), p.panel, Margin { left: 10, right: 10, top: 0, bottom: 10 }))
            .show(ui, |ui| super::devices::show(ui, &p, state, sessions));
    }
    Panel::left("chat_list")
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
                    "Belum ada chat",
                    "Pesan masuk akan muncul di sini. Riwayat lama hanya dikirim WhatsApp saat pairing.",
                )
            } else {
                ("Pilih chat", "Pilih percakapan di sebelah kiri untuk membaca dan membalas.")
            };
            super::empty_state(ui, &p, "inbox", title, desc);
        });
        return commands;
    };

    // The contact panel only when the conversation keeps room to breathe.
    if state.contact_panel && ui.available_width() >= 980.0 {
        Panel::right("contact")
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
    Panel::top("conversation_header")
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
                        ui.label(rich("masuk ke", 12.0, Weight::Regular, p.muted));
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
                    if !chat.is_group() {
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
    Panel::bottom("composer")
        .resizable(false)
        .frame(Frame::NONE.fill(surface).corner_radius(theme::bottom_corners()).inner_margin(Margin { left: 32, right: 32, top: 8, bottom: 20 }))
        .show_separator_line(false)
        .show(ui, |ui| {
            let to = super::chat_title(&book, chat);
            let (id, jid) = (session.id.clone(), chat.jid.clone());
            reply_bar(ui, &p, state, &jid);
            match composer(ui, &p, state, &to, session, &book, several, false) {
                Some(Outgoing::Text(text)) => commands.push(text_command(state, id, jid, text)),
                Some(Outgoing::File(path, caption)) => commands.push(Command::SendFile { id, chat: jid, path, caption }),
                Some(Outgoing::Sticker(path)) => commands.push(Command::SendSticker { id, chat: jid, path }),
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
                if older_bar(ui, &p, state, session, chat) {
                    state.older_asked = Some((chat.jid.clone(), std::time::Instant::now()));
                    commands.push(Command::LoadOlder { id: session.id.clone(), chat: chat.jid.clone() });
                }
                pinned_bar(ui, &p, chat);
                if let Some(event) = messages(ui, &p, &book, chat, downloads, auto_stickers, typing_label(state, &session.id, &book, chat).as_deref()) {
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
    // Opening a private chat asks to hear its typing, once per run.
    if let Some((session, chat)) = &open {
        if !chat.ends_with("@g.us") && state.presence_watched.insert((session.clone(), chat.clone())) {
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
    let what = if typing.recording { "merekam suara…" } else { "mengetik…" };
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
        ("check", "Selesai", "Tandai perlu balas lagi")
    } else {
        ("check", "Tandai selesai", "Pindahkan ke tab Selesai sampai ada pesan baru")
    };
    let kind = if done { Kind::Primary } else { Kind::Secondary };
    widgets::button(ui, Some(icon), label, kind).on_hover_text(hint).clicked().then_some(!done)
}

/// The soft orange "Belum dibalas" tag (Pego `v5/Inbox · Chat`).
fn reply_badge(ui: &mut Ui, p: &Palette) {
    let galley = ui.painter().layout_no_wrap("Belum dibalas".to_owned(), fonts::ui(12.0, Weight::SemiBold), theme::FOREST_ORANGE);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, 24.0), Sense::hover());
    let (fill, color) = if theme::template().top_nav() { (p.orange_soft, theme::FOREST_ORANGE) } else { (p.raised, p.amber) };
    ui.painter().rect_filled(rect, 12.0, fill);
    widgets::paint_text(ui, rect, "Belum dibalas", fonts::ui(12.0, Weight::SemiBold), color, Align::Center);
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
                "Meminta riwayat ke HP. HP harus online; pesan lama muncul begitu dikirim.",
                12.0,
                Weight::Regular,
                p.muted,
            ));
        } else if session.status == Status::Connected {
            clicked = widgets::button(ui, Some("download"), "Muat pesan lama dari HP", Kind::Secondary).clicked();
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
        None if state.inbox_grouped && several => "Per nomor".to_owned(),
        None => "Semua nomor".to_owned(),
    };
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(rich(&title, 17.0, Weight::SemiBold, p.text));
            let numbers = if combined { format!("{} nomor · ", scoped.len()) } else { String::new() };
            ui.label(rich(&format!("{numbers}{} percakapan · {unread} belum dibaca", chats.len()), 12.0, Weight::Regular, p.muted));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.add_space(4.0);
            let hint = if state.devices_panel { "Sembunyikan panel Perangkat" } else { "Tampilkan panel Perangkat" };
            if widgets::icon_button(ui, "panel-left", 32.0, state.devices_panel).on_hover_text(hint).clicked() {
                state.devices_panel = !state.devices_panel;
            }
        });
    });
    ui.add_space(6.0);
    if combined {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let mut mode = usize::from(state.inbox_grouped);
            widgets::tabs(ui, &["Gabung semua nomor", "Pisah per nomor"], &mut mode);
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
        let labels = [format!("Perlu balas · {waiting}"), format!("Semua · {}", chats.len()), "Selesai".to_owned(), format!("Diarsipkan · {archived}")];
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        widgets::tabs(ui, &labels, &mut state.chat_tab);
    });
    // Classic tabs sit on a rule; Forest's pills need none.
    if !theme::template().top_nav() {
        widgets::hline(ui, ui.max_rect(), ui.cursor().top() - 2.0, p.border);
    }
    ui.add_space(6.0);
    widgets::search(ui, &mut state.chat_search, "Cari nama atau nomor", ui.available_width(), false);
    ui.add_space(4.0);

    let lines = lines_for(state, &scoped, state.chat_tab, combined && state.inbox_grouped);
    let via = combined && !state.inbox_grouped;
    let height = |line: &Line| match line {
        Line::Header(..) => HEADER,
        Line::Chat(..) if via => ROW_VIA,
        Line::Chat(..) => ROW,
    };
    let total: f32 = lines.iter().map(|l| height(l) + 2.0).sum();
    if lines.is_empty() {
        ui.add_space(20.0);
        let empty = match state.chat_tab {
            TAB_REPLY => "Semua chat sudah dibalas.",
            TAB_DONE => "Belum ada chat yang ditandai selesai.",
            TAB_ARCHIVED => "Belum ada chat yang diarsipkan.",
            _ => "Tidak ada chat di sini.",
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
        for line in &lines {
            let h = height(line);
            let visible = y + h >= viewport.min.y && y <= viewport.max.y;
            let rect = Rect::from_min_size(pos2(origin.x, origin.y + y), vec2(width, h));
            y += h + 2.0;
            if !visible {
                continue;
            }
            match line {
                Line::Header(s, count) => section_header(ui, p, rect, s, *count),
                Line::Chat(s, chat) => {
                    // Pictures are asked only for rows on screen; the backend paces them.
                    if !avatars.contains_key(&chat.jid) && state.avatar_asked.insert(chat.jid.clone()) {
                        commands.push(Command::Avatar { id: s.id.clone(), jid: chat.jid.clone() });
                    }
                    let picture = avatars.get(&chat.jid).and_then(|p| p.as_deref());
                    let selected = state.inbox_session.as_deref() == Some(s.id.as_str())
                        && state.chat_selected.as_deref() == Some(chat.jid.as_str());
                    let title = super::chat_title(&s.book, chat);
                    let via_number = via.then_some(*s);
                    let row = chat_row(ui, p, rect, &title, chat, picture, selected, via_number, typing_label(state, &s.id, &s.book, chat).as_deref());
                    chat_menu(&row, s, chat, &mut commands);
                    if row.clicked() {
                        state.inbox_session = Some(s.id.clone());
                        state.chat_selected = Some(chat.jid.clone());
                        if chat.unread > 0 {
                            commands.push(Command::Seen { id: s.id.clone(), chat: chat.jid.clone() });
                        }
                    }
                }
            }
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
            lines.extend(mine.into_iter().map(|c| Line::Chat(s, c)));
        }
    } else {
        let mut mine: Vec<(&SessionView, &Chat)> =
            scoped.iter().flat_map(|s| s.book.chats.iter().map(move |c| (*s, c))).filter(|(s, c)| keep(s, c)).collect();
        mine.sort_by_key(|(_, c)| std::cmp::Reverse(c.last_ts()));
        lines.extend(mine.into_iter().map(|(s, c)| Line::Chat(s, c)));
    }
    lines
}

/// Right-click menu of a chat row (wide and phone lists): read state and archive.
fn chat_menu(row: &egui::Response, session: &SessionView, chat: &Chat, commands: &mut Vec<Command>) {
    row.context_menu(|ui| {
        ui.set_min_width(180.0);
        let (id, jid) = (session.id.clone(), chat.jid.clone());
        if chat.unread > 0 {
            if ui.button("Tandai sudah dibaca").clicked() {
                commands.push(Command::Seen { id: id.clone(), chat: jid.clone() });
            }
        } else if ui.button("Tandai belum dibaca").clicked() {
            commands.push(Command::MarkUnread { id: id.clone(), chat: jid.clone() });
        }
        if ui.button(if chat.archived { "Keluarkan dari arsip" } else { "Arsipkan" }).clicked() {
            commands.push(Command::Archive { id, chat: jid, archived: !chat.archived });
        }
    });
}

/// "● CS Utama  4410 ........ 3 chat", red when the number cannot reply.
fn section_header(ui: &Ui, p: &Palette, rect: Rect, session: &SessionView, count: usize) {
    let band = rect.shrink2(vec2(0.0, 4.0));
    let down = session.status != Status::Connected;
    ui.painter().rect_filled(band, theme::r_inner(), if down { p.red_soft } else { p.raised });
    let dot = super::devices::status_dot(p, &session.status);
    ui.painter().circle_filled(pos2(band.left() + 14.0, band.center().y), 3.5, dot);
    let name = Rect::from_min_max(pos2(band.left() + 26.0, band.top()), pos2(band.right() - 90.0, band.bottom()));
    widgets::paint_text(ui, name, &super::devices::chip_label(session), fonts::ui(13.0, Weight::SemiBold), p.text, Align::Min);
    let right = Rect::from_min_max(pos2(band.right() - 90.0, band.top()), pos2(band.right() - 12.0, band.bottom()));
    let (label, color) = if down {
        let (_, _, label, _) = super::nomor::describe(p, &session.status);
        (label, p.red)
    } else {
        (format!("{count} chat"), p.muted)
    };
    widgets::paint_text(ui, right, &label, fonts::ui(12.0, Weight::Medium), color, Align::Max);
}

/// "Percakapan ini terikat ke CS Utama…" above the messages, with several numbers.
fn bound_banner(ui: &mut Ui, p: &Palette, session: &SessionView) {
    let phone = session.phone.clone().unwrap_or_else(|| "nomor ini".to_owned());
    let text = format!("Percakapan ini terikat ke {}. Semua balasan dikirim dari {phone}.", session.name);
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
    row: Rect,
    title: &str,
    chat: &Chat,
    picture: Option<&Path>,
    selected: bool,
    via: Option<&SessionView>,
    typing: Option<&str>,
) -> egui::Response {
    let resp = ui.interact(row, ui.scope_id().with(("chat_row", via.map(|s| s.id.as_str()), &chat.jid)), Sense::click());
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
    let reserve = if chat.unread > 0 { 30.0 } else { 0.0 };
    let preview_rect = Rect::from_min_max(bottom.min, pos2(right - reserve, bottom.bottom()));
    paint_preview(ui, p, preview_rect, chat, 13.0, p.muted, typing);
    if chat.unread > 0 {
        let badge = Rect::from_min_size(pos2(right - 22.0, bottom.center().y - 9.0), vec2(22.0, 18.0));
        ui.painter().rect_filled(badge, 9.0, p.primary);
        let count = if chat.unread > 99 { "99+".to_owned() } else { chat.unread.to_string() };
        widgets::paint_text(ui, badge, &count, fonts::ui(11.0, Weight::SemiBold), p.on_primary, Align::Center);
    }
    if let Some(session) = via {
        let label = ui.painter().layout_no_wrap("masuk ke".to_owned(), fonts::ui(11.0, Weight::Regular), p.muted);
        let label_w = label.size().x;
        let y = row.top() + 54.0;
        ui.painter().galley(pos2(left, y + 10.0 - label.size().y / 2.0), label, p.muted);
        number_chip(ui, p, pos2(left + label_w + 6.0, y), session);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// WhatsApp's blue "read" ticks.
const READ_BLUE: Color32 = Color32::from_rgb(0x53, 0xBD, 0xEB);

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
fn paint_preview(ui: &Ui, p: &Palette, rect: Rect, chat: &Chat, size: f32, color: Color32, typing: Option<&str>) {
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
    widgets::paint_text(ui, rect, &m.text.replace('\n', " "), fonts::ui(size, Weight::Regular), color, Align::Min);
}

pub fn delivery_label(status: &Delivery) -> &'static str {
    match status {
        Delivery::Received => "",
        Delivery::Queued => "Antre",
        Delivery::Sent => "Terkirim",
        Delivery::Delivered => "Diterima",
        Delivery::Read => "Dibaca",
        Delivery::Failed(_) => "Gagal",
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
        }
    }
}

fn media_box(media: &Media, wrap: f32) -> MediaBox {
    if media.kind == "sticker" {
        return MediaBox::Sticker(media.file.clone());
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

fn media_title(media: &Media) -> String {
    if let Some(name) = media.name.as_ref().filter(|n| !n.is_empty()) {
        return name.clone();
    }
    match media.kind.as_str() {
        "image" => "Foto",
        "video" => "Video",
        "sticker" => "Stiker",
        "audio" => "Pesan suara",
        _ => "Dokumen",
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

/// What clicking the media does next, as a short label.
fn media_action(m: &ChatMsg, media: &Media, downloads: &Downloads) -> &'static str {
    if media.file.is_some() {
        return "Buka";
    }
    match downloads.get(&m.id) {
        Some(None) => "Mengunduh…",
        Some(Some(_)) => "Gagal, ulangi",
        None if media.downloadable() => "Unduh",
        None => "Tidak tersedia",
    }
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
        "image" | "video" => "image",
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
    let action = media_action(m, media, downloads);
    let response = ui.interact(rect, ui.scope_id().with(("media", &m.id)), Sense::click());
    match shown {
        MediaBox::Picture(source, _) => {
            if let Some(uri) = widgets::picture_uri(ui.ctx(), source) {
                egui::Image::from_uri(uri).corner_radius(R_MD).fit_to_exact_size(rect.size()).paint_at(ui, rect);
            } else {
                ui.painter().rect_filled(rect, R_MD, p.border);
            }
            if action != "Buka" {
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
                let text = if action == "Unduh" { "Memuat…" } else { action };
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
                parts.push(format!("{} halaman", info.pages));
            }
            if let Some(size) = media.size {
                parts.push(human_size(size));
            }
            parts.push(if action == "Buka" && file.is_some() { "Lihat".to_owned() } else { action.to_owned() });
            paint_card(ui, p, card, m, media, &parts.join(" · "));
        }
    }
    let hint = match downloads.get(&m.id) {
        Some(Some(reason)) => format!("Gagal mengunduh: {reason}"),
        _ => match action {
            "Buka" if matches!(shown, MediaBox::Document { pdf: Some(_), .. }) => "Lihat PDF".to_owned(),
            "Buka" => "Buka dengan aplikasi bawaan".to_owned(),
            "Unduh" | "Gagal, ulangi" => "Unduh file ini".to_owned(),
            _ => action.to_owned(),
        },
    };
    // Stickers download by themselves once on screen, like in WhatsApp.
    let auto = auto_stickers && matches!(shown, MediaBox::Sticker(None)) && action == "Unduh" && ui.is_rect_visible(rect);
    if auto {
        return Some(MediaClick::Download(m.id.clone()));
    }
    let clickable = matches!(action, "Buka" | "Unduh" | "Gagal, ulangi");
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
            let author = if m.from_me { "Anda".to_owned() } else { super::sender_label(&session.book, &m.sender) };
            let snippet = m.text.split_whitespace().collect::<Vec<_>>().join(" ");
            state.reply = Some(ReplyTarget { chat: chat.jid.clone(), id: m.id.clone(), author, snippet });
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
fn text_command(state: &mut State, id: String, chat: String, text: String) -> Command {
    match state.reply.take_if(|r| r.chat == chat) {
        Some(reply) => Command::Reply { id, chat, text, reply_to: reply.id },
        None => Command::Send { id, chat, text },
    }
}

/// "Membalas <nama>: <cuplikan>" above the composer; x or Esc cancels. A reply
/// started in another chat is dropped.
fn reply_bar(ui: &mut Ui, p: &Palette, state: &mut State, chat: &str) {
    if state.reply.as_ref().is_some_and(|r| r.chat != chat) {
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
                let label = format!("Membalas {}: {snippet}", reply.author);
                ui.add(egui::Label::new(rich(&label, 13.0, Weight::Regular, p.text_2)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    cancel |= widgets::icon_button(ui, "x", 24.0, false).on_hover_text("Batalkan balasan (Esc)").clicked();
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
    let response = response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Buka pesan yang disematkan");
    ui.painter().rect(rect, R_MD, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
    icons::paint(ui, Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(14.0, 14.0)), "pin", 14.0, p.primary);
    let count = if pins.len() > 1 { format!("{} sematan", pins.len()) } else { String::new() };
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
    if ui.button("Balas").clicked() {
        action = Some(MsgAction::Reply);
    }
    // Text only for now; media forwarding is stage 2.
    if m.forwardable() && ui.button("Teruskan").clicked() {
        action = Some(MsgAction::Forward);
    }
    // Media without a caption has nothing to copy.
    if !caption(m).is_empty() && ui.button("Salin").clicked() {
        action = Some(MsgAction::Copy);
    }
    // A message still in the send queue has no WhatsApp id to pin yet.
    if !m.id.starts_with("local-") {
        if pinned {
            if ui.button("Lepas sematan").clicked() {
                action = Some(MsgAction::Unpin);
            }
        } else {
            ui.menu_button("Sematkan", |ui| {
                for (label, length) in [("24 jam", PinLength::Hours24), ("7 hari", PinLength::Days7), ("30 hari", PinLength::Days30)] {
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
fn messages(ui: &mut Ui, p: &Palette, book: &Book, chat: &Chat, downloads: &Downloads, auto_stickers: bool, typing: Option<&str>) -> Option<MsgEvent> {
    const GAP: f32 = 6.0;
    const DIVIDER: f32 = 36.0;
    let wrap = (ui.available_width() * 0.7).min(420.0);
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

    struct Bubble<'a> {
        m: &'a ChatMsg,
        media: Option<(&'a Media, MediaBox)>,
        text: Option<std::sync::Arc<egui::Galley>>,
        /// Delivery mark after the time on our messages.
        tick: Option<(&'static str, Color32)>,
        /// "Status · …": what the message replies to.
        quote: Option<std::sync::Arc<egui::Galley>>,
        meta: std::sync::Arc<egui::Galley>,
        size: egui::Vec2,
    }
    enum Item<'a> {
        Day(String),
        Bubble(Bubble<'a>),
        /// "••• sedang mengetik" after the last message (Pego `v5/Inbox · Chat`).
        Typing(String),
    }
    const TYPING_ROW: f32 = 36.0;
    let mut items = Vec::new();
    let mut last_day = None;
    for m in shown {
        let day = time::day(m.ts);
        if last_day != Some(day) {
            items.push(Item::Day(time::day_label(m.ts)));
            last_day = Some(day);
        }
        // A failed message shows it with the red mark; the time keeps its colour.
        let (fg, meta_color, code_bg) = if m.from_me {
            (p.on_primary, p.on_primary_muted, p.on_primary_muted.gamma_multiply(0.35))
        } else {
            (p.text, p.muted, p.border)
        };
        let media = m.media.as_ref().map(|media| (media, media_box(media, wrap)));
        let media_size = media.as_ref().map(|(_, b)| b.size());
        let sticker = matches!(media, Some((_, MediaBox::Sticker(_))));
        let meta_color = if sticker { p.muted } else { meta_color };
        let caption = caption(m);
        let text = (!caption.is_empty()).then(|| ui.painter().layout_job(markup::job(&caption, 14.0, fg, code_bg, wrap)));
        let mut meta = time::clock(m.ts);
        let tick = if m.from_me { tick(p, &m.status, meta_color, !sticker && is_dark(p.primary)) } else { None };
        if chat.is_group() && !m.from_me {
            meta = format!("{} · {meta}", super::sender_label(book, &m.sender));
        }
        if m.forwarded {
            meta = format!("Diteruskan · {meta}");
        }
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
        let size = vec2(
            text_size.x.max(meta.size().x + if tick.is_some() { 20.0 } else { 0.0 }).max(media_space.x).max(quote_space.x) + 28.0,
            quote_space.y + media_space.y + text_size.y + meta.size().y + 20.0,
        );
        items.push(Item::Bubble(Bubble { m, media, text, tick, quote, meta, size }));
    }
    if let Some(label) = typing {
        items.push(Item::Typing(if label.ends_with("mengetik…") { label.replace("mengetik…", "sedang mengetik") } else { label.to_owned() }));
    }
    let content: f32 = items
        .iter()
        .map(|i| match i {
            Item::Day(_) => DIVIDER,
            Item::Bubble(b) => b.size.y + GAP,
            Item::Typing(_) => TYPING_ROW + GAP,
        })
        .sum();

    ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.add_space(12.0);
            let height = content.max(ui.available_height());
            let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
            let painter = ui.painter();
            let mut y = area.bottom() - content;
            if start > 0 {
                let note = Rect::from_min_size(pos2(area.left(), y - 24.0), vec2(area.width(), 16.0));
                widgets::paint_text(ui, note, &format!("{start} pesan lebih lama tidak ditampilkan"), fonts::ui(12.0, Weight::Regular), p.muted, Align::Center);
            }
            for item in items {
                match item {
                    Item::Typing(label) => {
                        let dots = Rect::from_min_size(pos2(area.left(), y), vec2(52.0, TYPING_ROW));
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
                    Item::Day(label) => {
                        // A pill, so it reads on the wallpaper (Pego m5/Chat "Hari ini").
                        let galley = painter.layout_no_wrap(label, fonts::ui(12.0, Weight::Medium), p.text_2);
                        let (w, cy, cx) = (galley.size().x, y + DIVIDER / 2.0, area.center().x);
                        let pill = Rect::from_center_size(pos2(cx, cy), vec2(w + 24.0, 24.0));
                        painter.rect(pill, 12.0, p.bubble_in, Stroke::new(1.0, p.border), StrokeKind::Inside);
                        painter.galley(pos2(cx - w / 2.0, cy - galley.size().y / 2.0), galley, p.text_2);
                        y += DIVIDER;
                    }
                    Item::Bubble(Bubble { m, media, text, tick, quote, meta, size }) => {
                        let (fill, stroke) = if m.from_me {
                            (p.primary, Stroke::NONE)
                        } else {
                            (p.bubble_in, Stroke::new(1.0, p.border))
                        };
                        let left = if m.from_me { area.right() - size.x } else { area.left() };
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
                        match action {
                            Some(MsgAction::Copy) => ui.ctx().copy_text(caption(m)),
                            Some(action) => clicked = Some(MsgEvent::Menu(action, m.id.clone())),
                            None => {}
                        }
                        let mut at = rect.min + vec2(14.0, 10.0);
                        if let Some(quote) = quote {
                            let boxed = Rect::from_min_size(at, vec2(size.x - 28.0, quote.size().y + 10.0));
                            let (fill, bar) = if m.from_me { (Color32::from_white_alpha(28), p.on_primary_muted) } else { (p.raised, p.primary) };
                            painter.rect_filled(boxed, R_MD, fill);
                            painter.rect_filled(Rect::from_min_size(boxed.min, vec2(3.0, boxed.height())), egui::CornerRadius { nw: R_MD, sw: R_MD, ne: 0, se: 0 }, bar);
                            painter.galley(boxed.min + vec2(10.0, 5.0), quote, p.muted);
                            at.y += boxed.height() + 8.0;
                        }
                        if let Some((media, shown)) = &media {
                            let media_size = shown.size();
                            let media_rect = Rect::from_min_size(at, media_size);
                            if let Some(click) = paint_media(ui, p, media_rect, m, media, shown, downloads, auto_stickers) {
                                clicked = Some(MsgEvent::Media(click));
                            }
                            at.y += media_size.y + 8.0;
                        }
                        if let Some(text) = text {
                            let height = text.size().y;
                            painter.galley(at, text, p.text);
                            at.y += height + 4.0;
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
                                .on_hover_text(format!("Gagal terkirim: {reason}"));
                        }
                        y += size.y + GAP;
                    }
                }
            }
        });
    clicked
}

/// Returns the text to send when Enter (or Kirim) is pressed. Shift+Enter
/// adds a new line, like WhatsApp Web.
/// Runs the desktop's file dialog (zenity) off the UI thread.
fn pick_file(ctx: &egui::Context) -> std::sync::mpsc::Receiver<Option<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let picked = crate::platform::pick_file("Pilih foto atau dokumen", false);
        let _ = tx.send(picked);
        ctx.request_repaint();
    });
    rx
}

/// The file waiting to be sent: icon, name, size and a remove button.
/// Returns false when the file cannot be sent (missing or too large).
fn attachment_chip(ui: &mut Ui, p: &Palette, attachment: &mut Option<PathBuf>) -> bool {
    let Some(path) = attachment.as_ref() else {
        return true;
    };
    let size = std::fs::metadata(path).map(|m| m.len()).ok();
    let (sendable, note) = match size {
        None => (false, "file tidak ditemukan".to_owned()),
        Some(n) if n > MAX_FILE_BYTES => (false, format!("lebih dari {} MB", MAX_FILE_BYTES / 1024 / 1024)),
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
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::show(ui, if picture { "image" } else { "file-text" }, 16.0, p.text_2);
                ui.label(rich(&name, 13.0, Weight::Medium, p.text));
                let color = if sendable { p.muted } else { p.red_text };
                ui.label(rich(&note, 12.0, Weight::Regular, color));
                remove = widgets::icon_button(ui, "x", 24.0, false).on_hover_text("Batalkan lampiran").clicked();
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
    Text(String),
    /// A file with its caption (may be empty).
    File(PathBuf, String),
    Sticker(PathBuf),
}

fn composer(
    ui: &mut Ui,
    p: &Palette,
    state: &mut State,
    to: &str,
    session: &SessionView,
    book: &Book,
    several: bool,
    compact: bool,
) -> Option<Outgoing> {
    let connected = session.status == Status::Connected;
    let id = ui.make_persistent_id("composer_text");
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
    let mut send = connected
        && ui.memory(|m| m.has_focus(id))
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let mut sendable = true;
    let mut sticker_sent = None;
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
        let hint = if state.attachment.is_some() { "Tambahkan keterangan" } else { "Ketik balasan" };
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
                    let edit_w = (ui.available_width() - 70.0).max(60.0);
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .id(id)
                            .layouter(&mut layouter)
                            .frame(Frame::NONE)
                            .hint_text(rich(hint, 15.0, Weight::Regular, p.muted))
                            .font(fonts::ui(14.0, Weight::Regular))
                            .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                            .desired_rows(1)
                            .desired_width(edit_w),
                    );
                    let sticker = widgets::icon_button(ui, "sticker", 32.0, false).on_hover_text("Stiker");
                    sticker_sent = stickers::picker(ui, &sticker, book, connected);
                    let clip = widgets::icon_button(ui, "paperclip", 32.0, state.picking.is_some()).on_hover_text("Lampirkan foto atau dokumen");
                    if clip.clicked() && state.picking.is_none() {
                        state.picking = Some(pick_file(ui.ctx()));
                    }
                });
            });
            let (rect, response) = ui.allocate_exact_size(vec2(48.0, 48.0), if connected { Sense::click() } else { Sense::hover() });
            let fill = if connected { p.primary } else { p.border_strong };
            ui.painter().circle_filled(rect.center(), 24.0, fill);
            icons::paint(ui, Rect::from_center_size(rect.center() + vec2(-1.0, 1.0), vec2(20.0, 20.0)), "send", 20.0, p.on_primary);
            send |= connected && response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        });
        if !connected {
            ui.add_space(4.0);
            ui.label(rich(&format!("{} tidak terhubung, pesan belum bisa dikirim", session.name), 12.0, Weight::Regular, p.red_text));
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
                "Lepas file di sini untuk melampirkan".to_owned()
            } else if state.attachment.is_some() {
                "Tambahkan keterangan (opsional)".to_owned()
            } else {
                format!("Balas ke {to}")
            };
            ui.add(
                egui::TextEdit::multiline(text)
                    .id(id)
                    .layouter(&mut layouter)
                    .frame(Frame::NONE)
                    .hint_text(rich(&hint, 14.0, Weight::Regular, p.muted))
                    .font(fonts::ui(14.0, Weight::Regular))
                    .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                    .desired_rows(1)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            widgets::row(
                ui,
                32.0,
                |ui| {
                    let clip = widgets::icon_button(ui, "paperclip", 28.0, state.picking.is_some())
                        .on_hover_text("Lampirkan foto atau dokumen (atau seret file ke sini)");
                    if clip.clicked() && state.picking.is_none() {
                        state.picking = Some(pick_file(ui.ctx()));
                    }
                    let smile = widgets::icon_button(ui, "smile", 28.0, false).on_hover_text("Emoji");
                    emoji::picker(ui, &smile, id, text);
                    let sticker = widgets::icon_button(ui, "sticker", 28.0, false).on_hover_text("Stiker");
                    sticker_sent = stickers::picker(ui, &sticker, book, connected);
                    ui.add_space(8.0);
                    let note = if connected && several {
                        format!("Membalas dari {} · dikunci ke nomor penerima", super::devices::chip_label(session))
                    } else if connected {
                        "Enter kirim, Shift+Enter baris baru".to_owned()
                    } else {
                        format!("{} tidak terhubung, pesan belum bisa dikirim", session.name)
                    };
                    let color = if connected { p.muted } else { p.red_text };
                    ui.add(egui::Label::new(rich(&note, 12.0, Weight::Regular, color)).truncate());
                },
                |ui| {
                    let button = ui.add_enabled_ui(connected, |ui| {
                        widgets::button(ui, Some("arrow-up"), "Kirim", Kind::Primary)
                    });
                    send |= connected && button.inner.clicked();
                    ui.add_space(8.0);
                    let queue = if session.queue == 0 {
                        "Antre 0".to_owned()
                    } else {
                        format!("Antre {} · jeda ± 4 dtk per pesan", session.queue)
                    };
                    ui.label(rich(&queue, 12.0, Weight::Regular, p.muted));
                },
            );
        });
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
    ui.memory_mut(|m| m.request_focus(id));
    Some(match state.attachment.take() {
        Some(path) => Outgoing::File(path, message),
        None => Outgoing::Text(message),
    })
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
    let values = [(total.to_string(), "pesan"), (mine.to_string(), "dari Anda"), (since, "sejak")];
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
        widgets::section_label(ui, "Teknis");
        let kind = if chat.is_group() { "Grup" } else { "Pribadi" };
        let replied_first = chat.messages.iter().any(|m| !m.from_me);
        for (key, value) in [
            ("JID", super::short_jid(&chat.jid)),
            ("Tipe", kind.to_owned()),
            ("Pernah chat duluan", if replied_first { "Ya".to_owned() } else { "Belum".to_owned() }),
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
    Panel::top("m_inbox_head")
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
                widgets::search(ui, &mut state.chat_search, "Cari nama atau nomor", ui.available_width(), false);
            }
            if several {
                ui.add_space(14.0);
                // "Gabung semua" / "Per nomor" as two halves of one track.
                let (track, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
                ui.painter().rect_filled(track, 22.0, if forest { p.panel } else { p.raised });
                for (i, (icon, label)) in [("layers", "Gabung semua"), ("columns-2", "Per nomor")].into_iter().enumerate() {
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
                        if scope_chip(ui, &p, &format!("Semua · {total}"), state.inbox_scope.is_none()).clicked() {
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
            let label = if archived_view { "← Kembali ke semua chat".to_owned() } else { format!("Diarsipkan · {archived}") };
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
                "Belum ada nomor. Tautkan nomor di tab Perangkat."
            } else if archived_view {
                "Belum ada chat yang diarsipkan."
            } else {
                "Tidak ada chat di sini."
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
                    Line::Header(s, count) => section_header(ui, &p, rect.shrink2(vec2(PAD - 8.0, 0.0)), s, *count),
                    Line::Chat(s, chat) => {
                        if !avatars.contains_key(&chat.jid) && state.avatar_asked.insert(chat.jid.clone()) {
                            commands.push(Command::Avatar { id: s.id.clone(), jid: chat.jid.clone() });
                        }
                        let picture = avatars.get(&chat.jid).and_then(|p| p.as_deref());
                        let title = super::chat_title(&s.book, chat);
                        let row = compact_row(ui, &p, rect, &title, chat, picture, via.then_some(*s), typing_label(state, &s.id, &s.book, chat).as_deref());
                        chat_menu(&row, s, chat, &mut commands);
                        if row.clicked() {
                            state.inbox_session = Some(s.id.clone());
                            state.chat_selected = Some(chat.jid.clone());
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
fn compact_row(ui: &Ui, p: &Palette, row: Rect, title: &str, chat: &Chat, picture: Option<&Path>, via: Option<&SessionView>, typing: Option<&str>) -> egui::Response {
    use crate::ui::compact::{self as phone, PAD};
    let resp = ui.interact(row, ui.scope_id().with(("m_chat_row", via.map(|s| s.id.as_str()), &chat.jid)), Sense::click());
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
    let reserve = if chat.unread > 0 { 32.0 } else { 0.0 };
    let line = Rect::from_min_max(pos2(left, row.top() + 35.0), pos2(right - reserve, row.top() + 56.0));
    paint_preview(ui, p, line, chat, 14.0, p.text_2, typing);
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
        widgets::paint_text(ui, label, &format!("ke {}", super::devices::chip_label(session)), fonts::ui(13.0, Weight::Medium), color, Align::Min);
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
    Panel::top("m_chat_header")
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
                    if widgets::icon_button(ui, "chevron-left", 36.0, false).on_hover_text("Kembali").clicked() {
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
                        widgets::paint_text(ui, label, &format!("Chat masuk ke {}", super::devices::chip_label(session)), fonts::ui(12.0, Weight::Medium), color, Align::Min);
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
                        if ui.button("Muat pesan lama dari HP").clicked() {
                            state.older_asked = Some((chat.jid.clone(), std::time::Instant::now()));
                            commands.push(Command::LoadOlder { id: session.id.clone(), chat: chat.jid.clone() });
                        }
                        if ui.button("Salin nomor").clicked() {
                            ui.ctx().copy_text(chat.jid.split('@').next().unwrap_or_default().to_owned());
                        }
                        if !chat.is_group() {
                            let done = chat.is_done();
                            if ui.button(if done { "Tandai perlu balas" } else { "Tandai selesai" }).clicked() {
                                commands.push(Command::MarkDone { id: session.id.clone(), chat: chat.jid.clone(), done: !done });
                            }
                        }
                    });
                },
            );
        });
    Panel::bottom("m_composer")
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
                            ui.label(rich(&format!("Membalas dari {}", super::devices::chip_label(session)), 13.0, Weight::SemiBold, color));
                        });
                    });
                });
                ui.add_space(8.0);
            }
            let (id, jid) = (session.id.clone(), chat.jid.clone());
            reply_bar(ui, &p, state, &jid);
            match composer(ui, &p, state, &title, session, book, several, true) {
                Some(Outgoing::Text(text)) => commands.push(text_command(state, id, jid, text)),
                Some(Outgoing::File(path, caption)) => commands.push(Command::SendFile { id, chat: jid, path, caption }),
                Some(Outgoing::Sticker(path)) => commands.push(Command::SendSticker { id, chat: jid, path }),
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
            if let Some(event) = messages(ui, &p, book, chat, downloads, auto_stickers, typing_label(state, &session.id, book, chat).as_deref()) {
                on_message_event(event, state, session, chat, &mut commands);
            }
        });
    });
    commands
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
        let chat = Chat { jid: "62811@s.whatsapp.net".to_owned(), unread: 0, messages: Vec::new(), done_at: None };
        let book = Book::default();
        let mut state = State::default();
        assert_eq!(typing_label(&state, "s1", &book, &chat), None);
        let mut map = std::collections::HashMap::new();
        map.insert(Typing::key("s1", &chat.jid), Typing { who: None, recording: false, since: Instant::now() });
        state.typing_now = std::sync::Arc::new(map.clone());
        assert_eq!(typing_label(&state, "s1", &book, &chat).as_deref(), Some("mengetik…"));
        assert_eq!(typing_label(&state, "s2", &book, &chat), None, "another number's chat");
        map.insert(Typing::key("s1", &chat.jid), Typing { who: None, recording: true, since: Instant::now() - Typing::STALE - Duration::from_secs(1) });
        state.typing_now = std::sync::Arc::new(map);
        assert_eq!(typing_label(&state, "s1", &book, &chat), None, "stale");
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
