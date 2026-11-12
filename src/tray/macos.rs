//! The macOS menu bar icon (#134): an `NSStatusItem` with a two-entry menu.
//! A click on the icon opens the menu, as on every other menu bar app.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBitmapImageRep, NSCalibratedRGBColorSpace, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSSize, NSString, ns_string};

use super::{Event, Handler, Icon, Labels};

/// The icon's size in points. The picture has more pixels, so it stays sharp on retina.
const POINTS: f64 = 18.0;

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Target does not
    // implement Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "WarustTrayTarget"]
    #[ivars = Handler]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(open:))]
        fn open(&self, _sender: Option<&AnyObject>) {
            (self.ivars())(Event::Open);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            (self.ivars())(Event::Quit);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, handler: Handler) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(handler);
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// The menu bar icon while it is shown. Main thread only.
pub struct Tray {
    item: Retained<NSStatusItem>,
    // Menu item targets are weak, so the tray owns this.
    _target: Retained<Target>,
}

impl Tray {
    /// `None` when off the main thread or the picture is not `width * height` RGBA pixels.
    pub fn new(icon: &Icon, labels: &Labels, handler: Handler) -> Option<Self> {
        // AppKit only from the main thread, where the UI runs.
        let mtm = MainThreadMarker::new()?;
        let image = image(icon)?;
        let target = Target::new(mtm, handler);

        let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
        if let Some(button) = item.button(mtm) {
            button.setImage(Some(&image));
            button.setToolTip(Some(&NSString::from_str(&labels.tooltip)));
        }

        let menu = NSMenu::new(mtm);
        menu.addItem(&entry(mtm, &labels.open, sel!(open:), &target));
        menu.addItem(&entry(mtm, &labels.quit, sel!(quit:), &target));
        item.setMenu(Some(&menu));

        Some(Self { item, _target: target })
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}

/// A menu entry that calls `action` on `target`, without a shortcut.
fn entry(mtm: MainThreadMarker, title: &str, action: objc2::runtime::Sel, target: &Target) -> Retained<NSMenuItem> {
    // SAFETY: `action` is a selector `Target` implements, and the title and key are valid strings.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            ns_string!(""),
        )
    };
    // SAFETY: the tray keeps `target` alive for as long as the menu exists.
    unsafe { item.setTarget(Some(target)) };
    item
}

/// The picture as a full-colour `NSImage`, drawn `POINTS` wide whatever its pixel size.
fn image(icon: &Icon) -> Option<Retained<NSImage>> {
    let (width, height) = (isize::try_from(icon.width).ok()?, isize::try_from(icon.height).ok()?);
    if icon.rgba.len() != icon.width as usize * icon.height as usize * 4 {
        return None;
    }
    // SAFETY: null planes make the rep allocate its own buffer of `width * 4`
    // bytes per row, which is the layout asked for here.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            width * 4,
            32,
        )
    }?;
    let data = rep.bitmapData();
    if data.is_null() {
        return None;
    }
    // SAFETY: the rep's buffer holds exactly `rgba.len()` bytes (checked above)
    // and nothing else reads or writes it yet.
    let pixels = unsafe { std::slice::from_raw_parts_mut(data, icon.rgba.len()) };
    // A rep made this way takes its colours already multiplied by alpha; the
    // icon's are not, and would leave a bright fringe on its soft edges.
    for (to, from) in pixels.chunks_exact_mut(4).zip(icon.rgba.chunks_exact(4)) {
        let alpha = u16::from(from[3]);
        for channel in 0..3 {
            to[channel] = ((u16::from(from[channel]) * alpha + 127) / 255) as u8;
        }
        to[3] = from[3];
    }
    let size = NSSize::new(POINTS, POINTS);
    rep.setSize(size);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    image.addRepresentation(&rep);
    // Full colour: a template image would be drawn as a plain silhouette.
    image.setTemplate(false);
    Some(image)
}
