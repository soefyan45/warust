//! The voice call on screen (issue #39, Pego `v5/Call · Panggilan masuk`,
//! `v5/Call · Panggilan suara`, `m5/Call 1`, `m5/Call 2`, `FM/Call/Control`):
//! - ringing: a forest card at the top right (full screen in the phone
//!   layout) with the caller, how long it has rung, their last message, and
//!   Decline / Reply with message / Answer (Ctrl+Shift+A / Ctrl+Shift+D);
//! - in the call: a forest panel at the right with the caller's picture in
//!   rings that move with their voice, the time, "speaking", and Mute /
//!   Speaker / Video / End. The phone layout can make it a banner.

use egui::{Align2, Color32, Context, CursorIcon, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::backend::call::{CallView, Phase};
use crate::backend::video::Latest;
use crate::backend::{Command, SessionView};
use crate::i18n::tr;
use crate::ui::fonts::{self, Weight};
use crate::ui::theme::{self, Palette};
use crate::ui::{State, icons, privacy, widgets};

/// Draws the call; returns what the user asked of it.
pub fn show(ctx: &Context, state: &mut State, call: &CallView, sessions: &[SessionView], compact: bool) -> Vec<Command> {
    let p = theme::palette(ctx);
    let session = sessions.iter().find(|s| s.id == call.session);
    let mut commands = Vec::new();
    // Keeps the timers and the rings moving.
    ctx.request_repaint_after(std::time::Duration::from_millis(33));
    if call.phase == Phase::Ringing {
        state.call_minimized = false;
        let answer = ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::A)));
        let decline = ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::D)));
        let asked = if compact { ringing_full(ctx, &p, call, session) } else { ringing_card(ctx, &p, call, session) };
        match asked {
            _ if answer => commands.push(Command::AnswerCall),
            _ if decline => commands.push(Command::DeclineCall),
            Some(Ask::Answer) => commands.push(Command::AnswerCall),
            Some(Ask::Decline) => commands.push(Command::DeclineCall),
            Some(Ask::Reply) => {
                commands.push(Command::DeclineCall);
                state.open_chat_requested = Some((call.session.clone(), call.peer.clone()));
            }
            _ => {}
        }
        return commands;
    }
    let asked = if compact && state.call_minimized {
        banner(ctx, &p, call)
    } else if call.video_on {
        video_call(ctx, &p, call, session, &mut state.call_textures, compact)
    } else if compact {
        in_call(ctx, &p, call, session, true)
    } else {
        in_call(ctx, &p, call, session, false)
    };
    match asked {
        Some(Ask::End) => commands.push(Command::EndCall),
        Some(Ask::Camera) => commands.push(Command::CameraCall(!call.camera_on)),
        Some(Ask::StartVideo) => commands.push(Command::StartVideo),
        Some(Ask::AcceptVideo) => commands.push(Command::AcceptVideo),
        Some(Ask::DeclineVideo) => commands.push(Command::DeclineVideo),
        Some(Ask::Mute) => commands.push(Command::MuteCall(!call.muted)),
        Some(Ask::Speaker) => commands.push(Command::SpeakerOff(!call.speaker_off)),
        Some(Ask::Minimize) => state.call_minimized = true,
        Some(Ask::Restore) => state.call_minimized = false,
        _ => {}
    }
    commands
}

enum Ask {
    Answer,
    Decline,
    Reply,
    End,
    Mute,
    Speaker,
    Minimize,
    Restore,
    Camera,
    StartVideo,
    AcceptVideo,
    DeclineVideo,
}

/// The call's video pictures uploaded as textures, redone only when a new
/// picture arrives.
#[derive(Default)]
pub struct Textures {
    remote: Option<(egui::TextureHandle, u64)>,
    local: Option<(egui::TextureHandle, u64)>,
}

/// The texture of `latest`, refreshed when its picture changed.
fn texture(ctx: &Context, slot: &mut Option<(egui::TextureHandle, u64)>, latest: &Latest, name: &str) -> Option<egui::TextureHandle> {
    let picture = latest.lock().ok()?.clone();
    let Some(picture) = picture else {
        *slot = None;
        return None;
    };
    let fresh = slot.as_ref().is_none_or(|(_, generation)| *generation != picture.generation);
    if fresh {
        let image = egui::ColorImage::from_rgba_unmultiplied([picture.width, picture.height], &picture.rgba);
        match slot {
            Some((handle, generation)) => {
                handle.set(image, egui::TextureOptions::LINEAR);
                *generation = picture.generation;
            }
            None => *slot = Some((ctx.load_texture(name, image, egui::TextureOptions::LINEAR), picture.generation)),
        }
    }
    slot.as_ref().map(|(handle, _)| handle.clone())
}

/// `size` fitted inside `rect`, centred.
fn fit(rect: Rect, size: egui::Vec2) -> Rect {
    let scale = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0));
    Rect::from_center_size(rect.center(), size * scale)
}

/// A video call (Pego `v5/Call · Video call`, `m5/Call 3`): the caller's video
/// large, the time and that it is encrypted at the top, "You" in a corner, and
/// Mic / Camera / Screen / Background / End at the bottom. On the desktop it
/// takes the left of the window and leaves the chat on the right.
fn video_call(ctx: &Context, p: &Palette, call: &CallView, session: Option<&SessionView>, textures: &mut Textures, full: bool) -> Option<Ask> {
    let mut asked = None;
    let screen = ctx.content_rect();
    let area = if full {
        screen
    } else {
        Rect::from_min_max(pos2(screen.left() + 24.0, screen.top() + 70.0), pos2(screen.right() - 440.0, screen.bottom() - 24.0))
    };
    let remote = texture(ctx, &mut textures.remote, &call.remote, "call_remote");
    let local = texture(ctx, &mut textures.local, &call.local, "call_local");
    egui::Area::new(egui::Id::unique("call_video")).order(egui::Order::Foreground).fixed_pos(area.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(area.size(), Sense::click());
        let radius = if full { 0.0 } else { f32::from(theme::R_LG) };
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, radius, Color32::from_rgb(0x12, 0x14, 0x13));
        match &remote {
            Some(texture) => {
                let shown = fit(rect, texture.size_vec2());
                painter.image(texture.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                let name = caller(call, session);
                avatar_rings(ui, p, rect.center() - vec2(0.0, 30.0), 44.0, &crate::ui::screens::initials(&name), None);
                let wait = if call.phase == Phase::Active { tr("Waiting for their video…", "Menunggu video mereka…") } else { waiting(call) };
                widgets::paint_text(ui, Rect::from_center_size(rect.center() + vec2(0.0, 50.0), vec2(rect.width(), 20.0)), wait, fonts::ui(13.0, Weight::Regular), Color32::from_white_alpha(200), egui::Align::Center);
            }
        }
        // Top: who, how long, where; and the connection.
        let shade = Rect::from_min_size(rect.min, vec2(rect.width(), 64.0));
        painter.rect_filled(shade, radius, Color32::from_black_alpha(90));
        let name = caller(call, session);
        widgets::paint_text(ui, Rect::from_min_size(rect.min + vec2(18.0, 12.0), vec2(rect.width() - 200.0, 22.0)), &name, fonts::ui(16.0, Weight::SemiBold), Color32::WHITE, egui::Align::Min);
        let time = call.connected_at.map(|t| clock(t.elapsed().as_secs())).unwrap_or_default();
        let on = session.map(crate::ui::screens::devices::chip_label).unwrap_or_default();
        let line = crate::trf!("{time} · {on} · end-to-end encrypted", "{time} · {on} · terenkripsi end-to-end");
        widgets::paint_text(ui, Rect::from_min_size(rect.min + vec2(18.0, 36.0), vec2(rect.width() - 200.0, 16.0)), &line, fonts::ui(11.5, Weight::Regular), Color32::from_white_alpha(210), egui::Align::Min);
        let quality = if remote.is_some() { tr("Video connected", "Video tersambung") } else { tr("No video yet", "Belum ada video") };
        let pill = Rect::from_min_size(pos2(rect.right() - 170.0, rect.top() + 18.0), vec2(152.0, 26.0));
        painter.rect_filled(pill, 13.0, Color32::from_black_alpha(110));
        painter.circle_filled(pos2(pill.left() + 14.0, pill.center().y), 3.5, if remote.is_some() { p.green } else { p.amber });
        widgets::paint_text(ui, pill.shrink2(vec2(24.0, 0.0)).translate(vec2(4.0, 0.0)), quality, fonts::ui(11.5, Weight::Medium), Color32::WHITE, egui::Align::Min);
        // "You" in the corner: our camera, or that it is off.
        let you = Rect::from_min_size(pos2(rect.right() - 218.0, rect.top() + 58.0), vec2(200.0, 112.0));
        painter.rect_filled(you, 12.0, Color32::from_rgb(0x24, 0x27, 0x25));
        match &local {
            Some(texture) if call.camera_on => {
                egui::Image::from_texture(texture).corner_radius(12).fit_to_exact_size(you.size()).paint_at(ui, you);
            }
            _ => {
                icons::paint(ui, Rect::from_center_size(you.center() - vec2(0.0, 8.0), vec2(22.0, 22.0)), "video-off", 22.0, Color32::from_white_alpha(180));
                let why = call.camera_error.as_deref().unwrap_or(tr("Camera off", "Kamera mati"));
                widgets::paint_text(ui, Rect::from_center_size(you.center() + vec2(0.0, 22.0), vec2(you.width() - 12.0, 16.0)), why, fonts::ui(10.5, Weight::Regular), Color32::from_white_alpha(180), egui::Align::Center);
            }
        }
        painter.rect_stroke(you, 12.0, Stroke::new(1.0, Color32::from_white_alpha(60)), egui::StrokeKind::Inside);
        let tag = Rect::from_min_size(you.left_bottom() + vec2(8.0, -24.0), vec2(42.0, 18.0));
        painter.rect_filled(tag, 9.0, Color32::from_black_alpha(140));
        widgets::paint_text(ui, tag, tr("You", "Kamu"), fonts::ui(10.5, Weight::SemiBold), Color32::WHITE, egui::Align::Center);
        // The controls.
        let row = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 56.0), vec2(5.0 * 64.0 + 4.0 * 14.0, 80.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
        child.spacing_mut().item_spacing.x = 14.0;
        let dark = Color32::from_black_alpha(140);
        let (mic_icon, mic_fill, mic_glyph) = if call.muted { ("mic-off", Color32::WHITE, Color32::BLACK) } else { ("mic", dark, Color32::WHITE) };
        if control(&mut child, p, mic_icon, "Mic", mic_fill, mic_glyph, true, tr("Mute your microphone", "Bisukan mikrofon Anda")) {
            asked = Some(Ask::Mute);
        }
        let (cam_icon, cam_fill, cam_glyph) = if call.camera_on { ("video", dark, Color32::WHITE) } else { ("video-off", Color32::WHITE, Color32::BLACK) };
        if control(&mut child, p, cam_icon, tr("Camera", "Kamera"), cam_fill, cam_glyph, true, tr("Turn your camera off or on", "Matikan atau nyalakan kamera Anda")) {
            asked = Some(Ask::Camera);
        }
        control(&mut child, p, "monitor-up", tr("Screen", "Layar"), dark, Color32::WHITE, false, tr("Screen sharing comes in a later version", "Berbagi layar menyusul di versi berikutnya"));
        control(&mut child, p, "image", tr("Blur", "Latar blur"), dark, Color32::WHITE, false, tr("Background blur comes in a later version", "Latar blur menyusul di versi berikutnya"));
        if control(&mut child, p, "phone-off", tr("End", "Akhiri"), p.red, Color32::WHITE, true, tr("End the call", "Akhiri panggilan")) {
            asked = Some(Ask::End);
        }
    });
    asked
}

/// Who is calling: their name in this number's book, else what WhatsApp sent,
/// else the masked number.
fn caller(call: &CallView, session: Option<&SessionView>) -> String {
    if privacy::hide_names() {
        return tr("Hidden contact", "Kontak tersembunyi").to_owned();
    }
    session
        .and_then(|s| s.book.name_of(&call.peer).map(str::to_owned))
        .or_else(|| call.name.clone())
        .unwrap_or_else(|| crate::backend::store::short_jid(&call.peer))
}

fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The avatar in rings: they pulse slowly while ringing, and grow with the
/// caller's voice in the call (smoothed over 120 ms).
fn avatar_rings(ui: &Ui, p: &Palette, center: egui::Pos2, radius: f32, initials: &str, level: Option<f32>) {
    let t = ui.input(|i| i.time) as f32;
    let grow = match level {
        Some(level) => ui.ctx().animate_value_with_time(egui::Id::unique("call_voice_ring"), (level * 3.0).min(1.0), 0.12),
        None => 0.5 + 0.5 * (t * 2.2).sin(),
    };
    ui.painter().circle_filled(center, radius * (1.55 + 0.15 * grow), p.on_primary.gamma_multiply(0.06));
    ui.painter().circle_filled(center, radius * (1.25 + 0.12 * grow), p.on_primary.gamma_multiply(0.1));
    ui.painter().circle_filled(center, radius, p.accent);
    ui.painter().circle_stroke(center, radius + 3.0, Stroke::new(2.0, p.accent.gamma_multiply(0.7)));
    let galley = ui.painter().layout_no_wrap(initials.to_owned(), fonts::ui(radius * 0.62, Weight::SemiBold), p.accent_text);
    ui.painter().galley(center - galley.size() / 2.0, galley, p.accent_text);
}

/// A round control (`FM/Call/Control`): icon in a circle, label under it.
fn control(ui: &mut Ui, p: &Palette, icon: &str, label: &str, fill: Color32, glyph: Color32, enabled: bool, hint: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(64.0, 74.0), if enabled { Sense::click() } else { Sense::hover() });
    let center = pos2(rect.center().x, rect.top() + 26.0);
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered() && enabled, 0.12);
    let fill = if enabled { fill } else { fill.gamma_multiply(0.45) };
    ui.painter().circle_filled(center, 25.0 + 1.5 * hover, fill);
    icons::paint(ui, Rect::from_center_size(center, vec2(20.0, 20.0)), icon, 20.0, if enabled { glyph } else { glyph.gamma_multiply(0.5) });
    let text = Rect::from_min_max(pos2(rect.left() - 6.0, rect.top() + 56.0), pos2(rect.right() + 6.0, rect.bottom()));
    widgets::paint_text(ui, text, label, fonts::ui(11.5, Weight::Medium), p.on_primary, egui::Align::Center);
    let response = response.on_hover_text(hint);
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand).clicked()
    } else {
        false
    }
}

/// A pill button for the ringing card.
fn pill(ui: &mut Ui, icon: &str, label: &str, fill: Color32, fg: Color32, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
    ui.painter().rect_filled(rect, 19.0, fill.gamma_multiply(1.0 - 0.12 * hover));
    let galley = ui.painter().layout_no_wrap(label.to_owned(), fonts::ui(13.0, Weight::SemiBold), fg);
    let total = galley.size().x + 22.0;
    let left = rect.center().x - total / 2.0;
    icons::paint(ui, Rect::from_center_size(pos2(left + 8.0, rect.center().y), vec2(15.0, 15.0)), icon, 15.0, fg);
    ui.painter().galley(pos2(left + 22.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// The contact's last message, for the ringing card.
fn last_message(call: &CallView, session: Option<&SessionView>) -> Option<(String, i64)> {
    let chat = session?.book.chats.iter().find(|c| c.jid == call.peer)?;
    let m = chat.messages.iter().rev().find(|m| !m.from_me && m.media.as_ref().is_none_or(|x| x.kind != "call"))?;
    Some((m.text.clone(), m.ts))
}

/// Desktop: the forest card at the top right while it rings.
fn ringing_card(ctx: &Context, p: &Palette, call: &CallView, session: Option<&SessionView>) -> Option<Ask> {
    let mut asked = None;
    egui::Area::new(egui::Id::unique("call_ringing"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_TOP, vec2(-24.0, 70.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(p.primary)
                .corner_radius(theme::R_LG)
                .shadow(egui::epaint::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(60) })
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.horizontal(|ui| {
                        icons::show(ui, "phone-incoming", 14.0, p.on_primary);
                        let title = if call.video { tr("Incoming video call", "Panggilan video masuk") } else { tr("Incoming voice call", "Panggilan suara masuk") };
                        ui.label(widgets::rich(title, 12.5, Weight::SemiBold, p.on_primary));
                        if let Some(s) = session {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                egui::Frame::NONE.fill(p.accent).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                                    ui.label(widgets::rich(&crate::trf!("to {}", "ke {}", crate::ui::screens::devices::chip_label(s)), 11.0, Weight::Medium, p.accent_text));
                                });
                            });
                        }
                    });
                    let name = caller(call, session);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                        avatar_rings(ui, p, rect.center(), 24.0, &crate::ui::screens::initials(&name), None);
                        ui.vertical(|ui| {
                            ui.add_space(8.0);
                            ui.label(widgets::rich(&name, 18.0, Weight::SemiBold, p.on_primary));
                            let rang = clock(call.since.elapsed().as_secs());
                            ui.label(widgets::rich_mono(&crate::trf!("Ringing · {rang}", "Berdering · {rang}"), 12.0, p.on_primary_muted));
                        });
                    });
                    if let Some((text, ts)) = last_message(call, session) {
                        egui::Frame::NONE.fill(p.on_primary.gamma_multiply(0.1)).corner_radius(theme::R_MD).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let when = crate::ui::time::clock(ts);
                            ui.label(widgets::rich(&crate::trf!("LAST MESSAGE · {when}", "PESAN TERAKHIR · {when}"), 10.5, Weight::SemiBold, p.accent));
                            if privacy::blur_previews() {
                                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
                                privacy::frost_line(ui.painter(), rect, 0.8, p.on_primary.gamma_multiply(0.35));
                            } else {
                                let short: String = text.chars().take(140).collect();
                                ui.add(egui::Label::new(widgets::rich(&short, 13.0, Weight::Regular, p.on_primary)).wrap());
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let width = (ui.available_width() - 16.0) / 3.0;
                        if pill(ui, "phone-off", tr("Decline", "Tolak"), p.red, Color32::WHITE, width) {
                            asked = Some(Ask::Decline);
                        }
                        if pill(ui, "message-circle", tr("Reply", "Balas pesan"), p.on_primary.gamma_multiply(0.14), p.on_primary, width) {
                            asked = Some(Ask::Reply);
                        }
                        if pill(ui, "phone", tr("Answer", "Terima"), p.accent, p.accent_text, width) {
                            asked = Some(Ask::Answer);
                        }
                    });
                    ui.vertical_centered(|ui| {
                        ui.label(widgets::rich(
                            tr("Ctrl+Shift+A to answer · Ctrl+Shift+D to decline", "Ctrl+Shift+A untuk menerima · Ctrl+Shift+D untuk menolak"),
                            11.0,
                            Weight::Regular,
                            p.on_primary_muted,
                        ));
                    });
                });
        });
    asked
}

/// Phone layout: the whole window while it rings (`m5/Call 1`).
fn ringing_full(ctx: &Context, p: &Palette, call: &CallView, session: Option<&SessionView>) -> Option<Ask> {
    let mut asked = None;
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("call_ringing_full")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(rect, 0.0, p.primary);
        let name = caller(call, session);
        let center = pos2(rect.center().x, rect.top() + rect.height() * 0.32);
        let title = if call.video { tr("Incoming video call", "Panggilan video masuk") } else { tr("Incoming voice call", "Panggilan suara masuk") };
        widgets::paint_text(ui, Rect::from_center_size(pos2(center.x, rect.top() + 60.0), vec2(rect.width(), 20.0)), title, fonts::ui(13.0, Weight::SemiBold), p.on_primary, egui::Align::Center);
        if let Some(s) = session {
            let to = crate::trf!("to {}", "ke {}", crate::ui::screens::devices::chip_label(s));
            widgets::paint_text(ui, Rect::from_center_size(pos2(center.x, rect.top() + 84.0), vec2(rect.width(), 18.0)), &to, fonts::ui(12.0, Weight::Regular), p.on_primary_muted, egui::Align::Center);
        }
        avatar_rings(ui, p, center, 44.0, &crate::ui::screens::initials(&name), None);
        widgets::paint_text(ui, Rect::from_center_size(center + vec2(0.0, 96.0), vec2(rect.width() - 32.0, 26.0)), &name, fonts::ui(20.0, Weight::SemiBold), p.on_primary, egui::Align::Center);
        let rang = clock(call.since.elapsed().as_secs());
        widgets::paint_text(ui, Rect::from_center_size(center + vec2(0.0, 124.0), vec2(rect.width(), 18.0)), &crate::trf!("Ringing… {rang}", "Berdering… {rang}"), fonts::ui(13.0, Weight::Regular), p.on_primary_muted, egui::Align::Center);
        let row = Rect::from_min_max(pos2(rect.left() + 24.0, rect.bottom() - 120.0), pos2(rect.right() - 24.0, rect.bottom() - 30.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let gap = ((row.width() - 3.0 * 64.0) / 2.0).max(8.0);
        child.spacing_mut().item_spacing.x = gap;
        if control(&mut child, p, "phone-off", tr("Decline", "Tolak"), p.red, Color32::WHITE, true, tr("Decline (Ctrl+Shift+D)", "Tolak (Ctrl+Shift+D)")) {
            asked = Some(Ask::Decline);
        }
        if control(&mut child, p, "message-circle", tr("Reply", "Balas pesan"), p.on_primary.gamma_multiply(0.14), p.on_primary, true, tr("Decline and reply with a message", "Tolak lalu balas dengan pesan")) {
            asked = Some(Ask::Reply);
        }
        if control(&mut child, p, "phone", tr("Answer", "Terima"), p.accent, p.accent_text, true, tr("Answer (Ctrl+Shift+A)", "Terima (Ctrl+Shift+A)")) {
            asked = Some(Ask::Answer);
        }
    });
    asked
}

/// In the call: the forest panel at the right (desktop), or the whole window
/// with a minimise button (phone layout).
fn in_call(ctx: &Context, p: &Palette, call: &CallView, session: Option<&SessionView>, full: bool) -> Option<Ask> {
    let mut asked = None;
    let screen = ctx.content_rect();
    let area = if full {
        screen
    } else {
        Rect::from_min_max(pos2(screen.right() - 344.0, screen.top() + 70.0), pos2(screen.right() - 24.0, screen.bottom() - 24.0))
    };
    egui::Area::new(egui::Id::unique("call_panel")).order(egui::Order::Foreground).fixed_pos(area.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(area.size(), Sense::click());
        ui.painter().rect_filled(rect, if full { 0.0 } else { f32::from(theme::R_LG) }, p.primary);
        let name = caller(call, session);
        // Header: what it is, and that it is encrypted.
        let title = if call.video { tr("Video call", "Panggilan video") } else { tr("Voice call", "Panggilan suara") };
        widgets::paint_text(ui, Rect::from_min_size(rect.min + vec2(18.0, 16.0), vec2(220.0, 18.0)), title, fonts::ui(14.0, Weight::SemiBold), p.on_primary, egui::Align::Min);
        icons::paint(ui, Rect::from_min_size(rect.min + vec2(18.0, 38.0), vec2(12.0, 12.0)), "lock", 12.0, p.on_primary_muted);
        widgets::paint_text(ui, Rect::from_min_size(rect.min + vec2(34.0, 36.0), vec2(220.0, 16.0)), tr("End-to-end encrypted", "Terenkripsi end-to-end"), fonts::ui(11.5, Weight::Regular), p.on_primary_muted, egui::Align::Min);
        if full {
            let min = Rect::from_center_size(pos2(rect.right() - 28.0, rect.top() + 30.0), vec2(32.0, 32.0));
            let r = ui.interact(min, egui::Id::unique("call_minimize"), Sense::click());
            ui.painter().circle_filled(min.center(), 16.0, p.on_primary.gamma_multiply(if r.hovered() { 0.2 } else { 0.1 }));
            icons::paint(ui, min.shrink(8.0), "minimize-2", 16.0, p.on_primary);
            if r.on_hover_text(tr("Minimise", "Kecilkan")).clicked() {
                asked = Some(Ask::Minimize);
            }
        }
        // The caller in rings that follow their voice.
        let center = pos2(rect.center().x, rect.top() + rect.height() * 0.34);
        let connected = call.phase == Phase::Active;
        let remote = call.levels.remote();
        avatar_rings(ui, p, center, 46.0, &crate::ui::screens::initials(&name), connected.then_some(remote));
        widgets::paint_text(ui, Rect::from_center_size(center + vec2(0.0, 96.0), vec2(rect.width() - 24.0, 26.0)), &name, fonts::ui(19.0, Weight::SemiBold), p.on_primary, egui::Align::Center);
        let status = match call.connected_at {
            Some(t) if connected => clock(t.elapsed().as_secs()),
            _ => waiting(call).to_owned(),
        };
        widgets::paint_text(ui, Rect::from_center_size(center + vec2(0.0, 122.0), vec2(rect.width(), 18.0)), &status, fonts::mono(13.0), p.on_primary_muted, egui::Align::Center);
        // "Speaking", with a small wave, while the caller's voice is heard.
        let speaking = ui.ctx().animate_bool_with_time(egui::Id::unique("call_speaking"), connected && remote > 0.04, 0.25);
        if speaking > 0.01 {
            let pill = Rect::from_center_size(center + vec2(0.0, 152.0), vec2(180.0, 26.0));
            ui.painter().rect_filled(pill, 13.0, p.on_primary.gamma_multiply(0.12 * speaking));
            let t = ui.input(|i| i.time) as f32;
            for i in 0..4 {
                let h = 4.0 + 8.0 * (0.5 + 0.5 * (t * 9.0 + i as f32 * 1.3).sin()) * (remote * 4.0).min(1.0);
                let x = pill.left() + 16.0 + i as f32 * 5.0;
                ui.painter().rect_filled(Rect::from_center_size(pos2(x, pill.center().y), vec2(3.0, h)), 1.5, p.accent.gamma_multiply(speaking));
            }
            let label = Rect::from_min_max(pill.min + vec2(40.0, 0.0), pill.max);
            widgets::paint_text(ui, label, tr("Caller is speaking", "Pelanggan sedang bicara"), fonts::ui(11.5, Weight::Medium), p.accent.gamma_multiply(speaking), egui::Align::Min);
        }
        // The caller asks to turn on video.
        if call.video_request {
            let ask = Rect::from_min_size(pos2(rect.left() + 14.0, rect.bottom() - 206.0), vec2(rect.width() - 28.0, 48.0));
            ui.painter().rect_filled(ask, theme::R_MD, p.accent);
            widgets::paint_text(ui, Rect::from_min_size(ask.min + vec2(12.0, 0.0), vec2(ask.width() - 150.0, ask.height())), tr("Wants to turn on video", "Ingin menyalakan video"), fonts::ui(12.5, Weight::SemiBold), p.accent_text, egui::Align::Min);
            let yes = Rect::from_min_size(pos2(ask.right() - 136.0, ask.top() + 9.0), vec2(64.0, 30.0));
            let no = Rect::from_min_size(pos2(ask.right() - 68.0, ask.top() + 9.0), vec2(58.0, 30.0));
            let yes_r = ui.interact(yes, egui::Id::unique("call_video_yes"), Sense::click());
            let no_r = ui.interact(no, egui::Id::unique("call_video_no"), Sense::click());
            ui.painter().rect_filled(yes, 15.0, p.primary);
            widgets::paint_text(ui, yes, tr("Turn on", "Nyalakan"), fonts::ui(11.5, Weight::SemiBold), p.on_primary, egui::Align::Center);
            ui.painter().rect_stroke(no, 15.0, Stroke::new(1.0, p.accent_text), egui::StrokeKind::Inside);
            widgets::paint_text(ui, no, tr("Not now", "Nanti"), fonts::ui(11.5, Weight::Medium), p.accent_text, egui::Align::Center);
            if yes_r.clicked() {
                asked = Some(Ask::AcceptVideo);
            } else if no_r.clicked() {
                asked = Some(Ask::DeclineVideo);
            }
        }
        if let Some(error) = &call.camera_error {
            let line = Rect::from_min_size(pos2(rect.left() + 14.0, rect.bottom() - 176.0), vec2(rect.width() - 28.0, 18.0));
            widgets::paint_text(ui, line, error, fonts::ui(11.0, Weight::Regular), p.accent, egui::Align::Center);
        }
        // Which number it is on, then the controls.
        let bottom = Rect::from_min_max(pos2(rect.left() + 14.0, rect.bottom() - 150.0), pos2(rect.right() - 14.0, rect.bottom() - 14.0));
        ui.painter().rect_filled(bottom, theme::R_LG, p.on_primary.gamma_multiply(0.08));
        if let Some(s) = session {
            let line = Rect::from_min_size(bottom.min + vec2(12.0, 10.0), vec2(bottom.width() - 24.0, 26.0));
            ui.painter().rect_filled(line, 13.0, p.on_primary.gamma_multiply(0.08));
            let on = crate::trf!("On {}", "Lewat {}", crate::ui::screens::devices::chip_label(s));
            widgets::paint_text(ui, line.shrink2(vec2(12.0, 0.0)), &on, fonts::ui(11.5, Weight::Medium), p.on_primary_muted, egui::Align::Min);
        }
        let row = Rect::from_min_max(pos2(bottom.left() + 8.0, bottom.bottom() - 84.0), pos2(bottom.right() - 8.0, bottom.bottom() - 6.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
        child.spacing_mut().item_spacing.x = ((row.width() - 4.0 * 64.0) / 3.0).max(2.0);
        let soft = p.on_primary.gamma_multiply(0.14);
        let (mic_icon, mic_fill, mic_glyph) = if call.muted { ("mic-off", p.on_primary, p.primary) } else { ("mic", soft, p.on_primary) };
        if control(&mut child, p, mic_icon, if call.muted { tr("Unmute", "Suarakan") } else { tr("Mute", "Bisukan") }, mic_fill, mic_glyph, true, tr("Mute your microphone", "Bisukan mikrofon Anda")) {
            asked = Some(Ask::Mute);
        }
        let (spk_icon, spk_fill, spk_glyph) = if call.speaker_off { ("volume-x", p.on_primary, p.primary) } else { ("volume-2", soft, p.on_primary) };
        if control(&mut child, p, spk_icon, "Speaker", spk_fill, spk_glyph, true, tr("Silence the caller on this computer", "Senyapkan suara penelepon di komputer ini")) {
            asked = Some(Ask::Speaker);
        }
        let can_video = connected && crate::backend::video::ffmpeg().is_some();
        let hint = if crate::backend::video::ffmpeg().is_none() {
            tr("Video calls need ffmpeg installed", "Video call butuh ffmpeg terpasang")
        } else {
            tr("Turn this into a video call", "Jadikan video call")
        };
        if control(&mut child, p, "video", "Video", soft, p.on_primary, can_video, hint) {
            asked = Some(Ask::StartVideo);
        }
        let (end, end_hint) = if call.phase == Phase::Calling { (tr("Cancel", "Batalkan"), tr("Cancel the call", "Batalkan panggilan")) } else { (tr("End", "Akhiri"), tr("End the call", "Akhiri panggilan")) };
        if control(&mut child, p, "phone-off", end, p.red, Color32::WHITE, true, end_hint) {
            asked = Some(Ask::End);
        }
    });
    asked
}

/// What a call shows until it connects: ours "Calling…", then "Ringing…" once
/// their phone rings; an answered one "Connecting…".
fn waiting(call: &CallView) -> &'static str {
    match (call.phase, call.ringing) {
        (Phase::Calling, true) => tr("Ringing…", "Berdering…"),
        (Phase::Calling, false) => tr("Calling…", "Memanggil…"),
        _ => tr("Connecting…", "Menyambungkan…"),
    }
}

/// Phone layout, minimised: a forest strip at the top (`m5/Call 4`); a tap
/// opens the call again.
fn banner(ctx: &Context, p: &Palette, call: &CallView) -> Option<Ask> {
    let mut asked = None;
    let screen = ctx.content_rect();
    let rect = Rect::from_min_size(pos2(screen.left(), screen.top() + 40.0), vec2(screen.width(), 50.0));
    egui::Area::new(egui::Id::unique("call_banner")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        let (rect, response) = ui.allocate_exact_size(rect.size(), Sense::click());
        ui.painter().rect_filled(rect, 0.0, p.primary);
        let time = call.connected_at.map(|t| clock(t.elapsed().as_secs())).unwrap_or_default();
        let title = crate::trf!("Voice call · {time}", "Panggilan suara · {time}");
        widgets::paint_text(ui, Rect::from_min_size(rect.min + vec2(16.0, 7.0), vec2(rect.width() - 120.0, 18.0)), &title, fonts::ui(13.5, Weight::SemiBold), p.on_primary, egui::Align::Min);
        widgets::paint_text(ui, Rect::from_min_size(rect.min + vec2(16.0, 26.0), vec2(rect.width() - 120.0, 16.0)), tr("Tap to return to the call", "Ketuk untuk kembali ke panggilan"), fonts::ui(11.0, Weight::Regular), p.on_primary_muted, egui::Align::Min);
        let end = Rect::from_center_size(pos2(rect.right() - 30.0, rect.center().y), vec2(34.0, 34.0));
        let mute = Rect::from_center_size(pos2(rect.right() - 74.0, rect.center().y), vec2(34.0, 34.0));
        let end_r = ui.interact(end, egui::Id::unique("call_banner_end"), Sense::click());
        let mute_r = ui.interact(mute, egui::Id::unique("call_banner_mute"), Sense::click());
        ui.painter().circle_filled(end.center(), 17.0, p.red);
        icons::paint(ui, end.shrink(9.0), "phone-off", 16.0, Color32::WHITE);
        ui.painter().circle_filled(mute.center(), 17.0, p.on_primary.gamma_multiply(0.16));
        icons::paint(ui, mute.shrink(9.0), if call.muted { "mic-off" } else { "mic" }, 16.0, p.on_primary);
        if end_r.clicked() {
            asked = Some(Ask::End);
        } else if mute_r.clicked() {
            asked = Some(Ask::Mute);
        } else if response.clicked() {
            asked = Some(Ask::Restore);
        }
    });
    asked
}
