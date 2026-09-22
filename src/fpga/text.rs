//! The shared line-oriented tokenizer behind the `.dev` and `.rcf` formats.
//!
//! Both text formats in this module are line oriented, whitespace
//! separated, `#`- or `//`-commented, and case sensitive. They share this
//! tokenizer so that quoting, spans and error recovery behave identically
//! in a device database and in a constraints file.
//!
//! A [`Line`] is a non-empty list of [`Token`]s plus the span of the whole
//! line; blank and comment-only lines are dropped, so a parser never has to
//! think about them. Every token keeps its own [`Span`], which is what the
//! diagnostics point at.

use std::fmt;

use crate::source::{SourceId, Span};

/// One whitespace-separated word of a line, with its source span.
#[derive(Clone, Debug)]
pub(crate) struct Token {
    /// The token text, with quotes removed and escapes resolved.
    pub(crate) text: String,
    /// Where the token (including its quotes) came from.
    pub(crate) span: Span,
}

impl Token {
    /// The token text.
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    /// True when the token is exactly `word`.
    pub(crate) fn is(&self, word: &str) -> bool {
        self.text == word
    }

    /// Splits a `key=value` token into its halves.
    pub(crate) fn pair(&self) -> Option<(&str, &str)> {
        self.text.split_once('=')
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One non-empty source line.
#[derive(Clone, Debug)]
pub(crate) struct Line {
    /// The words of the line, in order; never empty.
    pub(crate) tokens: Vec<Token>,
    /// The span of the whole line, excluding the newline.
    pub(crate) span: Span,
}

impl Line {
    /// The first token, which every grammar here uses as the keyword.
    pub(crate) fn keyword(&self) -> &str {
        self.tokens[0].as_str()
    }

    /// The token at `index`, if the line is long enough.
    pub(crate) fn get(&self, index: usize) -> Option<&Token> {
        self.tokens.get(index)
    }
}

/// Splits `text` into lines of tokens, dropping comments and blank lines.
///
/// A token is a run of non-whitespace characters, or a double-quoted
/// string with `\"`, `\\`, `\n`, `\r` and `\t` escapes. `#` and `//` start
/// a comment that runs to the end of the line; neither is special inside a
/// quoted string.
pub(crate) fn tokenize(text: &str, file: SourceId) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for raw in text.split_inclusive('\n') {
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        let tokens = tokenize_line(trimmed, file, offset);
        if !tokens.is_empty() {
            let start = at(offset);
            let end = at(offset + trimmed.len());
            lines.push(Line {
                tokens,
                span: Span::new(file, start, end),
            });
        }
        offset += raw.len();
    }
    lines
}

/// Narrows a byte offset inside an accepted source file to `u32`.
///
/// [`crate::source::SourceMap::add`] rejects files that do not fit in 32
/// bits, so this cannot fail for text that came from the map; a caller that
/// tokenizes a string it never added gets a panic rather than a silent
/// truncation.
fn at(offset: usize) -> u32 {
    u32::try_from(offset).expect("source offset exceeds u32")
}

fn tokenize_line(line: &str, file: SourceId, base: usize) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b'#' || (bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/')) {
            break;
        }
        let start = i;
        let mut text = String::new();
        // A word runs to the next whitespace, but a quoted section may
        // start anywhere inside it, which is how `key="a value"` stays one
        // token and how a pin named `A 1` is written.
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'#' {
            if bytes[i] == b'"' {
                i = read_quoted(line, i, &mut text);
                continue;
            }
            let c = line[i..].chars().next().unwrap_or(' ');
            text.push(c);
            i += c.len_utf8();
        }
        tokens.push(Token {
            text,
            span: Span::new(file, at(base + start), at(base + i)),
        });
    }
    tokens
}

/// Reads the quoted section starting at `start` into `out` and returns the
/// offset just past its closing quote.
///
/// A missing closing quote ends the section at the end of the line; the
/// parser reports the shape that results, not the quote itself.
fn read_quoted(line: &str, start: usize, out: &mut String) -> usize {
    let bytes = line.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() && bytes[i] != b'"' {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            out.push(match bytes[i] {
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                other => char::from(other),
            });
            i += 1;
        } else {
            let c = line[i..].chars().next().unwrap_or('"');
            out.push(c);
            i += c.len_utf8();
        }
    }
    if i < bytes.len() { i + 1 } else { i }
}

/// Renders `word` so that [`tokenize`] reads it back unchanged.
///
/// Words made only of the characters a device or constraints file uses for
/// names, numbers and sized literals are written bare; anything else is
/// quoted and escaped.
pub(crate) fn quote(word: &str) -> String {
    let simple = !word.is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '_' | '$' | '.' | '/' | '-' | '+' | '\'' | '=' | ',' | ':'
                )
        })
        && !word.starts_with('#');
    if simple {
        return word.to_owned();
    }
    let mut out = String::with_capacity(word.len() + 2);
    out.push('"');
    for c in word.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn lines(text: &str) -> Vec<Vec<String>> {
        let mut map = SourceMap::new();
        let file = map.add("t.dev", text).unwrap();
        tokenize(text, file)
            .into_iter()
            .map(|l| l.tokens.into_iter().map(|t| t.text).collect())
            .collect()
    }

    #[test]
    fn splits_words_and_strings() {
        let got = lines("device a  # comment\n\n  pin \"A 1\" io // trailing\nend\n");
        assert_eq!(
            got,
            vec![
                vec!["device".to_owned(), "a".to_owned()],
                vec!["pin".to_owned(), "A 1".to_owned(), "io".to_owned()],
                vec!["end".to_owned()],
            ]
        );
    }

    #[test]
    fn spans_point_at_tokens() {
        let text = "family ice40\n";
        let mut map = SourceMap::new();
        let file = map.add("t.dev", text).unwrap();
        let lines = tokenize(text, file);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].keyword(), "family");
        let span = lines[0].tokens[1].span;
        assert_eq!(&text[span.start as usize..span.end as usize], "ice40");
        assert!(lines[0].get(2).is_none());
        assert!(lines[0].tokens[0].is("family"));
    }

    #[test]
    fn escapes_round_trip() {
        let text = "pin \"a\\\"b\" io\n";
        let got = lines(text);
        assert_eq!(got[0][1], "a\"b");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("A1"), "A1");
        assert_eq!(quote("6'b011001"), "6'b011001");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("x\n"), "\"x\\n\"");
        assert_eq!(quote("\\"), "\"\\\\\"");
        assert_eq!(quote("\r\t"), "\"\\r\\t\"");
    }

    #[test]
    fn pair_splits_on_equals() {
        let mut map = SourceMap::new();
        let file = map.add("t", "pad=PACKAGE_PIN plain").unwrap();
        let l = tokenize("pad=PACKAGE_PIN plain", file);
        assert_eq!(l[0].tokens[0].pair(), Some(("pad", "PACKAGE_PIN")));
        assert_eq!(l[0].tokens[1].pair(), None);
    }
}
