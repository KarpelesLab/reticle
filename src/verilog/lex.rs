//! The Verilog / SystemVerilog lexer.
//!
//! [`Lexer`] turns text into a `Vec<Token>` in one pass, with a trailing
//! [`TokenKind::Eof`]. It works on bytes and only ever splits the text at
//! ASCII characters, so multi-byte UTF-8 in comments, strings and escaped
//! identifiers never lands a span in the middle of a character.
//!
//! The lexer is usually run on preprocessed text (see
//! [`crate::verilog::preprocess`]) and given the [`SpanMap`] from that run
//! so every token and diagnostic reports its original location. Run
//! without a map it reports offsets into the text it was given.
//!
//! Numbers are not evaluated here: a [`TokenKind::Number`] keeps the
//! literal text (`8'hff`, `1.5e3`, `10ns`) for the value parser to
//! interpret with the width and sign rules that depend on context. The lexer
//! does validate the shape of a literal (digits legal for the base, digits
//! present after the base) and reports a malformed one as an error token.
//!
//! Comments are not tokens but are kept in [`Lexed::comments`] for tooling
//! such as a formatter or a documentation extractor.
//!
//! `` `begin_keywords `` / `` `end_keywords `` are consumed here, since they
//! change which words are reserved; every other directive that reaches the
//! lexer becomes a [`TokenKind::Directive`] for the parser.

use crate::diag::Diagnostics;
use crate::source::{SourceId, Span};

use super::preprocess::SpanMap;
use super::token::{Dialect, Keyword, Punct, Token, TokenKind};

/// Narrows a byte offset to the `u32` used in spans.
///
/// Lexer input came from a [`crate::source::SourceMap`] (directly or via
/// the preprocessor) and is bounded by the same limit, so the conversion
/// cannot fail.
fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("lexer offset exceeds u32")
}

/// Which kind of comment was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentKind {
    /// `// ...` to the end of the line.
    Line,
    /// `/* ... */`.
    Block,
}

/// The result of lexing one text.
#[derive(Clone, Debug, Default)]
pub struct Lexed {
    /// The tokens, ending with [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Every comment, in source order, with the span of its full text
    /// including the delimiters.
    pub comments: Vec<(Span, CommentKind)>,
}

/// A one-pass lexer over a text.
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    file: SourceId,
    dialect: Dialect,
    /// Dialects saved by `` `begin_keywords ``.
    dialect_stack: Vec<Dialect>,
    spans: Option<&'a SpanMap>,
    diags: &'a mut Diagnostics,
    out: Lexed,
    /// Set after the `(` of a `(*)` wildcard so the `*` is not taken as
    /// the start of `*)`.
    wildcard_star: bool,
}

impl<'a> Lexer<'a> {
    /// Creates a lexer over `src`, reporting spans in `file` and errors to
    /// `diags`.
    pub fn new(src: &'a str, file: SourceId, dialect: Dialect, diags: &'a mut Diagnostics) -> Self {
        Lexer {
            src,
            pos: 0,
            file,
            dialect,
            dialect_stack: Vec::new(),
            spans: None,
            diags,
            out: Lexed::default(),
            wildcard_star: false,
        }
    }

    /// Maps every span through `spans` (from the preprocessor) instead of
    /// reporting offsets into `src`.
    pub fn with_span_map(mut self, spans: &'a SpanMap) -> Self {
        self.spans = Some(spans);
        self
    }

    /// Lexes the whole text.
    pub fn run(mut self) -> Lexed {
        loop {
            self.skip_trivia();
            let start = self.pos;
            let Some(b) = self.peek() else {
                let span = self.span(start, start);
                self.out.tokens.push(Token::new(TokenKind::Eof, span));
                break;
            };
            let kind = self.lex_token(b, start);
            if let Some(kind) = kind {
                let span = self.span(start, self.pos);
                self.out.tokens.push(Token::new(kind, span));
            }
        }
        self.out
    }

    // --- cursor helpers -------------------------------------------------

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + n).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s)
    }

    /// Advances over one character (of any width).
    fn bump_char(&mut self) {
        if let Some(c) = self.src[self.pos..].chars().next() {
            self.pos += c.len_utf8();
        }
    }

    /// Advances while `pred` holds for the byte under the cursor.
    fn eat_while(&mut self, pred: impl Fn(u8) -> bool) {
        while self.peek().is_some_and(&pred) {
            self.pos += 1;
        }
    }

    fn span(&self, start: usize, end: usize) -> Span {
        match self.spans {
            Some(map) => map.map(offset(start), offset(end)),
            None => Span::new(self.file, offset(start), offset(end)),
        }
    }

    fn error(&mut self, start: usize, end: usize, message: impl Into<String>) {
        let span = self.span(start, end);
        self.diags.error(span, message);
    }

    fn text(&self, start: usize) -> String {
        self.src[start..self.pos].to_string()
    }

    // --- trivia -----------------------------------------------------------

    /// Skips whitespace and comments, recording the comments.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') => self.pos += 1,
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    let start = self.pos;
                    self.eat_while(|b| b != b'\n');
                    let span = self.span(start, self.pos);
                    self.out.comments.push((span, CommentKind::Line));
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    let start = self.pos;
                    match self.src[start + 2..].find("*/") {
                        Some(n) => self.pos = start + 2 + n + 2,
                        None => {
                            self.pos = self.src.len();
                            self.error(start, start + 2, "unterminated block comment");
                        }
                    }
                    let span = self.span(start, self.pos);
                    self.out.comments.push((span, CommentKind::Block));
                }
                _ => return,
            }
        }
    }

    // --- tokens -----------------------------------------------------------

    /// Lexes one token starting at `start` with first byte `b`. Returns
    /// `None` for input that produced no token (keyword-set directives).
    fn lex_token(&mut self, b: u8, start: usize) -> Option<TokenKind> {
        Some(match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                self.eat_while(is_ident_char);
                let name = &self.src[start..self.pos];
                match Keyword::lookup(name, self.dialect) {
                    Some(kw) => TokenKind::Keyword(kw),
                    None => TokenKind::Ident {
                        name: name.to_string(),
                    },
                }
            }
            b'\\' => self.lex_escaped_ident(start),
            b'$' => {
                self.pos += 1;
                self.eat_while(is_ident_char);
                if self.pos == start + 1 {
                    TokenKind::Punct(Punct::Dollar)
                } else {
                    TokenKind::SystemIdent {
                        name: self.src[start + 1..self.pos].to_string(),
                    }
                }
            }
            b'0'..=b'9' => self.lex_number(start),
            b'\'' => self.lex_apostrophe(start),
            b'"' => self.lex_string(start),
            b'`' => return self.lex_directive(start),
            _ => match self.lex_punct() {
                Some(p) => TokenKind::Punct(p),
                None => {
                    self.bump_char();
                    let text = self.text(start);
                    self.error(start, self.pos, format!("unexpected character `{text}`"));
                    TokenKind::Error { text }
                }
            },
        })
    }

    /// `\name`: everything up to whitespace, stored without the backslash.
    fn lex_escaped_ident(&mut self, start: usize) -> TokenKind {
        self.pos += 1;
        self.eat_while(|b| !b.is_ascii_whitespace());
        if self.pos == start + 1 {
            self.error(start, self.pos, "escaped identifier has no characters");
            return TokenKind::Error {
                text: self.text(start),
            };
        }
        TokenKind::EscapedIdent {
            name: self.src[start + 1..self.pos].to_string(),
        }
    }

    /// A literal starting with a decimal digit: decimal, real, sized based
    /// literal or time literal.
    fn lex_number(&mut self, start: usize) -> TokenKind {
        self.eat_while(|b| b.is_ascii_digit() || b == b'_');

        // Real: `1.5`, `1e3`, `1.5e-3`.
        let mut is_real = false;
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
            self.eat_while(|b| b.is_ascii_digit() || b == b'_');
            is_real = true;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let sign = matches!(self.peek_at(1), Some(b'+' | b'-'));
            let digit_at = if sign { 2 } else { 1 };
            if self.peek_at(digit_at).is_some_and(|b| b.is_ascii_digit()) {
                self.pos += digit_at;
                self.eat_while(|b| b.is_ascii_digit() || b == b'_');
                is_real = true;
            }
        }
        if is_real {
            self.eat_time_unit();
            return TokenKind::Number {
                text: self.text(start),
            };
        }

        // Sized based literal: `8'hff`, also `8 'h ff` with blanks.
        let mut probe = self.pos;
        while matches!(self.src.as_bytes().get(probe), Some(b' ' | b'\t')) {
            probe += 1;
        }
        if self.src.as_bytes().get(probe) == Some(&b'\'') && self.base_at(probe + 1).is_some() {
            self.pos = probe;
            return self.lex_based(start);
        }

        self.eat_time_unit();
        TokenKind::Number {
            text: self.text(start),
        }
    }

    /// If a time unit (`s`, `ms`, `us`, `ns`, `ps`, `fs`) or `step` follows
    /// immediately and is not part of a longer identifier, consumes it.
    fn eat_time_unit(&mut self) {
        let rest = &self.src[self.pos..];
        let len = ["step", "ms", "us", "ns", "ps", "fs", "s"]
            .iter()
            .find(|u| rest.starts_with(*u))
            .map_or(0, |u| u.len());
        if len > 0 && !rest.as_bytes().get(len).is_some_and(|&b| is_ident_char(b)) {
            self.pos += len;
        }
    }

    /// The base letter of a based literal whose `'` is at `apostrophe`,
    /// with the offset just past it: handles `'h`, `'sh`, `'Sh`.
    fn base_at(&self, after_apostrophe: usize) -> Option<(u8, usize)> {
        let bytes = self.src.as_bytes();
        let mut i = after_apostrophe;
        if matches!(bytes.get(i), Some(b's' | b'S')) {
            i += 1;
        }
        match bytes.get(i) {
            Some(&b @ (b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H')) => {
                Some((b.to_ascii_lowercase(), i + 1))
            }
            _ => None,
        }
    }

    /// The `'` is under the cursor and a base letter follows: lexes the base
    /// and its digits. `start` is where the whole literal began.
    fn lex_based(&mut self, start: usize) -> TokenKind {
        let (base, after) = self.base_at(self.pos + 1).expect("caller checked base");
        self.pos = after;
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
        let digits_start = self.pos;
        self.eat_while(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'?');
        let digits = &self.src[digits_start..self.pos];
        let base_name = match base {
            b'b' => "binary",
            b'o' => "octal",
            b'd' => "decimal",
            _ => "hexadecimal",
        };
        if digits.is_empty() {
            self.pos = digits_start;
            // Leave any blanks out of the token.
            self.pos = self.src[..self.pos].trim_end_matches([' ', '\t']).len();
            self.error(
                start,
                self.pos,
                format!("{base_name} literal has no digits"),
            );
            return TokenKind::Error {
                text: self.text(start),
            };
        }
        let bad = digits.chars().find(|&c| !digit_ok(base, c));
        if let Some(c) = bad {
            self.error(
                start,
                self.pos,
                format!("invalid digit `{c}` in {base_name} literal"),
            );
            return TokenKind::Error {
                text: self.text(start),
            };
        }
        if base == b'd' {
            // Decimal allows either digits or a single x/z/? (with
            // underscores after it), not a mix.
            let cleaned: String = digits.chars().filter(|&c| c != '_').collect();
            let has_xz = cleaned
                .chars()
                .any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'));
            if has_xz && cleaned.len() != 1 {
                self.error(
                    start,
                    self.pos,
                    "decimal literal may be a single `x` or `z`, not mixed with digits",
                );
                return TokenKind::Error {
                    text: self.text(start),
                };
            }
        }
        TokenKind::Number {
            text: self.text(start),
        }
    }

    /// A `'` at the cursor: `'{`, an unsized based literal (`'hff`), an
    /// unsized single-bit literal (`'0`, `'1`, `'x`, `'z`) or the cast
    /// apostrophe.
    fn lex_apostrophe(&mut self, start: usize) -> TokenKind {
        if self.peek_at(1) == Some(b'{') {
            self.pos += 2;
            return TokenKind::Punct(Punct::ApostropheBrace);
        }
        if self.base_at(start + 1).is_some() {
            return self.lex_based(start);
        }
        if let Some(b'0' | b'1' | b'x' | b'X' | b'z' | b'Z') = self.peek_at(1)
            && !self.peek_at(2).is_some_and(is_ident_char)
        {
            self.pos += 2;
            return TokenKind::Number {
                text: self.text(start),
            };
        }
        self.pos += 1;
        TokenKind::Punct(Punct::Apostrophe)
    }

    /// A string literal; the value has escapes decoded.
    fn lex_string(&mut self, start: usize) -> TokenKind {
        self.pos += 1;
        let mut value = String::new();
        loop {
            let Some(b) = self.peek() else {
                self.error(start, self.pos, "unterminated string literal");
                break;
            };
            match b {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\n' => {
                    self.error(start, self.pos, "unterminated string literal");
                    break;
                }
                b'\\' => {
                    let esc_start = self.pos;
                    self.pos += 1;
                    match self.peek() {
                        Some(b'n') => value.push('\n'),
                        Some(b't') => value.push('\t'),
                        Some(b'\\') => value.push('\\'),
                        Some(b'"') => value.push('"'),
                        Some(b'v') => value.push('\x0b'),
                        Some(b'f') => value.push('\x0c'),
                        Some(b'a') => value.push('\x07'),
                        Some(b'\n') => {
                            // Line continuation: nothing is added.
                        }
                        Some(b'\r') if self.peek_at(1) == Some(b'\n') => {
                            self.pos += 1;
                        }
                        Some(b'0'..=b'7') => {
                            let mut code = 0u32;
                            let mut n = 0;
                            while n < 3 && matches!(self.peek(), Some(b'0'..=b'7')) {
                                code = code * 8 + u32::from(self.peek().expect("checked") - b'0');
                                self.pos += 1;
                                n += 1;
                            }
                            // Back up one: the loop below advances past the
                            // last consumed byte.
                            self.pos -= 1;
                            match char::from_u32(code) {
                                Some(c) if code <= 0xff => value.push(c),
                                _ => {
                                    self.error(
                                        esc_start,
                                        self.pos + 1,
                                        "octal escape exceeds \\377",
                                    );
                                }
                            }
                        }
                        Some(b'x') => {
                            let hex_start = self.pos + 1;
                            let mut end = hex_start;
                            while end < self.src.len()
                                && end - hex_start < 2
                                && self.src.as_bytes()[end].is_ascii_hexdigit()
                            {
                                end += 1;
                            }
                            if end == hex_start {
                                self.error(esc_start, end, "`\\x` escape needs hex digits");
                            } else {
                                let code = u32::from_str_radix(&self.src[hex_start..end], 16)
                                    .expect("hex digits");
                                value.push(char::from_u32(code).expect("byte value is a char"));
                                self.pos = end - 1;
                            }
                        }
                        Some(_) => {
                            let c = self.src[self.pos..].chars().next().expect("in bounds");
                            self.error(
                                esc_start,
                                self.pos + c.len_utf8(),
                                format!("unknown escape sequence `\\{c}`"),
                            );
                            value.push(c);
                            self.pos += c.len_utf8() - 1;
                        }
                        None => {
                            self.error(start, self.pos, "unterminated string literal");
                            break;
                        }
                    }
                    self.pos += 1;
                }
                _ => {
                    let c = self.src[self.pos..].chars().next().expect("in bounds");
                    value.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        TokenKind::Str { value }
    }

    /// A backtick directive that survived preprocessing. Keyword-set
    /// directives are applied here and yield no token.
    fn lex_directive(&mut self, start: usize) -> Option<TokenKind> {
        self.pos += 1;
        self.eat_while(is_ident_char);
        let name = self.src[start + 1..self.pos].to_string();
        if name.is_empty() {
            self.error(start, self.pos, "stray backtick");
            return Some(TokenKind::Error {
                text: self.text(start),
            });
        }
        match name.as_str() {
            "begin_keywords" => {
                self.skip_trivia();
                let spec_start = self.pos;
                if self.peek() == Some(b'"') {
                    if let TokenKind::Str { value } = self.lex_string(spec_start) {
                        match Dialect::for_keywords_spec(&value) {
                            Some(d) => {
                                self.dialect_stack.push(self.dialect);
                                self.dialect = d;
                            }
                            None => self.error(
                                spec_start,
                                self.pos,
                                format!("unknown keyword set `{value}`"),
                            ),
                        }
                    }
                } else {
                    self.error(
                        start,
                        self.pos,
                        "expected a quoted version after `` `begin_keywords ``",
                    );
                }
                None
            }
            "end_keywords" => {
                match self.dialect_stack.pop() {
                    Some(d) => self.dialect = d,
                    None => self.error(
                        start,
                        self.pos,
                        "`` `end_keywords `` without `` `begin_keywords ``",
                    ),
                }
                None
            }
            _ => Some(TokenKind::Directive { name }),
        }
    }

    /// Punctuation, longest match first. `None` when no operator starts
    /// here.
    fn lex_punct(&mut self) -> Option<Punct> {
        // `(*` followed by optional blanks and `)` is `( * )`, the
        // wildcard sensitivity list, not an attribute.
        if self.wildcard_star {
            self.wildcard_star = false;
            if self.peek() == Some(b'*') {
                self.pos += 1;
                return Some(Punct::Star);
            }
        }
        if self.starts_with("(*") {
            let rest = &self.src[self.pos + 2..];
            let after = rest.trim_start_matches([' ', '\t']);
            if after.starts_with(')') {
                self.pos += 1;
                self.wildcard_star = true;
                return Some(Punct::LParen);
            }
        }
        for &(text, p) in PUNCT_TABLE {
            if self.starts_with(text) {
                self.pos += text.len();
                return Some(p);
            }
        }
        None
    }
}

/// Punctuation ordered so that any prefix of a longer operator comes
/// after it; the lexer takes the first match.
const PUNCT_TABLE: &[(&str, Punct)] = &[
    ("<<<=", Punct::AshlEq),
    (">>>=", Punct::AshrEq),
    ("<<<", Punct::Ashl),
    (">>>", Punct::Ashr),
    ("<<=", Punct::ShlEq),
    (">>=", Punct::ShrEq),
    ("===", Punct::CaseEq),
    ("!==", Punct::CaseNe),
    ("==?", Punct::WildEq),
    ("!=?", Punct::WildNe),
    ("&&&", Punct::AndAndAnd),
    ("<->", Punct::Equiv),
    ("->>", Punct::ArrowArrow),
    ("|->", Punct::ImplyOverlap),
    ("|=>", Punct::ImplyNext),
    ("#-#", Punct::HashMinusHash),
    ("#=#", Punct::HashEqHash),
    ("<<", Punct::Shl),
    (">>", Punct::Shr),
    ("==", Punct::EqEq),
    ("!=", Punct::BangEq),
    ("<=", Punct::Le),
    (">=", Punct::Ge),
    ("&&", Punct::AndAnd),
    ("||", Punct::OrOr),
    ("->", Punct::Arrow),
    ("=>", Punct::EqGt),
    ("*>", Punct::StarGt),
    ("::", Punct::ColonColon),
    (":=", Punct::ColonEq),
    (":/", Punct::ColonSlash),
    ("++", Punct::PlusPlus),
    ("--", Punct::MinusMinus),
    ("+=", Punct::PlusEq),
    ("-=", Punct::MinusEq),
    ("*=", Punct::StarEq),
    ("/=", Punct::SlashEq),
    ("%=", Punct::PercentEq),
    ("&=", Punct::AmpEq),
    ("|=", Punct::PipeEq),
    ("^=", Punct::CaretEq),
    ("**", Punct::StarStar),
    ("~&", Punct::TildeAmp),
    ("~|", Punct::TildePipe),
    ("~^", Punct::TildeCaret),
    ("^~", Punct::CaretTilde),
    ("##", Punct::HashHash),
    ("+:", Punct::PlusColon),
    ("-:", Punct::MinusColon),
    (".*", Punct::DotStar),
    ("(*", Punct::AttrOpen),
    ("*)", Punct::AttrClose),
    ("@@", Punct::AtAt),
    ("(", Punct::LParen),
    (")", Punct::RParen),
    ("[", Punct::LBracket),
    ("]", Punct::RBracket),
    ("{", Punct::LBrace),
    ("}", Punct::RBrace),
    (";", Punct::Semi),
    (",", Punct::Comma),
    (".", Punct::Dot),
    (":", Punct::Colon),
    ("?", Punct::Question),
    ("!", Punct::Bang),
    ("~", Punct::Tilde),
    ("&", Punct::Amp),
    ("|", Punct::Pipe),
    ("^", Punct::Caret),
    ("+", Punct::Plus),
    ("-", Punct::Minus),
    ("*", Punct::Star),
    ("/", Punct::Slash),
    ("%", Punct::Percent),
    ("<", Punct::Lt),
    (">", Punct::Gt),
    ("=", Punct::Eq),
    ("#", Punct::Hash),
    ("@", Punct::At),
];

/// True for the characters that may continue an identifier.
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// True when `c` is a legal digit for `base` (`b`, `o`, `d` or `h`).
fn digit_ok(base: u8, c: char) -> bool {
    if matches!(c, '_' | 'x' | 'X' | 'z' | 'Z' | '?') {
        return true;
    }
    match base {
        b'b' => matches!(c, '0' | '1'),
        b'o' => matches!(c, '0'..='7'),
        b'd' => c.is_ascii_digit(),
        _ => c.is_ascii_hexdigit(),
    }
}

/// Lexes `src` without preprocessing, reporting spans as offsets into it.
pub fn lex(src: &str, file: SourceId, dialect: Dialect, diags: &mut Diagnostics) -> Lexed {
    Lexer::new(src, file, dialect, diags).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn lex_all(src: &str, dialect: Dialect) -> (Vec<Token>, Diagnostics, SourceId) {
        let mut map = SourceMap::new();
        let id = map.add("t.v", src).unwrap();
        let mut diags = Diagnostics::new();
        let lexed = lex(src, id, dialect, &mut diags);
        (lexed.tokens, diags, id)
    }

    fn kinds(src: &str) -> Vec<String> {
        let (tokens, diags, _) = lex_all(src, Dialect::SystemVerilog);
        assert!(
            diags.is_empty(),
            "{:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        tokens
            .iter()
            .map(|t| format!("{}:{}", t.kind.kind_name(), t.kind.text()))
            .collect()
    }

    #[test]
    fn identifiers_keywords_and_dialects() {
        assert_eq!(
            kinds("module logic \\a+b \\logic $display $ a$b _x"),
            [
                "keyword:module",
                "keyword:logic",
                "escaped:a+b",
                "escaped:logic",
                "system:display",
                "punct:$",
                "ident:a$b",
                "ident:_x",
                "eof:"
            ]
        );
        let (tokens, _, _) = lex_all("logic", Dialect::Verilog2005);
        assert_eq!(
            tokens[0].kind,
            TokenKind::Ident {
                name: "logic".into()
            }
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds(
                "42 1_000 8'hff 4'b10xz 'h1 12'sd5 8 'h ff 1.5 1e3 2.5E-2 10ns 1.5us 1step 'x '1 'Z"
            ),
            [
                "number:42",
                "number:1_000",
                "number:8'hff",
                "number:4'b10xz",
                "number:'h1",
                "number:12'sd5",
                "number:8 'h ff",
                "number:1.5",
                "number:1e3",
                "number:2.5E-2",
                "number:10ns",
                "number:1.5us",
                "number:1step",
                "number:'x",
                "number:'1",
                "number:'Z",
                "eof:"
            ]
        );
        // Things that look like time units but are identifiers.
        assert_eq!(
            kinds("1sec 1.e"),
            [
                "number:1",
                "ident:sec",
                "number:1",
                "punct:.",
                "ident:e",
                "eof:"
            ]
        );
    }

    #[test]
    fn bad_numbers() {
        let (tokens, diags, _) = lex_all("4'b102 3'd1x 8'h", Dialect::SystemVerilog);
        assert_eq!(tokens.len(), 4);
        assert!(
            tokens[..3]
                .iter()
                .all(|t| matches!(t.kind, TokenKind::Error { .. }))
        );
        let msgs: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "invalid digit `2` in binary literal",
                "decimal literal may be a single `x` or `z`, not mixed with digits",
                "hexadecimal literal has no digits"
            ]
        );
        assert_eq!(tokens[2].kind.text(), "8'h");
    }

    #[test]
    fn apostrophe_forms() {
        assert_eq!(
            kinds("int'(x) '{1,2} a'"),
            [
                "keyword:int",
                "punct:'",
                "punct:(",
                "ident:x",
                "punct:)",
                "punct:'{",
                "number:1",
                "punct:,",
                "number:2",
                "punct:}",
                "ident:a",
                "punct:'",
                "eof:"
            ]
        );
    }

    #[test]
    fn strings_and_escapes() {
        let (tokens, diags, _) = lex_all(
            "\"a\\tb\\n\\\\\\\"\\101\\x41\\\nc\" \"ü\"",
            Dialect::SystemVerilog,
        );
        assert!(diags.is_empty());
        assert_eq!(
            tokens[0].kind,
            TokenKind::Str {
                value: "a\tb\n\\\"AAc".into()
            }
        );
        assert_eq!(tokens[1].kind, TokenKind::Str { value: "ü".into() });
        assert_eq!(tokens[1].span.start, 24);
        assert_eq!(tokens[1].span.end, 28);
    }

    #[test]
    fn string_errors() {
        let (tokens, diags, _) = lex_all("\"abc\nx \"\\q\" \"end", Dialect::SystemVerilog);
        let msgs: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "unterminated string literal",
                "unknown escape sequence `\\q`",
                "unterminated string literal"
            ]
        );
        assert_eq!(
            tokens[0].kind,
            TokenKind::Str {
                value: "abc".into()
            }
        );
        assert_eq!(tokens[1].kind, TokenKind::Ident { name: "x".into() });
        assert_eq!(tokens[2].kind, TokenKind::Str { value: "q".into() });
        assert_eq!(
            tokens[3].kind,
            TokenKind::Str {
                value: "end".into()
            }
        );
    }

    #[test]
    fn operators_longest_match() {
        assert_eq!(
            kinds("a<<<=b>>>c===d!==e==?f<->g->>h|->i|=>j::k++l--m+=n##o(*p*)q@(*)r@(* )s.*t"),
            [
                "ident:a",
                "punct:<<<=",
                "ident:b",
                "punct:>>>",
                "ident:c",
                "punct:===",
                "ident:d",
                "punct:!==",
                "ident:e",
                "punct:==?",
                "ident:f",
                "punct:<->",
                "ident:g",
                "punct:->>",
                "ident:h",
                "punct:|->",
                "ident:i",
                "punct:|=>",
                "ident:j",
                "punct:::",
                "ident:k",
                "punct:++",
                "ident:l",
                "punct:--",
                "ident:m",
                "punct:+=",
                "ident:n",
                "punct:##",
                "ident:o",
                "punct:(*",
                "ident:p",
                "punct:*)",
                "ident:q",
                "punct:@",
                "punct:(",
                "punct:*",
                "punct:)",
                "ident:r",
                "punct:@",
                "punct:(",
                "punct:*",
                "punct:)",
                "ident:s",
                "punct:.*",
                "ident:t",
                "eof:"
            ]
        );
    }

    #[test]
    fn comments_are_recorded_with_spans() {
        let src = "a // ünï\n/* b\nc */ d /* open";
        let mut map = SourceMap::new();
        let id = map.add("t.v", src).unwrap();
        let mut diags = Diagnostics::new();
        let lexed = lex(src, id, Dialect::SystemVerilog, &mut diags);
        assert_eq!(lexed.comments.len(), 3);
        assert_eq!(lexed.comments[0].1, CommentKind::Line);
        assert_eq!(lexed.comments[1].1, CommentKind::Block);
        let (s, _) = lexed.comments[1];
        assert_eq!(&src[s.start as usize..s.end as usize], "/* b\nc */");
        assert_eq!(
            diags.iter().next().unwrap().message,
            "unterminated block comment"
        );
        assert_eq!(lexed.tokens.len(), 3);
        assert_eq!(lexed.tokens[1].kind, TokenKind::Ident { name: "d".into() });
    }

    #[test]
    fn directives_and_keyword_sets() {
        assert_eq!(
            kinds("`default_nettype none `begin_keywords \"1364-2005\" logic `end_keywords logic"),
            [
                "directive:default_nettype",
                "ident:none",
                "ident:logic",
                "keyword:logic",
                "eof:"
            ]
        );
        let (_, diags, _) = lex_all(
            "`end_keywords `begin_keywords \"x\" `begin_keywords 1",
            Dialect::SystemVerilog,
        );
        let msgs: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "`` `end_keywords `` without `` `begin_keywords ``",
                "unknown keyword set `x`",
                "expected a quoted version after `` `begin_keywords ``"
            ]
        );
    }

    #[test]
    fn unexpected_characters_recover() {
        let (tokens, diags, _) = lex_all("a § b \\ c", Dialect::SystemVerilog);
        let k: Vec<_> = tokens.iter().map(|t| t.kind.kind_name()).collect();
        assert_eq!(k, ["ident", "error", "ident", "error", "ident", "eof"]);
        assert_eq!(diags.len(), 2);
        assert_eq!(tokens[1].span.end - tokens[1].span.start, 2);
    }

    #[test]
    fn spans_are_exact() {
        let (tokens, _, id) = lex_all("  wire\tx;", Dialect::Verilog2005);
        assert_eq!(tokens[0].span, Span::new(id, 2, 6));
        assert_eq!(tokens[1].span, Span::new(id, 7, 8));
        assert_eq!(tokens[2].span, Span::new(id, 8, 9));
        assert_eq!(tokens[3].span, Span::new(id, 9, 9));
    }
}
