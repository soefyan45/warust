//! `@` mentions in the composer of a group chat: finding the `@query` before
//! the cursor, listing matching members, and turning the typed `@Name`s into
//! the `@<number>` WhatsApp expects.

use egui::{Align, Align2, Color32, Frame, Margin, Order, Sense, Stroke, Ui, pos2, vec2};

use super::fonts::{self, Weight};
use super::screens::short_jid;
use super::theme::{Palette, R_MD};
use super::widgets;
use crate::backend::{Book, Member};

/// Most rows the picker shows.
pub const MAX_ROWS: usize = 8;

/// The user part of a JID: what follows `@` in the sent text.
pub fn user_part(jid: &str) -> &str {
    let user = jid.split('@').next().unwrap_or(jid);
    user.split(':').next().unwrap_or(user)
}

/// The `@query` the cursor sits in: the byte offset of its `@` and the text
/// after it. The `@` must start the text or follow whitespace and the query
/// holds no whitespace. `cursor` counts characters.
pub fn active_query(text: &str, cursor: usize) -> Option<(usize, String)> {
    let end = text.char_indices().nth(cursor).map_or(text.len(), |(i, _)| i);
    let before = &text[..end];
    let at = before.rfind(|c: char| c == '@' || c.is_whitespace()).filter(|&i| before[i..].starts_with('@'))?;
    if before[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    Some((at, before[at + 1..].to_owned()))
}

/// A picker row.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub name: String,
    /// Shown to the right of the name.
    pub number: String,
    pub jid: String,
}

/// A member's name, else their masked number.
fn member_name(book: &Book, m: &Member) -> String {
    m.phone
        .as_deref()
        .and_then(|p| book.name_of(p))
        .or_else(|| book.name_of(&m.jid))
        .map(str::to_owned)
        .unwrap_or_else(|| short_jid(m.phone.as_deref().unwrap_or(&m.jid)))
}

/// Members whose name or number contains `query` (case-insensitive), at most
/// `MAX_ROWS`, in the group's order.
pub fn candidates(book: &Book, members: &[Member], query: &str) -> Vec<Candidate> {
    let q = query.to_lowercase();
    members
        .iter()
        .filter_map(|m| {
            let name = member_name(book, m);
            let shown = m.phone.as_deref().unwrap_or(&m.jid);
            let hit = q.is_empty()
                || name.to_lowercase().contains(&q)
                || user_part(shown).contains(&q)
                || user_part(&m.jid).contains(&q);
            hit.then(|| Candidate { name, number: short_jid(shown), jid: m.jid.clone() })
        })
        .take(MAX_ROWS)
        .collect()
}

/// Replaces the `@query` (from byte `at` to the cursor, in characters) with
/// `@<name> `. Returns the new text and the cursor, in characters, after the
/// inserted text.
pub fn insert_pick(text: &str, at: usize, cursor_char: usize, name: &str) -> (String, usize) {
    let end = text.char_indices().nth(cursor_char).map_or(text.len(), |(i, _)| i);
    let shown = format!("@{name} ");
    let mut out = String::with_capacity(text.len() + shown.len());
    out.push_str(&text[..at]);
    out.push_str(&shown);
    out.push_str(&text[end..]);
    let cursor = text[..at].chars().count() + shown.chars().count();
    (out, cursor)
}

/// `text` as it goes on the wire: each remembered `(@Name, jid)` still in the
/// text becomes `@<user part of jid>`; returns that text and the mentioned
/// JIDs (each once). A name only counts as a whole word, so `@Ana` leaves
/// `@Anabel` alone, and the longest name wins when one starts another.
pub fn to_wire(text: &str, picks: &[(String, String)]) -> (String, Vec<String>) {
    if picks.is_empty() || !text.contains('@') {
        return (text.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(text.len());
    let mut jids: Vec<String> = Vec::new();
    let mut used = vec![false; picks.len()];
    let mut prev: Option<char> = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let hit = (c == '@' && prev.is_none_or(|p| !p.is_alphanumeric()))
            .then(|| {
                let whole = |d: &str| {
                    rest.strip_prefix(d).is_some_and(|after| after.chars().next().is_none_or(|n| !n.is_alphanumeric()))
                };
                let longest = picks.iter().filter(|(d, _)| whole(d)).map(|(d, _)| d.len()).max()?;
                // The same name picked for two members: take them in order.
                let same: Vec<usize> = (0..picks.len()).filter(|&i| picks[i].0.len() == longest && whole(&picks[i].0)).collect();
                let i = same.iter().copied().find(|&i| !used[i]).unwrap_or(same[0]);
                Some((i, longest))
            })
            .flatten();
        match hit {
            Some((i, len)) => {
                used[i] = true;
                let jid = &picks[i].1;
                out.push('@');
                out.push_str(user_part(jid));
                if !jids.contains(jid) {
                    jids.push(jid.clone());
                }
                prev = out.chars().next_back();
                rest = &rest[len..];
            }
            None => {
                out.push(c);
                prev = Some(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    (out, jids)
}

/// What the popup shows.
pub enum Rows<'a> {
    Loading,
    List(&'a [Candidate]),
}

/// Paints the picker above `anchor` (the top-left of the text box); returns
/// the row clicked.
pub fn popup(ui: &Ui, p: &Palette, anchor: egui::Pos2, width: f32, rows: Rows, selected: usize) -> Option<usize> {
    let mut clicked = None;
    let radius = super::theme::r_inner();
    egui::Area::new(ui.make_persistent_id("mention_picker"))
        .order(Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(anchor.x, anchor.y - 8.0))
        .show(ui.ctx(), |ui| {
            Frame::NONE
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(R_MD)
                .inner_margin(Margin::same(4))
                .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(40) })
                .show(ui, |ui| {
                    let width = width.clamp(200.0, 360.0);
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match rows {
                        Rows::Loading => {
                            let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
                            widgets::paint_text(ui, rect.shrink2(vec2(10.0, 0.0)), "Memuat anggota…", fonts::ui(13.0, Weight::Regular), p.muted, Align::Min);
                        }
                        Rows::List(list) => {
                            for (i, c) in list.iter().enumerate() {
                                let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
                                if i == selected {
                                    ui.painter().rect_filled(rect, radius, p.selected);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(rect, radius, p.raised);
                                }
                                let inner = rect.shrink2(vec2(10.0, 0.0));
                                let number_w = if c.name == c.number {
                                    0.0
                                } else {
                                    ui.painter().layout_no_wrap(c.number.clone(), fonts::mono(12.0), p.muted).size().x
                                };
                                let number = egui::Rect::from_min_max(pos2(inner.right() - number_w, inner.top()), inner.max);
                                let name = egui::Rect::from_min_max(inner.min, pos2(number.left() - 8.0, inner.bottom()));
                                widgets::paint_text(ui, name, &c.name, fonts::ui(13.0, Weight::SemiBold), p.text, Align::Min);
                                if number_w > 0.0 {
                                    widgets::paint_text(ui, number, &c.number, fonts::mono(12.0), p.muted, Align::Max);
                                }
                                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    clicked = Some(i);
                                }
                            }
                        }
                    }
                });
        });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(name: &str, jid: &str) -> (String, String) {
        (format!("@{name}"), jid.to_owned())
    }

    #[test]
    fn query_after_at() {
        assert_eq!(active_query("halo @bu", 8), Some((5, "bu".to_owned())));
        assert_eq!(active_query("@", 1), Some((0, String::new())));
        assert_eq!(active_query("a\n@x", 4), Some((2, "x".to_owned())));
        // Cursor in the middle of the query: only what is before it counts.
        assert_eq!(active_query("@budi", 3), Some((0, "bu".to_owned())));
    }

    #[test]
    fn no_query_for_email_or_space() {
        assert_eq!(active_query("a@b", 3), None);
        assert_eq!(active_query("@budi santoso", 13), None);
        assert_eq!(active_query("halo", 4), None);
        assert_eq!(active_query("@a halo", 0), None);
    }

    #[test]
    fn query_counts_characters_not_bytes() {
        assert_eq!(active_query("é @x", 4), Some((3, "x".to_owned())));
    }

    #[test]
    fn insert_replaces_query_and_places_cursor() {
        let (text, cursor) = insert_pick("halo @bu apa", 5, 8, "Budi");
        assert_eq!(text, "halo @Budi  apa");
        assert_eq!(cursor, 11);
        let (text, cursor) = insert_pick("@", 0, 1, "Ani Wati");
        assert_eq!(text, "@Ani Wati ");
        assert_eq!(cursor, 10);
    }

    #[test]
    fn one_mention() {
        let (text, jids) = to_wire("halo @Budi apa kabar", &[pick("Budi", "6281@s.whatsapp.net")]);
        assert_eq!(text, "halo @6281 apa kabar");
        assert_eq!(jids, ["6281@s.whatsapp.net"]);
    }

    #[test]
    fn two_mentions_lid_and_phone() {
        let picks = [pick("Budi", "6281@s.whatsapp.net"), pick("Ani Wati", "99887@lid")];
        let (text, jids) = to_wire("@Budi @Ani Wati tolong cek", &picks);
        assert_eq!(text, "@6281 @99887 tolong cek");
        assert_eq!(jids, ["6281@s.whatsapp.net", "99887@lid"]);
    }

    #[test]
    fn deleted_mention_is_not_sent() {
        let picks = [pick("Budi", "6281@s.whatsapp.net"), pick("Ani", "6282@s.whatsapp.net")];
        let (text, jids) = to_wire("halo @Ani", &picks);
        assert_eq!(text, "halo @6282");
        assert_eq!(jids, ["6282@s.whatsapp.net"]);
        let (text, jids) = to_wire("halo semua", &picks);
        assert_eq!(text, "halo semua");
        assert!(jids.is_empty());
    }

    #[test]
    fn same_name_twice_is_one_jid() {
        let (text, jids) = to_wire("@Budi dan @Budi lagi", &[pick("Budi", "6281@s.whatsapp.net")]);
        assert_eq!(text, "@6281 dan @6281 lagi");
        assert_eq!(jids, ["6281@s.whatsapp.net"]);
    }

    #[test]
    fn prefix_names_do_not_clash() {
        let picks = [pick("Ana", "1@s.whatsapp.net"), pick("Anabel", "2@s.whatsapp.net")];
        let (text, jids) = to_wire("@Anabel dan @Ana", &picks);
        assert_eq!(text, "@2 dan @1");
        assert_eq!(jids, ["2@s.whatsapp.net", "1@s.whatsapp.net"]);
        // Only @Anabel was picked: a typed "@Ana" is plain text.
        let (text, jids) = to_wire("@Ana", &[pick("Anabel", "2@s.whatsapp.net")]);
        assert_eq!(text, "@Ana");
        assert!(jids.is_empty());
        // Only @Ana was picked: "@Anabel" is left alone.
        let (text, jids) = to_wire("@Anabel", &[pick("Ana", "1@s.whatsapp.net")]);
        assert_eq!(text, "@Anabel");
        assert!(jids.is_empty());
    }

    #[test]
    fn two_members_with_the_same_name_go_in_order() {
        let picks = [pick("Andi", "1@s.whatsapp.net"), pick("Andi", "2@s.whatsapp.net")];
        let (text, jids) = to_wire("@Andi lalu @Andi", &picks);
        assert_eq!(text, "@1 lalu @2");
        assert_eq!(jids, ["1@s.whatsapp.net", "2@s.whatsapp.net"]);
    }

    #[test]
    fn candidates_filter_by_name_or_number() {
        let book = Book::default();
        let members = vec![
            Member { jid: "6281234@s.whatsapp.net".into(), phone: None },
            Member { jid: "5550001@lid".into(), phone: Some("6289876@s.whatsapp.net".into()) },
        ];
        assert_eq!(candidates(&book, &members, "").len(), 2);
        let by_number = candidates(&book, &members, "9876");
        assert_eq!(by_number.len(), 1);
        assert_eq!(by_number[0].jid, "5550001@lid");
        assert!(candidates(&book, &members, "zzz").is_empty());
        let many: Vec<Member> = (0..20).map(|i| Member { jid: format!("62{i}@s.whatsapp.net"), phone: None }).collect();
        assert_eq!(candidates(&book, &many, "").len(), MAX_ROWS);
    }
}
