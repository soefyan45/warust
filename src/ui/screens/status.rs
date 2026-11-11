//! Status page (Pego `v5/Status`): who posted in the last 24 hours on every
//! number, and a viewer that shows one update at a time. Showing a contact's
//! update sends the "seen" receipt, as opening it on the phone does, unless
//! that number has it turned off.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Align, CentralPanel, Color32, Key, Margin, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::backend::{Book, ChatMsg, Command, SessionView, Status, StatusPost, movie};
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette, R_LG};
use crate::ui::widgets::{self, rich};
use crate::ui::{State, icons, markup, time};

type Avatars = HashMap<String, Option<PathBuf>>;
type Downloads = HashMap<String, Option<String>>;

/// The poster's name, else their masked number; `None` is this number.
fn poster_title(book: &Book, poster: &Option<String>) -> String {
    match poster {
        None => tr("My status", "Status saya").to_owned(),
        Some(jid) => book.name_of(jid).map(str::to_owned).unwrap_or_else(|| super::short_jid(jid)),
    }
}

/// Posters with an update not yet seen (own updates do not count).
pub fn unseen_posters(book: &Book) -> usize {
    book.status_feed(time::now())
        .iter()
        .filter(|(poster, posts)| poster.is_some() && posts.iter().any(|p| !p.seen))
        .count()
}

fn argb(value: u32) -> Color32 {
    let [a, r, g, b] = value.to_be_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, if a == 0 { 255 } else { a })
}

/// Paints one update into `card`: a text on its colour, or the photo (asked
/// for when missing) with a play badge for videos. Clicks open or download it.
fn paint_post(ui: &mut Ui, p: &Palette, card: Rect, post: &StatusPost, session: &str, downloads: &Downloads, playing: Option<&movie::Status>) -> Vec<Command> {
    let mut commands = Vec::new();
    match &post.media {
        None => {
            let background = post.background.map(argb).unwrap_or(p.raised);
            let color = post.text_color.map(argb).unwrap_or(if post.background.is_some() { Color32::WHITE } else { p.text });
            ui.painter().rect_filled(card, R_LG, background);
            let job = markup::job(&post.text, 22.0, color, Color32::from_black_alpha(40), card.width() - 48.0);
            let mut job = job;
            job.halign = Align::Center;
            let galley = ui.painter().layout_job(job);
            let at = pos2(card.center().x, card.center().y - galley.size().y / 2.0);
            ui.painter().galley(at, galley, color);
        }
        // A video playing in the card (#84): its frames, and a note while it loads.
        Some(_) if playing.is_some() => {
            ui.painter().rect_filled(card, R_LG, Color32::BLACK);
            let status = playing.expect("checked");
            match widgets::frame_texture(ui.ctx(), "status_video", &status.picture) {
                Some(texture) => {
                    let size = texture.size_vec2();
                    let scale = (card.width() / size.x).min(card.height() / size.y);
                    let shown = Rect::from_center_size(card.center(), size * scale);
                    egui::Image::from_texture(&texture).corner_radius(R_LG).fit_to_exact_size(shown.size()).paint_at(ui, shown);
                }
                None => {
                    if let Some(uri) = picture(post).and_then(|source| widgets::picture_uri(ui.ctx(), &source)) {
                        egui::Image::from_uri(uri).corner_radius(R_LG).fit_to_exact_size(card.size()).paint_at(ui, card);
                    }
                }
            }
            if status.loading {
                let pill = Rect::from_center_size(pos2(card.center().x, card.bottom() - 28.0), vec2(120.0, 26.0));
                ui.painter().rect_filled(pill, 13.0, Color32::from_black_alpha(160));
                widgets::paint_text(ui, pill, tr("Loading…", "Memuat…"), fonts::ui(12.0, Weight::Medium), Color32::WHITE, Align::Center);
            }
        }
        Some(media) => {
            ui.painter().rect_filled(card, R_LG, Color32::BLACK);
            if let Some(source) = picture(post) {
                if let Some(uri) = widgets::picture_uri(ui.ctx(), &source) {
                    let aspect = match (media.width, media.height) {
                        (Some(w), Some(h)) if w > 0 => h as f32 / w as f32,
                        _ => 16.0 / 9.0,
                    };
                    let fit_w = card.width().min(card.height() / aspect);
                    let shown = Rect::from_center_size(card.center(), vec2(fit_w, fit_w * aspect));
                    egui::Image::from_uri(uri).corner_radius(R_LG).fit_to_exact_size(shown.size()).paint_at(ui, shown);
                }
            }
            let state_label = if media.file.is_some() {
                None
            } else {
                match downloads.get(&post.id) {
                    Some(None) => Some(tr("Downloading…", "Mengunduh…")),
                    Some(Some(_)) => Some(tr("Failed, click to retry", "Gagal, klik untuk ulangi")),
                    None if media.downloadable() => {
                        Some(if media.kind == "image" { tr("Loading…", "Memuat…") } else { tr("Click to download", "Klik untuk unduh") })
                    }
                    None => Some(tr("Not available", "Tidak tersedia")),
                }
            };
            if media.kind == "video" {
                let badge = Rect::from_center_size(card.center(), vec2(64.0, 64.0));
                ui.painter().circle_filled(badge.center(), 30.0, Color32::from_black_alpha(150));
                icons::paint(ui, badge, "video", 26.0, Color32::WHITE);
            }
            if let Some(label) = state_label {
                let pill = Rect::from_center_size(pos2(card.center().x, card.bottom() - 28.0), vec2(170.0, 26.0));
                ui.painter().rect_filled(pill, 13.0, Color32::from_black_alpha(160));
                widgets::paint_text(ui, pill, label, fonts::ui(12.0, Weight::Medium), Color32::WHITE, Align::Center);
            }
            let response = ui.interact(card, ui.scope_id().with(("status_media", &post.id)), Sense::click());
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            // Photos load by themselves; videos when clicked.
            let auto = media.kind == "image" && media.file.is_none() && media.downloadable() && !downloads.contains_key(&post.id);
            let retry = matches!(downloads.get(&post.id), Some(Some(_)));
            if response.clicked() {
                match &media.file {
                    Some(file) => {
                        crate::platform::open(file);
                    }
                    None if media.downloadable() => commands.push(Command::Download { id: session.to_owned(), msg_id: post.id.clone() }),
                    None => {}
                }
            } else if auto && !retry {
                commands.push(Command::Download { id: session.to_owned(), msg_id: post.id.clone() });
            }
        }
    }
    commands
}

/// The picture to show: the downloaded photo, else the inline thumbnail.
fn picture(post: &StatusPost) -> Option<PathBuf> {
    let media = post.media.as_ref()?;
    let downloaded = media.file.clone().filter(|f| {
        media.kind == "image"
            && matches!(f.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("jpg" | "jpeg" | "png"))
    });
    downloaded.or_else(|| media.thumb.clone())
}

// Status page (Pego `v5/Status`) and its phone layout: every number's view of
// its contacts' updates, a viewer that moves on by itself, reactions and
// comments sent from the number that saw the update, and what was sent.

/// How long a text or photo update stays before the next one.
const ADVANCE: std::time::Duration = std::time::Duration::from_secs(6);
const LIKE: &str = "❤️";
const QUICK: [&str; 5] = ["👍", "😂", "😮", "😢", "🙏"];
const POSTER_ROW: f32 = 68.0;

/// A contact's updates as one number sees them.
struct Entry<'a> {
    session: &'a SessionView,
    poster: String,
    posts: Vec<&'a StatusPost>,
}

impl<'a> Entry<'a> {
    fn unseen(&self) -> bool {
        self.posts.iter().any(|s| !s.seen)
    }

    fn last_ts(&self) -> i64 {
        self.posts.last().map(|s| s.ts).unwrap_or(0)
    }

    fn title(&self) -> String {
        poster_title(&self.session.book, &Some(self.poster.clone()))
    }

    /// Our comments and reactions on these updates, oldest first.
    fn replies(&self) -> Vec<&'a ChatMsg> {
        let ids: std::collections::HashSet<&str> = self.posts.iter().map(|s| s.id.as_str()).collect();
        let session: &'a SessionView = self.session;
        session
            .book
            .chats
            .iter()
            .find(|c| c.jid == self.poster)
            .map(|c| {
                c.messages
                    .iter()
                    .filter(|m| m.from_me && m.quote.as_ref().is_some_and(|q| q.status && ids.contains(q.id.as_str())))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Contacts' updates of the last 24 hours on every number (or the one in
/// `scope`) matching `search`: unseen first, newest first.
fn entries<'a>(sessions: &'a [SessionView], scope: Option<&str>, search: &str) -> Vec<Entry<'a>> {
    let now = time::now();
    let query = search.trim().to_lowercase();
    let mut all = Vec::new();
    for s in sessions.iter().filter(|s| scope.is_none_or(|id| s.id == id)) {
        for (poster, posts) in s.book.status_feed(now) {
            let Some(poster) = poster else { continue };
            let entry = Entry { session: s, poster, posts };
            if query.is_empty() || entry.title().to_lowercase().contains(&query) || entry.poster.contains(&query) {
                all.push(entry);
            }
        }
    }
    all.sort_by_key(|e| (!e.unseen(), std::cmp::Reverse(e.last_ts())));
    all
}

fn ago(ts: i64) -> String {
    match (time::now() - ts).max(0) {
        s if s < 60 => tr("just now", "baru saja").to_owned(),
        s if s < 3600 => crate::trf!("{} min ago", "{} mnt lalu", s / 60),
        s => crate::trf!("{} h ago", "{} jam lalu", s / 3600),
    }
}

/// A reply that is one emoji is a reaction; anything longer is a comment.
fn is_reaction(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && t.chars().count() <= 4 && !t.chars().any(|c| c.is_alphanumeric() || c.is_whitespace())
}

fn forest() -> bool {
    theme::template() == theme::Template::Forest
}

/// The dark stage behind the viewer: Forest olive, black, or raised in dark mode.
fn stage(ctx: &egui::Context) -> Color32 {
    if forest() {
        Color32::from_rgb(0x2F, 0x2C, 0x24)
    } else if ctx.theme() == egui::Theme::Dark {
        theme::DARK.raised
    } else {
        Color32::from_rgb(0x0A, 0x0A, 0x0A)
    }
}

fn open(state: &mut State, e: &Entry) {
    state.status_session = Some(e.session.id.clone());
    state.status_selected = Some(Some(e.poster.clone()));
    state.status_index = e.posts.iter().position(|s| !s.seen).unwrap_or(0);
    state.status_shown = None;
    state.status_comment.clear();
}

/// Dev aid: WARUST_STATUS=first opens the first poster once (for screenshots).
fn dev_open(state: &mut State, all: &[Entry]) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var("WARUST_STATUS").as_deref() == Ok("first") && !DONE.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(first) = all.first() {
            open(state, first);
            state.status_paused = true;
            DONE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn is_open(state: &State, e: &Entry) -> bool {
    state.status_session.as_deref() == Some(e.session.id.as_str()) && state.status_selected.as_ref().and_then(|p| p.as_ref()) == Some(&e.poster)
}

/// Desktop: posters on the left, the viewer, reactions and comments on the right.
pub fn page(ui: &mut Ui, state: &mut State, sessions: &[SessionView], avatars: &Avatars, downloads: &Downloads) -> Vec<Command> {
    let p = theme::palette(ui.ctx());
    let mut commands = Vec::new();
    if state.status_scope.as_ref().is_some_and(|id| !sessions.iter().any(|s| &s.id == id)) {
        state.status_scope = None;
    }
    egui::Panel::left("status_list")
        .default_size(340.0)
        .size_range(300.0..=420.0)
        .resizable(true)
        .show_separator_line(!forest())
        .frame(theme::pane(ui.ctx(), p.panel, Margin { left: 16, right: 16, top: 16, bottom: 12 }))
        .show(ui, |ui| commands.extend(poster_list(ui, &p, state, sessions, avatars, false)));
    let all = entries(sessions, None, "");
    dev_open(state, &all);
    let current = all.iter().position(|e| is_open(state, e));
    if let Some(i) = current {
        egui::Panel::right("status_side")
            .default_size(330.0)
            .size_range(290.0..=420.0)
            .resizable(true)
            .show_separator_line(!forest())
            .frame(theme::pane(ui.ctx(), p.panel, Margin::same(20)))
            .show(ui, |ui| commands.extend(side(ui, &p, state, &all[i])));
    }
    let frame = theme::pane(ui.ctx(), stage(ui.ctx()), Margin::ZERO).fill(stage(ui.ctx()));
    CentralPanel::default().frame(frame).show(ui, |ui| match current {
        Some(i) => {
            let next = all.iter().skip(i + 1).find(|e| e.unseen());
            commands.extend(stage_view(ui, &p, state, &all[i], next, downloads, false));
        }
        None => {
            let rect = ui.max_rect();
            icons::paint(ui, Rect::from_center_size(rect.center() - vec2(0.0, 30.0), vec2(28.0, 28.0)), "circle-dashed", 28.0, Color32::from_white_alpha(160));
            let title = Rect::from_center_size(rect.center() + vec2(0.0, 8.0), vec2(rect.width(), 22.0));
            widgets::paint_text(ui, title, tr("Select a status", "Pilih status"), fonts::ui(16.0, Weight::SemiBold), Color32::WHITE, Align::Center);
            let desc = Rect::from_center_size(rect.center() + vec2(0.0, 32.0), vec2(rect.width(), 18.0));
            widgets::paint_text(ui, desc, tr("Choose a contact on the left. Opening a status sends a seen receipt when allowed.", "Pilih kontak di sebelah kiri. Membuka status mengirim tanda dilihat bila diizinkan."), fonts::ui(13.0, Weight::Regular), Color32::from_white_alpha(170), Align::Center);
        }
    });
    commands
}

/// Phone: the poster list, or the viewer full screen with a back button.
pub fn compact(ui: &mut Ui, state: &mut State, sessions: &[SessionView], avatars: &Avatars, downloads: &Downloads) -> Vec<Command> {
    let p = theme::palette(ui.ctx());
    let all = entries(sessions, None, "");
    dev_open(state, &all);
    match all.iter().position(|e| is_open(state, e)) {
        Some(i) => {
            let next = all.iter().skip(i + 1).find(|e| e.unseen());
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, stage(ui.ctx()));
            stage_view(ui, &p, state, &all[i], next, downloads, true)
        }
        None => {
            state.status_selected = None;
            let mut commands = Vec::new();
            egui::Frame::NONE
                .inner_margin(Margin { left: crate::ui::compact::PAD as i8, right: crate::ui::compact::PAD as i8, top: 12, bottom: 8 })
                .show(ui, |ui| commands = poster_list(ui, &p, state, sessions, avatars, true));
            commands
        }
    }
}

fn poster_list(ui: &mut Ui, p: &Palette, state: &mut State, sessions: &[SessionView], avatars: &Avatars, phone: bool) -> Vec<Command> {
    let mut commands = Vec::new();
    ui.spacing_mut().item_spacing.y = 0.0;
    if phone {
        crate::ui::compact::page_title(ui, "Status", |_| {});
    } else {
        ui.label(rich("Status", 20.0, Weight::SemiBold, p.text));
        ui.add_space(2.0);
        ui.label(rich(tr("Contacts' statuses from connected numbers", "Status kontak dari nomor terhubung"), 13.0, Weight::Regular, p.muted));
    }
    ui.add_space(14.0);
    if sessions.len() > 1 {
        let mut labels = vec![tr("All numbers", "Semua nomor").to_owned()];
        labels.extend(sessions.iter().map(|s| s.name.clone()));
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut selected = state.status_scope.as_ref().and_then(|id| sessions.iter().position(|s| &s.id == id)).map_or(0, |i| i + 1);
        widgets::tabs(ui, &labels, &mut selected);
        state.status_scope = selected.checked_sub(1).map(|i| sessions[i].id.clone());
        ui.add_space(10.0);
    }
    widgets::search(ui, &mut state.status_search, tr("Search name or number", "Cari nama atau nomor"), ui.available_width(), false);
    ui.add_space(8.0);
    let list = entries(sessions, state.status_scope.as_deref(), &state.status_search);
    let footer = if phone { 0.0 } else { 28.0 };
    let several = sessions.len() > 1;
    let height = ui.available_height() - footer;
    ScrollArea::vertical().auto_shrink(false).max_height(height).show(ui, |ui| {
        if list.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(rich(tr("No statuses in the last 24 hours.", "Belum ada status dalam 24 jam terakhir."), 13.0, Weight::Regular, p.muted));
                ui.label(rich(tr("New statuses from contacts show up here.", "Status baru dari kontak muncul di sini."), 12.0, Weight::Regular, p.muted));
            });
        }
        let unseen = list.iter().filter(|e| e.unseen()).count();
        for (label, group) in [(tr("NOT SEEN", "BELUM DILIHAT"), &list[..unseen]), (tr("SEEN", "SUDAH DILIHAT"), &list[unseen..])] {
            if group.is_empty() {
                continue;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(rich(&format!("{label} · {}", group.len()), 11.0, Weight::SemiBold, p.muted));
            });
            ui.add_space(6.0);
            for e in group {
                if !avatars.contains_key(&e.poster) && state.avatar_asked.insert(e.poster.clone()) {
                    commands.push(Command::Avatar { id: e.session.id.clone(), jid: e.poster.clone() });
                }
                let picture = avatars.get(&e.poster).and_then(|p| p.as_deref());
                if poster_row(ui, p, e, picture, is_open(state, e), several).clicked() {
                    open(state, e);
                }
            }
        }
    });
    if !phone {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::show(ui, "clock", 14.0, p.muted);
            ui.label(rich(tr("Statuses disappear automatically after 24 hours", "Status hilang otomatis setelah 24 jam"), 12.0, Weight::Regular, p.muted));
        });
    }
    commands
}

fn poster_row(ui: &mut Ui, p: &Palette, e: &Entry, picture: Option<&Path>, selected: bool, several: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), POSTER_ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, theme::r_inner(), p.selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, theme::r_inner(), p.raised);
    }
    let title = e.title();
    let tile = Rect::from_center_size(pos2(rect.left() + 34.0, rect.center().y), vec2(42.0, 42.0));
    crate::ui::compact::avatar(ui, tile, &super::initials(&title), picture, true);
    // Ring: coloured while something is unseen, like the phone's status ring.
    let (ring, width) = if e.unseen() { (if forest() { p.primary } else { p.green }, 2.0) } else { (p.border_strong, 1.5) };
    ui.painter().circle_stroke(tile.center(), 24.5, Stroke::new(width, ring));
    let left = tile.right() + 16.0;
    let count = {
        let n = e.posts.len();
        let en = if n == 1 { "status" } else { "statuses" };
        crate::trf!("{n} {en}", "{n} status")
    };
    let count_w = ui.painter().layout_no_wrap(count.clone(), fonts::ui(12.0, Weight::Regular), p.muted).size().x;
    let name = Rect::from_min_max(pos2(left, rect.top() + 13.0), pos2(rect.right() - count_w - 20.0, rect.top() + 33.0));
    widgets::paint_text(ui, name, &title, fonts::ui(15.0, Weight::SemiBold), p.text, Align::Min);
    widgets::paint_text(ui, Rect::from_min_max(pos2(rect.right() - count_w - 12.0, rect.top() + 13.0), pos2(rect.right() - 12.0, rect.top() + 33.0)), &count, fonts::ui(12.0, Weight::Regular), p.muted, Align::Max);
    let when = ago(e.last_ts());
    let galley = ui.painter().layout_no_wrap(when, fonts::ui(12.0, Weight::Regular), p.muted);
    let when_w = galley.size().x;
    ui.painter().galley(pos2(left, rect.top() + 38.0), galley, p.muted);
    if several {
        super::inbox::number_chip(ui, p, pos2(left + when_w + 8.0, rect.top() + 36.0), e.session);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A round button on the dark stage. `active` fills it with the accent.
fn stage_button(ui: &mut Ui, rect: Rect, id: &str, enabled: bool, active: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, ui.scope_id().with(id), sense);
    let p = theme::palette(ui.ctx());
    let fill = if active {
        if forest() { p.accent } else { Color32::WHITE }
    } else if response.hovered() && enabled {
        Color32::from_white_alpha(46)
    } else {
        Color32::from_white_alpha(26)
    };
    ui.painter().circle_filled(rect.center(), rect.width() / 2.0, fill);
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

/// The viewer on the dark stage: progress, who and when, the update, the
/// arrows, reactions and the comment field.
#[allow(clippy::too_many_arguments)]
fn stage_view(ui: &mut Ui, p: &Palette, state: &mut State, e: &Entry, next: Option<&Entry>, downloads: &Downloads, phone: bool) -> Vec<Command> {
    let mut commands = Vec::new();
    let area = ui.max_rect();
    let white = Color32::WHITE;
    let soft = Color32::from_white_alpha(170);
    let count = e.posts.len();
    state.status_index = state.status_index.min(count - 1);
    let comment_id = ui.make_persistent_id("status_comment");
    let typing = ui.memory(|m| m.has_focus(comment_id)) || !state.status_comment.is_empty();
    let (left_key, right_key) = if typing { (false, false) } else { ui.input(|i| (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight))) };
    if left_key && state.status_index > 0 {
        state.status_index -= 1;
    }
    if right_key && state.status_index + 1 < count {
        state.status_index += 1;
    }
    let post = e.posts[state.status_index];
    let connected = e.session.status == Status::Connected;
    let can_reply = connected && !post.from_me;

    // The timer: frozen while paused or typing; at the end, the next update or poster.
    let now = std::time::Instant::now();
    let elapsed = match &state.status_shown {
        Some((id, since)) if id == &post.id => since.elapsed(),
        _ => std::time::Duration::ZERO,
    };
    let paused = state.status_paused || typing;
    if paused || !matches!(&state.status_shown, Some((id, _)) if id == &post.id) {
        state.status_shown = Some((post.id.clone(), now - elapsed));
    }
    // A downloaded video plays in the card; its length is the update's time (#84).
    let video = post.media.as_ref().filter(|m| m.kind == "video").and_then(|m| m.file.clone()).filter(|_| movie::available());
    movie::stop_unless(video.as_ref().map(|_| post.id.as_str()));
    let playing = video.as_ref().and_then(|file| {
        if movie::status(&post.id).is_none() {
            movie::play(&post.id, file, movie::repaint_for(ui.ctx()));
        }
        let status = movie::status(&post.id)?;
        if paused && status.playing {
            movie::pause(&post.id);
        } else if !paused && !status.playing && !status.ended {
            movie::play(&post.id, file, movie::repaint_for(ui.ctx()));
        }
        // A paused video still needs asking about now and then, or it stops.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        Some(status)
    });
    let (progress, finished) = match &playing {
        Some(s) => ((if s.duration > 0.0 { s.position / s.duration } else { 0.0 }).min(1.0), s.ended),
        None => ((elapsed.as_secs_f32() / ADVANCE.as_secs_f32()).min(1.0), elapsed >= ADVANCE),
    };
    if !paused {
        if finished {
            if state.status_index + 1 < count {
                state.status_index += 1;
            } else if let Some(next) = next {
                open(state, next);
            } else {
                state.status_paused = true;
            }
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    // Progress segments.
    let pad = if phone { 12.0 } else { 24.0 };
    let bar = Rect::from_min_max(pos2(area.left() + pad, area.top() + 14.0), pos2(area.right() - pad, area.top() + 17.0));
    let gap = 4.0;
    let seg_w = (bar.width() - gap * (count as f32 - 1.0)) / count as f32;
    for i in 0..count {
        let seg = Rect::from_min_size(pos2(bar.left() + i as f32 * (seg_w + gap), bar.top()), vec2(seg_w, 3.0));
        ui.painter().rect_filled(seg, 2.0, Color32::from_white_alpha(70));
        let done = if i < state.status_index { 1.0 } else if i == state.status_index { progress } else { 0.0 };
        if done > 0.0 {
            ui.painter().rect_filled(Rect::from_min_size(seg.min, vec2(seg.width() * done, 3.0)), 2.0, white);
        }
    }

    // Header: back (phone), who, which update, through which number; pause, menu.
    let title = e.title();
    let head = Rect::from_min_max(pos2(area.left() + pad, area.top() + 28.0), pos2(area.right() - pad, area.top() + 72.0));
    let mut x = head.left();
    if phone {
        let back = Rect::from_min_size(pos2(x - 4.0, head.center().y - 18.0), vec2(36.0, 36.0));
        let response = ui.interact(back, ui.scope_id().with("status_back"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        icons::paint(ui, back, "chevron-left", 22.0, white);
        if response.clicked() {
            state.status_selected = None;
            state.status_comment.clear();
        }
        x += 34.0;
    }
    let tile = Rect::from_min_size(pos2(x, head.center().y - 20.0), vec2(40.0, 40.0));
    crate::ui::compact::avatar(ui, tile, &super::initials(&title), None, true);
    let text_left = tile.right() + 12.0;
    let text_right = head.right() - 96.0;
    widgets::paint_text(ui, Rect::from_min_max(pos2(text_left, head.top() + 2.0), pos2(text_right, head.top() + 22.0)), &title, fonts::ui(15.0, Weight::SemiBold), white, Align::Min);
    let via = if phone { String::new() } else { crate::trf!(" · seen via {}", " · dilihat lewat {}", super::devices::chip_label(e.session)) };
    let sub = crate::trf!("Status {} of {count} · {}{via}", "Status {} dari {count} · {}{via}", state.status_index + 1, ago(post.ts));
    widgets::paint_text(ui, Rect::from_min_max(pos2(text_left, head.top() + 23.0), pos2(text_right, head.top() + 41.0)), &sub, fonts::ui(12.0, Weight::Regular), soft, Align::Min);
    let menu_rect = Rect::from_center_size(pos2(head.right() - 20.0, head.center().y), vec2(40.0, 40.0));
    let pause_rect = menu_rect.translate(vec2(-48.0, 0.0));
    // A video's sound on or off.
    if let Some(status) = &playing {
        let mute_rect = pause_rect.translate(vec2(-48.0, 0.0));
        let hint = if status.muted { tr("Sound on", "Nyalakan suara") } else { tr("Mute", "Bisukan") };
        if stage_button(ui, mute_rect, "status_mute", true, false).on_hover_text(hint).clicked() {
            movie::set_muted(!status.muted);
        }
        icons::paint(ui, Rect::from_center_size(mute_rect.center(), vec2(18.0, 18.0)), if status.muted { "volume-x" } else { "volume-2" }, 18.0, white);
    }
    if stage_button(ui, pause_rect, "status_pause", true, false).on_hover_text(if state.status_paused { tr("Resume", "Lanjut") } else { tr("Pause", "Jeda") }).clicked() {
        state.status_paused = !state.status_paused;
    }
    icons::paint(ui, Rect::from_center_size(pause_rect.center(), vec2(18.0, 18.0)), if state.status_paused { "play" } else { "pause" }, 18.0, white);
    let menu = stage_button(ui, menu_rect, "status_menu", true, false);
    icons::paint(ui, Rect::from_center_size(menu_rect.center(), vec2(18.0, 18.0)), "ellipsis", 18.0, white);
    egui::Popup::menu(&menu).show(|ui| {
        ui.set_min_width(190.0);
        if !post.text.is_empty() && ui.button(tr("Copy text", "Salin teks")).clicked() {
            ui.ctx().copy_text(post.caption());
        }
        if ui.button(crate::trf!("Open chat with {title}", "Buka chat {title}")).clicked() {
            state.open_chat_requested = Some((e.session.id.clone(), e.poster.clone()));
        }
    });

    // Bottom: reactions, the comment field and (desktop) the note.
    let note_h = if phone { 0.0 } else { 44.0 };
    let comment = Rect::from_center_size(
        pos2(area.center().x, area.bottom() - note_h - 16.0 - 26.0),
        vec2((area.width() - pad * 2.0).min(560.0), 52.0),
    );
    let size = if phone { 40.0 } else { 44.0 };
    let reactions_y = comment.top() - 14.0 - size / 2.0;

    // The update itself.
    let top = head.bottom() + 14.0;
    let bottom = reactions_y - size / 2.0 - 14.0;
    let height = (bottom - top).max(120.0);
    let side_room = if phone { pad * 2.0 } else { 150.0 };
    let width = (height * 9.0 / 16.0).max(240.0).min(area.width() - side_room);
    let card = Rect::from_min_size(pos2(area.center().x - width / 2.0, top), vec2(width, height));
    commands.extend(paint_post(ui, p, card, post, &e.session.id, downloads, playing.as_ref()));
    // A click on a playing video pauses or resumes it, like the pause button.
    if playing.is_some() && ui.interact(card, ui.scope_id().with(("status_movie", &post.id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        state.status_paused = !state.status_paused;
    }
    if !phone {
        for (dir, icon, enabled) in [(-1.0, "chevron-left", state.status_index > 0), (1.0, "chevron-right", state.status_index + 1 < count)] {
            let cx = if dir < 0.0 { card.left() - 42.0 } else { card.right() + 42.0 };
            let r = Rect::from_center_size(pos2(cx, card.center().y), vec2(44.0, 44.0));
            if !enabled {
                continue;
            }
            if stage_button(ui, r, icon, true, false).clicked() {
                if dir < 0.0 {
                    state.status_index -= 1;
                } else {
                    state.status_index += 1;
                }
            }
            icons::paint(ui, Rect::from_center_size(r.center(), vec2(20.0, 20.0)), icon, 20.0, white);
        }
    } else {
        // Phone: tap the left or right third of the update to go back or on.
        let response = ui.interact(card, ui.scope_id().with(("status_tap", &post.id)), Sense::click());
        if response.clicked() && post.media.is_none() {
            let x = response.interact_pointer_pos().map(|p| p.x).unwrap_or(card.center().x);
            if x < card.left() + card.width() / 3.0 && state.status_index > 0 {
                state.status_index -= 1;
            } else if x > card.right() - card.width() / 3.0 && state.status_index + 1 < count {
                state.status_index += 1;
            }
        }
    }

    // Reactions: like, five quick emoji, and more emoji for the comment.
    let sent: Vec<String> = e
        .replies()
        .iter()
        .filter(|m| m.quote.as_ref().is_some_and(|q| q.id == post.id) && is_reaction(&m.text))
        .map(|m| m.text.trim().to_owned())
        .collect();
    let like_w = if phone { size } else { 92.0 };
    let n = QUICK.len() as f32 + 1.0;
    let spacing = if phone { 6.0 } else { 10.0 };
    let total = like_w + n * size + n * spacing;
    let mut cx = area.center().x - total / 2.0;
    let like = Rect::from_min_size(pos2(cx, reactions_y - size / 2.0), vec2(like_w, size));
    let liked = sent.iter().any(|s| s == LIKE);
    let like_resp = {
        let sense = if can_reply { Sense::click() } else { Sense::hover() };
        let r = ui.interact(like, ui.scope_id().with("status_like"), sense);
        let fill = if liked { theme::palette(ui.ctx()).orange_soft } else if r.hovered() && can_reply { Color32::from_white_alpha(46) } else { Color32::from_white_alpha(26) };
        ui.painter().rect_filled(like, size / 2.0, fill);
        let color = if liked { theme::FOREST_ORANGE } else { white };
        if phone {
            icons::paint(ui, Rect::from_center_size(like.center(), vec2(18.0, 18.0)), "heart", 18.0, color);
        } else {
            icons::paint(ui, Rect::from_center_size(pos2(like.left() + 26.0, like.center().y), vec2(18.0, 18.0)), "heart", 18.0, color);
            widgets::paint_text(ui, Rect::from_min_max(pos2(like.left() + 40.0, like.top()), like.max), tr("Like", "Suka"), fonts::ui(15.0, Weight::SemiBold), color, Align::Min);
        }
        r.on_hover_text(tr("Send ❤️ as a status reply", "Kirim ❤️ sebagai balasan status"))
    };
    if like_resp.clicked() && !liked {
        commands.push(Command::StatusReply { id: e.session.id.clone(), msg_id: post.id.clone(), text: LIKE.to_owned() });
    }
    cx += like_w + spacing;
    for emoji in QUICK {
        let r = Rect::from_min_size(pos2(cx, reactions_y - size / 2.0), vec2(size, size));
        let active = sent.iter().any(|s| s == emoji);
        if stage_button(ui, r, emoji, can_reply, active).on_hover_text(crate::trf!("Send {emoji}", "Kirim {emoji}")).clicked() && !active {
            commands.push(Command::StatusReply { id: e.session.id.clone(), msg_id: post.id.clone(), text: emoji.to_owned() });
        }
        widgets::paint_text(ui, r, emoji, fonts::ui(size * 0.45, Weight::Regular), white, Align::Center);
        cx += size + spacing;
    }
    let plus = Rect::from_min_size(pos2(cx, reactions_y - size / 2.0), vec2(size, size));
    let plus_resp = stage_button(ui, plus, "status_more_emoji", can_reply, false).on_hover_text(tr("More emoji for the comment", "Emoji lain untuk komentar"));
    icons::paint(ui, Rect::from_center_size(plus.center(), vec2(18.0, 18.0)), "plus", 18.0, white);
    if can_reply {
        crate::ui::emoji::picker(ui, &plus_resp, comment_id, &mut state.status_comment);
    }

    // The comment field.
    ui.painter().rect_filled(comment, 26.0, if can_reply { p.panel } else { Color32::from_white_alpha(30) });
    icons::paint(ui, Rect::from_center_size(pos2(comment.left() + 24.0, comment.center().y), vec2(18.0, 18.0)), "message-circle", 18.0, p.muted);
    let send = Rect::from_center_size(pos2(comment.right() - 26.0, comment.center().y), vec2(40.0, 40.0));
    let chip = crate::trf!("from {}", "dari {}", super::devices::chip_label(e.session));
    let chip_galley = ui.painter().layout_no_wrap(chip, fonts::ui(12.0, Weight::Medium), p.text_2);
    let chip_w = chip_galley.size().x + 32.0;
    let show_chip = !phone && comment.width() > 420.0;
    let edit_right = if show_chip { send.left() - chip_w - 16.0 } else { send.left() - 10.0 };
    if show_chip {
        let r = Rect::from_min_size(pos2(send.left() - chip_w - 8.0, comment.center().y - 13.0), vec2(chip_w, 26.0));
        ui.painter().rect_filled(r, 13.0, p.raised);
        icons::paint(ui, Rect::from_min_size(pos2(r.left() + 9.0, r.center().y - 6.5), vec2(13.0, 13.0)), "lock", 13.0, p.text_2);
        ui.painter().galley(pos2(r.left() + 26.0, r.center().y - chip_galley.size().y / 2.0), chip_galley, p.text_2);
    }
    let edit_rect = Rect::from_min_max(pos2(comment.left() + 44.0, comment.center().y - 11.0), pos2(edit_right, comment.center().y + 11.0));
    let hint = if !connected {
        crate::trf!("{} is not connected", "{} tidak terhubung", e.session.name)
    } else if post.from_me {
        tr("Your status", "Status Anda").to_owned()
    } else {
        crate::trf!("Comment on {title}'s status", "Komentari status {title}")
    };
    let edit = ui.add_enabled_ui(can_reply, |ui| {
        ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut state.status_comment)
                .id(comment_id)
                .frame(egui::Frame::NONE)
                .hint_text(rich(&hint, 14.0, Weight::Regular, p.muted))
                .font(fonts::ui(14.0, Weight::Regular))
                .text_color(p.text),
        )
    });
    let enter = edit.inner.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let send_resp = ui.interact(send, ui.scope_id().with("status_send"), if can_reply { Sense::click() } else { Sense::hover() });
    ui.painter().circle_filled(send.center(), 20.0, if can_reply { p.primary } else { p.border_strong });
    icons::paint(ui, Rect::from_center_size(send.center() + vec2(-1.0, 1.0), vec2(18.0, 18.0)), "send", 18.0, p.on_primary);
    let text = state.status_comment.trim().to_owned();
    if can_reply && (enter || send_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()) && !text.is_empty() {
        commands.push(Command::StatusReply { id: e.session.id.clone(), msg_id: post.id.clone(), text });
        state.status_comment.clear();
    }
    if !phone {
        let note = Rect::from_center_size(pos2(area.center().x, area.bottom() - 30.0), vec2((area.width() - pad * 2.0).min(560.0), 36.0));
        let text = crate::trf!(
            "Comments and reactions are sent from the number that views this status. Comments land in the Inbox as a chat with {title}, quoting the status.",
            "Komentar dan reaksi dikirim dari nomor yang melihat status ini. Komentar masuk ke Inbox sebagai chat {title} dengan kutipan statusnya."
        );
        let mut job = egui::text::LayoutJob::simple(text, fonts::ui(12.0, Weight::Regular), soft, note.width());
        job.halign = Align::Center;
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(note.center().x, note.top()), galley, soft);
    }

    // The "seen" receipt, once per update, unless this number keeps views private.
    let private = state.settings.status_seen_off.contains(&e.session.id);
    if !post.from_me && !post.seen && connected && !private && state.status_marked.insert(post.id.clone()) {
        commands.push(Command::StatusSeen { id: e.session.id.clone(), msg_id: post.id.clone() });
    }
    commands
}

/// The right panel: the poster's updates, what was sent to them, the
/// per-number "seen" switch and a way to the chat.
fn side(ui: &mut Ui, p: &Palette, state: &mut State, e: &Entry) -> Vec<Command> {
    let mut commands = Vec::new();
    ui.spacing_mut().item_spacing.y = 0.0;
    let title = e.title();
    ui.label(rich(tr("Reactions & comments", "Reaksi & komentar"), 18.0, Weight::SemiBold, p.text));
    ui.add_space(4.0);
    ui.add(egui::Label::new(rich(&crate::trf!("What you sent to {title}'s status", "Yang sudah kamu kirim ke status {title}"), 13.0, Weight::Regular, p.muted)).truncate());
    ui.add_space(16.0);
    let replies = e.replies();
    let liked = |id: &str| replies.iter().any(|m| m.text.trim() == LIKE && m.quote.as_ref().is_some_and(|q| q.id == id));

    // One tile per update, three to a row.
    let per_row = 3usize;
    let gap = 8.0;
    let tile_w = (ui.available_width() - gap * (per_row as f32 - 1.0)) / per_row as f32;
    for chunk in e.posts.chunks(per_row).enumerate() {
        let (row_i, posts) = chunk;
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 82.0), Sense::hover());
        for (j, post) in posts.iter().enumerate() {
            let i = row_i * per_row + j;
            let r = Rect::from_min_size(pos2(row.left() + j as f32 * (tile_w + gap), row.top()), vec2(tile_w, 82.0));
            let response = ui.interact(r, ui.scope_id().with(("status_tile", &post.id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let current = i == state.status_index;
            let (fill, text, sub_color, sub) = if current {
                (p.primary, p.on_primary, if forest() { p.accent } else { p.on_primary_muted }, tr("Viewing now", "Sedang dilihat"))
            } else if liked(&post.id) {
                (p.orange_soft, theme::FOREST_ORANGE, theme::FOREST_ORANGE, tr("Liked", "Disukai"))
            } else if post.seen {
                (p.raised, p.text_2, p.muted, tr("Seen", "Dilihat"))
            } else {
                (p.raised, p.text_2, p.muted, tr("Not seen", "Belum dilihat"))
            };
            ui.painter().rect_filled(r, theme::r_inner(), fill);
            if current && forest() {
                ui.painter().rect_stroke(r, theme::r_inner(), Stroke::new(2.0, p.accent), StrokeKind::Inside);
            }
            widgets::paint_text(ui, Rect::from_min_max(pos2(r.left() + 10.0, r.bottom() - 40.0), pos2(r.right() - 8.0, r.bottom() - 24.0)), &crate::trf!("Status {}", "Status {}", i + 1), fonts::ui(12.0, Weight::SemiBold), text, Align::Min);
            let mut sub_left = r.left() + 10.0;
            if !current && liked(&post.id) {
                icons::paint(ui, Rect::from_min_size(pos2(sub_left, r.bottom() - 21.0), vec2(12.0, 12.0)), "heart", 12.0, sub_color);
                sub_left += 16.0;
            }
            widgets::paint_text(ui, Rect::from_min_max(pos2(sub_left, r.bottom() - 23.0), pos2(r.right() - 8.0, r.bottom() - 7.0)), sub, fonts::ui(11.0, Weight::Regular), sub_color, Align::Min);
            if response.clicked() {
                state.status_index = i;
                state.status_shown = None;
            }
        }
        ui.add_space(gap);
    }
    ui.add_space(12.0);

    // What was sent, newest first.
    let bottom_h = 150.0;
    let height = (ui.available_height() - bottom_h).max(60.0);
    ScrollArea::vertical().id_salt("status_replies").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
        if replies.is_empty() {
            ui.add_space(8.0);
            ui.label(rich(tr("No reactions or comments on this status yet.", "Belum ada reaksi atau komentar ke status ini."), 13.0, Weight::Regular, p.muted));
        }
        for (k, m) in replies.iter().rev().enumerate() {
            let quote = m.quote.as_ref();
            let number = quote.and_then(|q| e.posts.iter().position(|s| s.id == q.id)).map_or(0, |i| i + 1);
            let text = m.text.trim();
            let (icon, label) = if text == LIKE {
                (None, crate::trf!("Liked status {number}", "Menyukai status {number}"))
            } else if is_reaction(text) {
                (Some(text), crate::trf!("Reaction {text} to status {number}", "Reaksi {text} ke status {number}"))
            } else {
                (None, crate::trf!("Comment on status {number}", "Komentar ke status {number}"))
            };
            if k > 0 {
                ui.add_space(10.0);
                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(line, 0.0, p.border);
            }
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (circle, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                let comment = icon.is_none() && text != LIKE;
                let fill = if text == LIKE { p.orange_soft } else if forest() && !comment { p.accent_soft } else { p.raised };
                ui.painter().circle_filled(circle.center(), 16.0, fill);
                match icon {
                    Some(emoji) => widgets::paint_text(ui, circle, emoji, fonts::ui(15.0, Weight::Regular), p.text, Align::Center),
                    None if text == LIKE => icons::paint(ui, Rect::from_center_size(circle.center(), vec2(15.0, 15.0)), "heart", 15.0, theme::FOREST_ORANGE),
                    None => icons::paint(ui, Rect::from_center_size(circle.center(), vec2(15.0, 15.0)), "message-circle", 15.0, p.text_2),
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(rich(&label, 14.0, Weight::SemiBold, p.text));
                    if comment {
                        egui::Frame::NONE.fill(p.raised).corner_radius(theme::r_inner().min(10)).inner_margin(Margin { left: 10, right: 10, top: 6, bottom: 8 }).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let quoted = quote.map(|q| q.text.as_str()).unwrap_or_default();
                            ui.add(egui::Label::new(rich(&format!("Status · {quoted}"), 11.0, Weight::Regular, p.muted)).truncate());
                            ui.add(egui::Label::new(rich(text, 13.0, Weight::Regular, p.text)).wrap());
                        });
                    }
                    let status = super::inbox::delivery_label(&m.status);
                    let meta = crate::trf!(
                        "{} · from {}{}",
                        "{} · dari {}{}",
                        time::clock(m.ts),
                        super::devices::chip_label(e.session),
                        if status.is_empty() { String::new() } else { format!(" · {status}") }
                    );
                    ui.label(rich(&meta, 12.0, Weight::Regular, p.muted));
                });
            });
        }
    });

    // Pinned to the bottom: the "seen" switch and the chat.
    let rest = ui.available_rect_before_wrap();
    let bottom = Rect::from_min_max(pos2(rest.left(), rest.bottom() - bottom_h + 14.0), rest.max);
    let mut ui = widgets::child(ui, bottom, egui::Layout::top_down(Align::Min));
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::Frame::NONE.fill(p.raised).corner_radius(theme::r_inner()).inner_margin(Margin::same(14)).show(&mut ui, |ui| {
        ui.set_width(ui.available_width());
        let mut on = !state.settings.status_seen_off.contains(&e.session.id);
        let before = on;
        widgets::row(ui, 44.0, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(rich(tr("Send seen receipts", "Kirim tanda dilihat"), 14.0, Weight::SemiBold, p.text));
                ui.add(egui::Label::new(rich(tr("Contacts know this number has seen the status", "Kontak tahu status sudah dilihat nomor ini"), 12.0, Weight::Regular, p.muted)).wrap());
            });
        }, |ui| {
            widgets::toggle(ui, &mut on, "");
        });
        if on != before {
            let mut settings = (*state.settings).clone();
            settings.status_seen_off.retain(|id| id != &e.session.id);
            if !on {
                settings.status_seen_off.push(e.session.id.clone());
            }
            commands.push(Command::SaveSettings(settings));
        }
    });
    ui.add_space(12.0);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
    ui.painter().rect(rect, 21.0, if response.hovered() { p.raised } else { p.panel }, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    let label = ui.painter().layout_no_wrap(crate::trf!("Open chat with {title}", "Buka chat {title}"), fonts::ui(14.0, Weight::Medium), p.text);
    let total = label.size().x + 26.0;
    let left = rect.center().x - total / 2.0;
    icons::paint(&ui, Rect::from_center_size(pos2(left + 8.0, rect.center().y), vec2(16.0, 16.0)), "message-circle", 16.0, p.text);
    ui.painter().galley(pos2(left + 26.0, rect.center().y - label.size().y / 2.0), label, p.text);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        state.open_chat_requested = Some((e.session.id.clone(), e.poster.clone()));
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::is_reaction;

    #[test]
    fn one_emoji_is_a_reaction() {
        assert!(is_reaction("👍") && is_reaction("❤️") && is_reaction(" 🙏 "));
        assert!(!is_reaction("ok") && !is_reaction("Bu, saya ikut 2 porsi") && !is_reaction(""));
    }
}
