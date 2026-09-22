//! The tokenizer and cursor shared by the LEF and DEF readers.
//!
//! Both formats are whitespace-separated word streams where `;` ends a
//! statement, `(` and `)` delimit points, `#` starts a comment to the end
//! of the line and `"..."` quotes a string. Names may contain `[`, `]`,
//! `.`, `/` and `\`, so the tokenizer only ever splits on whitespace and on
//! the three punctuation characters.

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, Span};

/// What a token is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A bare word: keyword, name or number.
    Word,
    /// A double-quoted string; `text` is the content without quotes.
    Str,
    /// `;`
    Semi,
    /// `(`
    LParen,
    /// `)`
    RParen,
}

/// One token with its text and location.
#[derive(Clone, Copy, Debug)]
pub(super) struct Token<'a> {
    pub kind: Kind,
    pub text: &'a str,
    pub span: Span,
}

/// A recoverable syntax error at one location.
#[derive(Clone, Debug)]
pub(super) struct ParseError {
    pub span: Span,
    pub message: String,
}

impl ParseError {
    pub(super) fn new(span: Span, message: impl Into<String>) -> Self {
        ParseError {
            span,
            message: message.into(),
        }
    }

    pub(super) fn into_diagnostic(self) -> Diagnostic {
        Diagnostic::error(self.message).with_span(self.span)
    }
}

pub(super) type PResult<T> = Result<T, ParseError>;

/// A raw statement kept for constructs the typed reader does not model:
/// its tokens as written, strings re-quoted, without the closing `;`.
pub(super) fn raw_text(tokens: &[Token<'_>]) -> Vec<String> {
    tokens
        .iter()
        .map(|t| match t.kind {
            Kind::Str => format!("\"{}\"", t.text),
            _ => t.text.to_string(),
        })
        .collect()
}

fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("source offset exceeds u32")
}

/// Splits `text` into tokens. Never fails: an unterminated string runs to
/// the end of the file.
pub(super) fn tokenize(text: &str, file: SourceId) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        let (kind, end, text_range) = match c {
            b';' => (Kind::Semi, i + 1, (start, i + 1)),
            b'(' => (Kind::LParen, i + 1, (start, i + 1)),
            b')' => (Kind::RParen, i + 1, (start, i + 1)),
            b'"' => {
                i += 1;
                let content_start = i;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                let content_end = i;
                let end = if i < bytes.len() { i + 1 } else { i };
                (Kind::Str, end, (content_start, content_end))
            }
            _ => {
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b';' | b'(' | b')' | b'"' | b'#')
                {
                    i += 1;
                }
                (Kind::Word, i, (start, i))
            }
        };
        out.push(Token {
            kind,
            text: &text[text_range.0..text_range.1],
            span: Span::new(file, offset(start), offset(end)),
        });
        i = end;
    }
    out
}

/// A cursor over a token list with the small set of helpers the readers
/// need.
pub(super) struct Cursor<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
    eof: Span,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(text: &'a str, file: SourceId) -> Self {
        let tokens = tokenize(text, file);
        let end = offset(text.len());
        Cursor {
            tokens,
            pos: 0,
            eof: Span::new(file, end, end),
        }
    }

    pub(super) fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    pub(super) fn peek(&self) -> Option<Token<'a>> {
        self.tokens.get(self.pos).copied()
    }

    pub(super) fn peek_at(&self, n: usize) -> Option<Token<'a>> {
        self.tokens.get(self.pos + n).copied()
    }

    /// The span of the next token, or of the end of the file.
    pub(super) fn span(&self) -> Span {
        self.peek().map_or(self.eof, |t| t.span)
    }

    pub(super) fn next(&mut self) -> Option<Token<'a>> {
        let t = self.peek();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// True when the next token is the given word.
    pub(super) fn is_word(&self, word: &str) -> bool {
        self.peek()
            .is_some_and(|t| t.kind == Kind::Word && t.text == word)
    }

    pub(super) fn is_kind(&self, kind: Kind) -> bool {
        self.peek().is_some_and(|t| t.kind == kind)
    }

    /// Consumes the given word if it is next.
    pub(super) fn eat_word(&mut self, word: &str) -> bool {
        if self.is_word(word) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn eat_kind(&mut self, kind: Kind) -> bool {
        if self.is_kind(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn expect_word(&mut self, word: &str) -> PResult<()> {
        if self.eat_word(word) {
            Ok(())
        } else {
            Err(self.error(format!("expected `{word}`")))
        }
    }

    pub(super) fn expect_semi(&mut self) -> PResult<()> {
        if self.eat_kind(Kind::Semi) {
            Ok(())
        } else {
            Err(self.error("expected `;`"))
        }
    }

    pub(super) fn expect_lparen(&mut self) -> PResult<()> {
        if self.eat_kind(Kind::LParen) {
            Ok(())
        } else {
            Err(self.error("expected `(`"))
        }
    }

    pub(super) fn expect_rparen(&mut self) -> PResult<()> {
        if self.eat_kind(Kind::RParen) {
            Ok(())
        } else {
            Err(self.error("expected `)`"))
        }
    }

    /// Consumes a word or string and returns its text.
    pub(super) fn name(&mut self) -> PResult<&'a str> {
        match self.peek() {
            Some(t) if matches!(t.kind, Kind::Word | Kind::Str) => {
                self.pos += 1;
                Ok(t.text)
            }
            _ => Err(self.error("expected a name")),
        }
    }

    /// Consumes a bare word (not a string) and returns its text.
    pub(super) fn word(&mut self) -> PResult<&'a str> {
        match self.peek() {
            Some(t) if t.kind == Kind::Word => {
                self.pos += 1;
                Ok(t.text)
            }
            _ => Err(self.error("expected a keyword")),
        }
    }

    pub(super) fn number(&mut self) -> PResult<f64> {
        match self.peek() {
            Some(t) if t.kind == Kind::Word => match t.text.parse::<f64>() {
                Ok(v) => {
                    self.pos += 1;
                    Ok(v)
                }
                Err(_) => Err(self.error(format!("expected a number, found `{}`", t.text))),
            },
            _ => Err(self.error("expected a number")),
        }
    }

    pub(super) fn integer(&mut self) -> PResult<i64> {
        match self.peek() {
            Some(t) if t.kind == Kind::Word => {
                if let Ok(v) = t.text.parse::<i64>() {
                    self.pos += 1;
                    return Ok(v);
                }
                // DEF writers occasionally print integral coordinates as
                // `100.0`; accept those.
                match t.text.parse::<f64>().ok().and_then(super::float_to_int) {
                    Some(v) => {
                        self.pos += 1;
                        Ok(v)
                    }
                    None => Err(self.error(format!("expected an integer, found `{}`", t.text))),
                }
            }
            _ => Err(self.error("expected an integer")),
        }
    }

    /// Consumes tokens up to and including the next `;` and returns them
    /// (without the `;`). Used to keep unknown statements verbatim.
    pub(super) fn raw_statement(&mut self) -> Vec<String> {
        let start = self.pos;
        while let Some(t) = self.peek() {
            self.pos += 1;
            if t.kind == Kind::Semi {
                return raw_text(&self.tokens[start..self.pos - 1]);
            }
        }
        raw_text(&self.tokens[start..])
    }

    /// Consumes tokens until (not including) a `;`, a word from `stops`,
    /// or the end; returns them re-quoted.
    pub(super) fn raw_until(&mut self, stops: &[&str]) -> Vec<String> {
        let start = self.pos;
        while let Some(t) = self.peek() {
            if t.kind == Kind::Semi || (t.kind == Kind::Word && stops.contains(&t.text)) {
                break;
            }
            self.pos += 1;
        }
        raw_text(&self.tokens[start..self.pos])
    }

    /// Skips to just after the next `;`, for error recovery.
    pub(super) fn skip_statement(&mut self) {
        while let Some(t) = self.next() {
            if t.kind == Kind::Semi {
                break;
            }
        }
    }

    pub(super) fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError::new(self.span(), message)
    }

    /// Reports an error and skips past the statement it occurred in.
    pub(super) fn recover(&mut self, err: ParseError, diags: &mut Diagnostics) {
        diags.push(err.into_diagnostic());
        self.skip_statement();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    #[test]
    fn tokenizes_words_strings_and_punctuation() {
        let mut map = SourceMap::new();
        let text = "MACRO a[0] # comment\n  RECT 0 0.5 (1 2) ; \"x y\";";
        let id = map.add("t", text).unwrap();
        let toks = tokenize(text, id);
        let texts: Vec<(Kind, &str)> = toks.iter().map(|t| (t.kind, t.text)).collect();
        assert_eq!(
            texts,
            vec![
                (Kind::Word, "MACRO"),
                (Kind::Word, "a[0]"),
                (Kind::Word, "RECT"),
                (Kind::Word, "0"),
                (Kind::Word, "0.5"),
                (Kind::LParen, "("),
                (Kind::Word, "1"),
                (Kind::Word, "2"),
                (Kind::RParen, ")"),
                (Kind::Semi, ";"),
                (Kind::Str, "x y"),
                (Kind::Semi, ";"),
            ]
        );
        assert_eq!(map.locate(toks[2].span).1.line, 2);
    }

    #[test]
    fn cursor_helpers() {
        let mut map = SourceMap::new();
        let text = "A 1 2.5 ( 3 ) ; UNKNOWN x \"y z\" ;";
        let id = map.add("t", text).unwrap();
        let mut c = Cursor::new(text, id);
        assert!(c.expect_word("A").is_ok());
        assert_eq!(c.integer().unwrap(), 1);
        assert_eq!(c.number().unwrap(), 2.5);
        assert!(c.expect_lparen().is_ok());
        assert_eq!(c.integer().unwrap(), 3);
        assert!(c.expect_rparen().is_ok());
        assert!(c.expect_semi().is_ok());
        assert_eq!(c.raw_statement(), vec!["UNKNOWN", "x", "\"y z\""]);
        assert!(c.at_end());
        assert!(c.expect_word("B").is_err());
    }
}
