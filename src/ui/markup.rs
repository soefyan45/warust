//! WhatsApp text formatting: `*bold*`, `_italic_`, `~strike~`, `` `code` ``
//! and ```` ```monospace``` ````. The text is sent as typed (the recipient's
//! WhatsApp renders it); this module only renders it in our bubbles, with
//! web links underlined (`links` finds them for clicks).

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Stroke};

use super::emoji;
use super::fonts::{self, Weight};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
}

/// Splits `text` into runs of one style, with the markers removed.
pub fn spans(text: &str) -> Vec<(String, Style)> {
    let mut out: Vec<(String, Style)> = Vec::new();
    // ```blocks``` first: their content is shown as is.
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let Some(len) = after.find("```").filter(|len| *len > 0) else {
            break;
        };
        inline(&rest[..start], &mut out);
        push(&mut out, &after[..len], Style { code: true, ..Style::default() });
        rest = &after[len + 3..];
    }
    inline(rest, &mut out);
    out
}

fn push(out: &mut Vec<(String, Style)>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some((last, last_style)) if *last_style == style => last.push_str(text),
        _ => out.push((text.to_owned(), style)),
    }
}

fn is_marker(c: char) -> bool {
    matches!(c, '*' | '_' | '~' | '`')
}

/// Inline markers: an opener follows a non-word character and is followed by
/// a non-space; the closer is the nearest same marker on the same line that
/// follows a non-space and precedes a non-word character.
fn inline(text: &str, out: &mut Vec<(String, Style)>) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let word = |i: usize| chars[i].is_alphanumeric();
    let mut paired = vec![false; n];
    let mut literal = vec![false; n];
    for i in 0..n {
        let c = chars[i];
        if !is_marker(c) || paired[i] || literal[i] {
            continue;
        }
        let opens = (i == 0 || !word(i - 1)) && i + 1 < n && !chars[i + 1].is_whitespace();
        if !opens {
            continue;
        }
        let closer = (i + 2..n)
            .take_while(|&j| chars[j] != '\n')
            .find(|&j| {
                chars[j] == c
                    && !literal[j]
                    && !chars[j - 1].is_whitespace()
                    && (j + 1 == n || !word(j + 1))
            });
        if let Some(j) = closer {
            paired[i] = true;
            paired[j] = true;
            if c == '`' {
                // Inline code: nothing inside it is formatting.
                literal[i + 1..j].iter_mut().for_each(|l| *l = true);
            }
        }
    }

    let mut style = Style::default();
    let mut run = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if paired[i] {
            push(out, &run, style);
            run.clear();
            match c {
                '*' => style.bold = !style.bold,
                '_' => style.italic = !style.italic,
                '~' => style.strike = !style.strike,
                _ => style.code = !style.code,
            }
        } else {
            run.push(c);
        }
    }
    push(out, &run, style);
}

/// Lays out formatted text for a bubble, wrapped at `wrap_width`.
pub fn job(text: &str, size: f32, color: Color32, code_bg: Color32, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    for (run, style) in spans(text) {
        let font_id = if style.code {
            fonts::mono(size - 1.0)
        } else {
            fonts::ui(size, if style.bold { Weight::SemiBold } else { Weight::Regular })
        };
        let format = TextFormat {
            font_id,
            color,
            italics: style.italic,
            strikethrough: if style.strike {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            background: if style.code { code_bg } else { Color32::TRANSPARENT },
            ..TextFormat::default()
        };
        if style.code {
            emoji::append(&mut job, &run, format);
            continue;
        }
        // Links are underlined; the bubble opens them on click.
        let chars: Vec<char> = run.chars().collect();
        let mut at = 0;
        for (range, _) in links(&run) {
            emoji::append(&mut job, &chars[at..range.start].iter().collect::<String>(), format.clone());
            let underline = TextFormat { underline: Stroke::new(1.0, color), ..format.clone() };
            emoji::append(&mut job, &chars[range.clone()].iter().collect::<String>(), underline);
            at = range.end;
        }
        emoji::append(&mut job, &chars[at..].iter().collect::<String>(), format);
    }
    job
}

/// Web links in `text`: their char range and the URL to open (`www.` gets
/// `https://`). Only http(s), so a message cannot open a local file or app.
pub fn links(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let lower = word.to_ascii_lowercase();
        let Some(offset) = ["https://", "http://", "www."].iter().filter_map(|p| lower.find(p)).min() else {
            continue;
        };
        // Only at the word start or after an opening bracket/quote.
        let lead = word[..offset].chars().count();
        if !word[..offset].chars().all(|c| matches!(c, '(' | '[' | '<' | '"' | '\'')) {
            continue;
        }
        let mut end = i;
        while end > start + lead && matches!(chars[end - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '>' | '"' | '\'' | '*' | '_' | '~') {
            end -= 1;
        }
        let shown: String = chars[start + lead..end].iter().collect();
        let host = shown.split("://").nth(1).unwrap_or(&shown);
        if host.len() < 4 || !host.contains('.') {
            continue;
        }
        let url = if shown.to_ascii_lowercase().starts_with("www.") { format!("https://{shown}") } else { shown };
        out.push((start + lead..end, url));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(bold: bool, italic: bool, strike: bool, code: bool) -> Style {
        Style { bold, italic, strike, code }
    }

    #[test]
    fn finds_web_links() {
        let found = links("lihat https://maps.app.goo.gl/xDm?g_st=aw, atau (www.example.com).");
        let urls: Vec<&str> = found.iter().map(|(_, u)| u.as_str()).collect();
        assert_eq!(urls, ["https://maps.app.goo.gl/xDm?g_st=aw", "https://www.example.com"]);
        let text: Vec<char> = "lihat https://a.co.".chars().collect();
        let (range, _) = &links("lihat https://a.co.")[0];
        assert_eq!(text[range.clone()].iter().collect::<String>(), "https://a.co");
        assert!(links("file:///etc/passwd javascript:alert(1) halo.http://x").is_empty());
    }

    #[test]
    fn plain_text_is_one_run() {
        assert_eq!(spans("halo kak"), vec![("halo kak".into(), Style::default())]);
    }

    #[test]
    fn bold_italic_strike() {
        assert_eq!(
            spans("*test* _miring_ ~coret~"),
            vec![
                ("test".into(), s(true, false, false, false)),
                (" ".into(), Style::default()),
                ("miring".into(), s(false, true, false, false)),
                (" ".into(), Style::default()),
                ("coret".into(), s(false, false, true, false)),
            ]
        );
    }

    #[test]
    fn nested_markers() {
        assert_eq!(spans("*_dua_*"), vec![("dua".into(), s(true, true, false, false))]);
    }

    #[test]
    fn markers_inside_words_or_with_spaces_stay() {
        assert_eq!(spans("2*3*4"), vec![("2*3*4".into(), Style::default())]);
        assert_eq!(spans("* bukan *"), vec![("* bukan *".into(), Style::default())]);
        assert_eq!(spans("snake_case_name"), vec![("snake_case_name".into(), Style::default())]);
        assert_eq!(spans("*belum ditutup"), vec![("*belum ditutup".into(), Style::default())]);
    }

    #[test]
    fn markers_do_not_cross_lines() {
        assert_eq!(spans("*a\nb*"), vec![("*a\nb*".into(), Style::default())]);
    }

    #[test]
    fn code_keeps_markers_literal() {
        assert_eq!(spans("`*x*`"), vec![("*x*".into(), s(false, false, false, true))]);
        assert_eq!(
            spans("lihat ```a *b*``` ya"),
            vec![
                ("lihat ".into(), Style::default()),
                ("a *b*".into(), s(false, false, false, true)),
                (" ya".into(), Style::default()),
            ]
        );
    }
}
