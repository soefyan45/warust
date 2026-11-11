//! Marks the chat on screen read as messages arrive in it. Opening a chat
//! marks it read where it is opened; this covers what comes in afterwards,
//! which would otherwise keep its badge (and grey ticks at the sender) until
//! the chat is clicked again.

use std::collections::HashMap;

/// The newest incoming message seen in each chat while it was on screen.
#[derive(Default)]
pub struct Watch {
    newest: HashMap<(String, String), Option<String>>,
}

impl Watch {
    /// Whether to mark `viewing` read now. `viewing` is the chat (session,
    /// JID) in front of the user, `newest` the id of its newest incoming
    /// message and `unread` its count. True once per message that came in
    /// since the chat was last looked at, also when that was while the window
    /// was in the background. A chat seen for the first time is left to
    /// whatever opened it, and "mark unread" stays, as no message came in.
    pub fn check(&mut self, viewing: Option<&(String, String)>, newest: Option<&str>, unread: u32) -> bool {
        let Some(viewing) = viewing else { return false };
        let newest = newest.map(str::to_owned);
        match self.newest.get_mut(viewing) {
            Some(seen) if *seen == newest => false,
            Some(seen) => {
                *seen = newest;
                unread > 0
            }
            None => {
                self.newest.insert(viewing.clone(), newest);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(jid: &str) -> (String, String) {
        ("s1".to_owned(), jid.to_owned())
    }

    #[test]
    fn a_message_in_the_open_chat_marks_it_read_once() {
        let (mut watch, a) = (Watch::default(), chat("a"));
        // Opened with unread messages: the click marks it read, not the watch.
        assert!(!watch.check(Some(&a), Some("m1"), 2));
        assert!(!watch.check(Some(&a), Some("m1"), 0));
        assert!(watch.check(Some(&a), Some("m2"), 1), "a new message while open");
        assert!(!watch.check(Some(&a), Some("m2"), 1), "asked once, not every frame");
        assert!(!watch.check(Some(&a), Some("m2"), 0));
    }

    #[test]
    fn what_came_in_out_of_sight_is_read_on_return() {
        let (mut watch, a, b) = (Watch::default(), chat("a"), chat("b"));
        assert!(!watch.check(Some(&a), Some("m1"), 0));
        // The window in the background, then another chat in front.
        assert!(!watch.check(None, None, 0));
        assert!(!watch.check(Some(&b), None, 0));
        assert!(watch.check(Some(&a), Some("m2"), 1));
    }

    #[test]
    fn mark_unread_stays() {
        let (mut watch, a) = (Watch::default(), chat("a"));
        assert!(!watch.check(Some(&a), Some("m1"), 0));
        // "Mark unread" on the open chat: unread again, but nothing came in.
        assert!(!watch.check(Some(&a), Some("m1"), 1));
        // Our own message changes nothing either: `newest` counts incoming ones.
        assert!(!watch.check(Some(&a), Some("m1"), 1));
    }
}
