//! The profile photo viewer (#92): a pop-up over the window with the
//! picture at full size, the name above it. The preview shows until the
//! full-size picture arrives. Esc, × or a click outside closes it.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Color32, Frame, Margin, Stroke, vec2};

use crate::i18n::tr;
use crate::ui::fonts::Weight;
use crate::ui::theme::R_LG;
use crate::ui::widgets::{self, rich};

/// The photo being looked at.
pub struct Viewer {
    pub jid: String,
    pub title: String,
    /// The small picture already on screen, shown while the full one loads.
    pub preview: Option<PathBuf>,
}

pub fn show(ctx: &egui::Context, open: &mut Option<Viewer>, full: &HashMap<String, Option<PathBuf>>) {
    let Some(viewer) = open.as_ref() else { return };
    let screen = ctx.content_rect();
    let white = Color32::WHITE;
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("photo_viewer"))
        .backdrop_color(Color32::from_black_alpha(200))
        .frame(Frame::NONE.fill(Color32::from_rgb(0x16, 0x18, 0x17)).stroke(Stroke::new(1.0, Color32::from_white_alpha(30))).corner_radius(R_LG).inner_margin(Margin::same(14)))
        .show(ctx, |ui| {
            let limit = vec2((screen.width() - 96.0).max(200.0), (screen.height() - 150.0).max(200.0));
            let (picture, loading) = match full.get(&viewer.jid) {
                Some(Some(path)) => (Some(path.clone()), false),
                Some(None) => (viewer.preview.clone(), false),
                None => (viewer.preview.clone(), true),
            };
            widgets::row(
                ui,
                30.0,
                |ui| {
                    ui.add(egui::Label::new(rich(&viewer.title, 14.0, Weight::SemiBold, white)).truncate());
                    if loading {
                        ui.add_space(8.0);
                        ui.label(rich(tr("Loading full size…", "Memuat ukuran penuh…"), 12.0, Weight::Regular, Color32::from_white_alpha(150)));
                    }
                },
                |ui| {
                    close = widgets::icon_button(ui, "x", 28.0, false).on_hover_text(tr("Close (Esc)", "Tutup (Esc)")).clicked();
                },
            );
            ui.add_space(10.0);
            match picture.and_then(|path| widgets::picture_uri(ui.ctx(), &path)) {
                // The photo at its own size, never past it, fitted to the window.
                Some(uri) => {
                    let shown = if loading {
                        // The small preview stands in at a size worth looking at.
                        egui::Image::from_uri(uri).fit_to_exact_size(vec2(limit.x.min(limit.y).min(480.0), limit.x.min(limit.y).min(480.0)))
                    } else {
                        egui::Image::from_uri(uri).max_size(limit).maintain_aspect_ratio(true)
                    };
                    ui.add(shown.corner_radius(8));
                }
                None => {
                    ui.set_min_size(vec2(320.0, 200.0));
                    ui.label(rich(tr("No profile photo", "Tidak ada foto profil"), 13.0, Weight::Regular, Color32::from_white_alpha(170)));
                }
            }
        });
    if close || modal.should_close() {
        *open = None;
    }
}
