//! Log screen (Pego `v4/Log`): every message of every session, newest first,
//! and the detail panel for the selected one.

use egui::{Align, CentralPanel, Frame, Margin, Panel, ScrollArea, Sense, Stroke, Ui, vec2};

use crate::backend::{Chat, ChatMsg, Delivery, SessionView};
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette, R_MD};
use crate::ui::widgets::{self, rich, rich_mono};
use crate::ui::{State, icons, time};

const COLS: [Option<f32>; 6] = [Some(76.0), Some(16.0), Some(170.0), None, Some(100.0), Some(110.0)];
/// Narrow windows (phone layout): time, direction, contact and status; the
/// text and the number show in the detail.
const NARROW: [Option<f32>; 4] = [Some(64.0), Some(16.0), None, Some(84.0)];
const ROW: f32 = 44.0;

/// The cells of a header or row; `None` where a narrow window drops the column.
fn cells(rect: egui::Rect) -> [Option<egui::Rect>; 6] {
    let inner = rect.shrink2(vec2(12.0, 0.0));
    if inner.width() < 520.0 {
        let c = widgets::columns(inner, &NARROW, 10.0);
        [Some(c[0]), Some(c[1]), Some(c[2]), None, Some(c[3]), None]
    } else {
        let c = widgets::columns(inner, &COLS, 14.0);
        [Some(c[0]), Some(c[1]), Some(c[2]), Some(c[3]), Some(c[4]), Some(c[5])]
    }
}
/// The log shows the latest messages; the store keeps everything.
const LIMIT: usize = 500;

struct Entry<'a> {
    session: &'a SessionView,
    chat: &'a Chat,
    message: &'a ChatMsg,
}

fn status_of(p: &Palette, m: &ChatMsg) -> (egui::Color32, &'static str) {
    let color = match m.status {
        Delivery::Received => p.gray,
        Delivery::Queued => p.amber,
        Delivery::Sent => p.text_2,
        Delivery::Delivered | Delivery::Read => p.green,
        Delivery::Failed(_) => p.red,
    };
    (color, status_label(&m.status))
}

pub fn show(ui: &mut Ui, state: &mut State, sessions: &[SessionView]) {
    let p = theme::palette(ui.ctx());
    let mut entries: Vec<Entry> = sessions
        .iter()
        .flat_map(|s| {
            s.book.chats.iter().flat_map(move |c| {
                c.messages.iter().map(move |m| Entry { session: s, chat: c, message: m })
            })
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.message.ts));
    let failed = entries.iter().filter(|e| matches!(e.message.status, Delivery::Failed(_))).count();
    let visible: Vec<&Entry> = entries
        .iter()
        .filter(|e| match state.log_tab {
            1 => e.message.from_me,
            2 => !e.message.from_me,
            3 => matches!(e.message.status, Delivery::Failed(_)),
            _ => true,
        })
        .take(LIMIT)
        .collect();

    let selected = state.log_selected.as_ref().and_then(|(sid, mid)| {
        entries.iter().find(|e| &e.session.id == sid && &e.message.id == mid)
    });
    if let Some(entry) = selected {
        let mut close = false;
        Panel::right("log_detail")
            .default_size(380.0)
            .size_range(320.0..=520.0)
            .resizable(true)
            .show_separator_line(!theme::template().top_nav())
            .frame(theme::pane(ui.ctx(), p.panel, Margin::same(24)))
            .show(ui, |ui| close = detail(ui, &p, entry, &super::webhook_label(state, &entry.session.id)));
        if close {
            state.log_selected = None;
        }
    }

    CentralPanel::default()
        .frame(theme::page(ui.ctx()))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            let today = time::day(time::now());
            let count_today = entries.iter().filter(|e| time::day(e.message.ts) == today).count();
            super::page_head(
                ui,
                "Log pesan",
                &format!("{count_today} pesan hari ini dari {} nomor. Pilih baris untuk melihat detail.", sessions.len()),
            );
            if let Some((note, at)) = &state.export_note {
                if at.elapsed().as_secs() < 10 {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    ui.horizontal(|ui| {
                        icons::show(ui, "check", 14.0, p.green);
                        ui.label(rich(note, 12.0, Weight::Regular, p.text_2));
                    });
                }
            }
            theme::surface(ui.ctx()).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let labels = ["Semua".to_owned(), "Keluar".to_owned(), "Masuk".to_owned(), format!("Gagal {failed}")];
                widgets::row(
                    ui,
                    36.0,
                    |ui| {
                        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                        widgets::tabs(ui, &labels, &mut state.log_tab);
                    },
                    |ui| {
                        // Same right edge as the table's cells.
                        ui.add_space(12.0);
                        if visible.len() == LIMIT {
                            ui.label(rich(&format!("{LIMIT} terbaru"), 12.0, Weight::Regular, p.muted));
                        }
                    },
                );
                // Classic tabs sit on a rule; Forest's pills need none.
                if !theme::template().top_nav() {
                    widgets::hline(ui, ui.max_rect(), ui.cursor().top() - 0.5, p.border);
                }
                ui.add_space(8.0);
                if visible.is_empty() {
                    super::empty_state(ui, &p, "scroll-text", "Belum ada pesan", "Pesan masuk dan keluar dari semua nomor muncul di sini.");
                    return;
                }
                header(ui, &p);
                ScrollArea::vertical()
                    .auto_shrink(false)
                    .show_rows(ui, ROW, visible.len(), |ui, range| {
                        for entry in &visible[range] {
                            row(ui, &p, state, entry);
                        }
                    });
            });
            });
        });
}

/// Writes every message of every session to `data/exports/log-<time>.csv`,
/// newest first. Numbers stay masked like everywhere in the UI.
pub fn export_csv(sessions: &[SessionView]) -> std::io::Result<(std::path::PathBuf, usize)> {
    let mut entries: Vec<Entry> = sessions
        .iter()
        .flat_map(|s| {
            s.book.chats.iter().flat_map(move |c| {
                c.messages.iter().map(move |m| Entry { session: s, chat: c, message: m })
            })
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.message.ts));
    let field = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
    let mut csv = String::from("waktu,arah,lawan_bicara,jid,isi,status,alasan_gagal,nomor,id_pesan\n");
    for e in &entries {
        let m = e.message;
        let reason = match &m.status {
            Delivery::Failed(reason) => reason.as_str(),
            _ => "",
        };
        let row = [
            time::full(m.ts),
            if m.from_me { "keluar".into() } else { "masuk".into() },
            super::chat_title(&e.session.book, e.chat),
            super::short_jid(&e.chat.jid),
            m.text.clone(),
            status_label(&m.status).to_owned(),
            reason.to_owned(),
            e.session.name.clone(),
            m.id.clone(),
        ];
        csv.push_str(&row.iter().map(|f| field(f)).collect::<Vec<_>>().join(","));
        csv.push('\n');
    }
    let dir = crate::platform::data_dir().join("exports");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("log-{}.csv", time::stamp(time::now())));
    std::fs::write(&path, csv)?;
    Ok((path, entries.len()))
}

fn status_label(status: &Delivery) -> &'static str {
    match status {
        Delivery::Received => "Diterima",
        Delivery::Queued => "Antre",
        Delivery::Sent => "Terkirim",
        Delivery::Delivered => "Sampai",
        Delivery::Read => "Dibaca",
        Delivery::Failed(_) => "Gagal",
    }
}

fn header(ui: &mut Ui, p: &Palette) {
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    widgets::hline(ui, head, head.bottom() - 0.5, p.border);
    let cols = cells(head);
    for (i, title) in ["Waktu", "", "Lawan bicara", "Isi", "Status", "Nomor"].iter().enumerate() {
        if let Some(cell) = cols[i] {
            widgets::paint_text(ui, cell, title, fonts::ui(12.0, Weight::Medium), p.muted, Align::Min);
        }
    }
}

fn row(ui: &mut Ui, p: &Palette, state: &mut State, entry: &Entry) {
    let m = entry.message;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let key = (entry.session.id.clone(), m.id.clone());
    let selected = state.log_selected.as_ref() == Some(&key);
    if selected {
        ui.painter().rect_filled(rect, 0.0, p.raised);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.panel);
    }
    widgets::hline(ui, rect, rect.bottom() - 0.5, p.border);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        state.log_selected = Some(key);
    }
    let [time_c, dir_c, who_c, text_c, status_c, number_c] = cells(rect);
    let when = if time::day(m.ts) == time::day(time::now()) { time::clock_secs(m.ts) } else { time::day_label(m.ts) };
    if let Some(c) = time_c {
        widgets::paint_text(ui, c, &when, fonts::mono(12.0), p.muted, Align::Min);
    }
    let dir = if m.from_me { "arrow-up-right" } else { "arrow-down-left" };
    if let Some(c) = dir_c {
        icons::paint(ui, c, dir, 16.0, p.text_2);
    }
    if let Some(c) = who_c {
        widgets::paint_text(ui, c, &super::chat_title(&entry.session.book, entry.chat), fonts::ui(13.0, Weight::Regular), p.text, Align::Min);
    }
    if let Some(c) = text_c {
        widgets::paint_text(ui, c, &m.text.replace('\n', " "), fonts::ui(13.0, Weight::Regular), p.text, Align::Min);
    }
    let (color, label) = status_of(p, m);
    if let Some(c) = status_c {
        widgets::paint_status(ui, c, color, label);
    }
    if let Some(c) = number_c {
        widgets::paint_text(ui, c, &entry.session.name, fonts::ui(12.0, Weight::Regular), p.muted, Align::Min);
    }
}

/// Returns true when the panel asks to close.
fn detail(ui: &mut Ui, p: &Palette, entry: &Entry, webhook: &str) -> bool {
    let m = entry.message;
    let mut close = false;
    ui.spacing_mut().item_spacing.y = 20.0;
    widgets::row(
        ui,
        40.0,
        |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(rich("Detail pesan", 15.0, Weight::SemiBold, p.text));
                ui.label(rich_mono(&m.id, 12.0, p.muted));
            });
        },
        |ui| close = widgets::icon_button(ui, "x", 28.0, false).clicked(),
    );

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let chat = super::chat_title(&entry.session.book, entry.chat);
        let (from, to) = if m.from_me { (entry.session.name.clone(), chat) } else { (chat, entry.session.name.clone()) };
        kv(ui, p, "Dari", &from);
        kv(ui, p, "Ke", &to);
        kv(ui, p, "Waktu", &time::full(m.ts));
        let (_, label) = status_of(p, m);
        kv(ui, p, "Status", label);
        if let Delivery::Failed(reason) = &m.status {
            kv(ui, p, "Alasan", reason);
        }
        if entry.chat.is_group() && !m.from_me {
            kv(ui, p, "Pengirim", &super::sender_label(&entry.session.book, &m.sender));
        }
    });

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        widgets::section_label(ui, "Isi pesan");
        Frame::NONE
            .fill(p.bg)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(R_MD)
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(rich(&m.text, 13.0, Weight::Regular, p.text_2)).wrap());
            });
    });

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        widgets::section_label(ui, "Webhook nomor ini");
        ui.label(rich(webhook, 12.0, Weight::Regular, p.muted));
    });
    close
}

fn kv(ui: &mut Ui, p: &Palette, key: &str, value: &str) {
    widgets::row(
        ui,
        18.0,
        |ui| {
            ui.label(rich(key, 12.0, Weight::Regular, p.muted));
        },
        |ui| {
            ui.label(rich(value, 13.0, Weight::Regular, p.text));
        },
    );
}
