//! The tray icon (#134): the notification area on Windows and Linux, the menu
//! bar on macOS. While it is there, closing the main window only hides it;
//! the icon brings the window back or quits the app.
//!
//! Each OS has its own `Tray` behind the same three items: `Tray::new`, the
//! [`Event`]s it reports to the [`Handler`], and `Drop`, which takes the icon
//! away again.

use std::sync::Arc;

/// What the user picked on the tray icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Show the main window: a click on the icon, or "Open warust".
    Open,
    /// "Quit": end the app.
    Quit,
}

/// Called for each [`Event`], on whichever thread the OS reports it.
pub type Handler = Arc<dyn Fn(Event) + Send + Sync>;

/// The icon's picture: `width * height` pixels, four bytes each (R, G, B, A),
/// not premultiplied.
pub struct Icon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The icon's texts, in the app's language.
pub struct Labels {
    /// Shown when the pointer rests on the icon.
    pub tooltip: String,
    /// Menu item that shows the main window.
    pub open: String,
    /// Menu item that ends the app.
    pub quit: String,
}

#[cfg(target_os = "macos")]
#[path = "tray/macos.rs"]
mod imp;
#[cfg(windows)]
#[path = "tray/windows.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "tray/linux.rs"]
mod imp;

/// The icon while it is shown. `Tray::new(&Icon, &Labels, Handler)` returns
/// `None` when this desktop has no tray to put it in; call it on the main
/// thread. Dropping it removes the icon.
pub use imp::Tray;
