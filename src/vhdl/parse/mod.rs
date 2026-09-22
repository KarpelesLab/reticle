//! The VHDL parser: tokens to [`ast::DesignFile`].
//!
//! A hand-written recursive-descent parser over the token slice produced by
//! [`super::lex`]. Each grammar area lives in its own file:
//!
//! - `design_unit`: design files, context clauses, entities, architectures,
//!   packages, configurations and context declarations;
//! - `decl`: declarative parts, objects, subprograms, components, aliases,
//!   attributes and the rest;
//! - `types`: type definitions, subtype indications, constraints, ranges and
//!   signatures;
//! - `expr`: expressions by precedence, literals, aggregates, allocators;
//! - `names`: names and association lists;
//! - `concurrent`: processes, concurrent assignments, instantiations,
//!   generates and blocks;
//! - `sequential`: sequential statements.
//!
//! # Errors and recovery
//!
//! Parsing functions return [`PResult`]. A failed `expect` reports one
//! diagnostic of the form "expected X, found Y" at the offending token and
//! returns `Err(Recover)`, which propagates with `?` up to the nearest item
//! boundary: a statement, a declaration or a design unit. The boundary
//! records nothing further; it skips tokens to a resynchronisation point (a
//! `;`, which it consumes, or a keyword that starts the next item, which it
//! leaves) and carries on. So one mistake yields one diagnostic and the
//! rest of the file is still parsed and checked.
//!
//! Three refinements keep cascades short:
//!
//! - A diagnostic is never reported twice at the same token; when several
//!   levels fail on the same token only the innermost message survives.
//! - Tokens whose absence is harmless (`is`, `then`, `loop`, a final `;`
//!   before a keyword) are reported and assumed present, see
//!   `expect_soft` on the parser.
//! - `end` matching uses a stack of the constructs currently open. `end
//!   process` seen while an `if` is still open reports the missing `end if`
//!   and lets the process close; a stray `end loop` with no loop open is
//!   reported and skipped.

mod concurrent;
mod decl;
mod design_unit;
mod expr;
mod names;
mod sequential;
mod types;

use super::ast::{self, Ident};
use super::token::{Standard, Token, TokenKind};
use crate::diag::{Diagnostic, Diagnostics};
use crate::source::Span;

/// Marker returned by a parsing function that has already reported its
/// error; the caller resynchronises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recover;

/// The result of a parsing function.
pub type PResult<T> = Result<T, Recover>;

/// The parser state over one token stream.
///
/// `'t` is the lifetime of the token slice and `'src` that of the source
/// text the tokens borrow from.
#[derive(Debug)]
pub struct Parser<'t, 'src> {
    tokens: &'t [Token<'src>],
    pos: usize,
    standard: Standard,
    diags: Diagnostics,
    /// Position of the token the last error was reported at, so several
    /// failures at one token produce one message.
    last_error: usize,
    /// The keywords of the constructs currently open, innermost last, used
    /// to match `end` clauses (see the module docs).
    open: Vec<TokenKind>,
}

impl<'t, 'src> Parser<'t, 'src> {
    /// Creates a parser over `tokens`, which must end with
    /// [`TokenKind::Eof`] as the lexer guarantees. An empty slice parses as
    /// an empty design file.
    pub fn new(tokens: &'t [Token<'src>], standard: Standard) -> Self {
        Parser {
            tokens,
            pos: 0,
            standard,
            diags: Diagnostics::new(),
            last_error: usize::MAX,
            open: Vec::new(),
        }
    }

    /// Parses the whole token stream as a design file, appending
    /// diagnostics to `diags`.
    pub fn parse_design_file(mut self, diags: &mut Diagnostics) -> ast::DesignFile {
        let file = if self.tokens.is_empty() {
            ast::DesignFile::default()
        } else {
            self.design_file()
        };
        diags.append(&mut self.diags);
        file
    }

    /// The standard being parsed against.
    pub fn standard(&self) -> Standard {
        self.standard
    }

    // --- cursor -----------------------------------------------------------

    /// The token at the cursor; the final `Eof` once the input is consumed.
    fn peek(&self) -> &'t Token<'src> {
        let i = self.pos.min(self.tokens.len() - 1);
        &self.tokens[i]
    }

    /// The kind of the token at the cursor.
    fn kind(&self) -> TokenKind {
        self.peek().kind
    }

    /// The kind of the token `n` positions ahead of the cursor.
    fn kind_at(&self, n: usize) -> TokenKind {
        let i = (self.pos + n).min(self.tokens.len() - 1);
        self.tokens[i].kind
    }

    /// True when the cursor is at a token of kind `kind`.
    fn at(&self, kind: TokenKind) -> bool {
        self.kind() == kind
    }

    /// True when the cursor is at a token of one of `kinds`.
    fn at_any(&self, kinds: &[TokenKind]) -> bool {
        kinds.contains(&self.kind())
    }

    /// True at end of input.
    fn at_eof(&self) -> bool {
        self.at(TokenKind::Eof)
    }

    /// Consumes and returns the token at the cursor. Never moves past `Eof`.
    fn bump(&mut self) -> &'t Token<'src> {
        let t = self.peek();
        if t.kind != TokenKind::Eof {
            self.pos += 1;
        }
        t
    }

    /// Consumes the token at the cursor when it has kind `kind`.
    fn eat(&mut self, kind: TokenKind) -> Option<&'t Token<'src>> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }

    /// The span of the token before the cursor: the end of whatever was
    /// just parsed. At the start of the input it is the first token's span.
    fn prev_span(&self) -> Span {
        let i = self.pos.saturating_sub(1).min(self.tokens.len() - 1);
        self.tokens[i].span
    }

    /// The span of the token at the cursor.
    fn span(&self) -> Span {
        self.peek().span
    }

    /// A span from `start` to the end of the last consumed token.
    fn span_from(&self, start: Span) -> Span {
        start.to(self.prev_span())
    }

    /// Finds the position of the `)` matching the `(` at `open`, if the
    /// cursor is at one and it is closed before end of input.
    fn matching_paren(&self, open: usize) -> Option<usize> {
        if self.tokens.get(open)?.kind != TokenKind::LParen {
            return None;
        }
        let mut depth = 0usize;
        for (i, t) in self.tokens.iter().enumerate().skip(open) {
            match t.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                TokenKind::Eof => return None,
                _ => {}
            }
        }
        None
    }

    // --- diagnostics --------------------------------------------------------

    /// Reports an error at the cursor unless one was already reported
    /// there.
    fn error_here(&mut self, message: impl Into<String>) {
        if self.last_error == self.pos {
            return;
        }
        self.last_error = self.pos;
        let span = self.span();
        self.diags.push(Diagnostic::error(message).with_span(span));
    }

    /// Reports a diagnostic that is not tied to the cursor (a mismatched
    /// label, a construct the standard forbids). Never deduplicated.
    fn report(&mut self, diag: Diagnostic) {
        self.diags.push(diag);
    }

    /// Reports "expected `what`, found ..." at the cursor and returns the
    /// recovery marker so callers can `return Err(self.expected(..))`.
    fn expected(&mut self, what: &str) -> Recover {
        let found = describe(self.peek());
        self.error_here(format!("expected {what}, found {found}"));
        Recover
    }

    /// Consumes a token of kind `kind` or reports an error.
    fn expect(&mut self, kind: TokenKind) -> PResult<&'t Token<'src>> {
        if self.at(kind) {
            Ok(self.bump())
        } else {
            Err(self.expected(&kind.to_string()))
        }
    }

    /// Consumes a token of kind `kind` when present; otherwise reports the
    /// error and continues as if the token had been there. For tokens whose
    /// absence does not change how the rest is parsed (`is`, `then`, ...).
    fn expect_soft(&mut self, kind: TokenKind) {
        if !self.eat(kind).is_some() {
            let _ = self.expected(&kind.to_string());
        }
    }

    /// Consumes the `;` that ends an item. When it is missing and the next
    /// token starts another item the error is soft; otherwise the caller
    /// resynchronises.
    fn expect_semi(&mut self) -> PResult<()> {
        if self.eat(TokenKind::Semi).is_some() {
            return Ok(());
        }
        let k = self.kind();
        if is_item_keyword(k) {
            let _ = self.expected("`;`");
            Ok(())
        } else {
            Err(self.expected("`;`"))
        }
    }

    /// Reports a warning when parsing VHDL-93 and a 2008-only construct was
    /// used. The construct is still parsed.
    fn require_2008(&mut self, span: Span, what: &str) {
        if self.standard == Standard::Vhdl93 {
            self.report(
                Diagnostic::warning(format!("{what} requires VHDL-2008"))
                    .with_label(span, "not valid in VHDL-93"),
            );
        }
    }

    // --- recovery ---------------------------------------------------------

    /// Skips tokens after a failed item until a `;` (consumed) or a token in
    /// `boundary` (left in place), always consuming at least one token
    /// beyond `start` so the enclosing loop makes progress.
    fn recover(&mut self, start: usize, boundary: fn(TokenKind) -> bool) {
        loop {
            let k = self.kind();
            if k == TokenKind::Eof {
                return;
            }
            if k == TokenKind::Semi {
                self.bump();
                return;
            }
            if self.pos > start && boundary(k) {
                return;
            }
            self.bump();
        }
    }

    /// Skips tokens until one of `stop` (left in place), `;`, or `end`
    /// (both left in place), returning true when a `stop` token was found.
    /// Used inside compound statements to resume at `then`, `loop`, ...
    fn skip_to(&mut self, stop: &[TokenKind]) -> bool {
        loop {
            let k = self.kind();
            if stop.contains(&k) {
                return true;
            }
            if matches!(k, TokenKind::Eof | TokenKind::Semi | TokenKind::End) {
                return false;
            }
            self.bump();
        }
    }

    /// Consumes `kind` if present; otherwise skips to it when it appears
    /// before the end of the statement, and reports the error either way.
    fn expect_or_skip_to(&mut self, kind: TokenKind) -> PResult<()> {
        if self.eat(kind).is_some() {
            return Ok(());
        }
        let _ = self.expected(&kind.to_string());
        if self.skip_to(&[kind]) {
            self.bump();
            Ok(())
        } else {
            Err(Recover)
        }
    }

    // --- `end` clauses --------------------------------------------------------

    /// Runs `body` with `kw` pushed on the stack of open constructs.
    fn with_open<T>(&mut self, kw: TokenKind, body: impl FnOnce(&mut Self) -> T) -> T {
        self.open.push(kw);
        let r = body(self);
        self.open.pop();
        r
    }

    /// Parses `end kws... [name];`, where `kws` is the keyword sequence of
    /// the construct (`[Process]`, `[Package, Body]`, `[Case, Question]`).
    /// When `optional` is set the keywords may be omitted (design units,
    /// VHDL-93 style).
    ///
    /// The clause is matched against the stack of open constructs: an `end`
    /// followed by the keyword of an outer construct means this construct's
    /// `end` is missing; it is reported and the outer `end` is left for the
    /// outer construct. An `end` with a keyword that matches nothing open
    /// is a stray; it is reported and consumed.
    fn parse_end(
        &mut self,
        kws: &[TokenKind],
        optional: bool,
        name: Option<&Ident>,
    ) -> PResult<()> {
        if self.parse_end_clause(kws, optional, name)? {
            self.expect_semi()?;
        }
        Ok(())
    }

    /// [`Parser::parse_end`] without the final `;`, for the `end record`,
    /// `end units` and `end protected` clauses that share the `;` of the
    /// enclosing type declaration.
    fn parse_end_no_semi(&mut self, kws: &[TokenKind], name: Option<&Ident>) -> PResult<()> {
        self.parse_end_clause(kws, false, name).map(|_| ())
    }

    /// The shared part of the `end` parsers. Returns `false` when the `end`
    /// belongs to an outer construct and was left in place (this
    /// construct's `end` being reported as missing), `true` otherwise.
    fn parse_end_clause(
        &mut self,
        kws: &[TokenKind],
        optional: bool,
        name: Option<&Ident>,
    ) -> PResult<bool> {
        let kw = kws[0];
        loop {
            if self.consume_stray_end() {
                continue;
            }
            let save = self.pos;
            self.expect(TokenKind::End)?;
            if kw == TokenKind::Process {
                self.eat(TokenKind::Postponed);
            }
            let k = self.kind();
            if k == kw {
                self.bump();
                for &extra in &kws[1..] {
                    self.expect_soft(extra);
                }
                break;
            }
            if k.is_reserved_word() {
                // Not a stray (those were consumed above), so it closes an
                // outer construct: this one's `end` is missing.
                let msg = format!("expected `end {}`, found `end {}`", kw_text(kw), kw_text(k));
                self.error_here(msg);
                self.pos = save;
                return Ok(false);
            }
            if !optional {
                let msg = format!(
                    "expected `end {}`, found {}",
                    kw_text(kw),
                    describe(self.peek())
                );
                self.error_here(msg);
            }
            break;
        }
        self.parse_optional_end_name(name)?;
        Ok(true)
    }

    /// When the cursor is at an `end` whose keyword closes nothing that is
    /// open (a stray `end loop;` in a process, say), reports it, consumes
    /// the whole clause up to its `;` and returns true so the caller can
    /// carry on with the next item.
    fn consume_stray_end(&mut self) -> bool {
        if !self.at(TokenKind::End) {
            return false;
        }
        let k = self.kind_at(1);
        if !k.is_reserved_word() || k == TokenKind::Postponed || self.open.contains(&k) {
            return false;
        }
        self.bump();
        let msg = format!("`end {}` does not close anything open", kw_text(k));
        self.error_here(msg);
        while !self.at_any(&[TokenKind::Semi, TokenKind::End, TokenKind::Eof]) {
            self.bump();
        }
        self.eat(TokenKind::Semi);
        true
    }

    /// Parses the optional designator after `end ...` and checks it
    /// against the opener's name.
    fn parse_optional_end_name(&mut self, name: Option<&Ident>) -> PResult<()> {
        if !self.at_any(&[
            TokenKind::Ident,
            TokenKind::ExtendedIdent,
            TokenKind::StringLit,
            TokenKind::CharLit,
        ]) {
            return Ok(());
        }
        let d = self.parse_designator()?;
        match (name, &d) {
            (Some(n), ast::Designator::Ident(i)) if n.same_as(i) => {}
            (Some(n), d) => {
                let (message, label) = (
                    format!("`end {}` does not match `{}`", designator_text(d), n.name),
                    format!("declared as `{}` here", n.name),
                );
                self.report(
                    Diagnostic::error(message)
                        .with_span(d.span())
                        .with_secondary(n.span, label),
                );
            }
            (None, d) => {
                self.report(
                    Diagnostic::error(format!(
                        "`end {}` names a construct that has no label",
                        designator_text(d)
                    ))
                    .with_span(d.span()),
                );
            }
        }
        Ok(())
    }

    /// Parses `end [kws] [designator];` for subprograms, where the name is
    /// a designator rather than an identifier.
    fn parse_end_designator(
        &mut self,
        kws: &[TokenKind],
        designator: &ast::Designator,
    ) -> PResult<()> {
        match designator {
            ast::Designator::Ident(i) => self.parse_end(kws, true, Some(i)),
            other => {
                let kw = kws[0];
                self.expect(TokenKind::End)?;
                if self.at(TokenKind::Function) || self.at(TokenKind::Procedure) {
                    if !self.at(kw) {
                        let msg = format!(
                            "expected `end {}`, found {}",
                            kw_text(kw),
                            describe(self.peek())
                        );
                        self.error_here(msg);
                    }
                    self.bump();
                }
                if self.at_any(&[TokenKind::StringLit, TokenKind::CharLit]) {
                    let d = self.parse_designator()?;
                    if designator_text(&d).to_lowercase() != designator_text(other).to_lowercase() {
                        self.report(
                            Diagnostic::error(format!(
                                "`end {}` does not match `{}`",
                                designator_text(&d),
                                designator_text(other)
                            ))
                            .with_span(d.span())
                            .with_secondary(other.span(), "declared here"),
                        );
                    }
                } else if self.at_any(&[TokenKind::Ident, TokenKind::ExtendedIdent]) {
                    let d = self.parse_designator()?;
                    self.report(
                        Diagnostic::error(format!(
                            "`end {}` does not match `{}`",
                            designator_text(&d),
                            designator_text(other)
                        ))
                        .with_span(d.span()),
                    );
                }
                self.expect_semi()
            }
        }
    }

    // --- small shared pieces --------------------------------------------------

    /// Parses a basic or extended identifier.
    fn parse_ident(&mut self) -> PResult<Ident> {
        match self.kind() {
            TokenKind::Ident | TokenKind::ExtendedIdent => Ok(ident_of(self.bump())),
            _ => Err(self.expected("identifier")),
        }
    }

    /// Parses `ident {, ident}`.
    fn parse_ident_list(&mut self) -> PResult<Vec<Ident>> {
        let mut names = vec![self.parse_ident()?];
        while self.eat(TokenKind::Comma).is_some() {
            names.push(self.parse_ident()?);
        }
        Ok(names)
    }

    /// Parses an identifier, character literal or operator symbol.
    fn parse_designator(&mut self) -> PResult<ast::Designator> {
        match self.kind() {
            TokenKind::Ident | TokenKind::ExtendedIdent => {
                Ok(ast::Designator::Ident(ident_of(self.bump())))
            }
            TokenKind::CharLit => {
                let t = self.bump();
                Ok(ast::Designator::Char {
                    ch: char_of(t),
                    span: t.span,
                })
            }
            TokenKind::StringLit => {
                let t = self.bump();
                Ok(ast::Designator::Operator {
                    symbol: t.text().to_owned(),
                    span: t.span,
                })
            }
            _ => Err(self.expected("identifier, character literal or operator symbol")),
        }
    }

    /// Parses `label :` when the cursor is at an identifier followed by a
    /// colon.
    fn parse_optional_label(&mut self) -> PResult<Option<Ident>> {
        if matches!(self.kind(), TokenKind::Ident | TokenKind::ExtendedIdent)
            && self.kind_at(1) == TokenKind::Colon
        {
            let l = self.parse_ident()?;
            self.bump();
            Ok(Some(l))
        } else {
            Ok(None)
        }
    }

    /// Parses `( item {, item} )` with `item` parsed by `f`.
    fn parse_paren_list<T>(&mut self, f: impl Fn(&mut Self) -> PResult<T>) -> PResult<Vec<T>> {
        self.expect(TokenKind::LParen)?;
        let mut items = vec![f(self)?];
        while self.eat(TokenKind::Comma).is_some() {
            items.push(f(self)?);
        }
        self.expect(TokenKind::RParen)?;
        Ok(items)
    }
}

/// Converts an identifier token to an [`Ident`].
fn ident_of(t: &Token<'_>) -> Ident {
    Ident {
        name: t.text().to_owned(),
        span: t.span,
        extended: t.kind == TokenKind::ExtendedIdent,
    }
}

/// The character of a character literal token.
fn char_of(t: &Token<'_>) -> char {
    t.text().chars().next().unwrap_or('\0')
}

/// The text of a designator, for diagnostics.
fn designator_text(d: &ast::Designator) -> String {
    match d {
        ast::Designator::Ident(i) => i.name.clone(),
        ast::Designator::Char { ch, .. } => format!("'{ch}'"),
        ast::Designator::Operator { symbol, .. } => format!("\"{symbol}\""),
    }
}

/// The fixed text of a keyword, for diagnostics.
fn kw_text(k: TokenKind) -> &'static str {
    k.fixed_text().unwrap_or("?")
}

/// Describes a token for an "expected X, found Y" message.
fn describe(t: &Token<'_>) -> String {
    match t.kind {
        TokenKind::Ident | TokenKind::ExtendedIdent => format!("identifier `{}`", t.text()),
        TokenKind::Integer | TokenKind::Real | TokenKind::BitStringLit => {
            format!("{} `{}`", t.kind, t.text())
        }
        TokenKind::CharLit => format!("character literal `'{}'`", t.text()),
        TokenKind::StringLit => format!("string literal `\"{}\"`", t.text()),
        TokenKind::Error => format!("invalid character `{}`", t.text()),
        _ => t.kind.to_string(),
    }
}

/// True for keywords that begin a declaration, a statement or a design
/// unit: a missing `;` before one of them is reported softly.
fn is_item_keyword(k: TokenKind) -> bool {
    is_declaration_keyword(k)
        || is_sequential_keyword(k)
        || is_concurrent_keyword(k)
        || is_unit_keyword(k)
        || matches!(k, TokenKind::End | TokenKind::Begin)
}

/// True for keywords that begin a declarative item.
fn is_declaration_keyword(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::Constant
            | TokenKind::Signal
            | TokenKind::Variable
            | TokenKind::Shared
            | TokenKind::File
            | TokenKind::Type
            | TokenKind::Subtype
            | TokenKind::Alias
            | TokenKind::Attribute
            | TokenKind::Component
            | TokenKind::Function
            | TokenKind::Procedure
            | TokenKind::Pure
            | TokenKind::Impure
            | TokenKind::Use
            | TokenKind::For
            | TokenKind::Group
            | TokenKind::Disconnect
            | TokenKind::Package
    )
}

/// True for keywords that begin (or delimit) a sequential statement.
fn is_sequential_keyword(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::If
            | TokenKind::Elsif
            | TokenKind::Else
            | TokenKind::Case
            | TokenKind::When
            | TokenKind::For
            | TokenKind::While
            | TokenKind::Loop
            | TokenKind::Wait
            | TokenKind::Assert
            | TokenKind::Report
            | TokenKind::Return
            | TokenKind::Next
            | TokenKind::Exit
            | TokenKind::Null
            | TokenKind::With
    )
}

/// True for keywords that begin (or delimit) a concurrent statement.
fn is_concurrent_keyword(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::Process
            | TokenKind::Postponed
            | TokenKind::Block
            | TokenKind::Assert
            | TokenKind::With
            | TokenKind::Component
            | TokenKind::Entity
            | TokenKind::Configuration
            | TokenKind::Generate
            | TokenKind::Elsif
            | TokenKind::Else
            | TokenKind::When
    )
}

/// True for keywords that begin a design unit or a context clause.
fn is_unit_keyword(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::Library
            | TokenKind::Use
            | TokenKind::Context
            | TokenKind::Entity
            | TokenKind::Architecture
            | TokenKind::Package
            | TokenKind::Configuration
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;
    use crate::vhdl::lex_source;

    /// Parses `text` and returns the design file and rendered diagnostics.
    pub(super) fn parse(text: &str) -> (ast::DesignFile, String) {
        let mut map = SourceMap::new();
        let id = map.add("t.vhd", text).unwrap();
        let mut diags = Diagnostics::new();
        let tokens = lex_source(&map, id, Standard::Vhdl2008, &mut diags);
        let file = Parser::new(&tokens, Standard::Vhdl2008).parse_design_file(&mut diags);
        diags.sort();
        (file, diags.render(&map))
    }

    #[test]
    fn empty_input() {
        let (file, diags) = parse("");
        assert!(file.units.is_empty());
        assert!(diags.is_empty());
        let file = Parser::new(&[], Standard::Vhdl2008).parse_design_file(&mut Diagnostics::new());
        assert!(file.units.is_empty());
    }

    #[test]
    fn one_error_per_mistake() {
        let (file, diags) = parse(
            "entity e is end;\narchitecture a of e is\n  signal x : bit\n  signal y : bit;\nbegin\n  x <= y +;\n  y <= x;\nend;\n",
        );
        assert_eq!(file.units.len(), 2);
        assert_eq!(diags.matches("error").count(), 2, "{diags}");
        assert!(diags.contains("expected `;`, found `signal`"), "{diags}");
        assert!(diags.contains("expected expression, found `;`"), "{diags}");
        if let ast::LibraryUnit::Architecture(a) = &file.units[1].unit {
            assert_eq!(a.decls.len(), 2);
            assert_eq!(a.statements.len(), 1);
        } else {
            panic!("expected an architecture");
        }
    }

    #[test]
    fn missing_end_if_is_one_error() {
        let (file, diags) = parse(
            "architecture a of e is begin\n process begin\n if x then y <= '1';\n end process;\nend;\n",
        );
        assert_eq!(diags.matches("error").count(), 1, "{diags}");
        assert!(
            diags.contains("expected `end if`, found `end process`"),
            "{diags}"
        );
        assert_eq!(file.units.len(), 1);
    }

    #[test]
    fn end_label_mismatch_is_reported_not_fatal() {
        let (file, diags) = parse("entity e is end entity f;\n");
        assert_eq!(file.units.len(), 1);
        assert!(diags.contains("`end f` does not match `e`"), "{diags}");
    }
}
