//! warust desktop app: one process runs the UI, the WhatsApp sessions, the
//! REST API and the webhooks. The early test tool lives in src/bin/spike.rs.

// A GUI app on Windows: no console window behind it in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod i18n;
mod platform;
mod tray;
mod ui;
mod walink;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // Started for a "Chat on WhatsApp" link (#193): the warust already
    // running takes it, else this one opens it once its window is up.
    if let Some((arg, link)) = std::env::args().skip(1).find_map(|a| walink::parse(&a).map(|l| (a, l))) {
        if walink::hand_over(&arg) {
            return Ok(());
        }
        walink::push(link);
    }
    // Dev aid: WARUST_SIZE=390x844 opens the window at that size.
    let size = std::env::var("WARUST_SIZE")
        .ok()
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?])))
        .unwrap_or([1440.0, 900.0]);
    let viewport = egui::ViewportBuilder::default()
        .with_title("warust")
        // X11 shows this icon; Wayland takes it from warust.desktop by app id
        // (scripts/install-desktop.sh installs that).
        .with_app_id("warust")
        .with_icon(icon())
        .with_inner_size(size)
        // Narrower than 1100 px it switches to the phone layout.
        .with_min_inner_size([360.0, 560.0]);
    let viewport = if cfg!(target_os = "macos") {
        // The system's window, its title bar see-through under the app's
        // topbar; ui::window moves the window buttons into that bar.
        viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
    } else {
        // No OS title bar: the app's topbar carries the window controls.
        viewport.with_decorations(false)
    };
    // Changed below on Linux only.
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // Wayland cannot hide a window and bring it back, so closing to the tray
    // would quit there: with close-to-tray on, run through XWayland (#134).
    #[cfg(target_os = "linux")]
    if xwayland_for_tray() {
        log::info!("wayland: running through XWayland so the window can close to the tray");
        options.event_loop_builder = Some(Box::new(|builder| {
            use egui_winit::winit::platform::x11::EventLoopBuilderExtX11 as _;
            builder.with_x11();
        }));
    }
    let result = eframe::run_native(
        "warust",
        options,
        Box::new(|cc| {
            // Links from later starts wake the app, also from the tray.
            let ctx = cc.egui_ctx.clone();
            walink::listen(move || ctx.request_repaint());
            Ok(Box::new(ui::App::new(cc)))
        }),
    );
    // An update installed while running starts warust again (#93).
    backend::update::after_exit();
    result
}

/// A Wayland session with an X server for X apps (XWayland), and close to
/// tray on in the saved settings (its default when not saved yet).
/// `WARUST_WAYLAND=native` keeps native Wayland anyway.
#[cfg(target_os = "linux")]
fn xwayland_for_tray() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");
    if !wayland || std::env::var_os("DISPLAY").is_none() || std::env::var("WARUST_WAYLAND").is_ok_and(|v| v == "native") {
        return false;
    }
    let saved = std::fs::read(backend::Settings::path(platform::data_dir())).ok();
    let close_to_tray = saved
        .and_then(|bytes| whatsapp_rust::serde_json::from_slice::<whatsapp_rust::serde_json::Value>(&bytes).ok())
        .and_then(|settings| settings.get("close_to_tray").and_then(|v| v.as_bool()));
    close_to_tray.unwrap_or(true)
}

/// The app icon from Pego "Logo · Warust" (App icon 128), drawn at 256 px.
fn icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/app/warust-256.png"))
        .expect("the bundled icon is a valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData { rgba: image.into_raw(), width, height }
}
