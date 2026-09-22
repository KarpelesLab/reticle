//! The Verilog / SystemVerilog parser.
//!
//! A hand-written recursive-descent parser over the token slice produced by
//! [`crate::verilog::lex`], building the tree in [`crate::verilog::ast`].
//! It is split by grammar area:
//!
//! - this file: the [`Parser`] cursor, diagnostics, error recovery,
//!   attributes and the compilation-unit loop;
//! - `module`: module / interface / program / primitive headers, port
//!   lists, module and gate instantiations;
//! - `decl`: nets, variables, parameters, typedefs, functions and tasks,
//!   and the remaining declaration items;
//! - `types`: data types and dimensions;
//! - `expr`: expressions by precedence climbing;
//! - `stmt`: procedural statements, timing controls, blocks;
//! - `generate`: generate regions and constructs;
//! - `package`: packages, imports, exports, time units.
//!
//! # Error handling
//!
//! Every parsing function returns [`PResult`]. On a mismatch it reports
//! "expected X, found Y" at the offending token and returns
//! [`ParseError`], which propagates with `?` to the nearest statement or
//! item boundary. There the caller *recovers*: it skips tokens up to a
//! synchronisation point (a `;`, which is consumed, or an `end`-style or
//! item-starting keyword, which is not) and carries on. Errors reported at
//! the synchronisation token itself are suppressed, so a construct that
//! failed to parse leaves a single diagnostic rather than a cascade, and
//! the tree still holds everything parsed around it.
//!
//! # Ambiguities
//!
//! Verilog's grammar has a handful of places where the first token does
//! not decide the construct; each is resolved by a bounded lookahead:
//!
//! - `name name` at item or block level is a declaration with a
//!   user-defined type, `name name (` is an instantiation, `name [..] name`
//!   a declaration, `name (` at item level an instantiation;
//! - `#(` after a module name is a parameter override, elsewhere a delay;
//! - `(*` is an attribute, while `(*)` is lexed as `( * )` for `@(*)`;
//! - a statement starting with a name parses its left-hand side as a
//!   postfix expression (so `<=` is not taken as a comparison) and then
//!   decides between assignment, call and `++`/`--` by the next token;
//! - `label :` before a statement or item is a label, since no statement
//!   or item starts with a bare name followed by a colon.
//!
//! The parser is purely syntactic: it accepts every grammatical program
//! and leaves type checking, name resolution and dialect policing to
//! elaboration. The [`Dialect`] only reaches the lexer's keyword table.

mod decl;
mod expr;
mod generate;
mod module;
mod package;
mod stmt;
mod types;

use crate::diag::Diagnostics;
use crate::source::Span;

use super::ast::{Attribute, Ident, Item, ItemKind, RawTokens, SourceFile};
use super::token::{Dialect, Keyword, Punct, Token, TokenKind};

/// A parse failure already reported through the parser's diagnostics.
///
/// Carries no data: the diagnostic is the message, and the value is only
/// a signal to unwind to the nearest recovery point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

/// The result of every parsing function.
pub type PResult<T> = Result<T, ParseError>;

/// The parser state: a cursor over a token slice plus the diagnostics
/// collected so far.
///
/// ```
/// use reticle::diag::Diagnostics;
/// use reticle::source::SourceMap;
/// use reticle::verilog::{Dialect, NoIncludes, Parser, lex_source};
///
/// let mut map = SourceMap::new();
/// let id = map.add("t.v", "module m(input a, output y); assign y = ~a; endmodule").unwrap();
/// let mut diags = Diagnostics::new();
/// let tokens = lex_source(&mut map, id, Dialect::Verilog2005, &mut NoIncludes, &mut diags);
/// let mut parser = Parser::new(&tokens, Dialect::Verilog2005);
/// let file = parser.parse_source_file();
/// assert!(parser.diagnostics().is_empty());
/// assert_eq!(file.items.len(), 1);
/// ```
pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// Tokens at or past this index read as end of input. Lowered
    /// temporarily to parse a bounded sub-range (see
    /// [`Parser::with_limit`]).
    limit: usize,
    /// The token reported once the cursor runs off `tokens` (or `limit`).
    eof: Token,
    #[allow(dead_code, reason = "kept for dialect-specific rules to come")]
    dialect: Dialect,
    diags: Diagnostics,
    /// Position of the last consumed token, for span joining.
    last: usize,
    /// Errors at token positions up to and including this one are
    /// dropped: recovery just skipped to here, so a second complaint about
    /// the same token would be noise.
    suppress_until: Option<usize>,
}

impl<'a> Parser<'a> {
    /// Creates a parser over `tokens`, which must end with
    /// [`TokenKind::Eof`] as the lexer guarantees.
    ///
    /// # Panics
    ///
    /// If `tokens` is empty: there would be no file to attach spans to.
    pub fn new(tokens: &'a [Token], dialect: Dialect) -> Self {
        let last = tokens.last().expect("token stream ends with Eof");
        let (eof, limit) = if last.kind == TokenKind::Eof {
            (last.clone(), tokens.len() - 1)
        } else {
            let at = Span::new(last.span.file, last.span.end, last.span.end);
            (Token::new(TokenKind::Eof, at), tokens.len())
        };
        Parser {
            tokens,
            pos: 0,
            limit,
            eof,
            dialect,
            diags: Diagnostics::new(),
            last: 0,
            suppress_until: None,
        }
    }

    /// The diagnostics reported so far.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diags
    }

    /// Moves the diagnostics out, leaving the parser with none.
    pub fn take_diagnostics(&mut self) -> Diagnostics {
        std::mem::take(&mut self.diags)
    }

    /// Parses the whole token stream as a compilation unit.
    ///
    /// Never fails: unparseable regions are reported and skipped.
    pub fn parse_source_file(&mut self) -> SourceFile {
        let start = self.span();
        let items = self.parse_items_until(&[]);
        let span = if self.pos > 0 {
            start.to(self.prev_span())
        } else {
            start
        };
        SourceFile { items, span }
    }

    // --- cursor -----------------------------------------------------------

    /// The token under the cursor, or end of input.
    fn peek(&self) -> &Token {
        if self.pos < self.limit {
            &self.tokens[self.pos]
        } else {
            &self.eof
        }
    }

    /// The kind of the token under the cursor.
    fn kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    /// The kind of the token `n` past the cursor, `Eof` past the limit.
    fn nth(&self, n: usize) -> &TokenKind {
        self.kind_at(self.pos + n)
    }

    /// Kind of the token at absolute index `i`, `Eof` outside the limit.
    fn kind_at(&self, i: usize) -> &TokenKind {
        if i < self.limit {
            &self.tokens[i].kind
        } else {
            &TokenKind::Eof
        }
    }

    /// Span of the token under the cursor.
    fn span(&self) -> Span {
        self.peek().span
    }

    /// Span of the last consumed token (or of the first token before
    /// anything was consumed).
    fn prev_span(&self) -> Span {
        self.tokens.get(self.last).map_or(self.eof.span, |t| t.span)
    }

    /// A span from `start` to the end of the last consumed token.
    fn span_from(&self, start: Span) -> Span {
        start.to(self.prev_span())
    }

    /// True at end of input (or at the current limit).
    fn at_eof(&self) -> bool {
        self.pos >= self.limit
    }

    /// True when the cursor is on this punctuation.
    fn at_punct(&self, p: Punct) -> bool {
        self.kind().is_punct(p)
    }

    /// True when the cursor is on this keyword.
    fn at_kw(&self, kw: Keyword) -> bool {
        self.kind().is_kw(kw)
    }

    /// True when the cursor is on a simple or escaped identifier.
    fn at_ident(&self) -> bool {
        self.kind().is_ident()
    }

    /// True when the token `n` ahead is this punctuation.
    fn nth_is_punct(&self, n: usize, p: Punct) -> bool {
        self.nth(n).is_punct(p)
    }

    /// True when the token `n` ahead is this keyword.
    fn nth_is_kw(&self, n: usize, kw: Keyword) -> bool {
        self.nth(n).is_kw(kw)
    }

    /// Consumes the token under the cursor and returns its span. At end of
    /// input nothing is consumed.
    fn bump(&mut self) -> Span {
        let span = self.span();
        if self.pos < self.limit {
            self.last = self.pos;
            self.pos += 1;
        }
        span
    }

    /// Consumes this punctuation if present.
    fn eat_punct(&mut self, p: Punct) -> Option<Span> {
        self.at_punct(p).then(|| self.bump())
    }

    /// Consumes this keyword if present.
    fn eat_kw(&mut self, kw: Keyword) -> Option<Span> {
        self.at_kw(kw).then(|| self.bump())
    }

    /// Consumes this punctuation or reports "expected `p`".
    fn expect_punct(&mut self, p: Punct) -> PResult<Span> {
        match self.eat_punct(p) {
            Some(span) => Ok(span),
            None => Err(self.expected(&format!("`{p}`"))),
        }
    }

    /// Consumes this keyword or reports "expected `kw`".
    fn expect_kw(&mut self, kw: Keyword) -> PResult<Span> {
        match self.eat_kw(kw) {
            Some(span) => Ok(span),
            None => Err(self.expected(&format!("`{kw}`"))),
        }
    }

    /// Consumes an identifier or reports "expected identifier".
    fn expect_ident(&mut self) -> PResult<Ident> {
        match self.eat_ident() {
            Some(id) => Ok(id),
            None => Err(self.expected("identifier")),
        }
    }

    /// Consumes a simple or escaped identifier if present.
    fn eat_ident(&mut self) -> Option<Ident> {
        let name = self.kind().ident_name()?.to_string();
        let span = self.bump();
        Some(Ident { name, span })
    }

    /// Consumes `;` or reports its absence.
    ///
    /// When the next token is a keyword that starts another item or
    /// statement, the semicolon was merely forgotten: the error is
    /// reported and parsing carries on as if it were there, so the
    /// construct before it is kept.
    fn expect_semi(&mut self) -> PResult<Span> {
        if let Some(span) = self.eat_punct(Punct::Semi) {
            return Ok(span);
        }
        let err = self.expected("`;`");
        let kind = self.kind();
        if is_item_sync(kind) || is_stmt_sync(kind) {
            let at = self.span();
            return Ok(Span::new(at.file, at.start, at.start));
        }
        Err(err)
    }

    /// Consumes the end keyword `kw` if present, else reports it missing
    /// and leaves the token for the enclosing construct. Returns whether
    /// it was present. Used where the body was parsed up to a closing
    /// keyword anyway, so the construct is worth keeping either way.
    fn expect_end(&mut self, kw: Keyword) -> bool {
        if self.eat_kw(kw).is_some() {
            self.eat_end_label();
            return true;
        }
        self.expected(&format!("`{kw}`"));
        false
    }

    /// Consumes the optional `: name` after an end keyword.
    fn eat_end_label(&mut self) {
        if self.at_punct(Punct::Colon) && self.nth(1).is_ident() {
            self.bump();
            self.bump();
        }
    }

    // --- diagnostics ------------------------------------------------------

    /// Reports "expected `what`, found <token>" at the cursor.
    fn expected(&mut self, what: &str) -> ParseError {
        let found = self.kind().to_string();
        let span = self.span();
        self.error_at(span, format!("expected {what}, found {found}"))
    }

    /// Reports an error at `span`, unless recovery just landed here.
    fn error_at(&mut self, span: Span, message: impl Into<String>) -> ParseError {
        if self.suppress_until.is_some_and(|s| self.pos <= s) {
            return ParseError;
        }
        self.diags.error(span, message);
        ParseError
    }

    // --- recovery ---------------------------------------------------------

    /// Skips to the next item boundary: past a `;`, or up to (not past) a
    /// keyword that starts or ends an item.
    fn recover_item(&mut self) {
        self.recover(is_item_sync);
    }

    /// Skips to the next statement boundary: past a `;`, or up to (not
    /// past) a keyword that starts a statement or ends a block.
    fn recover_stmt(&mut self) {
        self.recover(is_stmt_sync);
    }

    /// The shared recovery loop; brackets are balanced so a stray `;`
    /// inside a parenthesised list does not end the skip early.
    fn recover(&mut self, sync: fn(&TokenKind) -> bool) {
        let mut depth = 0usize;
        loop {
            if self.at_eof() {
                break;
            }
            let kind = self.kind();
            if depth == 0 {
                if kind.is_punct(Punct::Semi) {
                    self.bump();
                    break;
                }
                if sync(kind) {
                    break;
                }
            }
            match kind {
                TokenKind::Punct(Punct::LParen | Punct::LBracket | Punct::LBrace) => depth += 1,
                TokenKind::Punct(Punct::RParen | Punct::RBracket | Punct::RBrace) => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.bump();
        }
        self.suppress_until = Some(self.pos);
    }

    /// Recovery inside a module header: skips (with brackets balanced) to
    /// the `;` that ends the header, consuming it, or stops before a
    /// design-unit keyword. Item keywords do not stop it, since `input`
    /// and friends belong to the header being skipped.
    fn recover_header(&mut self) {
        use Keyword as K;
        let mut depth = 0usize;
        while !self.at_eof() {
            match self.kind() {
                TokenKind::Punct(Punct::Semi) if depth == 0 => {
                    self.bump();
                    break;
                }
                TokenKind::Keyword(
                    K::Module
                    | K::Macromodule
                    | K::Interface
                    | K::Program
                    | K::Primitive
                    | K::Package
                    | K::Endmodule
                    | K::Endinterface
                    | K::Endprogram
                    | K::Endprimitive
                    | K::Endpackage,
                ) => break,
                TokenKind::Punct(Punct::LParen | Punct::LBracket | Punct::LBrace) => depth += 1,
                TokenKind::Punct(Punct::RParen | Punct::RBracket | Punct::RBrace) => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.bump();
        }
        self.suppress_until = Some(self.pos);
    }

    /// Skips (with brackets balanced) past the next `;`, for constructs
    /// that are reported as unsupported and dropped whole.
    fn skip_past_semi(&mut self) {
        let mut depth = 0usize;
        while !self.at_eof() {
            match self.kind() {
                TokenKind::Punct(Punct::Semi) if depth == 0 => {
                    self.bump();
                    return;
                }
                TokenKind::Punct(Punct::LParen | Punct::LBracket | Punct::LBrace) => depth += 1,
                TokenKind::Punct(Punct::RParen | Punct::RBracket | Punct::RBrace) => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.bump();
        }
    }

    // --- attributes and raw tokens -------------------------------------

    /// Parses any number of `(* name [= value], ... *)` groups.
    fn parse_attrs(&mut self) -> PResult<Vec<Attribute>> {
        let mut attrs = Vec::new();
        while self.eat_punct(Punct::AttrOpen).is_some() {
            loop {
                let name = self.expect_ident()?;
                let value = if self.eat_punct(Punct::Eq).is_some() {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                let span = self.span_from(name.span);
                attrs.push(Attribute { name, value, span });
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
            self.expect_punct(Punct::AttrClose)?;
        }
        Ok(attrs)
    }

    /// The tokens in `[from, to)` joined as text, for shallow constructs.
    fn raw_tokens(&self, from: usize, to: usize) -> RawTokens {
        let to = to.min(self.limit).max(from);
        let toks = &self.tokens[from..to];
        let mut text = String::new();
        for t in toks {
            if !text.is_empty() {
                text.push(' ');
            }
            match &t.kind {
                TokenKind::Str { value } => text.push_str(&format!("{value:?}")),
                TokenKind::SystemIdent { name } => {
                    text.push('$');
                    text.push_str(name);
                }
                TokenKind::EscapedIdent { name } => {
                    text.push('\\');
                    text.push_str(name);
                }
                TokenKind::Directive { name } => {
                    text.push('`');
                    text.push_str(name);
                }
                k => text.push_str(k.text()),
            }
        }
        let span = match (toks.first(), toks.last()) {
            (Some(a), Some(b)) => a.span.to(b.span),
            _ => {
                let s = self.span();
                Span::new(s.file, s.start, s.start)
            }
        };
        RawTokens { text, span }
    }

    /// Skips a balanced bracket group starting at the open bracket under
    /// the cursor, consuming the closing bracket. Stops at end of input.
    fn skip_balanced(&mut self) {
        let mut depth = 0usize;
        loop {
            if self.at_eof() {
                return;
            }
            match self.kind() {
                TokenKind::Punct(Punct::LParen | Punct::LBracket | Punct::LBrace) => depth += 1,
                TokenKind::Punct(Punct::RParen | Punct::RBracket | Punct::RBrace) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// Index of the token that closes the bracket at `open`, if any, with
    /// nesting of all bracket kinds respected.
    fn matching_close(&self, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut i = open;
        while i < self.limit {
            match &self.tokens[i].kind {
                TokenKind::Punct(Punct::LParen | Punct::LBracket | Punct::LBrace) => depth += 1,
                TokenKind::Punct(Punct::RParen | Punct::RBracket | Punct::RBrace) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// Index just past a balanced bracket group starting at `at`, or `at`
    /// itself when no bracket is there. Used by lookahead rules.
    fn after_balanced(&self, at: usize) -> usize {
        match self.kind_at(at) {
            TokenKind::Punct(Punct::LParen | Punct::LBracket | Punct::LBrace) => {
                self.matching_close(at).map_or(self.limit, |c| c + 1)
            }
            _ => at,
        }
    }

    /// Runs `f` with end of input moved to `limit`, then restores it.
    fn with_limit<T>(&mut self, limit: usize, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.limit;
        self.limit = limit.min(saved);
        let out = f(self);
        self.limit = saved;
        out
    }

    /// Skips tokens up to and including the keyword `end`, for constructs
    /// that are recognised but not modelled. Returns the raw tokens
    /// between the cursor and the end keyword.
    fn skip_to_kw(&mut self, end: Keyword) -> RawTokens {
        let from = self.pos;
        while !self.at_eof() && !self.at_kw(end) {
            self.bump();
        }
        let raw = self.raw_tokens(from, self.pos);
        self.eat_kw(end);
        raw
    }

    // --- items ------------------------------------------------------------

    /// Parses items until end of input or one of `terminators`, which is
    /// left for the caller. Failed items are skipped with recovery.
    fn parse_items_until(&mut self, terminators: &[Keyword]) -> Vec<Item> {
        let mut items = Vec::new();
        loop {
            if self.at_eof() {
                break;
            }
            if let TokenKind::Keyword(kw) = self.kind()
                && terminators.contains(kw)
            {
                break;
            }
            let before = self.pos;
            match self.parse_item() {
                Ok(Some(item)) => items.push(item),
                Ok(None) => {}
                Err(ParseError) => self.recover_item(),
            }
            if self.pos == before {
                // Neither the item nor recovery consumed anything (a stray
                // `end`, say): step over it to guarantee progress.
                self.bump();
            }
        }
        items
    }

    /// Parses one item. `Ok(None)` when a construct was recognised but
    /// produces no item (an unsupported one skipped with a diagnostic).
    fn parse_item(&mut self) -> PResult<Option<Item>> {
        let start = self.span();
        let attrs = self.parse_attrs()?;
        let Some(kind) = self.parse_item_kind()? else {
            return Ok(None);
        };
        Ok(Some(Item {
            attrs,
            kind,
            span: self.span_from(start),
        }))
    }

    /// Dispatches on the first token of an item.
    fn parse_item_kind(&mut self) -> PResult<Option<ItemKind>> {
        use Keyword as K;
        let kind = match self.kind() {
            TokenKind::Punct(Punct::Semi) => {
                self.bump();
                ItemKind::Empty
            }
            TokenKind::Keyword(kw) => match *kw {
                K::Module | K::Macromodule | K::Interface | K::Program | K::Primitive => {
                    ItemKind::Module(Box::new(self.parse_module()?))
                }
                K::Package => ItemKind::Package(self.parse_package()?),
                K::Wire
                | K::Tri
                | K::Tri0
                | K::Tri1
                | K::Triand
                | K::Trior
                | K::Trireg
                | K::Wand
                | K::Wor
                | K::Supply0
                | K::Supply1
                | K::Uwire
                | K::Interconnect => ItemKind::Net(self.parse_net_decl()?),
                K::Genvar => self.parse_genvar_decl()?,
                K::Parameter | K::Localparam | K::Specparam => {
                    ItemKind::Param(self.parse_param_decl()?)
                }
                K::Input | K::Output | K::Inout | K::Ref => ItemKind::Port(self.parse_port_decl()?),
                K::Typedef => ItemKind::Typedef(self.parse_typedef()?),
                K::Import | K::Export => match self.parse_import_or_export()? {
                    Some(kind) => kind,
                    None => return Ok(None),
                },
                K::Function => ItemKind::Function(self.parse_function()?),
                K::Task => ItemKind::Task(self.parse_task()?),
                K::Defparam => ItemKind::Defparam(self.parse_defparam()?),
                K::Specify => {
                    self.bump();
                    ItemKind::Specify(self.skip_to_kw(K::Endspecify))
                }
                K::Initial => {
                    self.bump();
                    ItemKind::Initial(self.parse_stmt()?)
                }
                K::Final => {
                    self.bump();
                    ItemKind::Final(self.parse_stmt()?)
                }
                K::Always | K::AlwaysComb | K::AlwaysFf | K::AlwaysLatch => {
                    let kind = match *kw {
                        K::AlwaysComb => super::ast::AlwaysKind::Comb,
                        K::AlwaysFf => super::ast::AlwaysKind::Ff,
                        K::AlwaysLatch => super::ast::AlwaysKind::Latch,
                        _ => super::ast::AlwaysKind::Always,
                    };
                    self.bump();
                    ItemKind::Always(kind, self.parse_stmt()?)
                }
                K::Assign => ItemKind::ContAssign(self.parse_cont_assign()?),
                K::Generate => {
                    self.bump();
                    let items = self.parse_items_until(&[K::Endgenerate]);
                    self.expect_end(K::Endgenerate);
                    ItemKind::Generate(items)
                }
                K::If => ItemKind::GenIf(self.parse_gen_if()?),
                K::Case => ItemKind::GenCase(self.parse_gen_case()?),
                K::For => ItemKind::GenFor(self.parse_gen_for()?),
                K::Begin => ItemKind::GenBlock(self.parse_gen_block()?),
                K::Alias => self.parse_alias()?,
                K::Assert | K::Assume | K::Cover | K::Restrict => {
                    ItemKind::Assertion(self.parse_assertion(None)?)
                }
                K::Bind => ItemKind::Bind(self.parse_bind()?),
                K::Clocking | K::Default | K::Global => ItemKind::Clocking(self.parse_clocking()?),
                K::Property | K::Sequence => ItemKind::PropertyDecl(self.parse_property_decl()?),
                K::Modport => ItemKind::Modport(self.parse_modports()?),
                K::Timeunit | K::Timeprecision => self.parse_timeunit()?,
                K::Table => self.parse_table()?,
                K::Class | K::Covergroup | K::Checker | K::Config | K::Constraint => {
                    self.skip_unsupported();
                    return Ok(None);
                }
                _ if self.at_gate_keyword() => ItemKind::Gate(self.parse_gate_decl()?),
                _ if self.at_var_decl_start() => ItemKind::Var(self.parse_var_decl()?),
                _ => return Err(self.expected("item")),
            },
            TokenKind::Directive { .. } => ItemKind::Directive(self.parse_directive()?),
            TokenKind::Ident { .. } | TokenKind::EscapedIdent { .. } => {
                if self.nth_is_punct(1, Punct::Colon) {
                    // `label : assert property (...)`.
                    let label = self.expect_ident()?;
                    self.bump();
                    ItemKind::Assertion(self.parse_assertion(Some(label))?)
                } else if expr::assign_op(self.nth(1)).is_some() {
                    // `a = b;` outside any procedure: the usual slip is a
                    // forgotten `assign`, so say so rather than complaining
                    // about the `=`.
                    let span = self.span();
                    return Err(self.error_at(
                        span,
                        "assignment outside a procedural block; use `assign`, \
                         or put it in an `initial` or `always` block",
                    ));
                } else if self.looks_like_instantiation() {
                    ItemKind::Instance(self.parse_instantiation()?)
                } else {
                    ItemKind::Var(self.parse_var_decl()?)
                }
            }
            _ => return Err(self.expected("item")),
        };
        Ok(Some(kind))
    }

    /// Reports and skips a construct outside the supported subset
    /// (`class`, `covergroup`, ...), up to its end keyword.
    fn skip_unsupported(&mut self) {
        let TokenKind::Keyword(kw) = *self.kind() else {
            return;
        };
        let span = self.span();
        self.error_at(span, format!("`{kw}` is not supported"));
        self.bump();
        match kw {
            Keyword::Constraint => {
                // `constraint c { ... }`: skip the brace group.
                while !self.at_eof() && !self.at_punct(Punct::LBrace) {
                    self.bump();
                }
                self.skip_balanced();
            }
            Keyword::Covergroup => {
                self.skip_to_kw(Keyword::Endgroup);
            }
            Keyword::Checker => {
                self.skip_to_kw(Keyword::Endchecker);
            }
            Keyword::Config => {
                self.skip_to_kw(Keyword::Endconfig);
            }
            _ => {
                self.skip_to_kw(Keyword::Endclass);
            }
        }
        self.eat_end_label();
    }

    /// `` `default_nettype wire ``, `` `timescale 1ns / 1ps ``,
    /// `` `resetall ``.
    fn parse_directive(&mut self) -> PResult<super::ast::Directive> {
        let TokenKind::Directive { name } = self.kind() else {
            return Err(self.expected("directive"));
        };
        let name = name.clone();
        let start = self.bump();
        let from = self.pos;
        match name.as_str() {
            "default_nettype" => {
                // One net type keyword or `none`.
                if self.at_ident() || self.kind().is_keyword() {
                    self.bump();
                }
            }
            "timescale" => {
                // `1ns / 1ps` or `10 ps / 1 ps`.
                for _ in 0..2 {
                    if matches!(self.kind(), TokenKind::Number { .. }) {
                        self.bump();
                    }
                    if self.at_ident() {
                        self.bump();
                    }
                    if self.eat_punct(Punct::Slash).is_none() {
                        break;
                    }
                }
            }
            _ => {}
        }
        let args = self.raw_tokens(from, self.pos).text;
        Ok(super::ast::Directive {
            name,
            args,
            span: self.span_from(start),
        })
    }
}

/// Keywords at which item recovery stops.
fn is_item_sync(kind: &TokenKind) -> bool {
    use Keyword as K;
    match kind {
        TokenKind::Keyword(kw) => matches!(
            kw,
            K::Module
                | K::Macromodule
                | K::Endmodule
                | K::Interface
                | K::Endinterface
                | K::Program
                | K::Endprogram
                | K::Package
                | K::Endpackage
                | K::Primitive
                | K::Endprimitive
                | K::Endgenerate
                | K::Endfunction
                | K::Endtask
                | K::Endcase
                | K::End
                | K::Wire
                | K::Reg
                | K::Logic
                | K::Integer
                | K::Parameter
                | K::Localparam
                | K::Input
                | K::Output
                | K::Inout
                | K::Typedef
                | K::Function
                | K::Task
                | K::Initial
                | K::Final
                | K::Always
                | K::AlwaysComb
                | K::AlwaysFf
                | K::AlwaysLatch
                | K::Assign
                | K::Generate
                | K::Genvar
                | K::Import
                | K::Modport
        ),
        TokenKind::Directive { .. } => true,
        _ => false,
    }
}

/// Keywords at which statement recovery stops.
fn is_stmt_sync(kind: &TokenKind) -> bool {
    use Keyword as K;
    match kind {
        TokenKind::Keyword(kw) => matches!(
            kw,
            K::End
                | K::Endcase
                | K::Join
                | K::JoinAny
                | K::JoinNone
                | K::Endfunction
                | K::Endtask
                | K::Endmodule
                | K::Endinterface
                | K::Endprogram
                | K::Endpackage
                | K::Else
                | K::Begin
                | K::If
                | K::Case
                | K::Casez
                | K::Casex
                | K::For
                | K::While
                | K::Repeat
                | K::Forever
                | K::Initial
                | K::Always
                | K::AlwaysComb
                | K::AlwaysFf
                | K::AlwaysLatch
                | K::Assign
                | K::Function
                | K::Task
        ),
        _ => false,
    }
}
