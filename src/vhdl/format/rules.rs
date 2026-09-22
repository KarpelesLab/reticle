//! The VHDL layout rules: one [`Doc`] per syntactic construct.
//!
//! Every rule here turns a piece of [`crate::vhdl::ast`] into a document
//! that says where the line *may* break; [`crate::fmt_doc`] then decides
//! where it *does*. Nothing in this module writes text directly, and
//! nothing looks at the original layout except through two windows:
//!
//! - [`Comments`], which hands back the comments and the blank lines that
//!   the parser threw away, and
//! - [`Comments::raw`], used for identifiers, literals and operator
//!   symbols so that spelling — `16#FF#`, `x"F_F"`, `\Extended Name\` —
//!   survives byte for byte.
//!
//! # Shape of the output
//!
//! - Reserved words are spelled through [`FormatOptions::keyword`]; names
//!   keep the case they were written with, because VHDL is
//!   case-insensitive but people are not.
//! - A declarative or statement part is a sequence: one construct per
//!   line, at most one blank line between two of them, comments in place.
//! - Interface lists (`generic (`, `port (`) and map aspects
//!   (`generic map (`, `port map (`) stay on one line while they fit and
//!   otherwise go one element per line with their `:` and `=>` aligned,
//!   which is what [`FormatOptions::align_port_lists`] controls.
//! - A run of consecutive assignments with nothing between them has its
//!   `<=` / `:=` aligned under [`FormatOptions::align_assignments`].
//! - Parentheses are never added or removed: the tree records them
//!   ([`Expr::Paren`]), so what comes out is what went in.
//! - Optional reserved words that the tree does not record — the `is` of a
//!   process, block or component, the `end for` of a configuration
//!   specification, the `parameter` of a subprogram header — are left out,
//!   and optional end labels are filled in from the tree when
//!   [`FormatOptions::complete_end_labels`] is set.
//!
//! # Deciding to break
//!
//! Most breaking is left to the printer's groups. Alignment cannot be:
//! padding has to be baked into the document before the printer sees it,
//! so the rules that align a column measure the construct flat
//! ([`Doc::flat_width`]) against the width left at the current nesting
//! depth and choose the one-line or the aligned form themselves. The
//! depth is tracked in [`Fmt::depth`], and the decision depends only on
//! the tree, so formatting an already formatted file makes the same
//! choices again.

use super::comments::{Comments, Piece};
use crate::fmt_doc::{Doc, FormatOptions};
use crate::source::Span;
use crate::vhdl::ast::*;
use crate::vhdl::lex::CommentKind;
use crate::vhdl::token::{Token, TokenKind};

// --- small spellings the AST does not provide -------------------------------

/// The reserved word of an interface mode.
fn mode_word(mode: Mode) -> &'static str {
    match mode {
        Mode::In => "in",
        Mode::Out => "out",
        Mode::Inout => "inout",
        Mode::Buffer => "buffer",
        Mode::Linkage => "linkage",
    }
}

/// The reserved word of an interface object class.
fn class_word(class: ObjectClass) -> &'static str {
    match class {
        ObjectClass::Constant => "constant",
        ObjectClass::Signal => "signal",
        ObjectClass::Variable => "variable",
        ObjectClass::File => "file",
    }
}

/// The reserved words introducing an object declaration.
fn object_word(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Constant => "constant",
        ObjectKind::Signal => "signal",
        ObjectKind::Variable => "variable",
        ObjectKind::SharedVariable => "shared variable",
    }
}

/// The reserved word of a guarded signal kind.
fn signal_kind_word(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Register => "register",
        SignalKind::Bus => "bus",
    }
}

/// The reserved word naming a subprogram kind.
fn subprogram_word(kind: SubprogramKind) -> &'static str {
    match kind {
        SubprogramKind::Procedure => "procedure",
        SubprogramKind::Function => "function",
    }
}

/// The reserved word of an external name's class.
fn external_word(class: ExternalClass) -> &'static str {
    match class {
        ExternalClass::Constant => "constant",
        ExternalClass::Signal => "signal",
        ExternalClass::Variable => "variable",
    }
}

/// How far apart the widest and narrowest target of a run of assignments
/// may be before aligning them stops being an improvement.
const MAX_ALIGN_SPREAD: usize = 8;

// --- sequences --------------------------------------------------------------

/// One line-level piece of a declarative or statement part.
struct Entry {
    doc: Doc,
    /// A blank line separated it from the previous entry.
    blank: bool,
}

/// A declarative or statement part under construction.
#[derive(Default)]
struct Seq {
    entries: Vec<Entry>,
}

impl Seq {
    fn push(&mut self, blank: bool, doc: Doc) {
        self.entries.push(Entry { doc, blank });
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The whole part, with no line break before the first entry or after
    /// the last one.
    fn finish(self) -> Doc {
        let mut parts = Vec::new();
        for (i, entry) in self.entries.into_iter().enumerate() {
            if i > 0 {
                parts.push(Doc::hardline());
                if entry.blank {
                    parts.push(Doc::hardline());
                }
            }
            parts.push(entry.doc);
        }
        Doc::concat(parts)
    }
}

// --- the formatter ----------------------------------------------------------

/// Turns a parsed design file into a document.
pub(crate) struct Fmt<'a> {
    opts: &'a FormatOptions,
    cm: Comments<'a>,
    /// The token stream, consulted only to find the `begin` that splits a
    /// declarative part from a statement part; nothing in the tree records
    /// where it is, and a comment on either side of it belongs to that
    /// side.
    tokens: &'a [Token<'a>],
    /// How deep the construct being written is nested, in indentation
    /// levels; only used to guess the column a list starts at.
    depth: usize,
}

impl<'a> Fmt<'a> {
    /// Builds a formatter over `src` and the lexer's comment table.
    pub(crate) fn new(
        src: &'a str,
        comments: &[(Span, CommentKind)],
        tokens: &'a [Token<'a>],
        opts: &'a FormatOptions,
    ) -> Self {
        Fmt {
            opts,
            cm: Comments::new(src, comments),
            tokens,
            depth: 0,
        }
    }

    /// Where the `begin` separating a declarative part from a statement
    /// part is, given the end of the declarations and the start of the
    /// statements. Comments before it belong to the declarations, comments
    /// after it to the statements.
    fn begin_between(&self, from: u32, before: u32) -> u32 {
        self.tokens
            .iter()
            .find(|t| t.kind == TokenKind::Begin && t.span.start >= from && t.span.start < before)
            .map_or(before, |t| t.span.start)
    }

    /// The limit up to which the declarations of a construct own the
    /// comments: its `begin`, or where the statements start.
    fn decls_limit(&self, start: u32, decls: &[Declaration], stmt_start: u32) -> u32 {
        let from = decls.last().map_or(start, |d| d.span().end);
        self.begin_between(from, stmt_start)
    }

    /// The whole file, ending with whatever comments follow the last unit.
    pub(crate) fn design_file(&mut self, file: &DesignFile) -> Doc {
        let mut tops: Vec<Top<'_>> = Vec::new();
        for unit in &file.units {
            for item in &unit.context {
                tops.push(Top::Context(item));
            }
            tops.push(Top::Unit(&unit.unit));
        }

        let mut seq = Seq::default();
        for (i, top) in tops.iter().enumerate() {
            let next = tops.get(i + 1).map_or(u32::MAX, Top::start);
            let span = top.span();
            match top {
                Top::Context(item) => self.entry(&mut seq, span, next, |s| s.context_item(item)),
                Top::Unit(unit) => self.entry(&mut seq, span, next, |s| s.library_unit(unit)),
            }
        }
        self.dangling(&mut seq, u32::MAX);
        seq.finish().append(Doc::hardline())
    }

    // --- helpers ------------------------------------------------------------

    /// A reserved word, spelled as the options ask.
    fn kws(&self, word: &str) -> String {
        self.opts.keyword(word)
    }

    /// A reserved word as a document.
    fn kw(&self, word: &str) -> Doc {
        Doc::text(self.kws(word))
    }

    /// The source text of `span`, for anything whose exact spelling
    /// matters.
    fn raw(&self, span: Span) -> String {
        self.cm.raw(span).to_owned()
    }

    /// An identifier, spelled exactly as written (`\Extended\` included).
    fn ident(&self, id: &Ident) -> String {
        self.raw(id.span)
    }

    /// The extent of a clause, from the reserved word that opens it to the
    /// `;` that closes it, given the extent of its elements.
    ///
    /// Only the elements carry spans in the tree, so the opening keyword is
    /// found in the token stream: it is the last `generic`, `port` or
    /// `parameter` before the first element. Without it, a comment written
    /// between the keyword and the first element would be taken for one
    /// written before the whole clause.
    fn clause_span(&self, elements: Span, keyword: TokenKind) -> Span {
        let start = self
            .tokens
            .iter()
            .rev()
            .find(|t| t.kind == keyword && t.span.start < elements.start)
            .map_or(elements.start, |t| t.span.start);
        Span::new(elements.file, start, self.cm.scan_past(elements.end, b';'))
    }

    /// The column a construct at the current depth starts in.
    fn base(&self) -> usize {
        self.depth * self.opts.indent.width()
    }

    /// True when `width` more columns still fit on a line at this depth.
    fn fits(&self, width: usize) -> bool {
        self.base() + width <= self.opts.line_width
    }

    /// Runs `f` one indentation level deeper.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.depth += 1;
        let out = f(self);
        self.depth -= 1;
        out
    }

    /// A comment as it will be written.
    fn comment(&self, piece: &Piece) -> Doc {
        if piece.is_multiline() {
            Doc::verbatim(piece.text.clone())
        } else {
            Doc::text(piece.text.clone())
        }
    }

    /// Adds one construct to `seq`, with the comments and blank lines that
    /// belong to it. `next` is where the following construct starts, which
    /// bounds the search for a trailing comment.
    fn entry<F>(&mut self, seq: &mut Seq, span: Span, next: u32, build: F)
    where
        F: FnOnce(&mut Self) -> Doc,
    {
        for piece in self.cm.leading(span.start) {
            let blank = piece.blank_before;
            let doc = self.comment(&piece);
            seq.push(blank, doc);
        }
        let blank = self.cm.blank_since(span.start);
        let mut doc = build(self);
        self.cm.advance(span.end);
        if let Some(piece) = self.cm.trailing(span.end, next) {
            let comment = self.comment(&piece);
            doc = Doc::concat([doc, Doc::space(), comment]);
        }
        seq.push(blank, doc);
    }

    /// Adds the comments left over before `limit`, which close a block.
    fn dangling(&mut self, seq: &mut Seq, limit: u32) {
        for piece in self.cm.leading(limit) {
            let blank = piece.blank_before && !seq.is_empty();
            let doc = self.comment(&piece);
            seq.push(blank, doc);
        }
    }

    /// The right-hand side of an operator such as `:=` or `<=`.
    ///
    /// A value that already breaks — an aggregate laid out one element per
    /// line — keeps its opening bracket on the operator's line, which is
    /// what a reader expects; anything else may move to a continuation
    /// line of its own when it does not fit.
    fn after_operator(operator: &str, value: Doc) -> Doc {
        if value.has_hard_break() {
            Doc::concat([Doc::text(format!("{operator} ")), value])
        } else {
            Doc::concat([Doc::text(operator.to_owned()), Doc::line(), value]).indent()
        }
    }

    /// `header`, an indented `body`, and `footer` on its own line.
    fn wrap(header: Doc, body: Doc, footer: Doc) -> Doc {
        let mut parts = vec![header];
        if !body.is_nil() {
            parts.push(Doc::concat([Doc::hardline(), body]).indent());
        }
        parts.push(Doc::hardline());
        parts.push(footer);
        Doc::concat(parts)
    }

    /// The `end ...;` of a construct, with its name repeated when the
    /// options ask for it.
    fn end(&self, words: &[&str], label: Option<&str>) -> Doc {
        let mut text = self.kws("end");
        for word in words {
            text.push(' ');
            text.push_str(&self.kws(word));
        }
        if let Some(label) = label
            && self.opts.complete_end_labels
        {
            text.push(' ');
            text.push_str(label);
        }
        text.push(';');
        Doc::text(text)
    }

    // --- context clauses ----------------------------------------------------

    fn context_item(&mut self, item: &ContextItem) -> Doc {
        match item {
            ContextItem::Library(clause) => {
                let names: Vec<String> = clause.names.iter().map(|n| self.ident(n)).collect();
                Doc::text(format!("{} {};", self.kws("library"), names.join(", ")))
            }
            ContextItem::Use(clause) => self.use_clause(clause),
            ContextItem::Context(clause) => {
                let names: Vec<String> = clause.names.iter().map(|n| self.name_text(n)).collect();
                Doc::text(format!("{} {};", self.kws("context"), names.join(", ")))
            }
        }
    }

    fn use_clause(&self, clause: &UseClause) -> Doc {
        let names: Vec<String> = clause.names.iter().map(|n| self.name_text(n)).collect();
        Doc::text(format!("{} {};", self.kws("use"), names.join(", ")))
    }

    // --- library units ------------------------------------------------------

    fn library_unit(&mut self, unit: &LibraryUnit) -> Doc {
        match unit {
            LibraryUnit::Entity(u) => self.entity(u),
            LibraryUnit::Architecture(u) => self.architecture(u),
            LibraryUnit::Package(u) => self.package(u, true),
            LibraryUnit::PackageBody(u) => self.package_body(u),
            LibraryUnit::PackageInstantiation(u) => self.package_instantiation(u),
            LibraryUnit::Configuration(u) => self.configuration(u),
            LibraryUnit::Context(u) => self.context_decl(u),
        }
    }

    fn entity(&mut self, unit: &EntityDecl) -> Doc {
        let name = self.ident(&unit.name);
        let header = Doc::text(format!(
            "{} {} {}",
            self.kws("entity"),
            name,
            self.kws("is")
        ));
        let stmt_start = unit
            .statements
            .first()
            .map_or(unit.span.end, |s| s.span.start);
        let decls_limit = self.decls_limit(unit.span.start, &unit.decls, stmt_start);
        let (decls, stmts) = self.nested(|s| {
            let mut seq = Seq::default();
            s.clauses(&mut seq, &unit.generics, &unit.ports, decls_limit);
            s.declarations(&mut seq, &unit.decls, decls_limit);
            let decls = seq.finish();
            let mut seq = Seq::default();
            s.concurrent_statements(&mut seq, &unit.statements, unit.span.end);
            (decls, seq.finish())
        });
        let end = self.end(&["entity"], Some(&name));
        self.unit_body(header, decls, stmts, !unit.statements.is_empty(), end)
    }

    fn architecture(&mut self, unit: &ArchitectureBody) -> Doc {
        let name = self.ident(&unit.name);
        let header = Doc::text(format!(
            "{} {} {} {} {}",
            self.kws("architecture"),
            name,
            self.kws("of"),
            self.name_text(&unit.entity),
            self.kws("is")
        ));
        let stmt_start = unit
            .statements
            .first()
            .map_or(unit.span.end, |s| s.span.start);
        let decls_limit = self.decls_limit(unit.span.start, &unit.decls, stmt_start);
        let (decls, stmts) = self.nested(|s| {
            let mut seq = Seq::default();
            s.declarations(&mut seq, &unit.decls, decls_limit);
            let decls = seq.finish();
            let mut seq = Seq::default();
            s.concurrent_statements(&mut seq, &unit.statements, unit.span.end);
            (decls, seq.finish())
        });
        let end = self.end(&["architecture"], Some(&name));
        self.unit_body(header, decls, stmts, true, end)
    }

    /// The common `is ... [begin ...] end` shape.
    fn unit_body(&self, header: Doc, decls: Doc, stmts: Doc, has_begin: bool, end: Doc) -> Doc {
        let mut parts = vec![header];
        if !decls.is_nil() {
            parts.push(Doc::concat([Doc::hardline(), decls]).indent());
        }
        if has_begin {
            parts.push(Doc::hardline());
            parts.push(self.kw("begin"));
            if !stmts.is_nil() {
                parts.push(Doc::concat([Doc::hardline(), stmts]).indent());
            }
        }
        parts.push(Doc::hardline());
        parts.push(end);
        Doc::concat(parts)
    }

    fn package(&mut self, unit: &PackageDecl, top_level: bool) -> Doc {
        let name = self.ident(&unit.name);
        let header = Doc::text(format!(
            "{} {} {}",
            self.kws("package"),
            name,
            self.kws("is")
        ));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            if !unit.generics.is_empty() {
                let span = s.clause_span(interface_span(&unit.generics), TokenKind::Generic);
                s.entry(&mut seq, span, unit.span.end, |s| {
                    s.interface_clause("generic", &unit.generics)
                });
            }
            if let Some(map) = &unit.generic_map
                && !map.is_empty()
            {
                let span = s.clause_span(assoc_span(map, unit.span), TokenKind::Generic);
                s.entry(&mut seq, span, unit.span.end, |s| {
                    let doc = s.map_aspect("generic", map);
                    doc.append(Doc::text(";"))
                });
            }
            s.declarations(&mut seq, &unit.decls, unit.span.end);
            seq.finish()
        });
        let end = self.end(&["package"], Some(&name));
        let _ = top_level;
        Self::wrap(header, body, end)
    }

    fn package_body(&mut self, unit: &PackageBody) -> Doc {
        let name = self.ident(&unit.name);
        let header = Doc::text(format!(
            "{} {} {} {}",
            self.kws("package"),
            self.kws("body"),
            name,
            self.kws("is")
        ));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            s.declarations(&mut seq, &unit.decls, unit.span.end);
            seq.finish()
        });
        let end = self.end(&["package", "body"], Some(&name));
        Self::wrap(header, body, end)
    }

    fn package_instantiation(&mut self, unit: &PackageInstantiation) -> Doc {
        let mut parts = vec![Doc::text(format!(
            "{} {} {} {} {}",
            self.kws("package"),
            self.ident(&unit.name),
            self.kws("is"),
            self.kws("new"),
            self.name_text(&unit.uninstantiated)
        ))];
        if !unit.generic_map.is_empty() {
            let map = self.nested(|s| s.map_aspect("generic", &unit.generic_map));
            parts.push(Doc::concat([Doc::line(), map]).indent());
        }
        parts.push(Doc::text(";"));
        Doc::concat(parts).group()
    }

    fn context_decl(&mut self, unit: &ContextDecl) -> Doc {
        let name = self.ident(&unit.name);
        let header = Doc::text(format!(
            "{} {} {}",
            self.kws("context"),
            name,
            self.kws("is")
        ));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            for (i, item) in unit.items.iter().enumerate() {
                let next = unit
                    .items
                    .get(i + 1)
                    .map_or(unit.span.end, |n| n.span().start);
                s.entry(&mut seq, item.span(), next, |s| s.context_item(item));
            }
            s.dangling(&mut seq, unit.span.end);
            seq.finish()
        });
        let end = self.end(&["context"], Some(&name));
        Self::wrap(header, body, end)
    }

    fn configuration(&mut self, unit: &ConfigurationDecl) -> Doc {
        let name = self.ident(&unit.name);
        let header = Doc::text(format!(
            "{} {} {} {} {}",
            self.kws("configuration"),
            name,
            self.kws("of"),
            self.name_text(&unit.entity),
            self.kws("is")
        ));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            s.declarations(&mut seq, &unit.decls, unit.block.span.start);
            s.entry(&mut seq, unit.block.span, unit.span.end, |s| {
                s.block_configuration(&unit.block)
            });
            s.dangling(&mut seq, unit.span.end);
            seq.finish()
        });
        let end = self.end(&["configuration"], Some(&name));
        Self::wrap(header, body, end)
    }

    fn block_configuration(&mut self, cfg: &BlockConfiguration) -> Doc {
        let header = Doc::text(format!("{} {}", self.kws("for"), self.name_text(&cfg.spec)));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            for clause in &cfg.uses {
                s.entry(&mut seq, clause.span, cfg.span.end, |s| {
                    s.use_clause(clause)
                });
            }
            for (i, item) in cfg.items.iter().enumerate() {
                let next = cfg
                    .items
                    .get(i + 1)
                    .map_or(cfg.span.end, |n| n.span().start);
                s.entry(&mut seq, item.span(), next, |s| match item {
                    ConfigurationItem::Block(b) => s.block_configuration(b),
                    ConfigurationItem::Component(c) => s.component_configuration(c),
                });
            }
            s.dangling(&mut seq, cfg.span.end);
            seq.finish()
        });
        Self::wrap(header, body, self.end(&["for"], None))
    }

    fn component_configuration(&mut self, cfg: &ComponentConfiguration) -> Doc {
        let header = Doc::text(format!(
            "{} {}",
            self.kws("for"),
            self.component_specification(&cfg.spec)
        ));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            if let Some(binding) = &cfg.binding {
                s.entry(&mut seq, binding.span, cfg.span.end, |s| {
                    s.binding_indication(binding).append(Doc::text(";"))
                });
            }
            if let Some(block) = &cfg.block {
                s.entry(&mut seq, block.span, cfg.span.end, |s| {
                    s.block_configuration(block)
                });
            }
            s.dangling(&mut seq, cfg.span.end);
            seq.finish()
        });
        Self::wrap(header, body, self.end(&["for"], None))
    }

    fn component_specification(&self, spec: &ComponentSpecification) -> String {
        let instances = match &spec.instances {
            InstantiationList::Labels(labels) => labels
                .iter()
                .map(|l| self.ident(l))
                .collect::<Vec<_>>()
                .join(", "),
            InstantiationList::Others(_) => self.kws("others"),
            InstantiationList::All(_) => self.kws("all"),
        };
        format!("{instances} : {}", self.name_text(&spec.component))
    }

    fn binding_indication(&mut self, binding: &BindingIndication) -> Doc {
        let mut parts = Vec::new();
        if let Some(aspect) = &binding.entity_aspect {
            let text = match aspect {
                EntityAspect::Entity {
                    name, architecture, ..
                } => {
                    let arch = architecture
                        .as_ref()
                        .map(|a| format!("({})", self.ident(a)))
                        .unwrap_or_default();
                    format!("{} {}{arch}", self.kws("entity"), self.name_text(name))
                }
                EntityAspect::Configuration(name) => {
                    format!("{} {}", self.kws("configuration"), self.name_text(name))
                }
                EntityAspect::Open(_) => self.kws("open"),
            };
            parts.push(Doc::text(format!("{} {text}", self.kws("use"))));
        }
        let mut maps = Vec::new();
        if let Some(map) = &binding.generic_map {
            maps.push(self.nested(|s| s.map_aspect("generic", map)));
        }
        if let Some(map) = &binding.port_map {
            maps.push(self.nested(|s| s.map_aspect("port", map)));
        }
        if parts.is_empty() {
            Doc::join(Doc::line(), maps).group()
        } else {
            let tail = Doc::concat(maps.into_iter().flat_map(|m| [Doc::line(), m])).indent();
            Doc::concat([Doc::concat(parts), tail]).group()
        }
    }

    // --- declarative parts --------------------------------------------------

    /// The `generic (...)` and `port (...)` clauses of an entity or
    /// component.
    fn clauses(
        &mut self,
        seq: &mut Seq,
        generics: &[InterfaceDecl],
        ports: &[InterfaceDecl],
        limit: u32,
    ) {
        if !generics.is_empty() {
            let span = self.clause_span(interface_span(generics), TokenKind::Generic);
            let next = ports
                .first()
                .map_or(limit, |p| p.span().start)
                .max(span.end);
            self.entry(seq, span, next, |s| s.interface_clause("generic", generics));
        }
        if !ports.is_empty() {
            let span = self.clause_span(interface_span(ports), TokenKind::Port);
            self.entry(seq, span, limit.max(span.end), |s| {
                s.interface_clause("port", ports)
            });
        }
    }

    fn declarations(&mut self, seq: &mut Seq, decls: &[Declaration], limit: u32) {
        for (i, decl) in decls.iter().enumerate() {
            let next = decls.get(i + 1).map_or(limit, |n| n.span().start);
            self.entry(seq, decl.span(), next, |s| s.declaration(decl));
        }
        self.dangling(seq, limit);
    }

    fn declaration(&mut self, decl: &Declaration) -> Doc {
        match decl {
            Declaration::Object(d) => self.object_decl(d),
            Declaration::File(d) => self.file_decl(d),
            Declaration::Type(d) => self.type_decl(d),
            Declaration::Subtype(d) => {
                let sub = self.subtype(&d.subtype);
                Doc::concat([
                    Doc::text(format!(
                        "{} {} {} ",
                        self.kws("subtype"),
                        self.ident(&d.name),
                        self.kws("is")
                    )),
                    sub,
                    Doc::text(";"),
                ])
            }
            Declaration::Alias(d) => self.alias_decl(d),
            Declaration::Attribute(d) => Doc::text(format!(
                "{} {} : {};",
                self.kws("attribute"),
                self.ident(&d.name),
                self.name_text(&d.type_mark)
            )),
            Declaration::AttributeSpec(d) => self.attribute_spec(d),
            Declaration::Component(d) => self.component_decl(d),
            Declaration::Subprogram(d) => self.subprogram_spec(&d.spec).append(Doc::text(";")),
            Declaration::SubprogramBody(d) => self.subprogram_body(d),
            Declaration::SubprogramInstantiation(d) => self.subprogram_instantiation(d),
            Declaration::Package(d) => self.package(d, false),
            Declaration::PackageBody(d) => self.package_body(d),
            Declaration::PackageInstantiation(d) => self.package_instantiation(d),
            Declaration::Use(d) => self.use_clause(d),
            Declaration::GroupTemplate(d) => self.group_template(d),
            Declaration::Group(d) => self.group_decl(d),
            Declaration::Disconnection(d) => self.disconnection(d),
            Declaration::ConfigurationSpec(d) => {
                let header = Doc::text(format!(
                    "{} {} ",
                    self.kws("for"),
                    self.component_specification(&d.spec)
                ));
                let binding = self.binding_indication(&d.binding);
                Doc::concat([header, binding, Doc::text(";")])
            }
        }
    }

    fn object_decl(&mut self, decl: &ObjectDecl) -> Doc {
        let names: Vec<String> = decl.names.iter().map(|n| self.ident(n)).collect();
        let mut parts = vec![Doc::text(format!(
            "{} {} : ",
            self.kws(object_word(decl.kind)),
            names.join(", ")
        ))];
        parts.push(self.subtype(&decl.subtype));
        if let Some(kind) = decl.signal_kind {
            parts.push(Doc::text(format!(" {}", self.kws(signal_kind_word(kind)))));
        }
        if let Some(init) = &decl.init {
            let value = self.expr(init);
            parts.push(Self::after_operator(" :=", value));
        }
        parts.push(Doc::text(";"));
        Doc::concat(parts).group()
    }

    fn file_decl(&mut self, decl: &FileDecl) -> Doc {
        let names: Vec<String> = decl.names.iter().map(|n| self.ident(n)).collect();
        let mut parts = vec![Doc::text(format!(
            "{} {} : ",
            self.kws("file"),
            names.join(", ")
        ))];
        parts.push(self.subtype(&decl.subtype));
        if let Some(kind) = &decl.open_kind {
            let kind = self.expr(kind);
            parts.push(Doc::text(format!(" {} ", self.kws("open"))));
            parts.push(kind);
        }
        if let Some(name) = &decl.logical_name {
            parts.push(Doc::text(format!(" {}", self.kws("is"))));
            if let Some(mode) = decl.mode87 {
                parts.push(Doc::text(format!(" {}", self.kws(mode_word(mode)))));
            }
            parts.push(Doc::space());
            let name = self.expr(name);
            parts.push(name);
        }
        parts.push(Doc::text(";"));
        Doc::concat(parts)
    }

    fn alias_decl(&mut self, decl: &AliasDecl) -> Doc {
        let mut text = format!(
            "{} {}",
            self.kws("alias"),
            self.designator(&decl.designator)
        );
        let mut parts = Vec::new();
        if let Some(subtype) = &decl.subtype {
            text.push_str(" : ");
            parts.push(Doc::text(std::mem::take(&mut text)));
            parts.push(self.subtype(subtype));
        }
        text.push_str(&format!(
            " {} {}",
            self.kws("is"),
            self.name_text(&decl.target)
        ));
        if let Some(signature) = &decl.signature {
            text.push_str(&self.signature(signature));
        }
        text.push(';');
        parts.push(Doc::text(text));
        Doc::concat(parts)
    }

    fn attribute_spec(&mut self, spec: &AttributeSpec) -> Doc {
        let entities = match &spec.entities {
            EntityNameList::Names(names) => names
                .iter()
                .map(|n| {
                    let mut text = self.designator(&n.designator);
                    if let Some(sig) = &n.signature {
                        text.push_str(&self.signature(sig));
                    }
                    text
                })
                .collect::<Vec<_>>()
                .join(", "),
            EntityNameList::Others(_) => self.kws("others"),
            EntityNameList::All(_) => self.kws("all"),
        };
        let value = self.expr(&spec.value);
        Doc::concat([
            Doc::text(format!(
                "{} {} {} {entities} : {} {} ",
                self.kws("attribute"),
                self.ident(&spec.attribute),
                self.kws("of"),
                self.kws(spec.class.as_str()),
                self.kws("is")
            )),
            value,
            Doc::text(";"),
        ])
        .group()
    }

    fn component_decl(&mut self, decl: &ComponentDecl) -> Doc {
        let name = self.ident(&decl.name);
        let header = Doc::text(format!("{} {name}", self.kws("component")));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            s.clauses(&mut seq, &decl.generics, &decl.ports, decl.span.end);
            s.dangling(&mut seq, decl.span.end);
            seq.finish()
        });
        Self::wrap(header, body, self.end(&["component"], Some(&name)))
    }

    fn group_template(&self, decl: &GroupTemplateDecl) -> Doc {
        let entries: Vec<String> = decl
            .entries
            .iter()
            .map(|e| {
                let class = self.kws(e.class.as_str());
                if e.unbounded {
                    format!("{class} <>")
                } else {
                    class
                }
            })
            .collect();
        Doc::text(format!(
            "{} {} {} ({});",
            self.kws("group"),
            self.ident(&decl.name),
            self.kws("is"),
            entries.join(", ")
        ))
    }

    fn group_decl(&mut self, decl: &GroupDecl) -> Doc {
        let items: Vec<Doc> = decl.constituents.iter().map(|e| self.expr(e)).collect();
        Doc::concat([
            Doc::text(format!(
                "{} {} : {} ",
                self.kws("group"),
                self.ident(&decl.name),
                self.name_text(&decl.template)
            )),
            Self::args(items),
            Doc::text(";"),
        ])
    }

    fn disconnection(&mut self, spec: &DisconnectionSpec) -> Doc {
        let signals = match &spec.signals {
            SignalList::Names(names) => names
                .iter()
                .map(|n| self.name_text(n))
                .collect::<Vec<_>>()
                .join(", "),
            SignalList::Others(_) => self.kws("others"),
            SignalList::All(_) => self.kws("all"),
        };
        let time = self.expr(&spec.time);
        Doc::concat([
            Doc::text(format!(
                "{} {signals} : {} {} ",
                self.kws("disconnect"),
                self.name_text(&spec.type_mark),
                self.kws("after")
            )),
            time,
            Doc::text(";"),
        ])
    }

    // --- types --------------------------------------------------------------

    fn type_decl(&mut self, decl: &TypeDecl) -> Doc {
        let name = self.ident(&decl.name);
        let Some(def) = &decl.def else {
            return Doc::text(format!("{} {name};", self.kws("type")));
        };
        let header = format!("{} {name} {} ", self.kws("type"), self.kws("is"));
        match def {
            TypeDef::Enumeration(literals) => {
                let items: Vec<Doc> = literals
                    .iter()
                    .map(|d| Doc::text(self.designator(d)))
                    .collect();
                Doc::concat([Doc::text(header), Self::args(items), Doc::text(";")])
            }
            TypeDef::Range(range) => {
                let range = self.range(range);
                Doc::concat([
                    Doc::text(format!("{header}{} ", self.kws("range"))),
                    range,
                    Doc::text(";"),
                ])
            }
            TypeDef::Physical(def) => self.physical_type(&header, &name, def),
            TypeDef::Array(def) => {
                let indices: Vec<Doc> = def
                    .indices
                    .iter()
                    .map(|i| match i {
                        ArrayIndex::Unbounded(mark) => {
                            Doc::text(format!("{} {} <>", self.name_text(mark), self.kws("range")))
                        }
                        ArrayIndex::Constrained(range) => self.discrete_range(range),
                    })
                    .collect();
                let element = self.subtype(&def.element);
                Doc::concat([
                    Doc::text(format!("{header}{} ", self.kws("array"))),
                    Self::args(indices),
                    Doc::text(format!(" {} ", self.kws("of"))),
                    element,
                    Doc::text(";"),
                ])
            }
            TypeDef::Record(def) => {
                let width = if self.opts.align_port_lists {
                    def.elements
                        .iter()
                        .map(|e| self.record_names(e).chars().count())
                        .max()
                        .unwrap_or(0)
                } else {
                    0
                };
                let body = self.nested(|s| {
                    let mut seq = Seq::default();
                    for (i, element) in def.elements.iter().enumerate() {
                        let next = def
                            .elements
                            .get(i + 1)
                            .map_or(def.span.end, |n| n.span.start);
                        s.entry(&mut seq, element.span, next, |s| {
                            s.record_element(element, width)
                        });
                    }
                    s.dangling(&mut seq, def.span.end);
                    seq.finish()
                });
                Self::wrap(
                    Doc::text(format!("{header}{}", self.kws("record"))),
                    body,
                    self.end(&["record"], Some(&name)),
                )
            }
            TypeDef::Access(subtype) => {
                let subtype = self.subtype(subtype);
                Doc::concat([
                    Doc::text(format!("{header}{} ", self.kws("access"))),
                    subtype,
                    Doc::text(";"),
                ])
            }
            TypeDef::File(mark) => Doc::text(format!(
                "{header}{} {} {};",
                self.kws("file"),
                self.kws("of"),
                self.name_text(mark)
            )),
            TypeDef::Protected(def) => {
                let body = self.nested(|s| {
                    let mut seq = Seq::default();
                    s.declarations(&mut seq, &def.decls, def.span.end);
                    seq.finish()
                });
                Self::wrap(
                    Doc::text(format!("{header}{}", self.kws("protected"))),
                    body,
                    self.end(&["protected"], Some(&name)),
                )
            }
            TypeDef::ProtectedBody(def) => {
                let body = self.nested(|s| {
                    let mut seq = Seq::default();
                    s.declarations(&mut seq, &def.decls, def.span.end);
                    seq.finish()
                });
                Self::wrap(
                    Doc::text(format!(
                        "{header}{} {}",
                        self.kws("protected"),
                        self.kws("body")
                    )),
                    body,
                    self.end(&["protected", "body"], Some(&name)),
                )
            }
        }
    }

    /// The declared names of a record element, which form its first
    /// column.
    fn record_names(&self, element: &RecordElement) -> String {
        element
            .names
            .iter()
            .map(|n| self.ident(n))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn record_element(&mut self, element: &RecordElement, width: usize) -> Doc {
        let names = self.record_names(element);
        let pad = " ".repeat(width.saturating_sub(names.chars().count()));
        let subtype = self.subtype(&element.subtype);
        Doc::concat([
            Doc::text(format!("{names}{pad} : ")),
            subtype,
            Doc::text(";"),
        ])
    }

    fn physical_type(&mut self, header: &str, name: &str, def: &PhysicalTypeDef) -> Doc {
        let range = self.range(&def.range);
        let primary = self.ident(&def.primary_unit);
        let units = self.nested(|s| {
            let mut seq = Seq::default();
            seq.push(false, Doc::text(format!("{primary};")));
            s.cm.advance(def.primary_unit.span.end);
            for (i, unit) in def.secondary_units.iter().enumerate() {
                let next = def
                    .secondary_units
                    .get(i + 1)
                    .map_or(def.span.end, |n| n.span.start);
                s.entry(&mut seq, unit.span, next, |s| {
                    let value = s.expr(&unit.value);
                    Doc::concat([
                        Doc::text(format!("{} = ", s.ident(&unit.name))),
                        value,
                        Doc::text(";"),
                    ])
                });
            }
            s.dangling(&mut seq, def.span.end);
            seq.finish()
        });
        let inner = Self::wrap(self.kw("units"), units, self.end(&["units"], Some(name)));
        Doc::concat([
            Doc::text(format!("{header}{} ", self.kws("range"))),
            range,
            Doc::concat([Doc::hardline(), inner]).indent(),
        ])
    }

    // --- subprograms --------------------------------------------------------

    fn subprogram_spec(&mut self, spec: &SubprogramSpec) -> Doc {
        let mut head = String::new();
        if let Some(pure) = spec.pure {
            head.push_str(&self.kws(if pure { "pure" } else { "impure" }));
            head.push(' ');
        }
        head.push_str(&self.kws(subprogram_word(spec.kind)));
        head.push(' ');
        head.push_str(&self.designator(&spec.designator));

        let mut parts = vec![Doc::text(head)];
        let mut tail = Vec::new();
        if !spec.generics.is_empty() {
            tail.push(Doc::line());
            let clause = self.nested(|s| s.interface_list("generic", &spec.generics));
            tail.push(clause);
        }
        if let Some(map) = &spec.generic_map {
            tail.push(Doc::line());
            let map = self.nested(|s| s.map_aspect("generic", map));
            tail.push(map);
        }
        if !spec.params.is_empty() {
            if tail.is_empty() {
                let list = self.nested(|s| s.interface_list("", &spec.params));
                parts.push(list);
            } else {
                // After a generic clause the optional `parameter` keyword
                // is what tells a reader (and the parser) that a second
                // parenthesised list is the parameter list.
                tail.push(Doc::line());
                let list = self.nested(|s| s.interface_list("parameter", &spec.params));
                tail.push(list);
            }
        }
        if !tail.is_empty() {
            parts.push(Doc::concat(tail).indent());
        }
        if let Some(ret) = &spec.return_type {
            parts.push(Doc::text(format!(
                " {} {}",
                self.kws("return"),
                self.name_text(ret)
            )));
        }
        Doc::concat(parts).group()
    }

    fn subprogram_body(&mut self, body: &SubprogramBody) -> Doc {
        let spec = self.subprogram_spec(&body.spec);
        let header = spec.append(Doc::text(format!(" {}", self.kws("is"))));
        let stmt_start = body
            .statements
            .first()
            .map_or(body.span.end, |s| s.span.start);
        let decls_limit = self.decls_limit(body.span.start, &body.decls, stmt_start);
        let (decls, stmts) = self.nested(|s| {
            let mut seq = Seq::default();
            s.declarations(&mut seq, &body.decls, decls_limit);
            let decls = seq.finish();
            let mut seq = Seq::default();
            s.sequential_statements(&mut seq, &body.statements, body.span.end);
            (decls, seq.finish())
        });
        let name = self.designator(&body.spec.designator);
        let end = self.end(&[subprogram_word(body.spec.kind)], Some(&name));
        self.unit_body(header, decls, stmts, true, end)
    }

    fn subprogram_instantiation(&mut self, decl: &SubprogramInstantiation) -> Doc {
        let mut head = format!(
            "{} {} {} {} {}",
            self.kws(subprogram_word(decl.kind)),
            self.designator(&decl.designator),
            self.kws("is"),
            self.kws("new"),
            self.name_text(&decl.uninstantiated)
        );
        if let Some(signature) = &decl.signature {
            head.push(' ');
            head.push_str(&self.signature(signature));
        }
        let mut parts = vec![Doc::text(head)];
        if !decl.generic_map.is_empty() {
            let map = self.nested(|s| s.map_aspect("generic", &decl.generic_map));
            parts.push(Doc::concat([Doc::line(), map]).indent());
        }
        parts.push(Doc::text(";"));
        Doc::concat(parts).group()
    }

    // --- interface and association lists ------------------------------------

    /// `generic (...)` or `port (...)`, terminated by `;`.
    fn interface_clause(&mut self, word: &str, items: &[InterfaceDecl]) -> Doc {
        self.interface_list(word, items).append(Doc::text(";"))
    }

    /// `word (...)`, or just `(...)` when `word` is empty.
    fn interface_list(&mut self, word: &str, items: &[InterfaceDecl]) -> Doc {
        let open = if word.is_empty() {
            "(".to_owned()
        } else {
            format!("{} (", self.kws(word))
        };
        let rows: Vec<Row> = items.iter().map(|i| self.interface_row(i)).collect();
        let spans: Vec<Span> = items.iter().map(InterfaceDecl::span).collect();
        // One column is kept for the `;` the caller appends.
        self.list(&open, rows, &spans, ";", ")", 1)
    }

    /// `generic map (...)` or `port map (...)`.
    fn map_aspect(&mut self, word: &str, args: &[AssociationElement]) -> Doc {
        let open = format!("{} {} (", self.kws(word), self.kws("map"));
        let rows: Vec<Row> = args.iter().map(|a| self.assoc_row(a)).collect();
        let spans: Vec<Span> = args.iter().map(|a| a.span).collect();
        self.list(&open, rows, &spans, ",", ")", 1)
    }

    /// A parenthesised list whose elements have spans, so that a comment
    /// written among them stays among them.
    ///
    /// A list with no comments inside is laid out by [`Fmt::rows_to_doc`],
    /// which may keep it on one line. One with comments is always broken,
    /// because a comment owns its line.
    fn list(
        &mut self,
        open: &str,
        rows: Vec<Row>,
        spans: &[Span],
        sep: &str,
        close: &str,
        reserve: usize,
    ) -> Doc {
        let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
            return self.rows_to_doc(open, rows, sep, close, reserve);
        };
        let limit = self.cm.scan_past(last.end, b')');
        if !self.cm.any_between(first.start, limit) {
            return self.rows_to_doc(open, rows, sep, close, reserve);
        }

        let (left, mid) = if self.opts.align_port_lists {
            (
                rows.iter().map(Row::left_width).max().unwrap_or(0),
                rows.iter().map(Row::mid_width).max().unwrap_or(0),
            )
        } else {
            (0, 0)
        };
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            for (i, (row, span)) in rows.iter().zip(spans).enumerate() {
                let next = spans.get(i + 1).map_or(limit, |n| n.start);
                let mut doc = row.aligned(left, mid);
                if i + 1 < rows.len() {
                    doc = doc.append(Doc::text(sep));
                }
                s.entry(&mut seq, *span, next, |_| doc);
            }
            s.dangling(&mut seq, limit);
            seq.finish()
        });
        Doc::concat([
            Doc::text(open.to_owned()),
            Doc::concat([Doc::hardline(), body]).indent(),
            Doc::hardline(),
            Doc::text(close),
        ])
    }

    /// Lays a list out on one line when it fits, and one element per line
    /// with its columns aligned when it does not.
    fn rows_to_doc(
        &self,
        open: &str,
        rows: Vec<Row>,
        sep: &str,
        close: &str,
        reserve: usize,
    ) -> Doc {
        if rows.is_empty() {
            return Doc::text(format!("{open}{close}"));
        }
        let flat: Vec<Doc> = rows.iter().map(Row::flat).collect();
        let width: usize = flat.iter().map(Doc::flat_width).sum::<usize>()
            + (flat.len() - 1) * (sep.len() + 1)
            + open.chars().count()
            + close.len()
            + reserve;
        if self.fits(width) {
            let joined = Doc::join(Doc::text(format!("{sep} ")), flat);
            return Doc::concat([Doc::text(open.to_owned()), joined, Doc::text(close)]);
        }

        let align = self.opts.align_port_lists;
        let left = if align {
            rows.iter().map(Row::left_width).max().unwrap_or(0)
        } else {
            0
        };
        let mid = if align {
            rows.iter().map(Row::mid_width).max().unwrap_or(0)
        } else {
            0
        };
        let body: Vec<Doc> = rows.iter().map(|r| r.aligned(left, mid)).collect();
        let joined = Doc::join(Doc::concat([Doc::text(sep), Doc::hardline()]), body);
        Doc::concat([
            Doc::text(open.to_owned()),
            Doc::concat([Doc::hardline(), joined]).indent(),
            Doc::hardline(),
            Doc::text(close),
        ])
    }

    fn interface_row(&mut self, decl: &InterfaceDecl) -> Row {
        match decl {
            InterfaceDecl::Object(obj) => {
                let mut left = String::new();
                if let Some(class) = obj.class {
                    left.push_str(&self.kws(class_word(class)));
                    left.push(' ');
                }
                let names: Vec<String> = obj.names.iter().map(|n| self.ident(n)).collect();
                left.push_str(&names.join(", "));
                let mid = obj.mode.map(|m| self.kws(mode_word(m))).unwrap_or_default();
                let mut right = vec![self.subtype(&obj.subtype)];
                if obj.bus {
                    right.push(Doc::text(format!(" {}", self.kws("bus"))));
                }
                if let Some(default) = &obj.default {
                    let value = self.expr(default);
                    right.push(Doc::text(" := "));
                    right.push(value);
                }
                Row::Object {
                    left,
                    mid,
                    right: Doc::concat(right),
                }
            }
            InterfaceDecl::Type(t) => Row::Plain(Doc::text(format!(
                "{} {}",
                self.kws("type"),
                self.ident(&t.name)
            ))),
            InterfaceDecl::Subprogram(s) => {
                let spec = self.subprogram_spec(&s.spec);
                let default = match &s.default {
                    Some(SubprogramDefault::Box(_)) => Doc::text(format!(" {} <>", self.kws("is"))),
                    Some(SubprogramDefault::Name(n)) => {
                        Doc::text(format!(" {} {}", self.kws("is"), self.name_text(n)))
                    }
                    None => Doc::nil(),
                };
                Row::Plain(spec.append(default))
            }
            InterfaceDecl::Package(p) => {
                let map = match &p.generic_map {
                    InterfacePackageMap::Box(_) => {
                        Doc::text(format!("{} {} (<>)", self.kws("generic"), self.kws("map")))
                    }
                    InterfacePackageMap::Default(_) => Doc::text(format!(
                        "{} {} ({})",
                        self.kws("generic"),
                        self.kws("map"),
                        self.kws("default")
                    )),
                    InterfacePackageMap::Map(args) => self.map_aspect("generic", args),
                };
                Row::Plain(Doc::concat([
                    Doc::text(format!(
                        "{} {} {} {} {} ",
                        self.kws("package"),
                        self.ident(&p.name),
                        self.kws("is"),
                        self.kws("new"),
                        self.name_text(&p.uninstantiated)
                    )),
                    map,
                ]))
            }
        }
    }

    fn assoc_row(&mut self, element: &AssociationElement) -> Row {
        let actual = self.actual(&element.actual);
        match &element.formal {
            Some(formal) => Row::Assoc {
                left: self.expr(formal).flat_text(),
                right: actual,
            },
            None => Row::Plain(actual),
        }
    }

    fn actual(&mut self, actual: &Actual) -> Doc {
        match actual {
            Actual::Expr(e) => self.expr(e),
            Actual::Open(_) => self.kw("open"),
            Actual::Inertial(e) => {
                let e = self.expr(e);
                Doc::concat([Doc::text(format!("{} ", self.kws("inertial"))), e])
            }
            Actual::Range(r) => self.discrete_range(r),
        }
    }

    // --- concurrent statements ----------------------------------------------

    fn concurrent_statements(&mut self, seq: &mut Seq, stmts: &[ConcurrentStatement], limit: u32) {
        let pads = self.concurrent_pads(stmts);
        for (i, stmt) in stmts.iter().enumerate() {
            let next = stmts.get(i + 1).map_or(limit, |n| n.span.start);
            let pad = pads[i];
            self.entry(seq, stmt.span, next, |s| s.concurrent_statement(stmt, pad));
        }
        self.dangling(seq, limit);
    }

    /// How much to pad each statement's target so that a run of plain
    /// assignments lines its `<=` up.
    fn concurrent_pads(&self, stmts: &[ConcurrentStatement]) -> Vec<usize> {
        let keys: Vec<Option<String>> = stmts
            .iter()
            .map(|s| match &s.kind {
                ConcurrentKind::SignalAssignment(a)
                    if !a.postponed
                        && !a.guarded
                        && !matches!(a.assignment.rhs, SignalAssignmentRhs::Selected { .. }) =>
                {
                    Some(self.assignment_key(s.label.as_ref(), &a.assignment.target))
                }
                _ => None,
            })
            .collect();
        self.pads(&keys, stmts.iter().map(|s| s.span).collect::<Vec<_>>())
    }

    fn assignment_key(&self, label: Option<&Ident>, target: &Target) -> String {
        let mut text = String::new();
        if let Some(label) = label {
            text.push_str(&self.ident(label));
            text.push_str(" : ");
        }
        text.push_str(&self.target(target).flat_text());
        text
    }

    /// Widths to pad each key to, grouping adjacent keys into runs that
    /// nothing — no blank line, no comment — separates.
    fn pads(&self, keys: &[Option<String>], spans: Vec<Span>) -> Vec<usize> {
        let mut pads = vec![0; keys.len()];
        if !self.opts.align_assignments {
            return pads;
        }
        let mut i = 0;
        while i < keys.len() {
            if keys[i].is_none() {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < keys.len()
                && keys[j].is_some()
                && !self.cm.blank_between(spans[j - 1].end, spans[j].start)
                && !self.cm.any_between(spans[j - 1].end, spans[j].start)
            {
                j += 1;
            }
            if j - i > 1 {
                let widths = || {
                    keys[i..j]
                        .iter()
                        .map(|k| k.as_ref().map_or(0, |k| k.chars().count()))
                };
                let width = widths().max().unwrap_or(0);
                let narrowest = widths().min().unwrap_or(0);
                // One very long target would push every other line of the
                // run far to the right; past this spread, leave the run
                // alone rather than make it harder to read.
                if width - narrowest <= MAX_ALIGN_SPREAD {
                    for (k, pad) in pads.iter_mut().enumerate().take(j).skip(i) {
                        *pad = width - keys[k].as_ref().map_or(0, |k| k.chars().count());
                    }
                }
            }
            i = j;
        }
        pads
    }

    fn concurrent_statement(&mut self, stmt: &ConcurrentStatement, pad: usize) -> Doc {
        let label = stmt.label.as_ref().map(|l| self.ident(l));
        let prefix = label
            .as_ref()
            .map(|l| Doc::text(format!("{l} : ")))
            .unwrap_or_default();
        match &stmt.kind {
            ConcurrentKind::Process(process) => {
                let body = self.process(process, label.as_deref());
                prefix.append(body)
            }
            ConcurrentKind::Block(block) => {
                let body = self.block(block, label.as_deref());
                prefix.append(body)
            }
            ConcurrentKind::SignalAssignment(assign) => {
                let mut head = Vec::new();
                if assign.postponed {
                    head.push(Doc::text(format!("{} ", self.kws("postponed"))));
                }
                let body = self.signal_assignment(
                    &assign.assignment,
                    assign.guarded,
                    pad,
                    label.is_some(),
                );
                Doc::concat([prefix, Doc::concat(head), body, Doc::text(";")])
            }
            ConcurrentKind::ProcedureCall { postponed, call } => {
                let mut text = String::new();
                if *postponed {
                    text.push_str(&self.kws("postponed"));
                    text.push(' ');
                }
                text.push_str(&self.name_text(call));
                text.push(';');
                prefix.append(Doc::text(text))
            }
            ConcurrentKind::Assertion {
                postponed,
                assertion,
            } => {
                let mut head = prefix;
                if *postponed {
                    head = head.append(Doc::text(format!("{} ", self.kws("postponed"))));
                }
                let body = self.assertion(assertion);
                Doc::concat([head, body, Doc::text(";")])
            }
            ConcurrentKind::Instantiation(inst) => {
                let body = self.instantiation(inst);
                prefix.append(body)
            }
            ConcurrentKind::ForGenerate(gene) => {
                let header = {
                    let range = self.discrete_range(&gene.range);
                    Doc::concat([
                        Doc::text(format!(
                            "{} {} {} ",
                            self.kws("for"),
                            self.ident(&gene.param),
                            self.kws("in")
                        )),
                        range,
                        Doc::text(format!(" {}", self.kws("generate"))),
                    ])
                };
                let body = self.generate_body(&gene.body, gene.span.end);
                let end = self.end(&["generate"], label.as_deref());
                Doc::concat([prefix, header, body, Doc::hardline(), end])
            }
            ConcurrentKind::IfGenerate(gene) => {
                let mut parts = vec![prefix];
                for (i, arm) in gene.arms.iter().enumerate() {
                    let word = if i == 0 { "if" } else { "elsif" };
                    let cond = self.expr(&arm.condition);
                    let mut header = vec![Doc::text(format!("{} ", self.kws(word)))];
                    if let Some(alt) = &arm.body.label {
                        header.push(Doc::text(format!("{} : ", self.ident(alt))));
                    }
                    header.push(cond);
                    header.push(Doc::text(format!(" {}", self.kws("generate"))));
                    let limit = gene.arms.get(i + 1).map_or(gene.span.end, |a| a.span.start);
                    let body = self.generate_body(&arm.body, limit);
                    if i > 0 {
                        parts.push(Doc::hardline());
                    }
                    parts.push(Doc::concat([Doc::concat(header), body]));
                }
                if let Some(arm) = &gene.else_arm {
                    let mut header = vec![Doc::text(self.kws("else"))];
                    if let Some(alt) = &arm.label {
                        header.push(Doc::text(format!(" {} :", self.ident(alt))));
                    }
                    header.push(Doc::text(format!(" {}", self.kws("generate"))));
                    let body = self.generate_body(arm, gene.span.end);
                    parts.push(Doc::hardline());
                    parts.push(Doc::concat([Doc::concat(header), body]));
                }
                parts.push(Doc::hardline());
                parts.push(self.end(&["generate"], label.as_deref()));
                Doc::concat(parts)
            }
            ConcurrentKind::CaseGenerate(gene) => {
                let selector = self.expr(&gene.expr);
                let header = Doc::concat([
                    Doc::text(format!("{} ", self.kws("case"))),
                    selector,
                    Doc::text(format!(" {}", self.kws("generate"))),
                ]);
                let arms = self.nested(|s| {
                    let mut parts = Vec::new();
                    for (i, arm) in gene.arms.iter().enumerate() {
                        if i > 0 {
                            parts.push(Doc::hardline());
                        }
                        let mut head = vec![Doc::text(format!("{} ", s.kws("when")))];
                        if let Some(alt) = &arm.body.label {
                            head.push(Doc::text(format!("{} : ", s.ident(alt))));
                        }
                        head.push(Doc::text(s.choices(&arm.choices)));
                        head.push(Doc::text(" =>"));
                        let limit = gene.arms.get(i + 1).map_or(gene.span.end, |a| a.span.start);
                        let body = s.generate_body(&arm.body, limit);
                        parts.push(Doc::concat([Doc::concat(head), body]));
                    }
                    Doc::concat(parts)
                });
                let end = self.end(&["generate"], label.as_deref());
                Doc::concat([prefix, Self::wrap(header, arms, end)])
            }
        }
    }

    /// Everything a generate body contributes after its header: the
    /// declarations indented, the `begin` that introduces the statements
    /// back at the header's own level, and the statements indented again.
    fn generate_body(&mut self, body: &GenerateBody, limit: u32) -> Doc {
        let stmt_start = body.statements.first().map_or(limit, |s| s.span.start);
        let decls_limit = self.decls_limit(
            body.decls.first().map_or(stmt_start, |d| d.span().start),
            &body.decls,
            stmt_start,
        );
        let (decls, stmts) = self.nested(|s| {
            let mut seq = Seq::default();
            s.declarations(&mut seq, &body.decls, decls_limit);
            let decls = seq.finish();
            let mut seq = Seq::default();
            s.concurrent_statements(&mut seq, &body.statements, limit);
            (decls, seq.finish())
        });
        let mut parts = Vec::new();
        if !decls.is_nil() {
            parts.push(Doc::concat([Doc::hardline(), decls]).indent());
            parts.push(Doc::hardline());
            parts.push(self.kw("begin"));
        }
        if !stmts.is_nil() {
            parts.push(Doc::concat([Doc::hardline(), stmts]).indent());
        }
        Doc::concat(parts)
    }

    fn process(&mut self, process: &ProcessStatement, label: Option<&str>) -> Doc {
        let mut header = Vec::new();
        if process.postponed {
            header.push(Doc::text(format!("{} ", self.kws("postponed"))));
        }
        header.push(self.kw("process"));
        if let Some(sensitivity) = &process.sensitivity {
            header.push(Doc::text(format!(" ({})", self.sensitivity(sensitivity))));
        }
        let stmt_start = process
            .statements
            .first()
            .map_or(process.span.end, |s| s.span.start);
        let decls_limit = self.decls_limit(process.span.start, &process.decls, stmt_start);
        let (decls, stmts) = self.nested(|s| {
            let mut seq = Seq::default();
            s.declarations(&mut seq, &process.decls, decls_limit);
            let decls = seq.finish();
            let mut seq = Seq::default();
            s.sequential_statements(&mut seq, &process.statements, process.span.end);
            (decls, seq.finish())
        });
        let end = self.end(&["process"], label);
        self.unit_body(Doc::concat(header), decls, stmts, true, end)
    }

    fn sensitivity(&self, sensitivity: &Sensitivity) -> String {
        match sensitivity {
            Sensitivity::Names(names) => names
                .iter()
                .map(|n| self.name_text(n))
                .collect::<Vec<_>>()
                .join(", "),
            Sensitivity::All(_) => self.kws("all"),
        }
    }

    fn block(&mut self, block: &BlockStatement, label: Option<&str>) -> Doc {
        let mut header = vec![self.kw("block")];
        if let Some(guard) = &block.guard {
            let guard = self.expr(guard);
            header.push(Doc::text(" ("));
            header.push(guard);
            header.push(Doc::text(")"));
        }
        let stmt_start = block
            .statements
            .first()
            .map_or(block.span.end, |s| s.span.start);
        let decls_limit = self.decls_limit(block.span.start, &block.decls, stmt_start);
        let (decls, stmts) = self.nested(|s| {
            let mut seq = Seq::default();
            if !block.generics.is_empty() {
                let span = s.clause_span(interface_span(&block.generics), TokenKind::Generic);
                s.entry(&mut seq, span, decls_limit, |s| {
                    s.interface_clause("generic", &block.generics)
                });
            }
            if let Some(map) = &block.generic_map {
                let span = s.clause_span(assoc_span(map, block.span), TokenKind::Generic);
                s.entry(&mut seq, span, decls_limit, |s| {
                    s.map_aspect("generic", map).append(Doc::text(";"))
                });
            }
            if !block.ports.is_empty() {
                let span = s.clause_span(interface_span(&block.ports), TokenKind::Port);
                s.entry(&mut seq, span, decls_limit, |s| {
                    s.interface_clause("port", &block.ports)
                });
            }
            if let Some(map) = &block.port_map {
                let span = s.clause_span(assoc_span(map, block.span), TokenKind::Port);
                s.entry(&mut seq, span, decls_limit, |s| {
                    s.map_aspect("port", map).append(Doc::text(";"))
                });
            }
            s.declarations(&mut seq, &block.decls, decls_limit);
            let decls = seq.finish();
            let mut seq = Seq::default();
            s.concurrent_statements(&mut seq, &block.statements, block.span.end);
            (decls, seq.finish())
        });
        let end = self.end(&["block"], label);
        self.unit_body(Doc::concat(header), decls, stmts, true, end)
    }

    fn instantiation(&mut self, inst: &Instantiation) -> Doc {
        let unit = match &inst.unit {
            InstantiatedUnit::Component(name) => self.name_text(name),
            InstantiatedUnit::Entity { name, architecture } => {
                let arch = architecture
                    .as_ref()
                    .map(|a| format!("({})", self.ident(a)))
                    .unwrap_or_default();
                format!("{} {}{arch}", self.kws("entity"), self.name_text(name))
            }
            InstantiatedUnit::Configuration(name) => {
                format!("{} {}", self.kws("configuration"), self.name_text(name))
            }
        };
        let mut maps = Vec::new();
        if let Some(map) = &inst.generic_map {
            maps.push(self.nested(|s| s.map_aspect("generic", map)));
        }
        if let Some(map) = &inst.port_map {
            maps.push(self.nested(|s| s.map_aspect("port", map)));
        }
        let tail = Doc::concat(maps.into_iter().flat_map(|m| [Doc::line(), m])).indent();
        Doc::concat([Doc::text(unit), tail, Doc::text(";")]).group()
    }

    fn assertion(&mut self, assertion: &Assertion) -> Doc {
        let condition = self.expr(&assertion.condition);
        let mut parts = vec![Doc::text(format!("{} ", self.kws("assert"))), condition];
        let mut tail = Vec::new();
        if let Some(report) = &assertion.report {
            let report = self.expr(report);
            tail.push(Doc::line());
            tail.push(Doc::concat([
                Doc::text(format!("{} ", self.kws("report"))),
                report,
            ]));
        }
        if let Some(severity) = &assertion.severity {
            let severity = self.expr(severity);
            tail.push(Doc::line());
            tail.push(Doc::concat([
                Doc::text(format!("{} ", self.kws("severity"))),
                severity,
            ]));
        }
        if !tail.is_empty() {
            parts.push(Doc::concat(tail).indent());
        }
        Doc::concat(parts).group()
    }

    // --- sequential statements ----------------------------------------------

    fn sequential_statements(&mut self, seq: &mut Seq, stmts: &[SequentialStatement], limit: u32) {
        let pads = self.sequential_pads(stmts);
        for (i, stmt) in stmts.iter().enumerate() {
            let next = stmts.get(i + 1).map_or(limit, |n| n.span.start);
            let pad = pads[i];
            self.entry(seq, stmt.span, next, |s| s.sequential_statement(stmt, pad));
        }
        self.dangling(seq, limit);
    }

    fn sequential_pads(&self, stmts: &[SequentialStatement]) -> Vec<usize> {
        let keys: Vec<Option<String>> = stmts
            .iter()
            .map(|s| match &s.kind {
                SequentialKind::SignalAssignment(a)
                    if !matches!(a.rhs, SignalAssignmentRhs::Selected { .. }) =>
                {
                    Some(self.assignment_key(s.label.as_ref(), &a.target))
                }
                SequentialKind::VariableAssignment(a)
                    if !matches!(a.rhs, VariableAssignmentRhs::Selected { .. }) =>
                {
                    Some(self.assignment_key(s.label.as_ref(), &a.target))
                }
                _ => None,
            })
            .collect();
        self.pads(&keys, stmts.iter().map(|s| s.span).collect::<Vec<_>>())
    }

    fn sequential_statement(&mut self, stmt: &SequentialStatement, pad: usize) -> Doc {
        let label = stmt.label.as_ref().map(|l| self.ident(l));
        let prefix = label
            .as_ref()
            .map(|l| Doc::text(format!("{l} : ")))
            .unwrap_or_default();
        match &stmt.kind {
            SequentialKind::Wait {
                sensitivity,
                condition,
                timeout,
            } => {
                let mut parts = vec![Doc::text(self.kws("wait"))];
                let mut tail = Vec::new();
                if let Some(sensitivity) = sensitivity {
                    tail.push(Doc::line());
                    tail.push(Doc::text(format!(
                        "{} {}",
                        self.kws("on"),
                        self.sensitivity(sensitivity)
                    )));
                }
                if let Some(condition) = condition {
                    let condition = self.expr(condition);
                    tail.push(Doc::line());
                    tail.push(Doc::concat([
                        Doc::text(format!("{} ", self.kws("until"))),
                        condition,
                    ]));
                }
                if let Some(timeout) = timeout {
                    let timeout = self.expr(timeout);
                    tail.push(Doc::line());
                    tail.push(Doc::concat([
                        Doc::text(format!("{} ", self.kws("for"))),
                        timeout,
                    ]));
                }
                if !tail.is_empty() {
                    parts.push(Doc::concat(tail).indent());
                }
                parts.push(Doc::text(";"));
                Doc::concat([prefix, Doc::concat(parts).group()])
            }
            SequentialKind::Assertion(assertion) => {
                let body = self.assertion(assertion);
                Doc::concat([prefix, body, Doc::text(";")])
            }
            SequentialKind::Report { message, severity } => {
                let message = self.expr(message);
                let mut parts = vec![Doc::text(format!("{} ", self.kws("report"))), message];
                if let Some(severity) = severity {
                    let severity = self.expr(severity);
                    parts.push(
                        Doc::concat([
                            Doc::line(),
                            Doc::text(format!("{} ", self.kws("severity"))),
                            severity,
                        ])
                        .indent(),
                    );
                }
                parts.push(Doc::text(";"));
                Doc::concat([prefix, Doc::concat(parts).group()])
            }
            SequentialKind::SignalAssignment(assign) => {
                let body = self.signal_assignment(assign, false, pad, stmt.label.is_some());
                Doc::concat([prefix, body, Doc::text(";")])
            }
            SequentialKind::VariableAssignment(assign) => {
                let body = self.variable_assignment(assign, pad, stmt.label.is_some());
                Doc::concat([prefix, body, Doc::text(";")])
            }
            SequentialKind::ProcedureCall(call) => {
                Doc::concat([prefix, Doc::text(format!("{};", self.name_text(call)))])
            }
            SequentialKind::If(if_stmt) => self.if_statement(if_stmt, prefix, label.as_deref()),
            SequentialKind::Case(case) => self.case_statement(case, prefix, label.as_deref()),
            SequentialKind::Loop(loop_stmt) => {
                self.loop_statement(loop_stmt, prefix, label.as_deref())
            }
            SequentialKind::Next { label, condition } => {
                self.next_or_exit("next", label.as_ref(), condition.as_ref(), prefix)
            }
            SequentialKind::Exit { label, condition } => {
                self.next_or_exit("exit", label.as_ref(), condition.as_ref(), prefix)
            }
            SequentialKind::Return(value) => match value {
                Some(value) => {
                    let value = self.expr(value);
                    Doc::concat([
                        prefix,
                        Doc::text(format!("{} ", self.kws("return"))),
                        value,
                        Doc::text(";"),
                    ])
                }
                None => Doc::concat([prefix, Doc::text(format!("{};", self.kws("return")))]),
            },
            SequentialKind::Null => {
                Doc::concat([prefix, Doc::text(format!("{};", self.kws("null")))])
            }
        }
    }

    fn next_or_exit(
        &mut self,
        word: &str,
        target: Option<&Ident>,
        condition: Option<&Expr>,
        prefix: Doc,
    ) -> Doc {
        let mut text = self.kws(word);
        if let Some(target) = target {
            text.push(' ');
            text.push_str(&self.ident(target));
        }
        let mut parts = vec![prefix, Doc::text(text)];
        if let Some(condition) = condition {
            let condition = self.expr(condition);
            parts.push(Doc::text(format!(" {} ", self.kws("when"))));
            parts.push(condition);
        }
        parts.push(Doc::text(";"));
        Doc::concat(parts).group()
    }

    fn if_statement(&mut self, stmt: &IfStatement, prefix: Doc, label: Option<&str>) -> Doc {
        let mut parts = vec![prefix];
        for (i, arm) in stmt.arms.iter().enumerate() {
            let word = if i == 0 { "if" } else { "elsif" };
            let condition = self.expr(&arm.condition);
            let header = Doc::concat([
                Doc::text(format!("{} ", self.kws(word))),
                condition,
                Doc::text(format!(" {}", self.kws("then"))),
            ])
            .group();
            let limit = stmt.arms.get(i + 1).map_or(stmt.span.end, |a| a.span.start);
            let body = self.nested(|s| {
                let mut seq = Seq::default();
                s.sequential_statements(&mut seq, &arm.statements, limit);
                seq.finish()
            });
            if i > 0 {
                parts.push(Doc::hardline());
            }
            parts.push(Doc::concat([header, body_block(body)]));
        }
        if let Some(statements) = &stmt.else_statements {
            let body = self.nested(|s| {
                let mut seq = Seq::default();
                s.sequential_statements(&mut seq, statements, stmt.span.end);
                seq.finish()
            });
            parts.push(Doc::hardline());
            parts.push(Doc::concat([self.kw("else"), body_block(body)]));
        }
        parts.push(Doc::hardline());
        parts.push(self.end(&["if"], label));
        Doc::concat(parts)
    }

    fn case_statement(&mut self, stmt: &CaseStatement, prefix: Doc, label: Option<&str>) -> Doc {
        let selector = self.expr(&stmt.expr);
        let question = if stmt.matching { "?" } else { "" };
        let header = Doc::concat([
            Doc::text(format!("{}{question} ", self.kws("case"))),
            selector,
            Doc::text(format!(" {}", self.kws("is"))),
        ]);
        let arms = self.nested(|s| {
            let mut parts = Vec::new();
            for (i, arm) in stmt.arms.iter().enumerate() {
                if i > 0 {
                    parts.push(Doc::hardline());
                }
                let head = Doc::text(format!("{} {} =>", s.kws("when"), s.choices(&arm.choices)));
                let limit = stmt.arms.get(i + 1).map_or(stmt.span.end, |a| a.span.start);
                parts.push(s.case_arm(head, &arm.statements, limit));
            }
            Doc::concat(parts)
        });
        let mut end = self.kws("end");
        end.push(' ');
        end.push_str(&self.kws("case"));
        end.push_str(question);
        if let Some(label) = label
            && self.opts.complete_end_labels
        {
            end.push(' ');
            end.push_str(label);
        }
        end.push(';');
        Doc::concat([prefix, Self::wrap(header, arms, Doc::text(end))])
    }

    /// One `when ... =>` arm, kept on a single line when it holds one
    /// short statement and the options allow it.
    fn case_arm(&mut self, head: Doc, statements: &[SequentialStatement], limit: u32) -> Doc {
        let one_line = self.opts.case_items_on_one_line
            && statements.len() == 1
            && is_simple(&statements[0])
            && !self.cm.any_between(
                statements[0].span.start.saturating_sub(1),
                statements[0].span.end,
            );
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            s.sequential_statements(&mut seq, statements, limit);
            seq.finish()
        });
        if body.is_nil() {
            return head;
        }
        if one_line && !body.has_hard_break() {
            let width = head.flat_width() + 1 + body.flat_width();
            if self.fits(width) {
                return Doc::concat([head, Doc::space(), body]);
            }
        }
        Doc::concat([head, Doc::concat([Doc::hardline(), body]).indent()])
    }

    fn loop_statement(&mut self, stmt: &LoopStatement, prefix: Doc, label: Option<&str>) -> Doc {
        let mut header = Vec::new();
        match &stmt.scheme {
            Some(IterationScheme::While(condition)) => {
                let condition = self.expr(condition);
                header.push(Doc::text(format!("{} ", self.kws("while"))));
                header.push(condition);
                header.push(Doc::space());
            }
            Some(IterationScheme::For { param, range }) => {
                let range = self.discrete_range(range);
                header.push(Doc::text(format!(
                    "{} {} {} ",
                    self.kws("for"),
                    self.ident(param),
                    self.kws("in")
                )));
                header.push(range);
                header.push(Doc::space());
            }
            None => {}
        }
        header.push(self.kw("loop"));
        let body = self.nested(|s| {
            let mut seq = Seq::default();
            s.sequential_statements(&mut seq, &stmt.statements, stmt.span.end);
            seq.finish()
        });
        Doc::concat([
            prefix,
            Self::wrap(
                Doc::concat(header).group(),
                body,
                self.end(&["loop"], label),
            ),
        ])
    }

    // --- assignments --------------------------------------------------------

    fn target(&self, target: &Target) -> Doc {
        match target {
            Target::Name(name) => Doc::text(self.name_text(name)),
            Target::Aggregate(aggregate) => self.aggregate(aggregate),
        }
    }

    fn signal_assignment(
        &mut self,
        assign: &SignalAssignment,
        guarded: bool,
        pad: usize,
        labelled: bool,
    ) -> Doc {
        if let SignalAssignmentRhs::Selected {
            selector,
            matching,
            arms,
        } = &assign.rhs
        {
            let selector = self.expr(selector);
            let target = self.target(&assign.target);
            let head = Doc::concat([
                Doc::text(format!("{} ", self.kws("with"))),
                selector,
                Doc::text(format!(
                    " {}{} ",
                    self.kws("select"),
                    if *matching { "?" } else { "" }
                )),
                target,
                Doc::text(" <="),
                self.assignment_prefix(guarded, assign.delay.as_ref()),
            ]);
            let arms: Vec<Doc> = arms
                .iter()
                .map(|arm| {
                    let waveform = self.waveform(&arm.waveform);
                    Doc::concat([
                        waveform,
                        Doc::text(format!(" {} ", self.kws("when"))),
                        Doc::text(self.choices(&arm.choices)),
                    ])
                })
                .collect();
            return Doc::concat([
                head,
                Doc::concat([
                    Doc::line(),
                    Doc::join(Doc::concat([Doc::text(","), Doc::line()]), arms),
                ])
                .indent(),
            ])
            .group();
        }

        let target = self.target(&assign.target);
        let padding = if labelled || pad == 0 {
            Doc::nil()
        } else {
            Doc::text(" ".repeat(pad))
        };
        let head = Doc::concat([target, padding]);
        let prefix = self.assignment_prefix(guarded, assign.delay.as_ref());
        let rhs = match &assign.rhs {
            SignalAssignmentRhs::Simple(waveform) => self.waveform(waveform),
            SignalAssignmentRhs::Conditional(arms) => self.conditional_waveforms(arms),
            SignalAssignmentRhs::Force { mode, arms } => {
                let mut text = self.kws("force");
                if let Some(mode) = mode {
                    text.push(' ');
                    text.push_str(&self.kws(mode_word(*mode)));
                }
                let arms = self.conditional_exprs(arms);
                Doc::concat([Doc::text(text), Doc::space(), arms])
            }
            SignalAssignmentRhs::Release { mode } => {
                let mut text = self.kws("release");
                if let Some(mode) = mode {
                    text.push(' ');
                    text.push_str(&self.kws(mode_word(*mode)));
                }
                Doc::text(text)
            }
            SignalAssignmentRhs::Selected { .. } => unreachable!("handled above"),
        };
        let operator = format!(" <={}", prefix.flat_text());
        Doc::concat([head, Self::after_operator(&operator, rhs)]).group()
    }

    /// The ` guarded`/` transport` that follows the assignment operator.
    fn assignment_prefix(&mut self, guarded: bool, delay: Option<&DelayMechanism>) -> Doc {
        let mut text = String::new();
        if guarded {
            text.push(' ');
            text.push_str(&self.kws("guarded"));
        }
        match delay {
            Some(DelayMechanism::Transport(_)) => {
                text.push(' ');
                text.push_str(&self.kws("transport"));
            }
            Some(DelayMechanism::Inertial { reject, .. }) => {
                if let Some(reject) = reject {
                    text.push(' ');
                    text.push_str(&self.kws("reject"));
                    text.push(' ');
                    text.push_str(&self.expr(reject).flat_text());
                }
                text.push(' ');
                text.push_str(&self.kws("inertial"));
            }
            None => {}
        }
        Doc::text(text)
    }

    fn variable_assignment(
        &mut self,
        assign: &VariableAssignment,
        pad: usize,
        labelled: bool,
    ) -> Doc {
        if let VariableAssignmentRhs::Selected {
            selector,
            matching,
            arms,
        } = &assign.rhs
        {
            let selector = self.expr(selector);
            let target = self.target(&assign.target);
            let head = Doc::concat([
                Doc::text(format!("{} ", self.kws("with"))),
                selector,
                Doc::text(format!(
                    " {}{} ",
                    self.kws("select"),
                    if *matching { "?" } else { "" }
                )),
                target,
                Doc::text(" :="),
            ]);
            let arms: Vec<Doc> = arms
                .iter()
                .map(|arm| {
                    let value = self.expr(&arm.value);
                    Doc::concat([
                        value,
                        Doc::text(format!(" {} ", self.kws("when"))),
                        Doc::text(self.choices(&arm.choices)),
                    ])
                })
                .collect();
            return Doc::concat([
                head,
                Doc::concat([
                    Doc::line(),
                    Doc::join(Doc::concat([Doc::text(","), Doc::line()]), arms),
                ])
                .indent(),
            ])
            .group();
        }

        let target = self.target(&assign.target);
        let padding = if labelled || pad == 0 {
            Doc::nil()
        } else {
            Doc::text(" ".repeat(pad))
        };
        let rhs = match &assign.rhs {
            VariableAssignmentRhs::Simple(value) => self.expr(value),
            VariableAssignmentRhs::Conditional(arms) => self.conditional_exprs(arms),
            VariableAssignmentRhs::Selected { .. } => unreachable!("handled above"),
        };
        Doc::concat([target, padding, Self::after_operator(" :=", rhs)]).group()
    }

    fn waveform(&mut self, waveform: &Waveform) -> Doc {
        match waveform {
            Waveform::Unaffected(_) => self.kw("unaffected"),
            Waveform::Elements(elements) => {
                let items: Vec<Doc> = elements
                    .iter()
                    .map(|element| {
                        let value = self.expr(&element.value);
                        match &element.after {
                            Some(after) => {
                                let after = self.expr(after);
                                Doc::concat([
                                    value,
                                    Doc::text(format!(" {} ", self.kws("after"))),
                                    after,
                                ])
                            }
                            None => value,
                        }
                    })
                    .collect();
                Doc::join(Doc::concat([Doc::text(","), Doc::line()]), items).group()
            }
        }
    }

    fn conditional_waveforms(&mut self, arms: &[ConditionalWaveform]) -> Doc {
        let mut parts = Vec::new();
        for (i, arm) in arms.iter().enumerate() {
            if i > 0 {
                parts.push(Doc::line());
                parts.push(Doc::text(format!("{} ", self.kws("else"))));
            }
            let waveform = self.waveform(&arm.waveform);
            parts.push(waveform);
            if let Some(condition) = &arm.condition {
                let condition = self.expr(condition);
                parts.push(Doc::text(format!(" {} ", self.kws("when"))));
                parts.push(condition);
            }
        }
        Doc::concat(parts).group()
    }

    fn conditional_exprs(&mut self, arms: &[ConditionalExpr]) -> Doc {
        let mut parts = Vec::new();
        for (i, arm) in arms.iter().enumerate() {
            if i > 0 {
                parts.push(Doc::line());
                parts.push(Doc::text(format!("{} ", self.kws("else"))));
            }
            let value = self.expr(&arm.value);
            parts.push(value);
            if let Some(condition) = &arm.condition {
                let condition = self.expr(condition);
                parts.push(Doc::text(format!(" {} ", self.kws("when"))));
                parts.push(condition);
            }
        }
        Doc::concat(parts).group()
    }

    // --- names, types and expressions ---------------------------------------

    fn designator(&self, designator: &Designator) -> String {
        self.raw(designator.span())
    }

    fn signature(&self, signature: &Signature) -> String {
        let mut text = String::from("[");
        let params: Vec<String> = signature.params.iter().map(|p| self.name_text(p)).collect();
        text.push_str(&params.join(", "));
        if let Some(ret) = &signature.return_type {
            if !params.is_empty() {
                text.push(' ');
            }
            text.push_str(&self.kws("return"));
            text.push(' ');
            text.push_str(&self.name_text(ret));
        }
        text.push(']');
        text
    }

    /// A name, always on one line: names are short and breaking one makes
    /// a design harder to read, not easier.
    fn name_text(&self, name: &Name) -> String {
        match name {
            Name::Simple(id) => self.ident(id),
            Name::Operator { span, .. } | Name::Char { span, .. } => self.raw(*span),
            Name::Selected { prefix, suffix, .. } => {
                let suffix = match suffix {
                    Suffix::Designator(d) => self.designator(d),
                    Suffix::All(_) => self.kws("all"),
                };
                format!("{}.{suffix}", self.name_text(prefix))
            }
            Name::Call { prefix, args, .. } => {
                let args: Vec<String> = args.iter().map(|a| self.assoc_text(a)).collect();
                format!("{}({})", self.name_text(prefix), args.join(", "))
            }
            Name::Slice { prefix, range, .. } => {
                format!(
                    "{}({})",
                    self.name_text(prefix),
                    self.discrete_range_text(range)
                )
            }
            Name::Attribute {
                prefix,
                signature,
                attribute,
                ..
            } => {
                let signature = signature
                    .as_ref()
                    .map(|s| self.signature(s))
                    .unwrap_or_default();
                format!(
                    "{}{signature}'{}",
                    self.name_text(prefix),
                    self.ident(attribute)
                )
            }
            Name::External(external) => {
                let path = self.external_path(&external.path);
                format!(
                    "<< {} {path} : {} >>",
                    self.kws(external_word(external.class)),
                    self.subtype(&external.subtype).flat_text()
                )
            }
        }
    }

    fn external_path(&self, path: &ExternalPath) -> String {
        let elements: Vec<String> = path
            .elements
            .iter()
            .map(|element| {
                let index = element
                    .index
                    .as_ref()
                    .map(|i| format!("({})", self.expr(i).flat_text()))
                    .unwrap_or_default();
                format!("{}{index}", self.ident(&element.name))
            })
            .collect();
        let joined = elements.join(".");
        match path.kind {
            ExternalPathKind::Package => format!("@{joined}"),
            ExternalPathKind::Absolute => format!(".{joined}"),
            ExternalPathKind::Relative(0) => joined,
            ExternalPathKind::Relative(steps) => {
                let up = "^.".repeat(usize::try_from(steps).expect("path depth fits usize"));
                format!("{up}{joined}")
            }
        }
    }

    fn assoc_text(&self, element: &AssociationElement) -> String {
        let actual = match &element.actual {
            Actual::Expr(e) => self.expr(e).flat_text(),
            Actual::Open(_) => self.kws("open"),
            Actual::Inertial(e) => format!("{} {}", self.kws("inertial"), self.expr(e).flat_text()),
            Actual::Range(r) => self.discrete_range_text(r),
        };
        match &element.formal {
            Some(formal) => format!("{} => {actual}", self.expr(formal).flat_text()),
            None => actual,
        }
    }

    fn subtype(&self, subtype: &SubtypeIndication) -> Doc {
        let mut text = String::new();
        if let Some(resolution) = &subtype.resolution {
            text.push_str(&self.resolution(resolution));
            text.push(' ');
        }
        text.push_str(&self.name_text(&subtype.type_mark));
        if let Some(constraint) = &subtype.constraint {
            text.push_str(&self.constraint(constraint));
        }
        Doc::text(text)
    }

    fn resolution(&self, resolution: &ResolutionIndication) -> String {
        match resolution {
            ResolutionIndication::Function(name) => self.name_text(name),
            ResolutionIndication::Array(inner, _) => format!("({})", self.resolution(inner)),
            ResolutionIndication::Record(elements, _) => {
                let items: Vec<String> = elements
                    .iter()
                    .map(|e| format!("{} {}", self.ident(&e.name), self.resolution(&e.resolution)))
                    .collect();
                format!("({})", items.join(", "))
            }
        }
    }

    fn constraint(&self, constraint: &Constraint) -> String {
        match constraint {
            Constraint::Range(range) => {
                format!(" {} {}", self.kws("range"), self.range_text(range))
            }
            Constraint::Array {
                indices, element, ..
            } => {
                let inner = if indices.is_empty() {
                    self.kws("open")
                } else {
                    indices
                        .iter()
                        .map(|i| self.discrete_range_text(i))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let element = element
                    .as_ref()
                    .map(|c| self.constraint(c))
                    .unwrap_or_default();
                format!("({inner}){element}")
            }
            Constraint::Record(elements, _) => {
                let items: Vec<String> = elements
                    .iter()
                    .map(|e| format!("{}{}", self.ident(&e.name), self.constraint(&e.constraint)))
                    .collect();
                format!("({})", items.join(", "))
            }
        }
    }

    fn range(&self, range: &Range) -> Doc {
        Doc::text(self.range_text(range))
    }

    fn range_text(&self, range: &Range) -> String {
        match range {
            Range::Bounds {
                left,
                direction,
                right,
                ..
            } => format!(
                "{} {} {}",
                self.expr(left).flat_text(),
                self.kws(direction.as_str()),
                self.expr(right).flat_text()
            ),
            Range::Attribute(name) => self.name_text(name),
        }
    }

    fn discrete_range(&self, range: &DiscreteRange) -> Doc {
        Doc::text(self.discrete_range_text(range))
    }

    fn discrete_range_text(&self, range: &DiscreteRange) -> String {
        match range {
            DiscreteRange::Range(range) => self.range_text(range),
            DiscreteRange::Subtype(subtype) => self.subtype(subtype).flat_text(),
        }
    }

    fn choices(&self, choices: &[Choice]) -> String {
        choices
            .iter()
            .map(|choice| match choice {
                Choice::Expr(e) => self.expr(e).flat_text(),
                Choice::Range(r) => self.discrete_range_text(r),
                Choice::Others(_) => self.kws("others"),
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// A parenthesised, comma-separated list that may break.
    fn args(items: Vec<Doc>) -> Doc {
        if items.is_empty() {
            return Doc::text("()");
        }
        Doc::concat([
            Doc::text("("),
            Doc::concat([
                Doc::softline(),
                Doc::join(Doc::concat([Doc::text(","), Doc::line()]), items),
            ])
            .indent(),
            Doc::softline(),
            Doc::text(")"),
        ])
        .group()
    }

    fn expr(&self, expr: &Expr) -> Doc {
        match expr {
            Expr::Binary { op, lhs, rhs, .. } => {
                let mut operands = Vec::new();
                flatten(lhs, *op, &mut operands);
                operands.push(rhs.as_ref());
                let mut parts = vec![self.expr(operands[0])];
                let mut tail = Vec::new();
                for operand in &operands[1..] {
                    tail.push(Doc::line());
                    tail.push(Doc::text(format!("{} ", self.kws(op.as_str()))));
                    tail.push(self.expr(operand));
                }
                parts.push(Doc::concat(tail).indent());
                Doc::concat(parts).group()
            }
            Expr::Unary { op, operand, .. } => {
                let word = self.kws(op.as_str());
                // `-x` and `+x` bind tight; `not x`, `abs x` and the
                // condition operator `?? x` need the separator.
                let space = if matches!(op, UnaryOp::Plus | UnaryOp::Minus) {
                    ""
                } else {
                    " "
                };
                Doc::concat([Doc::text(format!("{word}{space}")), self.expr(operand)])
            }
            Expr::Name(name) => Doc::text(self.name_text(name)),
            Expr::Literal(literal) => Doc::text(self.raw(literal.span)),
            Expr::Aggregate(aggregate) => self.aggregate(aggregate),
            Expr::Qualified {
                type_mark, operand, ..
            } => Doc::concat([
                Doc::text(format!("{}'", self.name_text(type_mark))),
                self.expr(operand),
            ]),
            Expr::Allocator { kind, .. } => match kind.as_ref() {
                Allocator::Subtype(subtype) => Doc::concat([
                    Doc::text(format!("{} ", self.kws("new"))),
                    self.subtype(subtype),
                ]),
                Allocator::Qualified {
                    type_mark, operand, ..
                } => Doc::concat([
                    Doc::text(format!(
                        "{} {}'",
                        self.kws("new"),
                        self.name_text(type_mark)
                    )),
                    self.expr(operand),
                ]),
            },
            Expr::Paren { inner, .. } => Doc::concat([
                Doc::text("("),
                Doc::concat([Doc::softline(), self.expr(inner)]).indent(),
                Doc::softline(),
                Doc::text(")"),
            ])
            .group(),
            Expr::Open(_) => self.kw("open"),
            Expr::Error(span) => Doc::text(self.raw(*span)),
        }
    }

    /// An aggregate, with its `=>` aligned when it has to be broken.
    fn aggregate(&self, aggregate: &Aggregate) -> Doc {
        if aggregate.elements.is_empty() {
            return Doc::text("()");
        }
        let rows: Vec<Row> = aggregate
            .elements
            .iter()
            .map(|element| {
                let value = self.expr(&element.value);
                if element.choices.is_empty() {
                    Row::Plain(value)
                } else {
                    Row::Assoc {
                        left: self.choices(&element.choices),
                        right: value,
                    }
                }
            })
            .collect();
        self.rows_to_doc("(", rows, ",", ")", 1)
    }
}

/// A `then`/`else`/`loop` body, indented, or nothing when it is empty.
fn body_block(body: Doc) -> Doc {
    if body.is_nil() {
        Doc::nil()
    } else {
        Doc::concat([Doc::hardline(), body]).indent()
    }
}

/// Flattens the left spine of a chain of one operator, so that
/// `a and b and c` breaks into three operands rather than nesting twice.
fn flatten<'e>(expr: &'e Expr, op: BinaryOp, out: &mut Vec<&'e Expr>) {
    if let Expr::Binary {
        op: inner,
        lhs,
        rhs,
        ..
    } = expr
        && *inner == op
    {
        flatten(lhs, op, out);
        out.push(rhs);
    } else {
        out.push(expr);
    }
}

/// One element of a list that may have its columns aligned.
enum Row {
    /// An interface object: names, mode and the rest.
    Object {
        left: String,
        mid: String,
        right: Doc,
    },
    /// A named association or aggregate element: `formal => actual`.
    Assoc { left: String, right: Doc },
    /// Anything with no column structure.
    Plain(Doc),
}

impl Row {
    /// The element on one line, with single spaces.
    fn flat(&self) -> Doc {
        match self {
            Row::Object { left, mid, right } => {
                let mid = if mid.is_empty() {
                    String::new()
                } else {
                    format!("{mid} ")
                };
                Doc::concat([Doc::text(format!("{left} : {mid}")), right.clone()])
            }
            Row::Assoc { left, right } => {
                Doc::concat([Doc::text(format!("{left} => ")), right.clone()])
            }
            Row::Plain(doc) => doc.clone(),
        }
    }

    fn left_width(&self) -> usize {
        match self {
            Row::Object { left, .. } | Row::Assoc { left, .. } => left.chars().count(),
            Row::Plain(_) => 0,
        }
    }

    fn mid_width(&self) -> usize {
        match self {
            Row::Object { mid, .. } => mid.chars().count(),
            _ => 0,
        }
    }

    /// The element with its columns padded to the widths of the list.
    fn aligned(&self, left_width: usize, mid_width: usize) -> Doc {
        match self {
            Row::Object { left, mid, right } => {
                let left_pad = " ".repeat(left_width.saturating_sub(left.chars().count()));
                let mid_pad = if mid_width == 0 {
                    String::new()
                } else {
                    format!(
                        "{mid}{} ",
                        " ".repeat(mid_width.saturating_sub(mid.chars().count()))
                    )
                };
                Doc::concat([
                    Doc::text(format!("{left}{left_pad} : {mid_pad}")),
                    right.clone(),
                ])
            }
            Row::Assoc { left, right } => {
                let pad = " ".repeat(left_width.saturating_sub(left.chars().count()));
                Doc::concat([Doc::text(format!("{left}{pad} => ")), right.clone()])
            }
            Row::Plain(doc) => doc.clone(),
        }
    }
}

/// A top-level piece of a design file.
enum Top<'a> {
    Context(&'a ContextItem),
    Unit(&'a LibraryUnit),
}

impl Top<'_> {
    fn span(&self) -> Span {
        match self {
            Top::Context(item) => item.span(),
            Top::Unit(unit) => unit.span(),
        }
    }

    fn start(&self) -> u32 {
        self.span().start
    }
}

/// The extent of an interface list, used to place the comments written
/// around a `generic` or `port` clause.
fn interface_span(items: &[InterfaceDecl]) -> Span {
    let first = items.first().expect("non-empty interface list").span();
    let last = items.last().expect("non-empty interface list").span();
    first.to(last)
}

/// The extent of an association list, for the same reason; an empty list
/// has no span of its own and borrows the enclosing construct's.
fn assoc_span(items: &[AssociationElement], fallback: Span) -> Span {
    match (items.first(), items.last()) {
        (Some(first), Some(last)) => first.span.to(last.span),
        _ => fallback,
    }
}

/// True for a statement short enough to share a line with its `when`.
fn is_simple(stmt: &SequentialStatement) -> bool {
    matches!(
        stmt.kind,
        SequentialKind::Null
            | SequentialKind::Return(_)
            | SequentialKind::Next { .. }
            | SequentialKind::Exit { .. }
            | SequentialKind::SignalAssignment(_)
            | SequentialKind::VariableAssignment(_)
            | SequentialKind::ProcedureCall(_)
    )
}
