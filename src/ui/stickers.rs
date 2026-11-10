//! Sticker panel for the composer: stickers received in this number's chats
//! (once downloaded) and the user's own collection in `data/stickers/`.
//! A click sends the sticker right away, like WhatsApp.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{PopupCloseBehavior, Response, ScrollArea, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::backend::{Book, make_sticker};

use super::fonts::Weight;
use super::theme::{self, R_MD};
use super::widgets::{self, Kind, rich};

const CELL: f32 = 76.0;
const COLUMNS: usize = 4;
const RECENT: usize = 16;

/// True while a picked file is being turned into a sticker.
static ADDING: AtomicBool = AtomicBool::new(false);

pub fn dir() -> PathBuf {
    crate::platform::data_dir().join("stickers")
}

/// The collection, newest first.
fn collection() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("webp")))
        .filter_map(|p| Some((std::fs::metadata(&p).and_then(|m| m.modified()).ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, p)| p).collect()
}

/// Downloaded stickers from this number's chats, newest first.
fn received(book: &Book) -> Vec<PathBuf> {
    let mut found: Vec<(i64, PathBuf)> = book
        .chats
        .iter()
        .flat_map(|c| &c.messages)
        .filter_map(|m| {
            let media = m.media.as_ref().filter(|media| media.kind == "sticker")?;
            Some((m.ts, media.file.clone()?))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    let mut seen = std::collections::HashSet::new();
    found
        .into_iter()
        .map(|(_, p)| p)
        .filter(|p| p.exists() && seen.insert(p.clone()))
        .take(RECENT)
        .collect()
}

/// Asks for a picture with the OS's dialog and adds it to the collection, off the UI thread.
fn add_from_file(ctx: &egui::Context) {
    if ADDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let picked = crate::platform::pick_file("Pilih gambar untuk dijadikan stiker", true);
        if let Some(source) = picked {
            if let Err(e) = make_sticker(&source, &dir()) {
                log::warn!("cannot add a sticker: {e}");
            }
        }
        ADDING.store(false, Ordering::SeqCst);
        ctx.request_repaint();
    });
}

fn grid(ui: &mut Ui, files: &[PathBuf], enabled: bool) -> Option<PathBuf> {
    let p = theme::palette(ui.ctx());
    let mut picked = None;
    for chunk in files.chunks(COLUMNS) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for file in chunk {
                let sense = if enabled { Sense::click() } else { Sense::hover() };
                let (rect, response) = ui.allocate_exact_size(vec2(CELL, CELL), sense);
                if response.hovered() && enabled {
                    ui.painter().rect_filled(rect, R_MD, p.raised);
                }
                if let Some(uri) = widgets::picture_uri(ui.ctx(), file) {
                    egui::Image::from_uri(uri)
                        .fit_to_exact_size(rect.shrink(6.0).size())
                        .paint_at(ui, rect.shrink(6.0));
                } else {
                    ui.painter().rect(rect.shrink(6.0), R_MD, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
                }
                if response.clicked() {
                    picked = Some(file.clone());
                }
            }
        });
    }
    picked
}

/// Opens the panel under `button`; returns the sticker to send.
pub fn picker(ui: &Ui, button: &Response, book: &Book, connected: bool) -> Option<PathBuf> {
    let p = theme::palette(ui.ctx());
    let mut picked = None;
    egui::Popup::menu(button)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(COLUMNS as f32 * CELL + 16.0)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let recent = received(book);
            let own = collection();
            ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                widgets::section_label(ui, "Koleksi saya");
                if own.is_empty() {
                    ui.label(rich("Belum ada. Tambahkan gambar PNG/JPG/WebP.", 12.0, Weight::Regular, p.muted));
                }
                picked = picked.take().or(grid(ui, &own, connected));
                widgets::section_label(ui, "Diterima di chat");
                if recent.is_empty() {
                    ui.label(rich("Stiker yang masuk akan muncul di sini.", 12.0, Weight::Regular, p.muted));
                }
                picked = picked.take().or(grid(ui, &recent, connected));
            });
            ui.separator();
            ui.horizontal(|ui| {
                let adding = ADDING.load(Ordering::SeqCst);
                let label = if adding { "Memproses…" } else { "Tambah dari file" };
                if widgets::button(ui, Some("plus"), label, Kind::Secondary).clicked() && !adding {
                    add_from_file(ui.ctx());
                }
                if widgets::button(ui, None, "Buka folder", Kind::Secondary).clicked() {
                    let _ = std::fs::create_dir_all(dir());
                    crate::platform::open(dir());
                }
            });
            if !connected {
                ui.label(rich("Nomor tidak terhubung, stiker belum bisa dikirim", 12.0, Weight::Regular, p.red_text));
            }
            if picked.is_some() {
                ui.close();
            }
        });
    picked
}
