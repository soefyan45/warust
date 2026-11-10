//! macOS notifications through UserNotifications, so a click opens the chat.
//! They need an app bundle (warust.app); a bare binary (`cargo run`) falls
//! back to `osascript`, which cannot report the click.

use std::sync::OnceLock;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use super::{Updater, parse_key};

/// Where a click sends the chat to open.
static STATE: OnceLock<Updater> = OnceLock::new();

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Delegate does not
    // implement Drop.
    #[unsafe(super = NSObject)]
    #[name = "WarustNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Shown even while warust is in front: `incoming` already skips the
        /// chat on screen.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
        }

        /// The click: the request id is the chat key.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, completion: &DynBlock<dyn Fn()>) {
            let key = response.notification().request().identifier().to_string();
            if let (Some(state), Some((session, jid))) = (STATE.get(), parse_key(&key)) {
                state.request_open(session, jid);
            }
            completion.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// Running from warust.app. Outside a bundle the notification center throws.
fn bundled() -> bool {
    static BUNDLED: OnceLock<bool> = OnceLock::new();
    *BUNDLED.get_or_init(|| NSBundle::mainBundle().bundleIdentifier().is_some())
}

pub(super) fn init(state: &Updater) {
    if !bundled() || STATE.set(state.clone()).is_err() {
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = Delegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The center holds its delegate weakly; it lives as long as the app.
    std::mem::forget(delegate);
    // Asks once; macOS remembers the answer (System Settings › Notifications).
    let answered = RcBlock::new(|granted: Bool, error: *mut NSError| {
        if !granted.as_bool() {
            // SAFETY: the error, when there is one, is valid for this call.
            let reason = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            log::info!("notifications not allowed: {}", reason.unwrap_or_else(|| "ditolak".to_owned()));
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &answered);
}

/// Shows or replaces the chat's notification. False outside warust.app, so the
/// caller can fall back to `osascript`.
pub(super) fn show(key: &str, title: &str, body: &str) -> bool {
    if !bundled() || STATE.get().is_none() {
        return false;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setThreadIdentifier(&NSString::from_str(key));
    // The same id replaces the chat's earlier notification.
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(key), &content, None);
    let added = RcBlock::new(|error: *mut NSError| {
        // SAFETY: the error, when there is one, is valid for this call.
        if let Some(error) = unsafe { error.as_ref() } {
            log::warn!("notification not shown: {}", error.localizedDescription());
        }
    });
    UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&added));
    true
}
