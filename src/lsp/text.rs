//! Small text helpers shared by the index builders and the features.
//!
//! Everything here works on byte offsets and clamps rather than panics: a
//! language server is regularly handed a position that belongs to a
//! revision it has not seen yet, and answering nothing is always better
//! than unwinding out of a request handler.

use crate::source::Span;

/// How long a declaration shown in a hover may get before it is elided.
const MAX_DECL_TEXT: usize = 160;

/// The text a span covers, or `""` when the span is out of range or cuts
/// a character in half.
pub fn slice(text: &str, span: Span) -> &str {
    let (Ok(start), Ok(end)) = (usize::try_from(span.start), usize::try_from(span.end)) else {
        return "";
    };
    let start = start.min(text.len());
    let end = end.clamp(start, text.len());
    if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return "";
    }
    &text[start..end]
}

/// Collapses runs of whitespace to single spaces and elides what is too
/// long to read in a tooltip.
pub fn collapse(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.len() > MAX_DECL_TEXT {
            // Cut on a character boundary at or below the limit, then mark
            // the elision.
            let mut cut = MAX_DECL_TEXT.min(out.len());
            while cut > 0 && !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
            out.push('…');
            return out;
        }
    }
    out
}

/// The first non-empty line, collapsed, which is how a module or entity
/// header is shown without its whole body.
pub fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or(String::new(), collapse)
}

/// True for a byte that may appear inside an HDL identifier.
///
/// `$` is included because Verilog system names are spelled `$display`,
/// and the leading character is checked separately by the callers that
/// care.
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// The identifier ending at `offset`, which is what the user has typed so
/// far when completion is triggered.
pub fn prefix_at(text: &str, offset: u32) -> &str {
    let Ok(end) = usize::try_from(offset) else {
        return "";
    };
    let end = end.min(text.len());
    let bytes = text.as_bytes();
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return "";
    }
    &text[start..end]
}

/// The byte range of the identifier surrounding `offset`, if there is one.
pub fn word_span_at(text: &str, offset: u32) -> Option<(u32, u32)> {
    let Ok(pos) = usize::try_from(offset) else {
        return None;
    };
    let pos = pos.min(text.len());
    let bytes = text.as_bytes();
    let mut start = pos;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = pos;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    if start == end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    Some((u32::try_from(start).ok()?, u32::try_from(end).ok()?))
}

/// The identifier surrounding `offset`, extending in both directions.
pub fn word_at(text: &str, offset: u32) -> &str {
    let Ok(pos) = usize::try_from(offset) else {
        return "";
    };
    let pos = pos.min(text.len());
    let bytes = text.as_bytes();
    let mut start = pos;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = pos;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return "";
    }
    &text[start..end]
}

/// The last non-whitespace byte before the identifier ending at `offset`.
///
/// This is how the completion code tells `.clk` (a port name inside a
/// connection list) from a bare `clk`, without a second parser.
pub fn char_before_prefix(text: &str, offset: u32) -> Option<char> {
    let Ok(end) = usize::try_from(offset) else {
        return None;
    };
    let end = end.min(text.len());
    let bytes = text.as_bytes();
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    text[..start].chars().rev().find(|c| !c.is_whitespace())
}

/// True when `candidate` starts with `prefix`, ignoring case.
///
/// Completion filtering is the client's job, but a server that returns a
/// bounded list has to pre-filter, and both languages' users expect
/// case-insensitive matching while typing.
pub fn matches_prefix(candidate: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    candidate.len() >= prefix.len()
        && candidate
            .chars()
            .zip(prefix.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn span(text: &str, start: u32, end: u32) -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t.v", text).unwrap();
        Span::new(id, start, end)
    }

    #[test]
    fn slices_and_clamps() {
        let text = "module m;";
        assert_eq!(slice(text, span(text, 0, 6)), "module");
        assert_eq!(slice(text, span(text, 7, 900)), "m;");
        assert_eq!(slice(text, span(text, 900, 900)), "");
        // A span cutting a multi-byte character yields nothing.
        let uni = "é_a";
        assert_eq!(slice(uni, span(uni, 0, 1)), "");
        assert_eq!(slice(uni, span(uni, 0, 2)), "é");
    }

    #[test]
    fn collapses_and_elides() {
        assert_eq!(collapse("  wire\n   [7:0]\tq ;"), "wire [7:0] q ;");
        assert_eq!(collapse(""), "");
        let long = "x ".repeat(200);
        let collapsed = collapse(&long);
        assert!(collapsed.ends_with('…'));
        assert!(collapsed.chars().count() <= MAX_DECL_TEXT + 1);
        // Elision never splits a character.
        let wide = "é ".repeat(200);
        assert!(collapse(&wide).ends_with('…'));
    }

    #[test]
    fn first_line_skips_blanks() {
        assert_eq!(first_line("\n\n  module m;\n  wire a;\n"), "module m;");
        assert_eq!(first_line("   "), "");
    }

    #[test]
    fn finds_words_and_prefixes() {
        let text = "assign y = a_b + c;";
        let at = |needle: &str| u32::try_from(text.find(needle).unwrap()).unwrap();
        assert_eq!(prefix_at(text, at("_b") + 2), "a_b");
        assert_eq!(word_at(text, at("a_b") + 1), "a_b");
        assert_eq!(word_at(text, at(" + ")), "a_b");
        assert_eq!(prefix_at(text, 0), "");
        assert_eq!(word_at(text, 9999), "");
        assert_eq!(
            word_span_at(text, at("a_b") + 1),
            Some((at("a_b"), at("a_b") + 3))
        );
        assert_eq!(word_span_at(text, 9), None);

        let dotted = "sub u0 (.clk";
        let end = u32::try_from(dotted.len()).unwrap();
        assert_eq!(prefix_at(dotted, end), "clk");
        assert_eq!(char_before_prefix(dotted, end), Some('.'));
        assert_eq!(char_before_prefix("a b", 3), Some('a'));
        assert_eq!(char_before_prefix("abc", 3), None);
    }

    #[test]
    fn non_ascii_identifiers_hold_together() {
        let text = "wire é_x;";
        let at = u32::try_from(text.find(';').unwrap()).unwrap();
        assert_eq!(word_at(text, at), "é_x");
        assert_eq!(prefix_at(text, at), "é_x");
    }

    #[test]
    fn prefix_matching_ignores_case() {
        assert!(matches_prefix("Counter", "cou"));
        assert!(matches_prefix("counter", ""));
        assert!(!matches_prefix("cou", "counter"));
        assert!(!matches_prefix("other", "cou"));
    }
}
