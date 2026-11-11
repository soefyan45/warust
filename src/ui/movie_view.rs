//! The in-app video viewer for chats (#84): the video large on a dark stage,
//! play/pause, a seek bar with the time, mute, and the system player as a
//! way out. Esc or × closes it and stops the video.

use std::path::PathBuf;

use egui::{Align, Color32, CursorIcon, Frame, Margin, Rect, Sense, Stroke, pos2, vec2};

use crate::backend::movie;
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, R_LG};
use crate::ui::widgets::{self, Kind, rich};
use crate::ui::icons;

/// The video open in the viewer.
pub struct Viewer {
    /// Player id: the file itself, so the same video reopens where it was.
    id: String,
    pub file: PathBuf,
    pub title: String,
}

impl Viewer {
    pub fn new(file: PathBuf, title: String) -> Self {
        Self { id: file.to_string_lossy().into_owned(), file, title }
    }
}

/// "1:05", or "1:02:05" past an hour.
fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

pub fn show(ctx: &egui::Context, open: &mut Option<Viewer>) {
    let Some(viewer) = open.as_ref() else { return };
    let p = theme::palette(ctx);
    let screen = ctx.content_rect();
    if movie::status(&viewer.id).is_none() {
        movie::play(&viewer.id, &viewer.file, movie::repaint_for(ctx));
    }
    let status = movie::status(&viewer.id);
    // A paused video still needs asking about now and then, or it stops.
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    let mut close = false;
    let size = vec2((screen.width() - 80.0).clamp(320.0, 1100.0), (screen.height() - 80.0).clamp(300.0, 900.0));
    let modal = egui::Modal::new(egui::Id::unique("movie_viewer"))
        .frame(Frame::NONE.fill(p.panel).stroke(Stroke::new(1.0, p.border_strong)).corner_radius(R_LG).inner_margin(Margin::same(16)))
        .show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            widgets::row(
                ui,
                32.0,
                |ui| {
                    icons::show(ui, "video", 16.0, p.text_2);
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(rich(&viewer.title, 14.0, Weight::SemiBold, p.text)).truncate());
                },
                |ui| {
                    close = widgets::icon_button(ui, "x", 28.0, false).on_hover_text(tr("Close (Esc)", "Tutup (Esc)")).clicked();
                    ui.add_space(8.0);
                    if widgets::button(ui, None, tr("Open in system player", "Buka di pemutar sistem"), Kind::Secondary).clicked() {
                        movie::stop();
                        crate::platform::open(&viewer.file);
                        close = true;
                    }
                },
            );
            ui.add_space(10.0);
            // The stage, with the video fitted in it; a click pauses or plays.
            let controls_h = 44.0;
            let stage_h = (ui.available_height() - controls_h - 10.0).max(120.0);
            let (stage, response) = ui.allocate_exact_size(vec2(ui.available_width(), stage_h), Sense::click());
            ui.painter().rect_filled(stage, R_LG, Color32::from_rgb(0x12, 0x14, 0x13));
            let texture = status.as_ref().and_then(|s| widgets::frame_texture(ui.ctx(), "chat_video", &s.picture));
            if let Some(texture) = &texture {
                let tex = texture.size_vec2();
                let scale = (stage.width() / tex.x).min(stage.height() / tex.y);
                let shown = Rect::from_center_size(stage.center(), tex * scale);
                egui::Image::from_texture(texture).fit_to_exact_size(shown.size()).paint_at(ui, shown);
            }
            let loading = status.as_ref().is_none_or(|s| s.loading) && texture.is_none();
            let paused = status.as_ref().is_some_and(|s| !s.playing);
            if loading {
                widgets::paint_text(ui, stage, tr("Loading…", "Memuat…"), fonts::ui(13.0, Weight::Medium), Color32::from_white_alpha(200), Align::Center);
            } else if paused {
                ui.painter().circle_filled(stage.center(), 30.0, Color32::from_black_alpha(150));
                icons::paint(ui, Rect::from_center_size(stage.center(), vec2(26.0, 26.0)), "play", 26.0, Color32::WHITE);
            }
            if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                movie::toggle(&viewer.id);
            }
            ui.add_space(10.0);
            // Controls: play/pause, the seek bar with the time, mute.
            let Some(s) = status.as_ref() else { return };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if widgets::icon_button(ui, if s.playing { "pause" } else { "play" }, 32.0, false).on_hover_text(if s.playing { tr("Pause", "Jeda") } else { tr("Play", "Putar") }).clicked() {
                    movie::toggle(&viewer.id);
                }
                let time = format!("{} / {}", clock(s.position), clock(s.duration));
                let time_w = ui.painter().layout_no_wrap(time.clone(), fonts::mono(12.0), p.muted).size().x;
                let bar_w = (ui.available_width() - time_w - 60.0).max(60.0);
                let (bar, seek) = ui.allocate_exact_size(vec2(bar_w, 24.0), Sense::click_and_drag());
                let track = Rect::from_center_size(bar.center(), vec2(bar.width(), 4.0));
                ui.painter().rect_filled(track, 2.0, p.border);
                let done = if s.duration > 0.0 { (s.position / s.duration).clamp(0.0, 1.0) } else { 0.0 };
                ui.painter().rect_filled(Rect::from_min_size(track.min, vec2(track.width() * done, 4.0)), 2.0, p.primary);
                let knob = ui.ctx().animate_bool_with_time(seek.id, seek.hovered() || seek.dragged(), 0.12);
                ui.painter().circle_filled(pos2(track.left() + track.width() * done, track.center().y), 5.0 + 2.0 * knob, p.primary);
                if seek.clicked() || seek.drag_stopped() {
                    if let Some(at) = seek.interact_pointer_pos() {
                        movie::seek(&viewer.id, (at.x - track.left()) / track.width().max(1.0));
                    }
                }
                ui.label(widgets::rich_mono(&time, 12.0, p.muted));
                let hint = if s.muted { tr("Sound on", "Nyalakan suara") } else { tr("Mute", "Bisukan") };
                if widgets::icon_button(ui, if s.muted { "volume-x" } else { "volume-2" }, 32.0, s.muted).on_hover_text(hint).clicked() {
                    movie::set_muted(!s.muted);
                }
            });
        });
    if close || modal.should_close() {
        movie::stop();
        *open = None;
    }
}
