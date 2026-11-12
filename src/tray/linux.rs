//! The tray icon on Linux (#134): a StatusNotifierItem over D-Bus (`ksni`), so
//! there is no GTK and no libappindicator. KDE, XFCE and GNOME with the
//! AppIndicator extension show it; stock GNOME has no host, so `Tray::new`
//! returns `None` there and closing the window keeps quitting the app.

use std::sync::mpsc;
use std::time::Duration;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Category, Icon as Pixmap, MenuItem, ToolTip};

use super::{Event, Handler, Icon, Labels};

/// Longest `Tray::new` waits for the session bus and the watcher. A missing
/// bus or watcher fails at once; this only covers a bus that stalls.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(3);

/// What ksni asks for: the picture, the texts, and where clicks go.
struct Item {
    pixmap: Pixmap,
    labels: Labels,
    handler: Handler,
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "warust".into()
    }

    fn title(&self) -> String {
        self.labels.tooltip.clone()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<Pixmap> {
        vec![self.pixmap.clone()]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.labels.tooltip.clone(),
            ..Default::default()
        }
    }

    // Left click.
    fn activate(&mut self, _x: i32, _y: i32) {
        (self.handler)(Event::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: self.labels.open.clone(),
                activate: Box::new(|this: &mut Self| (this.handler)(Event::Open)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.labels.quit.clone(),
                activate: Box::new(|this: &mut Self| (this.handler)(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// RGBA (not premultiplied) to the ARGB32, network byte order, that
/// StatusNotifierItem wants: the bytes A, R, G, B.
fn to_pixmap(icon: &Icon) -> Pixmap {
    let mut data = icon.rgba.clone();
    data.chunks_exact_mut(4).for_each(|px| px.rotate_right(1));
    Pixmap {
        width: icon.width as i32,
        height: icon.height as i32,
        data,
    }
}

/// The icon while it is shown. ksni runs the D-Bus service on its own threads.
pub struct Tray {
    handle: Handle<Item>,
}

impl Tray {
    /// `None` when this desktop has no tray: no session bus, no
    /// StatusNotifierWatcher, or no host that shows icons. The reason is
    /// logged once.
    pub fn new(icon: &Icon, labels: &Labels, handler: Handler) -> Option<Self> {
        let item = Item {
            pixmap: to_pixmap(icon),
            labels: Labels {
                tooltip: labels.tooltip.clone(),
                open: labels.open.clone(),
                quit: labels.quit.clone(),
            },
            handler,
        };
        // `spawn` blocks on D-Bus calls that have no timeout of their own, so it
        // runs on a helper thread and the UI thread only waits a bounded time.
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("tray-register".into())
            .spawn(move || {
                let result = item.spawn();
                if let Err(mpsc::SendError(Ok(late))) = tx.send(result) {
                    // Nobody is waiting any more: take the icon back down.
                    let _ = late.shutdown();
                }
            });
        if let Err(err) = spawned {
            log::info!("tray: cannot start the registration thread: {err}");
            return None;
        }
        match rx.recv_timeout(REGISTER_TIMEOUT) {
            Ok(Ok(handle)) => Some(Self { handle }),
            Ok(Err(err)) => {
                log::info!("tray: not available on this desktop: {err}");
                None
            }
            Err(_) => {
                log::info!("tray: no answer from the session bus in {REGISTER_TIMEOUT:?}");
                // An answer that came in just now would otherwise leave an
                // icon up that the app does not know about.
                if let Ok(Ok(late)) = rx.try_recv() {
                    let _ = late.shutdown();
                }
                None
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // Only queues the request; the service thread closes the connection,
        // which also makes the bus drop the item. Not waited for, so the UI
        // thread never blocks.
        let _ = self.handle.shutdown();
    }
}
