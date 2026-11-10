//! Lists in the composer: a new line inside `1. …` or `* …` starts the next
//! item by itself, and a new line on an item left empty ends the list.

/// A list item's line split into its indent, its marker and what follows the
/// marker's space. Markers are a number with `.` or `)`, or one of `*`, `-`,
/// `•`; without the space it is no item (`*bold*`, `1.5`).
fn item(line: &str) -> Option<(&str, &str, &str)> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - body.len()];
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let marker = if digits > 0 {
        let after = body[digits..].chars().next().filter(|c| matches!(c, '.' | ')'))?;
        digits + after.len_utf8()
    } else {
        body.chars().next().filter(|c| matches!(c, '*' | '-' | '•'))?.len_utf8()
    };
    let rest = body[marker..].strip_prefix(' ')?;
    Some((indent, &body[..marker], rest))
}

/// The marker of the item after this one: the number plus one, a bullet as is.
fn next(marker: &str) -> Option<String> {
    let digits = marker.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Some(marker.to_owned());
    }
    let number: u32 = marker[..digits].parse().ok()?;
    Some(format!("{}{}", number.checked_add(1)?, &marker[digits..]))
}

/// What a new line at `cursor` (in characters) does inside a list: the new
/// text and where the cursor goes. `None` outside a list, where the text box
/// adds its plain new line.
pub fn new_line(text: &str, cursor: usize) -> Option<(String, usize)> {
    let at = text.char_indices().nth(cursor).map_or(text.len(), |(i, _)| i);
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let (indent, marker, rest) = item(&text[start..at])?;
    if rest.trim().is_empty() {
        // An empty item ends the list, unless text follows the cursor on this line.
        let line_end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        if !text[at..line_end].trim().is_empty() {
            return None;
        }
        let out = format!("{}{}", &text[..start], &text[line_end..]);
        return Some((out, text[..start].chars().count()));
    }
    let added = format!("\n{indent}{} ", next(marker)?);
    let out = format!("{}{added}{}", &text[..at], &text[at..]);
    Some((out, cursor + added.chars().count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new line at the end of `text`.
    fn at_end(text: &str) -> Option<(String, usize)> {
        new_line(text, text.chars().count())
    }

    #[test]
    fn numbered_items_count_up() {
        assert_eq!(at_end("1. satu"), Some(("1. satu\n2. ".to_owned(), 11)));
        assert_eq!(at_end("halo\n9) sembilan").unwrap().0, "halo\n9) sembilan\n10) ");
        assert_eq!(at_end("  3. tiga").unwrap().0, "  3. tiga\n  4. ");
    }

    #[test]
    fn bullets_repeat() {
        assert_eq!(at_end("* satu").unwrap().0, "* satu\n* ");
        assert_eq!(at_end("- satu").unwrap().0, "- satu\n- ");
        assert_eq!(at_end("• satu").unwrap().0, "• satu\n• ");
    }

    #[test]
    fn the_cursor_splits_an_item() {
        // "1. sa|tu": the rest of the line moves to the new item.
        assert_eq!(new_line("1. satu\nakhir", 5), Some(("1. sa\n2. tu\nakhir".to_owned(), 9)));
        // Counted in characters, not bytes.
        assert_eq!(at_end("• café ☕"), Some(("• café ☕\n• ".to_owned(), 11)));
    }

    #[test]
    fn an_empty_item_ends_the_list() {
        assert_eq!(at_end("1. satu\n2. "), Some(("1. satu\n".to_owned(), 8)));
        assert_eq!(at_end("* "), Some((String::new(), 0)));
        // Text after the cursor stays an item: a plain new line.
        assert_eq!(new_line("1. satu", 3), None);
    }

    #[test]
    fn other_lines_are_left_alone() {
        for text in ["halo", "*tebal* sekali", "1.5 juta", "-", "2024 tahun", "1. satu\nlalu ini", ""] {
            assert_eq!(at_end(text), None, "{text:?}");
        }
        assert_eq!(at_end("4294967295. banyak"), None);
    }
}
