//! The VHDL lexer: source text to a flat token list.
//!
//! [`lex_source`] turns one file of a [`SourceMap`] into a `Vec<Token>`
//! ending in [`TokenKind::Eof`]. The lexer never stops early: every problem
//! becomes a diagnostic plus the best token it can make (an unterminated
//! string still yields a `StringLit`, an unknown character yields an
//! [`TokenKind::Error`] token) so that the parser sees one stream and can
//! report its own errors in the right places.
//!
//! # Design
//!
//! - **Bytes for structure, chars for letters.** Every delimiter, digit and
//!   quote is ASCII, so lookahead for them is byte-based; UTF-8 continuation
//!   bytes are all `>= 0x80` and can never be mistaken for one. Identifiers
//!   are scanned by `char` because VHDL letters include the Latin-1 range
//!   (`À`..`ÿ`, minus `×` and `÷`), which is two bytes in UTF-8. Non-Latin-1
//!   text is accepted inside comments and string literals so offsets stay
//!   correct for the rest of the file, and is an error anywhere else.
//! - **Literals stay raw.** Numeric and bit-string literal tokens carry their
//!   source text; only well-formedness is checked here (base in `2..=16`,
//!   digits valid for the base, underscores between digits). Their value
//!   depends on types resolved later.
//! - **Comments go to a side table**, `Vec<(Span, CommentKind)>`, in source
//!   order, so a formatter or documentation extractor can find them without
//!   the parser having to skip them. VHDL-2008 tool directives (`` `protect
//!   ... ``) are recorded there too, and reported as a note since the
//!   compiler ignores them.
//! - **The standard only gates reserved words** (see [`Standard`]); 2008
//!   syntax in VHDL-93 mode is lexed and reported with one diagnostic.
//!
//! # The apostrophe
//!
//! `'` starts a character literal (`'a'`) or is the attribute / qualified
//! expression tick (`x'event`, `t'(v)`), and `t'('a')` contains both. The
//! rule, the same one GHDL and nvc apply:
//!
//! 1. If the apostrophe is followed by exactly one character and then
//!    another apostrophe, it is a character literal, **except** when that
//!    middle character is `(` and the previous token is an identifier
//!    (basic or extended), `)`, `]`, the reserved word `all`, or a string,
//!    bit-string or character literal. In that position the prefix is a
//!    name and `'(` must open a qualified expression.
//! 2. Otherwise it is a tick.
//!
//! So `'0'` and `('0','1')` are character literals; `sig'event`, `x'(1)`,
//! `t'image(v)` are ticks; and `std_logic'('1')` is tick, `(`, character
//! literal `'1'`, `)`.

use std::borrow::Cow;

use super::token::{Standard, Token, TokenKind};
use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, SourceMap, Span};

/// Which comment syntax produced a side-table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentKind {
    /// `-- ...` to the end of the line (the span excludes the line break).
    Line,
    /// `/* ... */`, VHDL-2008; may span lines.
    Delimited,
    /// `` `directive ... `` to the end of the line, VHDL-2008 (clause 15.11).
    ToolDirective,
}

/// Everything the lexer produces for one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lexed<'src> {
    /// The tokens, ending in [`TokenKind::Eof`].
    pub tokens: Vec<Token<'src>>,
    /// Comments and tool directives, in source order.
    pub comments: Vec<(Span, CommentKind)>,
}

/// Lexes one file of `map` under `standard`, reporting into `diags`.
///
/// Comments are dropped; use [`Lexer`] directly to keep them.
pub fn lex_source<'a>(
    map: &'a SourceMap,
    id: SourceId,
    standard: Standard,
    diags: &mut Diagnostics,
) -> Vec<Token<'a>> {
    Lexer::new(map.file(id).text(), id, standard)
        .lex(diags)
        .tokens
}

/// Narrows a byte offset to the `u32` used in spans.
///
/// Offsets are bounded by the length of a file that [`SourceMap::add`]
/// accepted, so the conversion cannot fail; the check is an assertion.
fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("source offset exceeds u32")
}

/// A separator: space, format effectors, line breaks and the non-breaking
/// space (allowed by VHDL-2008 clause 15.3).
fn is_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{A0}')
}

/// A VHDL letter: ASCII or Latin-1 upper and lower case letters (clause
/// 15.2), which excludes `×` (U+00D7) and `÷` (U+00F7).
fn is_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
        || matches!(c, '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{FF}')
}

/// A character that may continue a basic identifier.
fn is_ident_char(c: char) -> bool {
    is_letter(c) || c.is_ascii_digit() || c == '_'
}

/// A Latin-1 graphic character: printable ASCII or `U+00A0..=U+00FF`.
fn is_graphic(c: char) -> bool {
    matches!(c, ' '..='~' | '\u{A0}'..='\u{FF}')
}

/// A line break; the lexer treats `\r` alone as one too.
fn is_line_break(b: u8) -> bool {
    b == b'\n' || b == b'\r'
}

/// Value of a based-literal digit, or `None` when `b` is not one.
fn digit_value(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some(u32::from(b - b'0')),
        b'a'..=b'f' => Some(u32::from(b - b'a') + 10),
        b'A'..=b'F' => Some(u32::from(b - b'A') + 10),
        _ => None,
    }
}

/// Length of a bit-string base specifier at the start of `rest`, when one
/// is immediately followed by a string delimiter: `b"`, `UX"`, `d%`...
fn bit_string_prefix(rest: &[u8]) -> Option<usize> {
    let is_base = |b: u8| matches!(b.to_ascii_lowercase(), b'b' | b'o' | b'x' | b'd');
    let is_quote = |b: u8| b == b'"' || b == b'%';
    match rest {
        [b, q, ..] if is_base(*b) && is_quote(*q) => Some(1),
        [s, b, q, ..]
            if matches!(s.to_ascii_lowercase(), b'u' | b's') && is_base(*b) && is_quote(*q) =>
        {
            Some(2)
        }
        _ => None,
    }
}

/// Renders a character for a diagnostic: quoted when printable, else as
/// `U+XXXX`.
fn describe_char(c: char) -> String {
    if c.is_control() || c.is_whitespace() {
        format!("U+{:04X}", u32::from(c))
    } else {
        format!("`{c}`")
    }
}

/// The lexer state for one file. See the module docs for the design.
#[derive(Debug)]
pub struct Lexer<'src> {
    text: &'src str,
    file: SourceId,
    standard: Standard,
    /// Byte offset of the next unread character.
    pos: usize,
    tokens: Vec<Token<'src>>,
    comments: Vec<(Span, CommentKind)>,
    diags: Diagnostics,
}

impl<'src> Lexer<'src> {
    /// Creates a lexer over `text`, which is the content of file `file`.
    pub fn new(text: &'src str, file: SourceId, standard: Standard) -> Self {
        Lexer {
            text,
            file,
            standard,
            pos: 0,
            tokens: Vec::new(),
            comments: Vec::new(),
            diags: Diagnostics::new(),
        }
    }

    /// Runs the lexer to the end of the file.
    ///
    /// Diagnostics are appended to `diags` in source order.
    pub fn lex(mut self, diags: &mut Diagnostics) -> Lexed<'src> {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                c if is_whitespace(c) => self.pos += c.len_utf8(),
                '-' if self.byte(1) == Some(b'-') => self.line_comment(start),
                '/' if self.byte(1) == Some(b'*') => self.delimited_comment(start),
                '`' => self.tool_directive(start),
                '"' | '%' => self.string(start, c),
                '\\' => self.extended_ident(start),
                '\'' => self.tick_or_char(start),
                '0'..='9' => self.number(start),
                c if is_letter(c) => self.ident(start),
                c => self.delimiter(start, c),
            }
        }
        let end = self.text.len();
        self.push(TokenKind::Eof, end, end);
        diags.append(&mut self.diags);
        Lexed {
            tokens: self.tokens,
            comments: self.comments,
        }
    }

    // --- cursor helpers -------------------------------------------------

    /// The next unread character.
    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    /// The byte `n` positions ahead of the cursor.
    fn byte(&self, n: usize) -> Option<u8> {
        self.text.as_bytes().get(self.pos + n).copied()
    }

    /// True at end of input or at a line break.
    fn at_line_end(&self) -> bool {
        self.byte(0).is_none_or(is_line_break)
    }

    /// Advances to the end of the current line, not consuming the break.
    fn skip_to_line_end(&mut self) {
        while !self.at_line_end() {
            self.pos += 1;
        }
    }

    fn slice(&self, start: usize, end: usize) -> &'src str {
        &self.text[start..end]
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.file, offset(start), offset(end))
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        let span = self.span(start, end);
        self.tokens
            .push(Token::new(kind, span, self.slice(start, end)));
    }

    fn push_with_text(&mut self, kind: TokenKind, start: usize, end: usize, text: Cow<'src, str>) {
        let span = self.span(start, end);
        self.tokens.push(Token { kind, span, text });
    }

    fn error(&mut self, start: usize, end: usize, message: impl Into<String>) {
        let span = self.span(start, end);
        self.diags.error(span, message);
    }

    // --- comments and directives ---------------------------------------

    fn line_comment(&mut self, start: usize) {
        self.skip_to_line_end();
        let span = self.span(start, self.pos);
        self.comments.push((span, CommentKind::Line));
    }

    fn delimited_comment(&mut self, start: usize) {
        if self.standard == Standard::Vhdl93 {
            let span = self.span(start, start + 2);
            self.diags.push(
                Diagnostic::warning("delimited comments require VHDL-2008")
                    .with_label(span, "`/* ... */` comment in VHDL-93 mode"),
            );
        }
        self.pos += 2;
        match self.text[self.pos..].find("*/") {
            Some(rel) => self.pos += rel + 2,
            None => {
                self.pos = self.text.len();
                let span = self.span(start, start + 2);
                self.diags.push(
                    Diagnostic::error("unterminated delimited comment")
                        .with_label(span, "comment opened here")
                        .with_note("expected `*/` before the end of the file"),
                );
            }
        }
        let span = self.span(start, self.pos);
        self.comments.push((span, CommentKind::Delimited));
    }

    fn tool_directive(&mut self, start: usize) {
        self.pos += 1;
        let name_start = self.pos;
        while self.peek().is_some_and(is_ident_char) {
            self.pos += 1;
        }
        let name = self.slice(name_start, self.pos).to_owned();
        self.skip_to_line_end();
        let span = self.span(start, self.pos);
        self.comments.push((span, CommentKind::ToolDirective));
        let message = if name.is_empty() {
            "tool directive ignored".to_owned()
        } else {
            format!("tool directive `{name}` ignored")
        };
        self.diags.push(
            Diagnostic::note(message)
                .with_label(span, "")
                .with_note("reticle does not act on tool directives"),
        );
    }

    // --- string-like tokens -------------------------------------------

    /// Scans a string literal delimited by `delim` (`"` or `%`), unescaping
    /// doubled delimiters. Stops with an error at the end of the line.
    fn string(&mut self, start: usize, delim: char) {
        self.pos += 1;
        let inner_start = self.pos;
        let text = self.scan_quoted(inner_start, delim, "string literal");
        self.push_with_text(TokenKind::StringLit, start, self.pos, text);
    }

    /// Scans an extended identifier `\...\`, unescaping `\\`.
    fn extended_ident(&mut self, start: usize) {
        self.pos += 1;
        let inner_start = self.pos;
        let text = self.scan_quoted(inner_start, '\\', "extended identifier");
        if text.is_empty() {
            self.error(start, self.pos, "extended identifier is empty");
        }
        self.push_with_text(TokenKind::ExtendedIdent, start, self.pos, text);
    }

    /// Scans up to and including the closing `delim`, treating a doubled
    /// delimiter as one literal delimiter character. Returns the unescaped
    /// content, borrowed when no doubling occurred.
    fn scan_quoted(&mut self, inner_start: usize, delim: char, what: &str) -> Cow<'src, str> {
        let delim_byte = u8::try_from(delim).expect("delimiters are ASCII");
        let mut owned: Option<String> = None;
        loop {
            match self.byte(0) {
                Some(b) if b == delim_byte => {
                    if self.byte(1) == Some(delim_byte) {
                        let buf = owned
                            .get_or_insert_with(|| self.text[inner_start..self.pos].to_owned());
                        buf.push(delim);
                        self.pos += 2;
                    } else {
                        let content = self.pos;
                        self.pos += 1;
                        return match owned {
                            Some(s) => Cow::Owned(s),
                            None => Cow::Borrowed(self.slice(inner_start, content)),
                        };
                    }
                }
                Some(b) if is_line_break(b) => break,
                Some(_) => {
                    // Any character other than the delimiter and a line
                    // break is accepted, including non-Latin-1 text, so a
                    // stray emoji in a report string cannot derail lexing.
                    let c = self.peek().expect("byte present");
                    if let Some(buf) = owned.as_mut() {
                        buf.push(c);
                    }
                    self.pos += c.len_utf8();
                }
                None => break,
            }
        }
        let span_start = inner_start - 1;
        self.diags.push(
            Diagnostic::error(format!("unterminated {what}"))
                .with_label(
                    self.span(span_start, self.pos),
                    format!("expected `{delim}`"),
                )
                .with_note(format!("{what}s cannot span lines")),
        );
        match owned {
            Some(s) => Cow::Owned(s),
            None => Cow::Borrowed(self.slice(inner_start, self.pos)),
        }
    }

    /// Applies the apostrophe rule from the module docs.
    fn tick_or_char(&mut self, start: usize) {
        let after_name = matches!(
            self.tokens.last().map(|t| t.kind),
            Some(
                TokenKind::Ident
                    | TokenKind::ExtendedIdent
                    | TokenKind::RParen
                    | TokenKind::RBracket
                    | TokenKind::All
                    | TokenKind::StringLit
                    | TokenKind::BitStringLit
                    | TokenKind::CharLit
            )
        );
        let middle = self.text[start + 1..].chars().next();
        if let Some(c) = middle
            && c != '\n'
            && c != '\r'
            && self.text.as_bytes().get(start + 1 + c.len_utf8()) == Some(&b'\'')
            && !(after_name && c == '(')
        {
            let end = start + 2 + c.len_utf8();
            if !is_graphic(c) {
                self.error(
                    start,
                    end,
                    format!(
                        "character literal {} is not a Latin-1 graphic character",
                        describe_char(c)
                    ),
                );
            }
            self.pos = end;
            let text = self.slice(start + 1, end - 1);
            self.push_with_text(TokenKind::CharLit, start, end, Cow::Borrowed(text));
        } else {
            self.pos = start + 1;
            self.push(TokenKind::Tick, start, self.pos);
        }
    }

    // --- identifiers ----------------------------------------------------

    fn ident(&mut self, start: usize) {
        while let Some(c) = self.peek() {
            if !is_ident_char(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        let text = self.slice(start, self.pos);
        if text.contains("__") {
            self.error(
                start,
                self.pos,
                "identifier contains consecutive underscores",
            );
        } else if text.ends_with('_') {
            self.error(start, self.pos, "identifier ends with an underscore");
        }
        if let Some(len) = bit_string_prefix(&self.text.as_bytes()[start..])
            && len == text.len()
        {
            self.bit_string(start, start);
            return;
        }
        let kind = TokenKind::classify_ident(text, self.standard);
        self.push(kind, start, self.pos);
    }

    // --- numbers and bit strings ---------------------------------------

    /// Scans a run of digits (per `is_digit`) with single underscores
    /// between them, reporting misplaced underscores once. Returns true when
    /// at least one digit was consumed.
    fn scan_digits(&mut self, is_digit: fn(u8) -> bool) -> bool {
        let start = self.pos;
        let mut any = false;
        let mut after_underscore = false;
        let mut reported = false;
        loop {
            match self.byte(0) {
                Some(b) if is_digit(b) => {
                    any = true;
                    after_underscore = false;
                }
                Some(b'_') => {
                    if (!any || after_underscore) && !reported {
                        reported = true;
                        self.error(
                            self.pos,
                            self.pos + 1,
                            "underscore must separate two digits",
                        );
                    }
                    after_underscore = true;
                }
                _ => break,
            }
            self.pos += 1;
        }
        if after_underscore && !reported {
            self.error(start, self.pos, "literal ends with an underscore");
        }
        any
    }

    /// Scans an optional exponent `E[+-]digits`. Returns whether one was
    /// present and whether it was negative.
    fn scan_exponent(&mut self) -> (bool, bool) {
        if !matches!(self.byte(0), Some(b'e' | b'E')) {
            return (false, false);
        }
        let (sign_len, negative) = match self.byte(1) {
            Some(b'+') => (1, false),
            Some(b'-') => (1, true),
            _ => (0, false),
        };
        if !self.byte(1 + sign_len).is_some_and(|b| b.is_ascii_digit()) {
            return (false, false);
        }
        self.pos += 1 + sign_len;
        self.scan_digits(|b| b.is_ascii_digit());
        (true, negative)
    }

    fn number(&mut self, start: usize) {
        self.scan_digits(|b| b.is_ascii_digit());
        match self.byte(0) {
            Some(b'#') => self.based(start),
            Some(b'.') if self.byte(1).is_some_and(|b| b.is_ascii_digit()) => {
                self.pos += 1;
                self.scan_digits(|b| b.is_ascii_digit());
                self.scan_exponent();
                self.push(TokenKind::Real, start, self.pos);
            }
            _ => {
                if let Some(len) = bit_string_prefix(&self.text.as_bytes()[self.pos..]) {
                    let spec = self.pos;
                    self.pos += len;
                    self.bit_string(start, spec);
                    return;
                }
                let (_, negative) = self.scan_exponent();
                if negative {
                    self.error(
                        start,
                        self.pos,
                        "integer literal cannot have a negative exponent",
                    );
                }
                self.push(TokenKind::Integer, start, self.pos);
            }
        }
    }

    /// Scans the rest of a based literal; the cursor is on the first `#`
    /// and `start..pos` holds the base.
    fn based(&mut self, start: usize) {
        let base = self
            .slice(start, self.pos)
            .bytes()
            .filter(u8::is_ascii_digit)
            .fold(0u32, |acc, b| {
                acc.saturating_mul(10).saturating_add(u32::from(b - b'0'))
            });
        if !(2..=16).contains(&base) {
            self.error(
                start,
                self.pos,
                format!("base {base} is not in the range 2 to 16"),
            );
        }
        self.pos += 1;
        let mut real = false;
        let digits_start = self.pos;
        let is_based_digit = |b: u8| digit_value(b).is_some();
        if !self.scan_digits(is_based_digit) {
            self.error(digits_start, self.pos, "based literal has no digits");
        }
        if self.byte(0) == Some(b'.') {
            real = true;
            self.pos += 1;
            let frac_start = self.pos;
            if !self.scan_digits(is_based_digit) {
                self.error(
                    frac_start,
                    self.pos,
                    "based literal has no digits after the point",
                );
            }
        }
        // Report the first digit that does not fit the base.
        if (2..=16).contains(&base) {
            let bad = self.text.as_bytes()[digits_start..self.pos]
                .iter()
                .position(|&b| digit_value(b).is_some_and(|v| v >= base));
            if let Some(i) = bad {
                let at = digits_start + i;
                let d = self.text.as_bytes()[at] as char;
                self.error(
                    at,
                    at + 1,
                    format!("digit `{d}` is not valid in base {base}"),
                );
            }
        }
        if self.byte(0) == Some(b'#') {
            self.pos += 1;
        } else {
            self.diags.push(
                Diagnostic::error("unterminated based literal")
                    .with_label(self.span(start, self.pos), "expected a closing `#`"),
            );
            let kind = if real {
                TokenKind::Real
            } else {
                TokenKind::Integer
            };
            self.push(kind, start, self.pos);
            return;
        }
        let (_, negative) = self.scan_exponent();
        if negative && !real {
            self.error(
                start,
                self.pos,
                "integer literal cannot have a negative exponent",
            );
        }
        let kind = if real {
            TokenKind::Real
        } else {
            TokenKind::Integer
        };
        self.push(kind, start, self.pos);
    }

    /// Scans the quoted part of a bit-string literal. `start` is the start
    /// of the whole literal (length prefix included), `spec` the start of
    /// the base specifier, and the cursor is on the opening delimiter.
    fn bit_string(&mut self, start: usize, spec: usize) {
        let delim = self.byte(0).expect("cursor is on the delimiter");
        debug_assert!(delim == b'"' || delim == b'%');
        let spec_text = self.slice(spec, self.pos);
        if self.standard == Standard::Vhdl93 {
            let extended = spec_text.len() == 2 || spec_text.eq_ignore_ascii_case("d");
            if extended {
                self.error(
                    spec,
                    self.pos,
                    format!("bit-string base specifier `{spec_text}` requires VHDL-2008"),
                );
            } else if spec != start {
                self.error(
                    start,
                    spec,
                    "bit-string literal length prefix requires VHDL-2008",
                );
            }
        }
        self.pos += 1;
        let terminated = loop {
            match self.byte(0) {
                Some(b) if b == delim => {
                    self.pos += 1;
                    break true;
                }
                Some(b) if is_line_break(b) => break false,
                Some(_) => self.pos += 1,
                None => break false,
            }
        };
        if !terminated {
            let delim = delim as char;
            self.diags.push(
                Diagnostic::error("unterminated bit-string literal")
                    .with_label(self.span(start, self.pos), format!("expected `{delim}`")),
            );
        }
        self.push(TokenKind::BitStringLit, start, self.pos);
    }

    // --- delimiters -----------------------------------------------------

    fn delimiter(&mut self, start: usize, c: char) {
        use TokenKind::*;
        let (kind, len) = match (c, self.byte(1), self.byte(2)) {
            ('?', Some(b'/'), Some(b'=')) => (QNeq, 3),
            ('?', Some(b'<'), Some(b'=')) => (QLe, 3),
            ('?', Some(b'>'), Some(b'=')) => (QGe, 3),
            ('=', Some(b'>'), _) => (Arrow, 2),
            ('*', Some(b'*'), _) => (StarStar, 2),
            (':', Some(b'='), _) => (ColonEq, 2),
            ('/', Some(b'='), _) => (Neq, 2),
            ('>', Some(b'='), _) => (Ge, 2),
            ('<', Some(b'='), _) => (Le, 2),
            ('<', Some(b'>'), _) => (Box, 2),
            ('<', Some(b'<'), _) => (LtLt, 2),
            ('>', Some(b'>'), _) => (GtGt, 2),
            ('?', Some(b'?'), _) => (QQ, 2),
            ('?', Some(b'='), _) => (QEq, 2),
            ('?', Some(b'<'), _) => (QLt, 2),
            ('?', Some(b'>'), _) => (QGt, 2),
            ('&', ..) => (Amp, 1),
            ('(', ..) => (LParen, 1),
            (')', ..) => (RParen, 1),
            ('*', ..) => (Star, 1),
            ('+', ..) => (Plus, 1),
            (',', ..) => (Comma, 1),
            ('-', ..) => (Minus, 1),
            ('.', ..) => (Dot, 1),
            ('/', ..) => (Slash, 1),
            (':', ..) => (Colon, 1),
            (';', ..) => (Semi, 1),
            ('<', ..) => (Lt, 1),
            ('=', ..) => (Eq, 1),
            ('>', ..) => (Gt, 1),
            ('?', ..) => (Question, 1),
            ('@', ..) => (At, 1),
            ('[', ..) => (LBracket, 1),
            (']', ..) => (RBracket, 1),
            ('|' | '!', ..) => (Bar, 1),
            ('^', ..) => (Caret, 1),
            _ => {
                let end = start + c.len_utf8();
                self.error(
                    start,
                    end,
                    format!("illegal character {}", describe_char(c)),
                );
                self.pos = end;
                self.push(Error, start, end);
                return;
            }
        };
        self.pos = start + len;
        self.push(kind, start, self.pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(text: &str) -> (Lexed<'_>, Diagnostics) {
        lex_std(text, Standard::Vhdl2008)
    }

    fn lex_std(text: &str, standard: Standard) -> (Lexed<'_>, Diagnostics) {
        let mut diags = Diagnostics::new();
        let lexed = Lexer::new(text, file_id(), standard).lex(&mut diags);
        (lexed, diags)
    }

    /// A file id for tests that never render diagnostics.
    fn file_id() -> SourceId {
        SourceMap::new().add("t.vhd", "").unwrap()
    }

    fn kinds(text: &str) -> Vec<TokenKind> {
        lex(text).0.tokens.into_iter().map(|t| t.kind).collect()
    }

    fn texts(text: &str) -> Vec<String> {
        lex(text)
            .0
            .tokens
            .into_iter()
            .map(|t| t.text.into_owned())
            .collect()
    }

    #[test]
    fn tick_disambiguation() {
        use TokenKind::*;
        assert_eq!(kinds("x'(1)"), [Ident, Tick, LParen, Integer, RParen, Eof]);
        assert_eq!(
            kinds("t'image(v)"),
            [Ident, Tick, Ident, LParen, Ident, RParen, Eof]
        );
        assert_eq!(kinds("'0'"), [CharLit, Eof]);
        assert_eq!(
            kinds("('0','1')"),
            [LParen, CharLit, Comma, CharLit, RParen, Eof]
        );
        assert_eq!(kinds("sig'event"), [Ident, Tick, Ident, Eof]);
        assert_eq!(
            kinds("std_logic'('1')"),
            [Ident, Tick, LParen, CharLit, RParen, Eof]
        );
        assert_eq!(
            kinds("\\ext\\'('a')"),
            [ExtendedIdent, Tick, LParen, CharLit, RParen, Eof]
        );
        assert_eq!(
            kinds("x := '('; y := ')';"),
            [
                Ident, ColonEq, CharLit, Semi, Ident, ColonEq, CharLit, Semi, Eof
            ]
        );
        assert_eq!(
            kinds("a(1)'length"),
            [Ident, LParen, Integer, RParen, Tick, Ident, Eof]
        );
        assert_eq!(kinds("''"), [Tick, Tick, Eof]);
    }

    #[test]
    fn strings_unescape_and_keep_offsets() {
        let (lexed, diags) = lex("\"a\"\"b\" %c%%d% \"é🙂\" \"x");
        assert!(diags.has_errors(), "unterminated string is an error");
        let t = &lexed.tokens;
        assert_eq!(t[0].text, "a\"b");
        assert_eq!(t[1].text, "c%d");
        assert_eq!(t[2].text, "é🙂");
        assert_eq!(t[3].text, "x");
        assert_eq!(t[3].kind, TokenKind::StringLit);
        // The span of the last string starts where the source says it does.
        assert_eq!(t[3].span.start as usize, "\"a\"\"b\" %c%%d% \"é🙂\" ".len());
        assert!(matches!(t[0].text, Cow::Owned(_)));
        assert!(matches!(t[2].text, Cow::Borrowed(_)));
    }

    #[test]
    fn extended_identifiers() {
        let (lexed, diags) = lex("\\foo bar\\ \\a\\\\b\\ \\\\ \\open");
        let t = &lexed.tokens;
        assert_eq!(t[0].text, "foo bar");
        assert_eq!(t[1].text, "a\\b");
        assert_eq!(t[2].text, "");
        assert_eq!(t[3].text, "open");
        assert_eq!(t[3].kind, TokenKind::ExtendedIdent);
        assert_eq!(diags.error_count(), 2, "empty and unterminated");
    }

    #[test]
    fn numbers() {
        use TokenKind::*;
        assert_eq!(
            kinds("42 1_000 1E3 2#1010# 16#FF#E2"),
            [Integer, Integer, Integer, Integer, Integer, Eof]
        );
        assert_eq!(
            kinds("3.14 1.0e-3 16#F.8# 2#1.1#e-1"),
            [Real, Real, Real, Real, Eof]
        );
        assert_eq!(texts("16#FF#E2 3.14"), ["16#FF#E2", "3.14", ""]);
        // `1.` followed by a non-digit is an integer and a dot.
        assert_eq!(kinds("1.x"), [Integer, Dot, Ident, Eof]);
        // Physical literals are two tokens; the parser joins them.
        assert_eq!(kinds("10 ns"), [Integer, Ident, Eof]);
        // An `e` not followed by digits is not an exponent.
        assert_eq!(kinds("1 else"), [Integer, Else, Eof]);
    }

    #[test]
    fn number_errors() {
        let (_, d) = lex("1E-3");
        assert_eq!(d.error_count(), 1, "negative exponent on integer");
        let (_, d) = lex("17#0#");
        assert_eq!(d.error_count(), 1, "base out of range");
        let (_, d) = lex("8#9#");
        assert_eq!(d.error_count(), 1, "digit out of range");
        let (l, d) = lex("16#FF");
        assert_eq!(d.error_count(), 1, "unterminated based literal");
        assert_eq!(l.tokens[0].kind, TokenKind::Integer);
        let (_, d) = lex("1__0 1_ 16#_F#");
        assert_eq!(d.error_count(), 3);
        let (l, d) = lex("2#1010#");
        assert!(d.is_empty());
        assert_eq!(l.tokens[0].text, "2#1010#");
    }

    #[test]
    fn bit_strings() {
        use TokenKind::*;
        let (l, d) = lex("x\"FF\" B\"1_0\" sb\"1010\" 8ux\"F_F\" d\"255\" 12X%A%");
        assert!(d.is_empty(), "{}", d.render(&SourceMap::new()));
        let k: Vec<_> = l.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            k,
            [
                BitStringLit,
                BitStringLit,
                BitStringLit,
                BitStringLit,
                BitStringLit,
                BitStringLit,
                Eof
            ]
        );
        assert_eq!(l.tokens[3].text, "8ux\"F_F\"");
        assert_eq!(l.tokens[5].text, "12X%A%");
        // Not bit strings: an identifier that merely starts with `x`, and a
        // specifier separated from its quote.
        assert_eq!(
            kinds("xy\"a\" x \"a\""),
            [Ident, StringLit, Ident, StringLit, Eof]
        );
    }

    #[test]
    fn bit_strings_in_vhdl93() {
        let (l, d) = lex_std("x\"FF\" ux\"F\" 8x\"F\" d\"9\"", Standard::Vhdl93);
        assert_eq!(d.error_count(), 3);
        assert!(
            l.tokens[..4]
                .iter()
                .all(|t| t.kind == TokenKind::BitStringLit)
        );
    }

    #[test]
    fn comments_go_to_side_table() {
        let src = "a -- é comment\n/* multi\nline */ b `protect begin\nc";
        let (l, d) = lex(src);
        let kinds: Vec<_> = l.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                TokenKind::Ident,
                TokenKind::Ident,
                TokenKind::Ident,
                TokenKind::Eof
            ]
        );
        let comments: Vec<_> = l
            .comments
            .iter()
            .map(|(s, k)| (&src[s.start as usize..s.end as usize], *k))
            .collect();
        assert_eq!(
            comments,
            [
                ("-- é comment", CommentKind::Line),
                ("/* multi\nline */", CommentKind::Delimited),
                ("`protect begin", CommentKind::ToolDirective),
            ]
        );
        assert_eq!(d.len(), 1, "one note for the tool directive");
        assert!(!d.has_errors());
        // Offsets after the multi-byte comment are still right.
        let b = &l.tokens[1];
        assert_eq!(&src[b.span.start as usize..b.span.end as usize], "b");
    }

    #[test]
    fn delimited_comment_errors() {
        let (l, d) = lex("a /* never closed");
        assert_eq!(d.error_count(), 1);
        assert_eq!(l.tokens.len(), 2);
        let (_, d) = lex_std("/* x */", Standard::Vhdl93);
        assert_eq!(d.warning_count(), 1);
        assert!(!d.has_errors());
    }

    #[test]
    fn identifiers_and_reserved_words() {
        use TokenKind::*;
        assert_eq!(
            kinds("ENTITY Entity entity_ies"),
            [Entity, Entity, Ident, Eof]
        );
        assert_eq!(texts("ENTITY"), ["ENTITY", ""]);
        assert_eq!(kinds("señal Ärger"), [Ident, Ident, Eof]);
        let (_, d) = lex("a__b c_ d");
        assert_eq!(d.error_count(), 2);
        // `×` is not a letter.
        let (l, d) = lex("a×b");
        assert_eq!(d.error_count(), 1);
        assert_eq!(l.tokens[1].kind, Error);
        assert_eq!(l.tokens[1].text, "×");
        assert_eq!(l.tokens[2].text, "b");
    }

    #[test]
    fn standard_gates_reserved_words() {
        use TokenKind::*;
        let (l, _) = lex_std("context default parameter", Standard::Vhdl93);
        assert!(l.tokens[..3].iter().all(|t| t.kind == Ident));
        let (l, _) = lex_std("context default parameter", Standard::Vhdl2008);
        let k: Vec<_> = l.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(k, [Context, Default, Parameter, Eof]);
    }

    #[test]
    fn delimiters() {
        use TokenKind::*;
        assert_eq!(
            kinds("=> ** := /= >= <= <> ?? ?= ?/= ?< ?<= ?> ?>= << >> ^ @ ! | ? [ ]"),
            [
                Arrow, StarStar, ColonEq, Neq, Ge, Le, Box, QQ, QEq, QNeq, QLt, QLe, QGt, QGe,
                LtLt, GtGt, Caret, At, Bar, Bar, Question, LBracket, RBracket, Eof
            ]
        );
        assert_eq!(kinds("a<=b"), [Ident, Le, Ident, Eof]);
        assert_eq!(kinds("a<<b"), [Ident, LtLt, Ident, Eof]);
        assert_eq!(kinds("a-b--c"), [Ident, Minus, Ident, Eof]);
        assert_eq!(kinds("a/b/*c*/d"), [Ident, Slash, Ident, Ident, Eof]);
    }

    #[test]
    fn illegal_characters_and_eof() {
        let (l, d) = lex("a $ b\u{1}");
        assert_eq!(d.error_count(), 2);
        assert_eq!(l.tokens[1].kind, TokenKind::Error);
        assert_eq!(l.tokens[3].kind, TokenKind::Error);
        let eof = l.tokens.last().unwrap();
        assert_eq!(eof.kind, TokenKind::Eof);
        assert!(eof.span.is_empty());
        assert_eq!(eof.span.start as usize, "a $ b\u{1}".len());
        let (l, d) = lex("");
        assert!(d.is_empty());
        assert_eq!(l.tokens.len(), 1);
    }

    #[test]
    fn lex_source_uses_the_map() {
        let mut map = SourceMap::new();
        let id = map.add("t.vhd", "entity e is end;").unwrap();
        let mut diags = Diagnostics::new();
        let tokens = lex_source(&map, id, Standard::Vhdl2008, &mut diags);
        assert!(diags.is_empty());
        assert_eq!(tokens.len(), 6);
        assert!(tokens.iter().all(|t| t.span.file == id));
    }

    #[test]
    fn crlf_line_endings() {
        let (l, d) = lex("a -- c\r\n\"s\r\nb");
        assert_eq!(d.error_count(), 1, "string stops at CR");
        assert_eq!(l.tokens[1].text, "s");
        assert_eq!(l.tokens[2].text, "b");
    }
}
