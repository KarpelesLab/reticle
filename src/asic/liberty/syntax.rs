//! The generic Liberty grammar: lexer, values, attributes and groups.
//!
//! Liberty is one grammar used for hundreds of differently named
//! constructs, so the reader is split in two: this module understands only
//! the shape
//!
//! ```text
//! group_name (arg, arg) {
//!     simple_attribute : value ;
//!     complex_attribute (value, value) ;
//!     nested_group (arg) { ... }
//! }
//! ```
//!
//! and produces a [`Group`] tree, and the typed view in the parent module
//! picks the constructs it knows out of that tree. Unknown attributes and
//! groups stay in the tree and are never an error, which is what keeps the
//! reader usable across PDKs that each add their own extensions.
//!
//! Lexical details handled here: `/* */` and `//` comments, `\` line
//! continuations (both between tokens and inside strings, where SKY130 and
//! Nangate split long table rows), numbers with unit suffixes (`1ns`,
//! `0.01pf`), and the optional `;` after a simple attribute.

use std::fmt;

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, Span};

/// The value of a Liberty attribute or group argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A plain number: `3.75`, `1e-3`.
    Number(f64),
    /// A number followed by a unit suffix in one token: `1ns`, `0.01pf`.
    Quantity(f64, String),
    /// A double-quoted string, with line continuations already removed.
    String(String),
    /// A bare identifier: `input`, `combinational`, `tt_025C_1v80`.
    Ident(String),
    /// `true` or `false`.
    Bool(bool),
    /// A multi-token unquoted value, kept verbatim: `VDD * 0.5`.
    Expr(String),
    /// The arguments of a complex attribute: `values("...", "...")`.
    List(Vec<Value>),
}

impl Value {
    /// The numeric value of a [`Value::Number`] or [`Value::Quantity`], or
    /// of a string/identifier that parses as a number (`area : "3.75"` is
    /// seen in the wild).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(v) | Value::Quantity(v, _) => Some(*v),
            Value::String(s) | Value::Ident(s) | Value::Expr(s) => s.trim().parse().ok(),
            Value::Bool(_) | Value::List(_) => None,
        }
    }

    /// The text of a string, identifier or expression value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) | Value::Ident(s) | Value::Expr(s) => Some(s),
            _ => None,
        }
    }

    /// A boolean, accepting `true`/`false` as identifiers or strings too.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::String(s) | Value::Ident(s) => match s.as_str() {
                "true" | "TRUE" => Some(true),
                "false" | "FALSE" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// The elements of a [`Value::List`], or the value itself as a
    /// one-element list.
    pub fn as_list(&self) -> Vec<&Value> {
        match self {
            Value::List(items) => items.iter().collect(),
            other => vec![other],
        }
    }

    /// The numbers in a table-style string (`"0.1, 0.2, 0.3"`) or list of
    /// such strings, flattened.
    pub fn numbers(&self) -> Vec<f64> {
        let mut out = Vec::new();
        self.collect_numbers(&mut out);
        out
    }

    fn collect_numbers(&self, out: &mut Vec<f64>) {
        match self {
            Value::Number(v) | Value::Quantity(v, _) => out.push(*v),
            Value::String(s) | Value::Ident(s) | Value::Expr(s) => {
                for part in s.split(|c: char| c == ',' || c.is_whitespace()) {
                    if !part.is_empty()
                        && let Ok(v) = part.parse::<f64>()
                    {
                        out.push(v);
                    }
                }
            }
            Value::Bool(_) => {}
            Value::List(items) => {
                for item in items {
                    item.collect_numbers(out);
                }
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Number(v) => write!(f, "{}", super::super::fmt_num(*v)),
            Value::Quantity(v, unit) => write!(f, "{}{unit}", super::super::fmt_num(*v)),
            Value::String(s) => write!(f, "\"{}\"", s.replace('"', "\\\"")),
            Value::Ident(s) | Value::Expr(s) => f.write_str(s),
            Value::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            Value::List(items) => {
                f.write_str("(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// One attribute of a group. Simple attributes (`name : value ;`) carry
/// their value directly; complex attributes (`name (a, b) ;`) carry a
/// [`Value::List`] and have `complex` set.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    /// The attribute name.
    pub name: String,
    /// The value (a list for complex attributes).
    pub value: Value,
    /// True for the `name (args) ;` form.
    pub complex: bool,
    /// Where the attribute was written.
    pub span: Span,
}

/// A group: `name (args) { attributes and nested groups }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The group type: `library`, `cell`, `pin`, `timing`, ...
    pub name: String,
    /// The arguments in the parentheses; usually the object's name.
    pub args: Vec<Value>,
    /// Attributes in source order.
    pub attrs: Vec<Attribute>,
    /// Nested groups in source order.
    pub groups: Vec<Group>,
    /// Where the group header was written.
    pub span: Span,
}

impl Group {
    /// The first argument as text, which is the group's own name for
    /// `cell`, `pin`, `bus`, `lu_table_template` and most others.
    pub fn arg_name(&self) -> Option<&str> {
        self.args.first().and_then(Value::as_str)
    }

    /// The first attribute with this name.
    pub fn attr(&self, name: &str) -> Option<&Value> {
        self.attrs.iter().find(|a| a.name == name).map(|a| &a.value)
    }

    /// The first attribute with this name, with its span.
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attrs.iter().find(|a| a.name == name)
    }

    /// Every attribute with this name, in order.
    pub fn attrs_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Attribute> + 'a {
        self.attrs.iter().filter(move |a| a.name == name)
    }

    /// A numeric attribute.
    pub fn attr_f64(&self, name: &str) -> Option<f64> {
        self.attr(name).and_then(Value::as_f64)
    }

    /// A string or identifier attribute.
    pub fn attr_str(&self, name: &str) -> Option<&str> {
        self.attr(name).and_then(Value::as_str)
    }

    /// A boolean attribute.
    pub fn attr_bool(&self, name: &str) -> Option<bool> {
        self.attr(name).and_then(Value::as_bool)
    }

    /// The first nested group with this type.
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// Every nested group with this type, in order.
    pub fn groups_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Group> + 'a {
        self.groups.iter().filter(move |g| g.name == name)
    }

    /// Renders the group tree back to Liberty syntax.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        out.push_str(&indent);
        out.push_str(&self.name);
        out.push_str(" (");
        for (i, arg) in self.args.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&arg.to_string());
        }
        out.push_str(") {\n");
        for attr in &self.attrs {
            out.push_str(&indent);
            out.push_str("  ");
            out.push_str(&attr.name);
            if attr.complex {
                match &attr.value {
                    Value::List(_) => out.push_str(&attr.value.to_string()),
                    other => {
                        out.push('(');
                        out.push_str(&other.to_string());
                        out.push(')');
                    }
                }
            } else {
                out.push_str(" : ");
                out.push_str(&attr.value.to_string());
            }
            out.push_str(";\n");
        }
        for group in &self.groups {
            group.write(out, depth + 1);
        }
        out.push_str(&indent);
        out.push_str("}\n");
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Word,
    Str,
    LBrace,
    RBrace,
    LParen,
    RParen,
    Colon,
    Semi,
    Comma,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    text: String,
    span: Span,
    /// True when this token is the first on its (logical) line, which is
    /// how a missing `;` after a simple attribute is tolerated.
    line_start: bool,
}

fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("source offset exceeds u32")
}

fn lex(text: &str, file: SourceId, diags: &mut Diagnostics) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'\\' {
            // Line continuation: `\` followed by optional whitespace and a
            // newline. A stray backslash elsewhere is just skipped.
            i += 1;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let start = i;
            i += 2;
            loop {
                if i + 1 >= bytes.len() {
                    diags.push(
                        Diagnostic::error("unterminated block comment").with_span(Span::new(
                            file,
                            offset(start),
                            offset(start + 2),
                        )),
                    );
                    i = bytes.len();
                    break;
                }
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        let (kind, end, tok_text) = match c {
            b'{' | b'}' | b'(' | b')' | b':' | b';' | b',' => {
                let kind = match c {
                    b'{' => Kind::LBrace,
                    b'}' => Kind::RBrace,
                    b'(' => Kind::LParen,
                    b')' => Kind::RParen,
                    b':' => Kind::Colon,
                    b';' => Kind::Semi,
                    _ => Kind::Comma,
                };
                (kind, start + 1, text[start..start + 1].to_string())
            }
            b'"' => {
                // Find the closing quote, skipping over escaped characters
                // (including `\`-newline continuations), then decode the
                // body in one place.
                i += 1;
                let content_start = i;
                let mut terminated = false;
                while i < bytes.len() {
                    match bytes[i] {
                        b'"' => {
                            terminated = true;
                            break;
                        }
                        b'\\' => i += 2,
                        _ => i += 1,
                    }
                }
                let mut content_end = i.min(bytes.len());
                while !text.is_char_boundary(content_end) {
                    content_end -= 1;
                }
                let content = unescape_continuations(&text[content_start..content_end]);
                if terminated {
                    i += 1;
                } else {
                    diags.push(
                        Diagnostic::error("unterminated string").with_span(Span::new(
                            file,
                            offset(start),
                            offset(start + 1),
                        )),
                    );
                    i = bytes.len();
                }
                (Kind::Str, i, content)
            }
            _ => {
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(
                        bytes[i],
                        b'{' | b'}' | b'(' | b')' | b':' | b';' | b',' | b'"' | b'\\'
                    )
                {
                    i += 1;
                }
                (Kind::Word, i, text[start..i].to_string())
            }
        };
        i = end;
        out.push(Token {
            kind,
            text: tok_text,
            span: Span::new(file, offset(start), offset(end)),
            line_start,
        });
        line_start = false;
    }
    out
}

/// Removes `\`-newline continuations from a string body (the non-ASCII
/// path of the lexer).
fn unescape_continuations(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(' ' | '\t' | '\r')) {
                lookahead.next();
            }
            if lookahead.peek() == Some(&'\n') {
                lookahead.next();
                while matches!(lookahead.peek(), Some(' ' | '\t')) {
                    lookahead.next();
                }
                chars = lookahead;
                s.push(' ');
                continue;
            }
            if let Some(n) = chars.next() {
                s.push(n);
            }
            continue;
        }
        s.push(c);
    }
    s
}

/// Classifies a bare word as a number, quantity, boolean or identifier.
pub(super) fn classify_word(word: &str) -> Value {
    match word {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(v) = word.parse::<f64>() {
        return Value::Number(v);
    }
    // A number followed by letters: `1ns`, `0.01pf`, `1.8V`.
    let split = word
        .char_indices()
        .find(|(i, c)| {
            c.is_ascii_alphabetic()
                && !(*i > 0 && (*c == 'e' || *c == 'E') && exp_follows(word, *i))
        })
        .map(|(i, _)| i);
    if let Some(at) = split
        && at > 0
        && let Ok(v) = word[..at].parse::<f64>()
    {
        return Value::Quantity(v, word[at..].to_string());
    }
    Value::Ident(word.to_string())
}

/// True when the `e` at `at` starts an exponent (`1e-3`, `2E5`).
fn exp_follows(word: &str, at: usize) -> bool {
    let rest = &word[at + 1..];
    let rest = rest.strip_prefix(['+', '-']).unwrap_or(rest);
    rest.chars().next().is_some_and(|c| c.is_ascii_digit())
        && rest.chars().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Resolves `include_file(name)` to the text of the included file and the
/// [`SourceId`] it was registered under. Returning `None` reports an error
/// at the include site.
pub type IncludeResolver<'a> = dyn FnMut(&str) -> Option<(String, SourceId)> + 'a;

struct Parser<'d, 'r> {
    tokens: Vec<Token>,
    pos: usize,
    eof: Span,
    diags: &'d mut Diagnostics,
    resolver: Option<&'r mut IncludeResolver<'r>>,
    depth: u32,
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<Kind> {
        self.peek().map(|t| t.kind)
    }

    fn span(&self) -> Span {
        self.peek().map_or(self.eof, |t| t.span)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, kind: Kind) -> bool {
        if self.peek_kind() == Some(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(message).with_span(span));
    }

    /// Skips to the end of the current statement: past the next `;`, or
    /// over a balanced `{ }` block, or up to a `}` (not consumed).
    fn recover(&mut self) {
        while let Some(t) = self.peek() {
            match t.kind {
                Kind::Semi => {
                    self.pos += 1;
                    return;
                }
                Kind::RBrace => return,
                Kind::LBrace => {
                    self.skip_block();
                    return;
                }
                _ => self.pos += 1,
            }
        }
    }

    fn skip_block(&mut self) {
        let mut depth = 0usize;
        while let Some(t) = self.bump() {
            match t.kind {
                Kind::LBrace => depth += 1,
                Kind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Parses the file: a sequence of groups at the top level.
    fn file(&mut self) -> Vec<Group> {
        let mut groups = Vec::new();
        while let Some(t) = self.peek() {
            if t.kind != Kind::Word {
                let span = t.span;
                let text = t.text.clone();
                self.error(span, format!("expected a group name, found `{text}`"));
                self.pos += 1;
                continue;
            }
            match self.item() {
                Some(Item::Group(g)) => groups.push(g),
                Some(Item::Attr(a)) => {
                    self.error(
                        a.span,
                        "expected a group at the top level, found an attribute",
                    );
                }
                Some(Item::Include(_)) | None => {}
            }
        }
        groups
    }

    /// The body of a group, up to and including its `}`.
    fn body(&mut self, group: &mut Group) {
        loop {
            match self.peek_kind() {
                None => {
                    let span = self.eof;
                    self.error(span, format!("unterminated group `{}`", group.name));
                    return;
                }
                Some(Kind::RBrace) => {
                    self.pos += 1;
                    return;
                }
                Some(Kind::Semi) => {
                    // Stray semicolon; harmless.
                    self.pos += 1;
                }
                Some(Kind::Word) => match self.item() {
                    Some(Item::Group(g)) => group.groups.push(g),
                    Some(Item::Attr(a)) => group.attrs.push(a),
                    Some(Item::Include(items)) => {
                        for item in items {
                            match item {
                                Item::Group(g) => group.groups.push(g),
                                Item::Attr(a) => group.attrs.push(a),
                                Item::Include(_) => {}
                            }
                        }
                    }
                    None => {}
                },
                Some(_) => {
                    let span = self.span();
                    let text = self.peek().map(|t| t.text.clone()).unwrap_or_default();
                    self.error(span, format!("unexpected `{text}`"));
                    self.recover();
                }
            }
        }
    }

    /// One attribute or group. The leading word has been checked to be a
    /// `Word`; returns `None` after reporting an error.
    fn item(&mut self) -> Option<Item> {
        let head = self.bump()?;
        match self.peek_kind() {
            Some(Kind::Colon) => {
                self.pos += 1;
                let value = self.simple_value(head.span)?;
                let end = self.tokens[self.pos - 1].span;
                self.eat(Kind::Semi);
                Some(Item::Attr(Attribute {
                    name: head.text,
                    value,
                    complex: false,
                    span: head.span.to(end),
                }))
            }
            Some(Kind::LParen) => {
                self.pos += 1;
                let args = self.args();
                if !self.eat(Kind::RParen) {
                    let span = self.span();
                    self.error(span, format!("expected `)` to close `{}(`", head.text));
                    self.recover();
                    return None;
                }
                let end = self.tokens[self.pos - 1].span;
                if self.eat(Kind::LBrace) {
                    if self.depth > 64 {
                        self.error(head.span, "groups nested too deeply");
                        self.skip_block();
                        return None;
                    }
                    let mut group = Group {
                        name: head.text,
                        args,
                        attrs: Vec::new(),
                        groups: Vec::new(),
                        span: head.span.to(end),
                    };
                    self.depth += 1;
                    self.body(&mut group);
                    self.depth -= 1;
                    return Some(Item::Group(group));
                }
                self.eat(Kind::Semi);
                if head.text == "include_file" {
                    return Some(self.include(&args, head.span.to(end)));
                }
                Some(Item::Attr(Attribute {
                    name: head.text,
                    value: Value::List(args),
                    complex: true,
                    span: head.span.to(end),
                }))
            }
            _ => {
                let span = self.span();
                self.error(span, format!("expected `:` or `(` after `{}`", head.text));
                self.recover();
                None
            }
        }
    }

    fn include(&mut self, args: &[Value], span: Span) -> Item {
        let Some(name) = args.first().and_then(Value::as_str) else {
            self.error(span, "include_file needs a file name");
            return Item::Include(Vec::new());
        };
        let resolved = self.resolver.as_mut().and_then(|r| r(name));
        match resolved {
            Some((text, file)) => {
                if self.depth > 16 {
                    self.error(span, "include_file nested too deeply");
                    return Item::Include(Vec::new());
                }
                let tokens = lex(&text, file, self.diags);
                let end = offset(text.len());
                let mut sub = Parser {
                    tokens,
                    pos: 0,
                    eof: Span::new(file, end, end),
                    diags: self.diags,
                    resolver: None,
                    depth: self.depth + 1,
                };
                let mut items = Vec::new();
                while let Some(t) = sub.peek() {
                    match t.kind {
                        Kind::Word => {
                            if let Some(item) = sub.item() {
                                items.push(item);
                            }
                        }
                        Kind::Semi => sub.pos += 1,
                        _ => {
                            let span = t.span;
                            let text = t.text.clone();
                            sub.error(span, format!("unexpected `{text}` in included file"));
                            sub.recover();
                        }
                    }
                }
                Item::Include(items)
            }
            None => {
                self.error(span, format!("cannot resolve include_file `{name}`"));
                Item::Include(Vec::new())
            }
        }
    }

    /// The value of a simple attribute: tokens up to `;`, `}` or the next
    /// line when the `;` is missing.
    fn simple_value(&mut self, head: Span) -> Option<Value> {
        let mut parts: Vec<Value> = Vec::new();
        let mut first = true;
        // The pattern copies the token's small fields so the borrow of
        // `self` ends before the body advances the cursor; the text is
        // taken from the token only on the paths that keep it.
        while let Some(&Token {
            kind, line_start, ..
        }) = self.peek()
        {
            match kind {
                Kind::Semi | Kind::RBrace | Kind::LBrace => break,
                _ if !first && line_start => break,
                Kind::Str => {
                    let s = self.tokens[self.pos].text.clone();
                    self.pos += 1;
                    parts.push(Value::String(s));
                }
                Kind::Word => {
                    let v = classify_word(&self.tokens[self.pos].text);
                    self.pos += 1;
                    parts.push(v);
                }
                Kind::LParen | Kind::RParen | Kind::Colon | Kind::Comma => {
                    let text = self.tokens[self.pos].text.clone();
                    self.pos += 1;
                    parts.push(Value::Ident(text));
                }
            }
            first = false;
        }
        match parts.len() {
            0 => {
                self.error(head, "attribute has no value");
                self.recover();
                None
            }
            1 => parts.pop(),
            _ => {
                let mut text = String::new();
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        text.push(' ');
                    }
                    match p {
                        Value::String(s) => text.push_str(s),
                        other => text.push_str(&other.to_string()),
                    }
                }
                Some(Value::Expr(text))
            }
        }
    }

    /// Comma-separated (commas optional) values up to `)`.
    ///
    /// A `:` glued between two words (`pin (D[7:0])`) is part of the name:
    /// the lexer splits it because it cannot know it is not an attribute
    /// separator, and this is where the pieces are joined back.
    fn args(&mut self) -> Vec<Value> {
        let mut args = Vec::new();
        let mut last_end: Option<u32> = None;
        while let Some(&Token { kind, span, .. }) = self.peek() {
            let (start, end) = (span.start, span.end);
            match kind {
                Kind::RParen | Kind::LBrace | Kind::Semi | Kind::RBrace => break,
                Kind::Comma => self.pos += 1,
                Kind::Str => {
                    let s = self.tokens[self.pos].text.clone();
                    self.pos += 1;
                    args.push(Value::String(s));
                }
                Kind::Word => {
                    let v = classify_word(&self.tokens[self.pos].text);
                    self.pos += 1;
                    args.push(v);
                }
                Kind::Colon => {
                    self.pos += 1;
                    let glued_before = last_end == Some(start);
                    let next = self.peek().cloned();
                    match (args.last_mut(), next) {
                        (Some(Value::Ident(prev)), Some(n))
                            if glued_before && n.kind == Kind::Word && n.span.start == end =>
                        {
                            prev.push(':');
                            prev.push_str(&n.text);
                            self.pos += 1;
                            last_end = Some(n.span.end);
                            continue;
                        }
                        _ => args.push(Value::Ident(":".into())),
                    }
                }
                Kind::LParen => {
                    let text = self.tokens[self.pos].text.clone();
                    self.pos += 1;
                    args.push(Value::Ident(text));
                }
            }
            last_end = Some(end);
        }
        args
    }
}

enum Item {
    Group(Group),
    Attr(Attribute),
    Include(Vec<Item>),
}

/// Parses Liberty text into its top-level groups (normally exactly one
/// `library`). Syntax errors are reported to `diags` and skipped, so the
/// returned tree is the best-effort content.
pub fn parse_groups(text: &str, file: SourceId, diags: &mut Diagnostics) -> Vec<Group> {
    parse_groups_with_includes(text, file, diags, None)
}

/// Like [`parse_groups`], resolving `include_file(...)` through
/// `resolver`. Without a resolver every include is reported as an error.
pub fn parse_groups_with_includes<'r>(
    text: &str,
    file: SourceId,
    diags: &mut Diagnostics,
    resolver: Option<&'r mut IncludeResolver<'r>>,
) -> Vec<Group> {
    let tokens = lex(text, file, diags);
    let end = offset(text.len());
    let mut parser = Parser {
        tokens,
        pos: 0,
        eof: Span::new(file, end, end),
        diags,
        resolver,
        depth: 0,
    };
    parser.file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn parse(text: &str) -> (Vec<Group>, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let id = map.add("t.lib", text).unwrap();
        let mut diags = Diagnostics::new();
        let groups = parse_groups(text, id, &mut diags);
        (groups, diags, map)
    }

    #[test]
    fn parses_attributes_groups_and_comments() {
        let text = r#"
            /* header */
            library (demo) {
              time_unit : "1ns"; // trailing
              capacitive_load_unit (1.0, pf);
              nom_voltage : 1.80;
              define (foo, cell, string);
              cell (inv) {
                area : 3.75;
                dont_use : true;
                pin (A) { direction : input; capacitance : 0.001pf; }
              }
            }
        "#;
        let (groups, diags, map) = parse(text);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        assert_eq!(groups.len(), 1);
        let lib = &groups[0];
        assert_eq!(lib.name, "library");
        assert_eq!(lib.arg_name(), Some("demo"));
        assert_eq!(lib.attr_str("time_unit"), Some("1ns"));
        assert_eq!(
            lib.attr("capacitive_load_unit"),
            Some(&Value::List(vec![
                Value::Number(1.0),
                Value::Ident("pf".into())
            ]))
        );
        assert_eq!(lib.attr_f64("nom_voltage"), Some(1.8));
        let cell = lib.group("cell").unwrap();
        assert_eq!(cell.attr_bool("dont_use"), Some(true));
        let pin = cell.group("pin").unwrap();
        assert_eq!(
            pin.attr("capacitance"),
            Some(&Value::Quantity(0.001, "pf".into()))
        );
    }

    #[test]
    fn multi_line_strings_and_continuations() {
        let text = "library (x) {\n  values ( \"0.1, 0.2\", \\\n    \"0.3, 0.4\" );\n  idx (\"1, 2, \\\n 3\");\n  long : a \\\n b;\n}\n";
        let (groups, diags, map) = parse(text);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        let g = &groups[0];
        assert_eq!(
            g.attr("values").unwrap().numbers(),
            vec![0.1, 0.2, 0.3, 0.4]
        );
        assert_eq!(g.attr("idx").unwrap().numbers(), vec![1.0, 2.0, 3.0]);
        assert_eq!(g.attr("long"), Some(&Value::Expr("a b".into())));
    }

    #[test]
    fn colons_inside_group_arguments_are_names() {
        let text = "library (x) {\n  pin (D[7:0]) { a : 1; }\n  pin(Q[3 : 0]) { }\n}\n";
        let (groups, diags, map) = parse(text);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        assert_eq!(groups[0].groups[0].arg_name(), Some("D[7:0]"));
        assert_eq!(groups[0].groups[0].attr_f64("a"), Some(1.0));
        assert_eq!(
            groups[0].groups[1].args,
            vec![
                Value::Ident("Q[3".into()),
                Value::Ident(":".into()),
                Value::Ident("0]".into())
            ]
        );
    }

    #[test]
    fn missing_semicolon_is_tolerated_by_line() {
        let text = "library (x) {\n  a : 1\n  b : two\n  c : 3;\n}\n";
        let (groups, diags, map) = parse(text);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        let g = &groups[0];
        assert_eq!(g.attr_f64("a"), Some(1.0));
        assert_eq!(g.attr_str("b"), Some("two"));
        assert_eq!(g.attr_f64("c"), Some(3.0));
    }

    #[test]
    fn errors_have_spans_and_recover() {
        let text = "library (x) {\n  a 1;\n  b : 2;\n}\n";
        let (groups, diags, map) = parse(text);
        assert_eq!(diags.error_count(), 1);
        let rendered = diags.render(&map);
        assert!(
            rendered.contains("expected `:` or `(` after `a`"),
            "{rendered}"
        );
        assert!(rendered.contains("--> t.lib:2:5"), "{rendered}");
        assert_eq!(groups[0].attr_f64("b"), Some(2.0));
    }

    #[test]
    fn unterminated_group_is_an_error() {
        let (_, diags, _) = parse("library (x) {\n a : 1;\n");
        assert_eq!(diags.error_count(), 1);
    }

    #[test]
    fn includes_are_resolved_through_the_callback() {
        let mut map = SourceMap::new();
        let main = "library (x) {\n include_file (units.lib);\n cell (c) { }\n}\n";
        let inc = "time_unit : \"1ns\";\ncell (d) { area : 1; }\n";
        let id = map.add("main.lib", main).unwrap();
        let inc_id = map.add("units.lib", inc).unwrap();
        let mut diags = Diagnostics::new();
        let mut resolver = |name: &str| (name == "units.lib").then(|| (inc.to_string(), inc_id));
        let groups = parse_groups_with_includes(main, id, &mut diags, Some(&mut resolver));
        assert!(diags.is_empty(), "{}", diags.render(&map));
        let lib = &groups[0];
        assert_eq!(lib.attr_str("time_unit"), Some("1ns"));
        assert_eq!(lib.groups.len(), 2);
        assert_eq!(lib.groups[0].arg_name(), Some("d"));

        let mut diags = Diagnostics::new();
        parse_groups(main, id, &mut diags);
        assert_eq!(diags.error_count(), 1);
    }

    #[test]
    fn word_classification() {
        assert_eq!(classify_word("1.5"), Value::Number(1.5));
        assert_eq!(classify_word("1e-3"), Value::Number(0.001));
        assert_eq!(classify_word("1ns"), Value::Quantity(1.0, "ns".into()));
        assert_eq!(classify_word("0.01pf"), Value::Quantity(0.01, "pf".into()));
        assert_eq!(classify_word("1.8V"), Value::Quantity(1.8, "V".into()));
        assert_eq!(classify_word("true"), Value::Bool(true));
        assert_eq!(
            classify_word("tt_025C_1v80"),
            Value::Ident("tt_025C_1v80".into())
        );
        assert_eq!(classify_word("e5"), Value::Ident("e5".into()));
    }

    #[test]
    fn to_text_round_trips() {
        let text = "library (demo) {\n  time_unit : \"1ns\";\n  capacitive_load_unit(1, pf);\n  cell (inv) {\n    area : 3.75;\n    pin (A) {\n      direction : input;\n    }\n  }\n}\n";
        let (groups, _, _) = parse(text);
        let printed = groups[0].to_text();
        assert_eq!(printed, text);
        let (again, _, _) = parse(&printed);
        assert_eq!(again[0].to_text(), printed);
    }
}
