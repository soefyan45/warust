//! warust desktop app: one process runs the UI, the WhatsApp sessions, the
//! REST API and the webhooks. The early test tool lives in src/bin/spike.rs.

// A GUI app on Windows: no console window behind it in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod i18n;
mod platform;
mod tray;
mod ui;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
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
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "warust",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    );
    // An update installed while running starts warust again (#93).
    backend::update::after_exit();
    result
}

/// The app icon from Pego "Logo · Warust" (App icon 128), drawn at 256 px.
fn icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/app/warust-256.png"))
        .expect("the bundled icon is a valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData { rgba: image.into_raw(), width, height }
}
