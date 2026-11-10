//! "Teruskan ke…" dialog: pick up to five chats of the same number and queue
//! the message to each (the paced send queue, so anti-ban gaps apply).

use egui::{Frame, Margin, ScrollArea, Stroke};

use crate::backend::{Command, SessionView};
use crate::ui::fonts::Weight;
use crate::ui::theme::{self, R_LG};
use crate::ui::widgets::{self, Kind, rich};
use crate::ui::screens::chat_title;

/// WhatsApp forwards to at most this many chats at once.
const MAX_TARGETS: usize = 5;

pub struct Dialog {
    /// The number the message belongs to; the chats offered are its own.
    session: String,
    text: String,
    search: String,
    picked: Vec<String>,
}

impl Dialog {
    pub fn new(session: String, text: String) -> Self {
        Self { session, text, search: String::new(), picked: Vec::new() }
    }
}

/// Draws the dialog over the window and returns the commands it produced;
/// clears `open` when it is closed or sent.
pub fn show(ctx: &egui::Context, open: &mut Option<Dialog>, sessions: &[SessionView]) -> Vec<Command> {
    let Some(dialog) = open.as_mut() else { return Vec::new() };
    let Some(session) = sessions.iter().find(|s| s.id == dialog.session) else {
        *open = None;
        return Vec::new();
    };
    let p = theme::palette(ctx);
    let width = 380.0_f32.min(ctx.content_rect().width() - 48.0).max(240.0);
    let mut commands = Vec::new();
    let (mut close, mut send) = (false, false);
    egui::Modal::new(egui::Id::NULL.with("forward_dialog"))
        .frame(
            Frame::NONE
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(R_LG)
                .inner_margin(Margin::same(20)),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(rich("Teruskan ke…", 16.0, Weight::SemiBold, p.text));
            widgets::search(ui, &mut dialog.search, "Cari chat", width, false);
            let query = dialog.search.trim().to_lowercase();
            let rows: Vec<(&str, String)> = session
                .book
                .chats
                .iter()
                .map(|c| (c.jid.as_str(), chat_title(&session.book, c)))
                .filter(|(jid, title)| query.is_empty() || title.to_lowercase().contains(&query) || jid.contains(&query))
                .collect();
            ScrollArea::vertical().max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
                if rows.is_empty() {
                    ui.label(rich("Tidak ada chat yang cocok.", 13.0, Weight::Regular, p.muted));
                }
                for (jid, title) in &rows {
                    let at = dialog.picked.iter().position(|j| j == jid);
                    let full = dialog.picked.len() >= MAX_TARGETS;
                    let mut checked = at.is_some();
                    let row = ui.add_enabled(at.is_some() || !full, egui::Checkbox::new(&mut checked, rich(title, 13.0, Weight::Regular, p.text)));
                    if row.changed() {
                        match at {
                            Some(i) => {
                                dialog.picked.remove(i);
                            }
                            None => dialog.picked.push((*jid).to_owned()),
                        }
                    }
                }
            });
            ui.label(rich(&format!("{} dari {MAX_TARGETS} dipilih", dialog.picked.len()), 12.0, Weight::Regular, p.muted));
            widgets::row(
                ui,
                32.0,
                |_| {},
                |ui| {
                    ui.add_enabled_ui(!dialog.picked.is_empty(), |ui| {
                        if widgets::button(ui, Some("send"), "Kirim", Kind::Primary).clicked() {
                            send = true;
                        }
                    });
                    if widgets::button(ui, None, "Batal", Kind::Secondary).clicked() {
                        close = true;
                    }
                },
            );
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if send {
        commands.push(Command::Forward { id: session.id.clone(), chats: dialog.picked.clone(), text: dialog.text.clone() });
        close = true;
    }
    if close {
        *open = None;
    }
    commands
}
