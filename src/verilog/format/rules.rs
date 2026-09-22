//! The Verilog layout rules: AST to [`Doc`].
//!
//! Every construct has one shape, chosen from the tree and the options, so
//! the output never depends on how the input happened to be written. The
//! rules that need to know whether something fits — port lists, instance
//! connections, argument lists — measure the flat form against the line
//! width at the indentation they will be printed at, and either keep it on
//! one line or put one entry per line; everything else is left to the
//! document printer's groups.
//!
//! # Parentheses
//!
//! The tree records precedence, not parentheses: `(a + b) * c` and
//! `a + b * c` differ only in shape. Expressions are therefore printed
//! through [`Rules::expr_prec`], which wraps a child in parentheses exactly
//! when its precedence is lower than its position allows. The formatted
//! text reparses to the same tree, which is what the round-trip test
//! checks.
//!
//! # Alignment
//!
//! Aligned columns (the type and name columns of a port list, the `=` of a
//! run of assignments, the `.port (net)` of an instance) are produced by
//! measuring the pieces with [`Doc::flat_width`] and emitting explicit
//! padding, never by nesting, so they survive a tab indent unchanged.

use std::fmt::Write;

use crate::fmt_doc::{Doc, FormatOptions};
use crate::source::Span;
use crate::verilog::ast::*;
use crate::verilog::token::Dialect;

use super::comments::{Comments, Piece};

/// Precedence levels, from the loosest to the tightest binding.
mod prec {
    /// `=`, `+=` and friends as an expression. Lowest of all, and
    /// parenthesised everywhere but in a `for` header, since a bare
    /// assignment expression is legal in very few places.
    pub(super) const ASSIGN: u8 = 1;
    /// `min:typ:max`.
    pub(super) const MIN_TYP_MAX: u8 = 2;
    /// `->`, `<->`.
    pub(super) const IMPLIES: u8 = 3;
    /// `?:`.
    pub(super) const TERNARY: u8 = 4;
    /// `||`.
    pub(super) const LOGIC_OR: u8 = 5;
    /// `&&`.
    pub(super) const LOGIC_AND: u8 = 6;
    /// `|`.
    pub(super) const BIT_OR: u8 = 7;
    /// `^`, `~^`.
    pub(super) const BIT_XOR: u8 = 8;
    /// `&`.
    pub(super) const BIT_AND: u8 = 9;
    /// `==`, `!=`, `===`, `!==`, `==?`, `!=?`.
    pub(super) const EQUALITY: u8 = 10;
    /// `<`, `<=`, `>`, `>=`, `inside`.
    pub(super) const RELATIONAL: u8 = 11;
    /// `<<`, `>>`, `<<<`, `>>>`.
    pub(super) const SHIFT: u8 = 12;
    /// `+`, `-`.
    pub(super) const ADDITIVE: u8 = 13;
    /// `*`, `/`, `%`.
    pub(super) const MULTIPLICATIVE: u8 = 14;
    /// `**`.
    pub(super) const POWER: u8 = 15;
    /// Prefix and postfix operators.
    pub(super) const UNARY: u8 = 16;
    /// Names, literals, calls, concatenations: never parenthesised.
    pub(super) const PRIMARY: u8 = 17;
}

/// The precedence of a binary operator.
fn binary_prec(op: BinaryOp) -> u8 {
    use BinaryOp as B;
    match op {
        B::Implies | B::Equiv => prec::IMPLIES,
        B::LogicOr => prec::LOGIC_OR,
        B::LogicAnd => prec::LOGIC_AND,
        B::BitOr => prec::BIT_OR,
        B::BitXor | B::BitXnor => prec::BIT_XOR,
        B::BitAnd => prec::BIT_AND,
        B::Eq | B::Ne | B::CaseEq | B::CaseNe | B::WildEq | B::WildNe => prec::EQUALITY,
        B::Lt | B::Le | B::Gt | B::Ge => prec::RELATIONAL,
        B::Shl | B::Shr | B::Ashl | B::Ashr => prec::SHIFT,
        B::Add | B::Sub => prec::ADDITIVE,
        B::Mul | B::Div | B::Mod => prec::MULTIPLICATIVE,
        B::Pow => prec::POWER,
    }
}

/// True for the right-associative binary operators.
fn right_associative(op: BinaryOp) -> bool {
    matches!(op, BinaryOp::Pow | BinaryOp::Implies | BinaryOp::Equiv)
}

/// The precedence of an expression, for deciding on parentheses.
fn expr_prec(kind: &ExprKind) -> u8 {
    match kind {
        ExprKind::MinTypMax { .. } => prec::MIN_TYP_MAX,
        ExprKind::Assign { .. } => prec::ASSIGN,
        ExprKind::Ternary { .. } => prec::TERNARY,
        ExprKind::Binary { op, .. } => binary_prec(*op),
        ExprKind::Inside { .. } => prec::RELATIONAL,
        ExprKind::Unary { .. } | ExprKind::IncDec { .. } => prec::UNARY,
        _ => prec::PRIMARY,
    }
}

/// The width of a document's first line, laid out flat.
fn first_line_width(doc: &Doc) -> usize {
    doc.flat_text()
        .split('\n')
        .next()
        .unwrap_or("")
        .chars()
        .count()
}

/// The ` signed` / ` unsigned` suffix of a type, or nothing.
fn signing_text(signing: Option<Signing>) -> &'static str {
    match signing {
        Some(Signing::Signed) => " signed",
        Some(Signing::Unsigned) => " unsigned",
        None => "",
    }
}

/// An identifier as it must be written: an escaped identifier regains its
/// backslash and the whitespace that terminates it.
fn id(name: &str) -> String {
    let simple = !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if simple {
        name.to_owned()
    } else {
        format!("\\{name} ")
    }
}

/// Re-quotes a string literal whose escapes the lexer has already decoded.
fn quote_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\{:03o}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The comments written around one port of an ANSI port list.
struct PortComments {
    /// Comments on the lines before the port.
    leading: Vec<Piece>,
    /// A comment after the port's comma, on the same line.
    trailing: Option<Piece>,
}

/// Collects the lines of a block, separated by one line break or, where the
/// source had a blank line, by two.
#[derive(Default)]
struct Lines {
    parts: Vec<Doc>,
}

impl Lines {
    fn push(&mut self, blank_before: bool, doc: Doc) {
        if doc.is_nil() {
            return;
        }
        if !self.parts.is_empty() {
            self.parts.push(if blank_before {
                Doc::blankline()
            } else {
                Doc::hardline()
            });
        }
        self.parts.push(doc);
    }

    fn finish(self) -> Doc {
        Doc::concat(self.parts)
    }
}

/// The formatter state: the options, the dialect and the comment table.
pub(crate) struct Rules<'a> {
    opts: &'a FormatOptions,
    dialect: Dialect,
    comments: Comments<'a>,
    /// Indentation level of what is currently being built, for measuring.
    level: usize,
}

impl<'a> Rules<'a> {
    /// Creates the rules for one file.
    pub(crate) fn new(opts: &'a FormatOptions, dialect: Dialect, comments: Comments<'a>) -> Self {
        Rules {
            opts,
            dialect,
            comments,
            level: 0,
        }
    }

    /// The document for a whole source file.
    pub(crate) fn source_file(&mut self, file: &SourceFile) -> Doc {
        let mut lines = Lines::default();
        self.item_lines(&mut lines, &file.items, u32::MAX);
        for piece in self.comments.remaining() {
            lines.push(piece.blank_before, piece.doc());
        }
        lines.finish()
    }

    // --- layout helpers --------------------------------------------------

    /// The column the current indentation level starts at.
    fn cols(&self) -> usize {
        self.level * self.opts.indent.width()
    }

    /// True when `doc`, written flat after `extra` more columns, still fits.
    fn fits(&self, extra: usize, doc: &Doc) -> bool {
        !doc.has_hard_break() && self.cols() + extra + doc.flat_width() <= self.opts.line_width
    }

    /// Runs `f` one indentation level deeper.
    fn deeper<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.level += 1;
        let out = f(self);
        self.level -= 1;
        out
    }

    /// `open entries close`, on one line when it fits and one entry per
    /// line otherwise. `flat` and `broken` are the same entries built with
    /// and without column padding.
    fn delimited(
        &self,
        open: &str,
        flat: Vec<Doc>,
        broken: Vec<Doc>,
        close: &str,
        extra: usize,
    ) -> Doc {
        let broken = broken.into_iter().map(|doc| (doc, Doc::nil())).collect();
        self.delimited_with_trails(open, flat, broken, close, extra, false)
    }

    /// Like [`Rules::delimited`], but each broken entry may carry a comment
    /// that belongs *after* its separating comma. `force` breaks the list
    /// whatever its width, which is what those comments require.
    fn delimited_with_trails(
        &self,
        open: &str,
        flat: Vec<Doc>,
        broken: Vec<(Doc, Doc)>,
        close: &str,
        extra: usize,
        force: bool,
    ) -> Doc {
        if flat.is_empty() && broken.is_empty() {
            return Doc::text(format!("{open}{close}"));
        }
        if !force {
            let one_line = Doc::concat([
                Doc::text(open.to_owned()),
                Doc::join(Doc::text(", "), flat),
                Doc::text(close.to_owned()),
            ]);
            if self.fits(extra, &one_line) {
                return one_line;
            }
        }
        let last = broken.len().saturating_sub(1);
        let mut body = Vec::new();
        for (i, (entry, trail)) in broken.into_iter().enumerate() {
            body.push(entry);
            if i != last {
                body.push(Doc::text(","));
            }
            body.push(trail);
            if i != last {
                body.push(Doc::hardline());
            }
        }
        Doc::concat([
            Doc::text(open.to_owned()),
            Doc::concat([Doc::hardline(), Doc::concat(body)]).indent(),
            Doc::hardline(),
            Doc::text(close.to_owned()),
        ])
    }

    /// The body of a `begin`/`end`-like construct: the items indented, then
    /// the end keyword on its own line.
    fn closed_body(&self, body: Doc, end: String) -> Doc {
        if body.is_nil() {
            Doc::concat([Doc::hardline(), Doc::text(end)])
        } else {
            Doc::concat([
                Doc::concat([Doc::hardline(), body]).indent(),
                Doc::hardline(),
                Doc::text(end),
            ])
        }
    }

    /// The comment that follows the header of a construct on the same
    /// line, which belongs at the end of that header rather than at the
    /// start of the body.
    fn head_trail(&mut self, head_end: u32, limit: u32) -> Doc {
        self.comments.advance(head_end);
        match self.comments.head_trailing(head_end, limit) {
            Some(piece) => Doc::space().append(piece.doc()),
            None => Doc::nil(),
        }
    }

    /// The `: name` written after an `end` keyword, when asked for and the
    /// dialect allows it.
    fn end_label(&self, name: Option<&Ident>) -> String {
        match name {
            Some(n) if self.opts.complete_end_labels && self.dialect == Dialect::SystemVerilog => {
                format!(" : {}", id(&n.name))
            }
            _ => String::new(),
        }
    }

    /// True when nothing but blanks and at most one line break separates
    /// two constructs, so they may share an alignment run.
    fn adjacent(&self, prev_end: u32, next_start: u32) -> bool {
        if next_start < prev_end {
            return false;
        }
        let between = self.comments.between(prev_end, next_start);
        between.bytes().filter(|&b| b == b'\n').count() <= 1
            && !between.contains("//")
            && !between.contains("/*")
    }

    /// The padding widths that align a run of consecutive assignments.
    fn alignment<T>(
        &self,
        items: &[T],
        width: impl Fn(&Self, &T) -> Option<usize>,
        span: impl Fn(&T) -> Span,
    ) -> Vec<usize> {
        let mut pads = vec![0; items.len()];
        if !self.opts.align_assignments {
            return pads;
        }
        let mut i = 0;
        while i < items.len() {
            let Some(first) = width(self, &items[i]) else {
                i += 1;
                continue;
            };
            let mut widths = vec![first];
            let mut j = i;
            while j + 1 < items.len()
                && self.adjacent(span(&items[j]).end, span(&items[j + 1]).start)
            {
                match width(self, &items[j + 1]) {
                    Some(w) => {
                        widths.push(w);
                        j += 1;
                    }
                    None => break,
                }
            }
            if j > i {
                let max = widths.iter().copied().max().unwrap_or(0);
                for (k, w) in widths.iter().enumerate() {
                    pads[i + k] = max - w;
                }
            }
            i = j + 1;
        }
        pads
    }

    /// Pads `doc` on the right to `pad` extra columns.
    fn pad(doc: Doc, pad: usize) -> Doc {
        if pad == 0 {
            doc
        } else {
            doc.append(Doc::text(" ".repeat(pad)))
        }
    }

    // --- items -----------------------------------------------------------

    /// Appends the lines of an item list, with their comments and blank
    /// lines, stopping at `block_end`.
    fn item_lines(&mut self, lines: &mut Lines, items: &[Item], block_end: u32) {
        let pads = self.alignment(
            items,
            |rules, item| rules.cont_assign_width(item),
            |item| item.span,
        );
        for (i, item) in items.iter().enumerate() {
            for piece in self.comments.leading(item.span.start) {
                lines.push(piece.blank_before, piece.doc());
            }
            let blank = self.comments.gap_blank(item.span.start);
            let doc = self.item(item, pads[i]);
            self.comments.advance(item.span.end);
            let next = items
                .get(i + 1)
                .map_or(block_end, |next| next.span.start.min(block_end));
            let doc = match self.comments.trailing(item.span.end, next) {
                Some(piece) => doc.append(Doc::space()).append(piece.doc()),
                None => doc,
            };
            lines.push(blank, doc);
        }
        for piece in self.comments.leading(block_end) {
            lines.push(piece.blank_before, piece.doc());
        }
    }

    /// The items nested inside a construct whose header ends at
    /// `head_end`: the comment trailing that header, and the body.
    fn items(&mut self, items: &[Item], head_end: u32, block_end: u32) -> (Doc, Doc) {
        let limit = items.first().map_or(block_end, |item| item.span.start);
        let trail = self.head_trail(head_end, limit);
        let mut lines = Lines::default();
        self.deeper(|rules| rules.item_lines(&mut lines, items, block_end));
        (trail, lines.finish())
    }

    /// The width of the left-hand side of a single continuous assignment,
    /// when the item is one and may take part in an alignment run.
    fn cont_assign_width(&self, item: &Item) -> Option<usize> {
        let ItemKind::ContAssign(assign) = &item.kind else {
            return None;
        };
        if !item.attrs.is_empty()
            || assign.strength.is_some()
            || assign.delay.is_some()
            || assign.assigns.len() != 1
        {
            return None;
        }
        let doc = self.expr(&assign.assigns[0].lhs);
        (!doc.has_hard_break()).then(|| doc.flat_width())
    }

    /// One item, with its attributes.
    fn item(&mut self, item: &Item, pad: usize) -> Doc {
        let attrs = self.attributes(&item.attrs);
        let body = self.item_kind(&item.kind, item.span, pad);
        if attrs.is_nil() {
            body
        } else {
            Doc::concat([attrs, Doc::hardline(), body])
        }
    }

    fn item_kind(&mut self, kind: &ItemKind, span: Span, pad: usize) -> Doc {
        match kind {
            ItemKind::Module(m) => self.module(m, span),
            ItemKind::Package(p) => self.package(p, span),
            ItemKind::Net(net) => self.net_decl(net).append(Doc::text(";")),
            ItemKind::Var(var) => self.var_decl(var).append(Doc::text(";")),
            ItemKind::Param(param) => self.param_decl(param).append(Doc::text(";")),
            ItemKind::Port(port) => self.port_decl(port).append(Doc::text(";")),
            ItemKind::Genvar(names) => Doc::text(format!(
                "genvar {};",
                names
                    .iter()
                    .map(|n| id(&n.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            ItemKind::Typedef(t) => self.typedef(t),
            ItemKind::Import(refs) => Doc::text(format!("import {};", self.package_refs(refs))),
            ItemKind::Export(refs) => Doc::text(format!("export {};", self.package_refs(refs))),
            ItemKind::Function(f) => self.subroutine(f, true, span),
            ItemKind::Task(t) => self.subroutine(t, false, span),
            ItemKind::Defparam(list) => {
                let parts: Vec<Doc> = list
                    .iter()
                    .map(|d| {
                        Doc::concat([self.expr(&d.target), Doc::text(" = "), self.expr(&d.value)])
                    })
                    .collect();
                Doc::concat([
                    Doc::text("defparam "),
                    Doc::join(Doc::text(", "), parts),
                    Doc::text(";"),
                ])
            }
            ItemKind::Specify(raw) => Doc::concat([
                Doc::text("specify"),
                self.closed_body(
                    if raw.text.is_empty() {
                        Doc::nil()
                    } else {
                        Doc::text(raw.text.clone())
                    },
                    "endspecify".to_owned(),
                ),
            ]),
            ItemKind::Initial(stmt) => self.attach(Doc::text("initial"), stmt),
            ItemKind::Final(stmt) => self.attach(Doc::text("final"), stmt),
            ItemKind::Always(kind, stmt) => self.attach(Doc::text(kind.as_str()), stmt),
            ItemKind::ContAssign(assign) => self.cont_assign(assign, pad),
            ItemKind::Gate(gate) => self.gate_decl(gate),
            ItemKind::Instance(inst) => self.instantiation(inst),
            ItemKind::Generate(items) => {
                let (trail, body) = self.items(items, span.start + 8, span.end);
                Doc::text("generate")
                    .append(trail)
                    .append(self.closed_body(body, "endgenerate".to_owned()))
            }
            ItemKind::GenIf(generate) => self.gen_if(generate),
            ItemKind::GenCase(generate) => self.gen_case(generate),
            ItemKind::GenFor(generate) => self.gen_for(generate),
            ItemKind::GenBlock(block) => self.gen_block(block, true),
            ItemKind::Alias(nets) => {
                let parts: Vec<Doc> = nets.iter().map(|n| self.expr(n)).collect();
                Doc::concat([
                    Doc::text("alias "),
                    Doc::join(Doc::text(" = "), parts),
                    Doc::text(";"),
                ])
            }
            ItemKind::Assertion(a) => self.assertion(a),
            ItemKind::Bind(bind) => {
                let mut parts = vec![Doc::text("bind "), self.expr(&bind.target)];
                if !bind.instances.is_empty() {
                    parts.push(Doc::text(" : "));
                    let list: Vec<Doc> = bind.instances.iter().map(|e| self.expr(e)).collect();
                    parts.push(Doc::join(Doc::text(", "), list));
                }
                parts.push(Doc::space());
                parts.push(self.instantiation(&bind.inst));
                Doc::concat(parts)
            }
            ItemKind::Clocking(c) => self.clocking(c),
            ItemKind::PropertyDecl(p) => {
                let kw = if p.is_sequence {
                    "sequence"
                } else {
                    "property"
                };
                let end = if p.is_sequence {
                    "endsequence"
                } else {
                    "endproperty"
                };
                let body = if p.body.text.is_empty() {
                    Doc::nil()
                } else {
                    Doc::text(p.body.text.clone())
                };
                Doc::concat([
                    Doc::text(format!("{kw} {}", id(&p.name.name))),
                    self.closed_body(body, end.to_owned()),
                ])
            }
            ItemKind::Modport(ports) => {
                let entries: Vec<Doc> = ports.iter().map(|m| self.modport(m)).collect();
                Doc::concat([
                    Doc::text("modport "),
                    Doc::join(Doc::text(", "), entries),
                    Doc::text(";"),
                ])
            }
            ItemKind::Timeunit { unit, precision } => {
                let mut text = format!("timeunit {}", self.literal(unit));
                if let Some(p) = precision {
                    let _ = write!(text, " / {}", self.literal(p));
                }
                text.push(';');
                Doc::text(text)
            }
            ItemKind::Timeprecision(lit) => {
                Doc::text(format!("timeprecision {};", self.literal(lit)))
            }
            ItemKind::Directive(d) => Doc::text(if d.args.is_empty() {
                format!("`{}", d.name)
            } else {
                format!("`{} {}", d.name, d.args)
            }),
            ItemKind::Table(rows) => {
                let mut lines = Lines::default();
                for row in rows {
                    lines.push(false, Doc::text(format!("{};", row.text)));
                }
                Doc::text("table").append(self.closed_body(lines.finish(), "endtable".to_owned()))
            }
            ItemKind::Empty => Doc::text(";"),
        }
    }

    /// `(* name = value, name *)`.
    fn attributes(&self, attrs: &[Attribute]) -> Doc {
        if attrs.is_empty() {
            return Doc::nil();
        }
        let entries: Vec<Doc> = attrs
            .iter()
            .map(|a| match &a.value {
                Some(v) => {
                    Doc::concat([Doc::text(id(&a.name.name)), Doc::text(" = "), self.expr(v)])
                }
                None => Doc::text(id(&a.name.name)),
            })
            .collect();
        Doc::concat([
            Doc::text("(* "),
            Doc::join(Doc::text(", "), entries),
            Doc::text(" *)"),
        ])
    }

    fn package_refs(&self, refs: &[PackageRef]) -> String {
        refs.iter()
            .map(|r| match &r.item {
                Some(item) => format!("{}::{}", id(&r.package.name), id(&item.name)),
                None => format!("{}::*", id(&r.package.name)),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A `module`, `interface`, `program` or `primitive` declaration.
    fn module(&mut self, m: &Module, span: Span) -> Doc {
        let end = match m.kind {
            ModuleKind::Module | ModuleKind::Macromodule => "endmodule",
            ModuleKind::Interface => "endinterface",
            ModuleKind::Program => "endprogram",
            ModuleKind::Primitive => "endprimitive",
        };
        // The port comments are claimed before the header is built, since
        // the header may be built twice, and a comment in the list forces
        // it to be broken.
        let port_comments = match &m.ports {
            Ports::Ansi(ports) => self.take_port_comments(ports),
            _ => Vec::new(),
        };
        let forced = port_comments
            .iter()
            .any(|c| !c.leading.is_empty() || c.trailing.is_some());
        let mut head = self.module_head(m, &port_comments, forced);
        if !forced && head.has_hard_break() {
            // One list had to break, so break them all: a one-line
            // parameter list above a stacked port list reads badly.
            head = self.module_head(m, &port_comments, true);
        }
        let (trail, body) = self.items(&m.items, module_head_end(m, span), span.end);
        head.append(trail)
            .append(self.closed_body(body, end.to_owned()))
    }

    /// The `module m #(...) (...);` header, with every list either on one
    /// line or one entry per line.
    fn module_head(&self, m: &Module, port_comments: &[PortComments], force: bool) -> Doc {
        let mut head = vec![Doc::text(m.kind.as_str())];
        if let Some(lifetime) = m.lifetime {
            head.push(Doc::text(format!(" {}", lifetime.as_str())));
        }
        head.push(Doc::text(format!(" {}", id(&m.name.name))));
        if !m.imports.is_empty() {
            head.push(Doc::text(format!(
                " import {};",
                self.package_refs(&m.imports)
            )));
        }
        if let Some(params) = &m.params {
            let width = Doc::concat(head.clone()).flat_width();
            head.push(Doc::text(" #"));
            head.push(self.param_port_list(params, width + 2, force));
        }
        match &m.ports {
            Ports::None => {}
            Ports::NonAnsi(ports) => {
                let width = Doc::concat(head.clone()).flat_width();
                let entries: Vec<Doc> = ports.iter().map(|p| self.non_ansi_port(p)).collect();
                let broken = entries.iter().cloned().map(|d| (d, Doc::nil())).collect();
                head.push(Doc::space());
                head.push(self.delimited_with_trails("(", entries, broken, ")", width + 1, force));
            }
            Ports::Ansi(ports) => {
                let width = Doc::concat(head.clone()).flat_width();
                head.push(Doc::space());
                head.push(self.ansi_port_list(ports, port_comments, width + 1, force));
            }
        }
        head.push(Doc::text(";"));
        Doc::concat(head)
    }

    /// Claims the comments written inside an ANSI port list, one entry per
    /// port. The last port has no trailing comment: anything after it
    /// belongs to the header line, which `head_trail` picks up.
    fn take_port_comments(&mut self, ports: &[Port]) -> Vec<PortComments> {
        let mut out = Vec::new();
        for (i, port) in ports.iter().enumerate() {
            let leading = self.comments.leading(port.span.start);
            self.comments.advance(port.span.end);
            let trailing = ports
                .get(i + 1)
                .and_then(|next| self.comments.trailing(port.span.end, next.span.start));
            out.push(PortComments { leading, trailing });
        }
        out
    }

    fn package(&mut self, p: &Package, span: Span) -> Doc {
        let mut head = String::from("package");
        if let Some(lifetime) = p.lifetime {
            let _ = write!(head, " {}", lifetime.as_str());
        }
        let _ = write!(head, " {};", id(&p.name.name));
        let (trail, body) = self.items(&p.items, p.name.span.end, span.end);
        Doc::text(head)
            .append(trail)
            .append(self.closed_body(body, "endpackage".to_owned()))
    }

    /// The `#( ... )` parameter port list of a module header.
    fn param_port_list(&self, params: &[ParamDecl], extra: usize, force: bool) -> Doc {
        let entries: Vec<Doc> = params.iter().map(|p| self.param_decl(p)).collect();
        let broken = entries.iter().cloned().map(|d| (d, Doc::nil())).collect();
        self.delimited_with_trails("(", entries, broken, ")", extra, force)
    }

    /// The ANSI port list of a module header, with its columns aligned and
    /// each port's own comments kept with it.
    fn ansi_port_list(
        &self,
        ports: &[Port],
        comments: &[PortComments],
        extra: usize,
        force: bool,
    ) -> Doc {
        let flat: Vec<Doc> = ports.iter().map(|p| self.ansi_port(p, None)).collect();
        let columns = self.opts.align_port_lists.then(|| self.port_columns(ports));
        let broken: Vec<(Doc, Doc)> = ports
            .iter()
            .enumerate()
            .map(|(i, port)| {
                let mut parts = Vec::new();
                let mut trail = Doc::nil();
                if let Some(comments) = comments.get(i) {
                    for piece in &comments.leading {
                        parts.push(piece.doc());
                        parts.push(Doc::hardline());
                    }
                    if let Some(piece) = &comments.trailing {
                        trail = Doc::space().append(piece.doc());
                    }
                }
                parts.push(self.ansi_port(port, columns.as_ref()));
                (Doc::concat(parts), trail)
            })
            .collect();
        self.delimited_with_trails("(", flat, broken, ")", extra, force)
    }

    /// The widths of the direction, type and packed-dimension columns of a
    /// port list.
    fn port_columns(&self, ports: &[Port]) -> [usize; 3] {
        let mut widths = [0usize; 3];
        for port in ports {
            for (i, piece) in self.port_pieces(port).iter().enumerate().take(3) {
                widths[i] = widths[i].max(piece.chars().count());
            }
        }
        widths
    }

    /// The direction, type and packed-dimension texts of one port.
    fn port_pieces(&self, port: &Port) -> [String; 3] {
        let direction = port
            .direction
            .map(Direction::as_str)
            .unwrap_or("")
            .to_owned();
        let mut ty = String::new();
        if let Some(net) = port.net_type {
            ty.push_str(net.as_str());
        }
        if port.var {
            if !ty.is_empty() {
                ty.push(' ');
            }
            ty.push_str("var");
        }
        let kind = self.type_kind(&port.data_type);
        if !kind.is_empty() {
            if !ty.is_empty() {
                ty.push(' ');
            }
            ty.push_str(&kind);
        }
        [direction, ty, self.packed_dims(&port.data_type)]
    }

    /// One ANSI port; `columns` pads it into an aligned list.
    fn ansi_port(&self, port: &Port, columns: Option<&[usize; 3]>) -> Doc {
        let pieces = self.port_pieces(port);
        let mut parts = Vec::new();
        let attrs = self.attributes(&port.attrs);
        if !attrs.is_nil() {
            parts.push(attrs);
            parts.push(Doc::space());
        }
        for (i, piece) in pieces.iter().enumerate() {
            let width = columns.map_or(piece.chars().count(), |c| c[i]);
            if width == 0 {
                continue;
            }
            if piece.is_empty() && columns.is_none() {
                continue;
            }
            let padding = width - piece.chars().count();
            parts.push(Doc::text(format!("{piece}{}", " ".repeat(padding))));
            parts.push(Doc::space());
        }
        parts.push(Doc::text(id(&port.name.name)));
        parts.push(self.dims(&port.dims));
        if let Some(default) = &port.default {
            parts.push(Doc::text(" = "));
            parts.push(self.expr(default));
        }
        Doc::concat(parts)
    }

    fn non_ansi_port(&self, port: &NonAnsiPort) -> Doc {
        match (&port.name, &port.expr) {
            (Some(name), Some(expr)) => Doc::concat([
                Doc::text(format!(".{}(", id(&name.name))),
                self.expr(expr),
                Doc::text(")"),
            ]),
            (Some(name), None) => Doc::text(format!(".{}()", id(&name.name))),
            (None, Some(expr)) => self.expr(expr),
            (None, None) => Doc::nil(),
        }
    }

    // --- declarations ----------------------------------------------------

    fn declarators(&self, decls: &[Declarator]) -> Doc {
        let entries: Vec<Doc> = decls
            .iter()
            .map(|d| {
                let mut parts = vec![Doc::text(id(&d.name.name)), self.dims(&d.dims)];
                if let Some(init) = &d.init {
                    parts.push(Doc::text(" = "));
                    parts.push(self.expr(init));
                }
                Doc::concat(parts)
            })
            .collect();
        Doc::join(Doc::text(", "), entries)
    }

    fn net_decl(&self, net: &NetDecl) -> Doc {
        let mut parts = vec![Doc::text(net.net_type.as_str())];
        if let Some(strength) = &net.strength {
            parts.push(Doc::space());
            parts.push(self.strength(strength));
        }
        if let Some(vectored) = net.vectored {
            parts.push(Doc::text(match vectored {
                Vectored::Vectored => " vectored",
                Vectored::Scalared => " scalared",
            }));
        }
        let ty = self.data_type(&net.data_type);
        if !ty.is_nil() {
            parts.push(Doc::space());
            parts.push(ty);
        }
        if let Some(delay) = &net.delay {
            parts.push(Doc::space());
            parts.push(self.delay(delay));
        }
        parts.push(Doc::space());
        parts.push(self.declarators(&net.decls));
        Doc::concat(parts)
    }

    fn var_decl(&self, var: &VarDecl) -> Doc {
        let mut parts = Vec::new();
        if let Some(lifetime) = var.lifetime {
            parts.push(Doc::text(format!("{} ", lifetime.as_str())));
        }
        if var.constant {
            parts.push(Doc::text("const "));
        }
        if var.var {
            parts.push(Doc::text("var "));
        }
        let ty = self.data_type(&var.data_type);
        if !ty.is_nil() {
            parts.push(ty);
            parts.push(Doc::space());
        }
        parts.push(self.declarators(&var.decls));
        Doc::concat(parts)
    }

    fn param_decl(&self, param: &ParamDecl) -> Doc {
        let mut parts = vec![Doc::text(param.kind.as_str())];
        if param.is_type {
            parts.push(Doc::text(" type"));
        }
        let ty = self.data_type(&param.data_type);
        if !ty.is_nil() {
            parts.push(Doc::space());
            parts.push(ty);
        }
        parts.push(Doc::space());
        parts.push(self.declarators(&param.decls));
        Doc::concat(parts)
    }

    fn port_decl(&self, port: &PortDecl) -> Doc {
        let mut parts = vec![Doc::text(port.direction.as_str())];
        if let Some(net) = port.net_type {
            parts.push(Doc::text(format!(" {}", net.as_str())));
        }
        if port.var {
            parts.push(Doc::text(" var"));
        }
        let ty = self.data_type(&port.data_type);
        if !ty.is_nil() {
            parts.push(Doc::space());
            parts.push(ty);
        }
        parts.push(Doc::space());
        parts.push(self.declarators(&port.decls));
        Doc::concat(parts)
    }

    fn typedef(&self, t: &Typedef) -> Doc {
        match &t.data_type {
            Some(ty) => Doc::concat([
                Doc::text("typedef "),
                self.data_type(ty),
                Doc::text(format!(" {}", id(&t.name.name))),
                self.dims(&t.dims),
                Doc::text(";"),
            ]),
            None => Doc::text(format!("typedef {};", id(&t.name.name))),
        }
    }

    fn subroutine(&mut self, sub: &Subroutine, is_function: bool, span: Span) -> Doc {
        let mut head = vec![Doc::text(if is_function { "function" } else { "task" })];
        if let Some(lifetime) = sub.lifetime {
            head.push(Doc::text(format!(" {}", lifetime.as_str())));
        }
        if let Some(ret) = &sub.ret {
            let ty = self.data_type(ret);
            if !ty.is_nil() {
                head.push(Doc::space());
                head.push(ty);
            }
        }
        head.push(Doc::text(format!(" {}", id(&sub.name.name))));
        if let Some(ports) = &sub.ports {
            let width = Doc::concat(head.clone()).flat_width();
            head.push(self.ansi_port_list(ports, &[], width, false));
        }
        head.push(Doc::text(";"));
        let head_end = sub.ports.as_ref().map_or(sub.name.span.end, |ports| {
            ports
                .iter()
                .map(|p| p.span.end)
                .max()
                .unwrap_or(sub.name.span.end)
        });
        let (trail, body) = self.stmts(&sub.body, head_end, span.end);
        let end = if is_function {
            "endfunction"
        } else {
            "endtask"
        };
        Doc::concat(head)
            .append(trail)
            .append(self.closed_body(body, end.to_owned()))
    }

    fn modport(&self, modport: &Modport) -> Doc {
        let entries: Vec<Doc> = modport
            .items
            .iter()
            .map(|item| match &item.kind {
                ModportItemKind::Port {
                    direction,
                    name,
                    expr,
                } => match expr {
                    Some(expr) => Doc::concat([
                        Doc::text(format!("{} .{}(", direction.as_str(), id(&name.name))),
                        self.expr(expr),
                        Doc::text(")"),
                    ]),
                    None => Doc::text(format!("{} {}", direction.as_str(), id(&name.name))),
                },
                ModportItemKind::Import(name) => Doc::text(format!("import {}", id(&name.name))),
                ModportItemKind::Export(name) => Doc::text(format!("export {}", id(&name.name))),
                ModportItemKind::Clocking(name) => {
                    Doc::text(format!("clocking {}", id(&name.name)))
                }
            })
            .collect();
        Doc::concat([
            Doc::text(format!("{} ", id(&modport.name.name))),
            // `modport ` before the name, and the `;` after the list.
            self.delimited(
                "(",
                entries.clone(),
                entries,
                ")",
                modport.name.name.len() + 10,
            ),
        ])
    }

    fn clocking(&self, c: &Clocking) -> Doc {
        let mut head = String::new();
        if c.is_default {
            head.push_str("default ");
        }
        if c.is_global {
            head.push_str("global ");
        }
        head.push_str("clocking");
        if let Some(name) = &c.name {
            let _ = write!(head, " {}", id(&name.name));
        }
        match &c.event {
            None => Doc::text(format!("{head};")),
            Some(event) => {
                let body = if c.body.text.is_empty() {
                    Doc::nil()
                } else {
                    Doc::text(c.body.text.clone())
                };
                Doc::concat([
                    Doc::text(head),
                    Doc::space(),
                    self.event_control(event),
                    Doc::text(";"),
                    self.closed_body(body, "endclocking".to_owned()),
                ])
            }
        }
    }

    // --- assignments, gates and instances --------------------------------

    fn cont_assign(&self, assign: &ContAssign, pad: usize) -> Doc {
        let mut parts = vec![Doc::text("assign")];
        if let Some(strength) = &assign.strength {
            parts.push(Doc::space());
            parts.push(self.strength(strength));
        }
        if let Some(delay) = &assign.delay {
            parts.push(Doc::space());
            parts.push(self.delay(delay));
        }
        parts.push(Doc::space());
        let pairs: Vec<Doc> = assign
            .assigns
            .iter()
            .enumerate()
            .map(|(i, pair)| {
                let lhs = self.expr(&pair.lhs);
                let lhs = if i == 0 { Self::pad(lhs, pad) } else { lhs };
                Doc::concat([
                    lhs,
                    Doc::text(" = "),
                    Doc::concat([Doc::softline(), self.expr(&pair.rhs)])
                        .indent()
                        .group(),
                ])
            })
            .collect();
        parts.push(Doc::join(Doc::text(", "), pairs));
        parts.push(Doc::text(";"));
        Doc::concat(parts)
    }

    fn gate_decl(&self, gate: &GateDecl) -> Doc {
        let mut parts = vec![Doc::text(gate.kind.as_str())];
        if let Some(strength) = &gate.strength {
            parts.push(Doc::space());
            parts.push(self.strength(strength));
        }
        if let Some(delay) = &gate.delay {
            parts.push(Doc::space());
            parts.push(self.delay(delay));
        }
        parts.push(Doc::space());
        let instances: Vec<Doc> = gate
            .instances
            .iter()
            .map(|inst| {
                let mut head = Vec::new();
                if let Some(name) = &inst.name {
                    head.push(Doc::text(id(&name.name)));
                }
                if !inst.dims.is_empty() {
                    head.push(Doc::space());
                    head.push(self.dims(&inst.dims));
                }
                if !head.is_empty() {
                    head.push(Doc::space());
                }
                let conns: Vec<Doc> = inst.conns.iter().map(|c| self.expr(c)).collect();
                head.push(Doc::concat([
                    Doc::text("("),
                    Doc::join(Doc::text(", "), conns),
                    Doc::text(")"),
                ]));
                Doc::concat(head)
            })
            .collect();
        parts.push(Doc::join(Doc::text(", "), instances));
        parts.push(Doc::text(";"));
        Doc::concat(parts)
    }

    fn instantiation(&self, inst: &Instantiation) -> Doc {
        let mut parts = vec![Doc::text(id(&inst.module.name))];
        if !inst.params.is_empty() {
            let entries: Vec<Doc> = inst
                .params
                .iter()
                .map(|p| match (&p.name, &p.value) {
                    (Some(name), Some(value)) => Doc::concat([
                        Doc::text(format!(".{}(", id(&name.name))),
                        self.expr(value),
                        Doc::text(")"),
                    ]),
                    (Some(name), None) => Doc::text(format!(".{}()", id(&name.name))),
                    (None, Some(value)) => self.expr(value),
                    (None, None) => Doc::nil(),
                })
                .collect();
            parts.push(Doc::text(" #"));
            let width = inst.module.name.chars().count() + 3;
            parts.push(self.delimited("(", entries.clone(), entries, ")", width));
        }
        let width = Doc::concat(parts.clone()).flat_width();
        let instances: Vec<Doc> = inst
            .instances
            .iter()
            .map(|one| self.instance(one, width))
            .collect();
        parts.push(Doc::space());
        parts.push(Doc::join(Doc::text(", "), instances));
        parts.push(Doc::text(";"));
        Doc::concat(parts)
    }

    fn instance(&self, inst: &Instance, extra: usize) -> Doc {
        let mut parts = Vec::new();
        if let Some(name) = &inst.name {
            parts.push(Doc::text(id(&name.name)));
        }
        if !inst.dims.is_empty() {
            parts.push(Doc::space());
            parts.push(self.dims(&inst.dims));
        }
        if !parts.is_empty() {
            parts.push(Doc::space());
        }
        let width = inst
            .conns
            .iter()
            .filter_map(|conn| match &conn.kind {
                PortConnKind::Named { name, conn } if !matches!(conn, NamedConn::Implicit) => {
                    Some(name.name.chars().count())
                }
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let flat: Vec<Doc> = inst.conns.iter().map(|c| self.port_conn(c, 0)).collect();
        let broken: Vec<Doc> = inst
            .conns
            .iter()
            .map(|c| self.port_conn(c, if self.opts.align_port_lists { width } else { 0 }))
            .collect();
        let head = Doc::concat(parts);
        let extra = extra + head.flat_width();
        Doc::concat([head, self.delimited("(", flat, broken, ")", extra)])
    }

    fn port_conn(&self, conn: &PortConn, width: usize) -> Doc {
        match &conn.kind {
            PortConnKind::Positional(None) => Doc::nil(),
            PortConnKind::Positional(Some(expr)) => self.expr(expr),
            PortConnKind::Wildcard => Doc::text(".*"),
            PortConnKind::Named { name, conn } => {
                // The space before the parenthesis is what the aligned
                // form lines up on; a one-line list writes `.name(net)`.
                let gap = if width == 0 {
                    String::new()
                } else {
                    format!(
                        "{} ",
                        " ".repeat(width.saturating_sub(name.name.chars().count()))
                    )
                };
                match conn {
                    NamedConn::Implicit => Doc::text(format!(".{}", id(&name.name))),
                    NamedConn::Open => Doc::text(format!(".{}{gap}()", id(&name.name))),
                    NamedConn::Expr(expr) => Doc::concat([
                        Doc::text(format!(".{}{gap}(", id(&name.name))),
                        self.expr(expr),
                        Doc::text(")"),
                    ]),
                }
            }
        }
    }

    // --- generate --------------------------------------------------------

    fn gen_block(&mut self, block: &GenBlock, force: bool) -> Doc {
        if block.items.is_empty() && block.label.is_none() && !force {
            return Doc::text(";");
        }
        let head = match &block.label {
            Some(label) => format!("begin : {}", id(&label.name)),
            None => "begin".to_owned(),
        };
        let head_end = block
            .label
            .as_ref()
            .map_or(block.span.start + 5, |label| label.span.end);
        let (trail, body) = self.items(&block.items, head_end, block.span.end);
        Doc::text(head)
            .append(trail)
            .append(self.closed_body(body, format!("end{}", self.end_label(block.label.as_ref()))))
    }

    fn gen_if(&mut self, generate: &GenIf) -> Doc {
        let mut parts = vec![
            Doc::text("if ("),
            self.expr(&generate.cond),
            Doc::text(") "),
            self.gen_block(&generate.then_block, false),
        ];
        if let Some(else_block) = &generate.else_block {
            parts.push(Doc::text(" else "));
            parts.push(self.gen_block(else_block, false));
        }
        Doc::concat(parts)
    }

    fn gen_case(&mut self, generate: &GenCase) -> Doc {
        let width = generate
            .items
            .iter()
            .map(|item| self.gen_case_label(item).chars().count())
            .max()
            .unwrap_or(0);
        let mut lines = Lines::default();
        for item in &generate.items {
            let label = self.gen_case_label(item);
            let padding = " ".repeat(width - label.chars().count());
            let block = self.deeper(|rules| rules.gen_block(&item.block, false));
            lines.push(false, Doc::text(format!("{label}{padding} ")).append(block));
        }
        let body = lines.finish();
        Doc::concat([
            Doc::text("case ("),
            self.expr(&generate.expr),
            Doc::text(")"),
            if body.is_nil() {
                Doc::nil()
            } else {
                Doc::concat([Doc::hardline(), body]).indent()
            },
            Doc::hardline(),
            Doc::text("endcase"),
        ])
    }

    fn gen_case_label(&self, item: &GenCaseItem) -> String {
        if item.patterns.is_empty() {
            "default:".to_owned()
        } else {
            let list: Vec<String> = item
                .patterns
                .iter()
                .map(|p| self.expr(p).flat_text())
                .collect();
            format!("{}:", list.join(", "))
        }
    }

    fn gen_for(&mut self, generate: &GenFor) -> Doc {
        let init = format!(
            "for ({}{} = {}; ",
            if generate.genvar { "genvar " } else { "" },
            id(&generate.var.name),
            self.expr(&generate.init).flat_text()
        );
        Doc::concat([
            Doc::text(init),
            self.expr(&generate.cond),
            Doc::text("; "),
            self.expr_any(&generate.step),
            Doc::text(") "),
            self.gen_block(&generate.body, true),
        ])
    }

    // --- statements ------------------------------------------------------

    /// The statements of a block whose header ends at `head_end`: the
    /// comment trailing that header, and the body.
    fn stmts(&mut self, stmts: &[Stmt], head_end: u32, block_end: u32) -> (Doc, Doc) {
        let limit = stmts.first().map_or(block_end, |stmt| stmt.span.start);
        let trail = self.head_trail(head_end, limit);
        let mut lines = Lines::default();
        self.deeper(|rules| {
            let pads = rules.alignment(
                stmts,
                |rules, stmt| rules.assign_width(stmt),
                |stmt| stmt.span,
            );
            for (i, stmt) in stmts.iter().enumerate() {
                for piece in rules.comments.leading(stmt.span.start) {
                    lines.push(piece.blank_before, piece.doc());
                }
                let blank = rules.comments.gap_blank(stmt.span.start);
                let doc = rules.stmt(stmt, pads[i]);
                rules.comments.advance(stmt.span.end);
                let next = stmts
                    .get(i + 1)
                    .map_or(block_end, |next| next.span.start.min(block_end));
                let doc = match rules.comments.trailing(stmt.span.end, next) {
                    Some(piece) => doc.append(Doc::space()).append(piece.doc()),
                    None => doc,
                };
                lines.push(blank, doc);
            }
            for piece in rules.comments.leading(block_end) {
                lines.push(piece.blank_before, piece.doc());
            }
        });
        (trail, lines.finish())
    }

    /// The width of the target of a simple assignment statement, for
    /// aligning a run of them.
    fn assign_width(&self, stmt: &Stmt) -> Option<usize> {
        let StmtKind::Assign(assign) = &stmt.kind else {
            return None;
        };
        if stmt.label.is_some() || !stmt.attrs.is_empty() || assign.timing.is_some() {
            return None;
        }
        let doc = self.expr(&assign.lhs);
        (!doc.has_hard_break()).then(|| doc.flat_width())
    }

    /// One statement, with its label and attributes.
    fn stmt(&mut self, stmt: &Stmt, pad: usize) -> Doc {
        let mut parts = Vec::new();
        if let Some(label) = &stmt.label {
            parts.push(Doc::text(format!("{} : ", id(&label.name))));
        }
        let attrs = self.attributes(&stmt.attrs);
        if !attrs.is_nil() {
            parts.push(attrs);
            parts.push(Doc::space());
        }
        parts.push(self.stmt_kind(&stmt.kind, stmt.span, pad));
        Doc::concat(parts)
    }

    fn stmt_kind(&mut self, kind: &StmtKind, span: Span, pad: usize) -> Doc {
        match kind {
            StmtKind::Null => Doc::text(";"),
            StmtKind::Block(block) => self.block(block, "begin", "end"),
            StmtKind::Fork(block, join) => self.block(block, "fork", join.as_str()),
            StmtKind::Assign(assign) => {
                let mut parts = vec![
                    Self::pad(self.expr(&assign.lhs), pad),
                    Doc::text(format!(" {} ", assign.op.as_str())),
                ];
                if let Some(timing) = &assign.timing {
                    parts.push(self.timing(timing));
                    parts.push(Doc::space());
                }
                parts.push(
                    Doc::concat([Doc::softline(), self.expr(&assign.rhs)])
                        .indent()
                        .group(),
                );
                parts.push(Doc::text(";"));
                Doc::concat(parts)
            }
            StmtKind::Expr(expr) => self.expr(expr).append(Doc::text(";")),
            StmtKind::If(if_stmt) => self.if_stmt(if_stmt),
            StmtKind::Case(case) => self.case(case, span),
            StmtKind::For(for_stmt) => {
                let mut head = vec![Doc::text("for (")];
                let inits: Vec<Doc> = for_stmt
                    .init
                    .iter()
                    .map(|init| match init {
                        ForInit::Decl(decl) => self.var_decl(decl),
                        ForInit::Assign(expr) => self.expr_any(expr),
                    })
                    .collect();
                head.push(Doc::join(Doc::text(", "), inits));
                head.push(Doc::text("; "));
                if let Some(cond) = &for_stmt.cond {
                    head.push(self.expr(cond));
                }
                head.push(Doc::text("; "));
                let steps: Vec<Doc> = for_stmt.step.iter().map(|s| self.expr_any(s)).collect();
                head.push(Doc::join(Doc::text(", "), steps));
                head.push(Doc::text(")"));
                self.attach(Doc::concat(head), &for_stmt.body)
            }
            StmtKind::While(cond, body) => {
                let head = Doc::concat([Doc::text("while ("), self.expr(cond), Doc::text(")")]);
                self.attach(head, body)
            }
            StmtKind::DoWhile(body, cond) => {
                let done = self.attach(Doc::text("do"), body);
                Doc::concat([
                    done,
                    Doc::text(" while ("),
                    self.expr(cond),
                    Doc::text(");"),
                ])
            }
            StmtKind::Repeat(count, body) => {
                let head = Doc::concat([Doc::text("repeat ("), self.expr(count), Doc::text(")")]);
                self.attach(head, body)
            }
            StmtKind::Forever(body) => self.attach(Doc::text("forever"), body),
            StmtKind::Foreach(foreach) => {
                let vars: Vec<String> = foreach
                    .vars
                    .iter()
                    .map(|v| v.as_ref().map_or(String::new(), |i| id(&i.name)))
                    .collect();
                let head = Doc::concat([
                    Doc::text("foreach ("),
                    self.expr(&foreach.array),
                    Doc::text(format!("[{}])", vars.join(", "))),
                ]);
                self.attach(head, &foreach.body)
            }
            StmtKind::Break => Doc::text("break;"),
            StmtKind::Continue => Doc::text("continue;"),
            StmtKind::Return(None) => Doc::text("return;"),
            StmtKind::Return(Some(expr)) => {
                Doc::concat([Doc::text("return "), self.expr(expr), Doc::text(";")])
            }
            StmtKind::Disable(target) => {
                Doc::concat([Doc::text("disable "), self.expr(target), Doc::text(";")])
            }
            StmtKind::DisableFork => Doc::text("disable fork;"),
            StmtKind::Timing(timing, body) => {
                let head = self.timing(timing);
                if matches!(body.kind, StmtKind::Null) && body.label.is_none() {
                    head.append(Doc::text(";"))
                } else {
                    self.attach(head, body)
                }
            }
            StmtKind::Wait(cond, body) => {
                let head = Doc::concat([Doc::text("wait ("), self.expr(cond), Doc::text(")")]);
                self.attach(head, body)
            }
            StmtKind::WaitFork => Doc::text("wait fork;"),
            StmtKind::ProcAssign(lhs, rhs) => Doc::concat([
                Doc::text("assign "),
                self.expr(lhs),
                Doc::text(" = "),
                self.expr(rhs),
                Doc::text(";"),
            ]),
            StmtKind::Deassign(lhs) => {
                Doc::concat([Doc::text("deassign "), self.expr(lhs), Doc::text(";")])
            }
            StmtKind::Force(lhs, rhs) => Doc::concat([
                Doc::text("force "),
                self.expr(lhs),
                Doc::text(" = "),
                self.expr(rhs),
                Doc::text(";"),
            ]),
            StmtKind::Release(lhs) => {
                Doc::concat([Doc::text("release "), self.expr(lhs), Doc::text(";")])
            }
            StmtKind::Trigger {
                nonblocking,
                target,
            } => Doc::concat([
                Doc::text(if *nonblocking { "->> " } else { "-> " }),
                self.expr(target),
                Doc::text(";"),
            ]),
            StmtKind::Assert(assertion) => self.assertion(assertion),
            StmtKind::Decl(item) => self.item(item, 0),
        }
    }

    fn block(&mut self, block: &Block, open: &str, close: &str) -> Doc {
        let head = match &block.label {
            Some(label) => format!("{open} : {}", id(&label.name)),
            None => open.to_owned(),
        };
        let head_end = block.label.as_ref().map_or(
            block.span.start + u32::try_from(open.len()).expect("keyword fits u32"),
            |label| label.span.end,
        );
        let (trail, body) = self.stmts(&block.stmts, head_end, block.span.end);
        Doc::text(head).append(trail).append(self.closed_body(
            body,
            format!("{close}{}", self.end_label(block.label.as_ref())),
        ))
    }

    /// A construct and the statement it introduces: on the same line when
    /// the statement's first line still fits there (which is how a `begin`
    /// block or an `@(posedge clk)` stays put), on the next line, indented,
    /// when it does not.
    fn attach(&mut self, head: Doc, stmt: &Stmt) -> Doc {
        let body = self.deeper(|rules| rules.stmt(stmt, 0));
        if self.cols() + head.flat_width() + 1 + first_line_width(&body) <= self.opts.line_width {
            head.append(Doc::space()).append(body)
        } else {
            head.append(Doc::concat([Doc::hardline(), body]).indent())
        }
    }

    /// Wraps a statement in `begin`/`end` when leaving it bare would let a
    /// following `else` attach to the wrong `if`.
    fn dangling(stmt: &Stmt) -> bool {
        match &stmt.kind {
            StmtKind::If(inner) => {
                inner.else_stmt.is_none()
                    || Self::dangling(inner.else_stmt.as_ref().expect("checked"))
            }
            StmtKind::For(f) => Self::dangling(&f.body),
            StmtKind::While(_, body)
            | StmtKind::Repeat(_, body)
            | StmtKind::Forever(body)
            | StmtKind::Wait(_, body)
            | StmtKind::Timing(_, body) => Self::dangling(body),
            StmtKind::Foreach(f) => Self::dangling(&f.body),
            _ => false,
        }
    }

    fn if_stmt(&mut self, if_stmt: &If) -> Doc {
        let mut parts = Vec::new();
        if let Some(qualifier) = if_stmt.qualifier {
            parts.push(Doc::text(format!("{} ", qualifier.as_str())));
        }
        parts.push(Doc::text("if ("));
        parts.push(self.expr(&if_stmt.cond));
        parts.push(Doc::text(")"));
        if if_stmt.else_stmt.is_some() && Self::dangling(&if_stmt.then_stmt) {
            // `if (a) if (b) x; else y;` would reattach the `else`.
            let (_, body) = self.stmts(
                std::slice::from_ref(&if_stmt.then_stmt),
                if_stmt.then_stmt.span.start,
                if_stmt.then_stmt.span.end,
            );
            parts.push(Doc::space());
            parts.push(Doc::text("begin").append(self.closed_body(body, "end".to_owned())));
        } else {
            let head = Doc::concat(std::mem::take(&mut parts));
            parts.push(self.attach(head, &if_stmt.then_stmt));
        }
        if let Some(else_stmt) = &if_stmt.else_stmt {
            // `end else` reads better than an `else` on its own line, but a
            // one-line `then` branch must not run into the `else`.
            let after_block = matches!(
                if_stmt.then_stmt.kind,
                StmtKind::Block(_) | StmtKind::Fork(..)
            ) || Self::dangling(&if_stmt.then_stmt);
            let head = if after_block {
                Doc::text(" else")
            } else {
                parts.push(Doc::hardline());
                Doc::text("else")
            };
            if matches!(else_stmt.kind, StmtKind::If(_)) && else_stmt.label.is_none() {
                parts.push(head);
                parts.push(Doc::space());
                parts.push(self.stmt(else_stmt, 0));
            } else {
                parts.push(self.attach(head, else_stmt));
            }
        }
        Doc::concat(parts)
    }

    fn case(&mut self, case: &Case, span: Span) -> Doc {
        let mut head = Vec::new();
        if let Some(qualifier) = case.qualifier {
            head.push(Doc::text(format!("{} ", qualifier.as_str())));
        }
        head.push(Doc::text(format!("{} (", case.kind.as_str())));
        head.push(self.expr(&case.expr));
        head.push(Doc::text(if case.inside { ") inside" } else { ")" }));

        let labels: Vec<String> = case.items.iter().map(|i| self.case_label(i)).collect();
        let width = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let mut lines = Lines::default();
        let block_end = span.end;
        self.deeper(|rules| {
            for (item, label) in case.items.iter().zip(&labels) {
                for piece in rules.comments.leading(item.span.start) {
                    lines.push(piece.blank_before, piece.doc());
                }
                let blank = rules.comments.gap_blank(item.span.start);
                let padding = " ".repeat(width - label.chars().count());
                let head = Doc::text(format!("{label}{padding}"));
                let doc = rules.case_body(head, &item.body);
                rules.comments.advance(item.span.end);
                lines.push(blank, doc);
            }
            for piece in rules.comments.leading(block_end) {
                lines.push(piece.blank_before, piece.doc());
            }
        });
        let body = lines.finish();
        Doc::concat([
            Doc::concat(head),
            if body.is_nil() {
                Doc::nil()
            } else {
                Doc::concat([Doc::hardline(), body]).indent()
            },
            Doc::hardline(),
            Doc::text("endcase"),
        ])
    }

    fn case_label(&self, item: &CaseItem) -> String {
        if item.patterns.is_empty() {
            "default:".to_owned()
        } else {
            let list: Vec<String> = item
                .patterns
                .iter()
                .map(|p| self.expr(p).flat_text())
                .collect();
            format!("{}:", list.join(", "))
        }
    }

    fn case_body(&mut self, head: Doc, stmt: &Stmt) -> Doc {
        let block =
            stmt.label.is_none() && matches!(stmt.kind, StmtKind::Block(_) | StmtKind::Fork(..));
        if !self.opts.case_items_on_one_line && !block {
            let body = self.deeper(|rules| rules.stmt(stmt, 0));
            return head.append(Doc::concat([Doc::hardline(), body]).indent());
        }
        self.attach(head, stmt)
    }

    fn assertion(&mut self, assertion: &Assertion) -> Doc {
        let mut parts = Vec::new();
        if let Some(label) = &assertion.label {
            parts.push(Doc::text(format!("{} : ", id(&label.name))));
        }
        parts.push(Doc::text(assertion.kind.as_str()));
        match assertion.deferred {
            Some(Deferred::Observed) => parts.push(Doc::text(" #0")),
            Some(Deferred::Final) => parts.push(Doc::text(" final")),
            None => {}
        }
        match &assertion.spec {
            AssertSpec::Expr(expr) => {
                parts.push(Doc::text(" ("));
                parts.push(self.expr(expr));
                parts.push(Doc::text(")"));
            }
            AssertSpec::Property(raw) => {
                parts.push(Doc::text(format!(" property ({})", raw.text)));
            }
        }
        match (&assertion.then_stmt, &assertion.else_stmt) {
            (None, None) => parts.push(Doc::text(";")),
            (then_stmt, else_stmt) => {
                if let Some(stmt) = then_stmt {
                    let head = Doc::concat(std::mem::take(&mut parts));
                    parts.push(self.attach(head, stmt));
                }
                if let Some(stmt) = else_stmt {
                    parts.push(self.attach(Doc::text(" else"), stmt));
                }
            }
        }
        Doc::concat(parts)
    }

    // --- timing ----------------------------------------------------------

    fn timing(&self, timing: &TimingControl) -> Doc {
        match &timing.kind {
            TimingKind::Delay(delay) => self.delay(delay),
            TimingKind::Event(event) => self.event_control(event),
            TimingKind::RepeatEvent(count, event) => Doc::concat([
                Doc::text("repeat ("),
                self.expr(count),
                Doc::text(") "),
                self.event_control(event),
            ]),
        }
    }

    fn delay(&self, delay: &Delay) -> Doc {
        match delay.values.as_slice() {
            [one]
                if matches!(
                    one.kind,
                    ExprKind::Literal(_) | ExprKind::Ident(_) | ExprKind::Scoped { .. }
                ) =>
            {
                Doc::text("#").append(self.expr(one))
            }
            values => {
                let list: Vec<Doc> = values.iter().map(|v| self.expr(v)).collect();
                Doc::concat([
                    Doc::text("#("),
                    Doc::join(Doc::text(", "), list),
                    Doc::text(")"),
                ])
            }
        }
    }

    fn event_control(&self, event: &EventControl) -> Doc {
        match &event.kind {
            EventControlKind::Any => Doc::text("@*"),
            EventControlKind::List(list) => {
                let entries: Vec<Doc> = list
                    .iter()
                    .map(|e| {
                        let mut parts = Vec::new();
                        if let Some(edge) = e.edge {
                            parts.push(Doc::text(format!("{} ", edge.as_str())));
                        }
                        parts.push(self.expr(&e.expr));
                        if let Some(iff) = &e.iff {
                            parts.push(Doc::text(" iff "));
                            parts.push(self.expr(iff));
                        }
                        Doc::concat(parts)
                    })
                    .collect();
                Doc::concat([
                    Doc::text("@("),
                    Doc::join(Doc::text(" or "), entries),
                    Doc::text(")"),
                ])
            }
        }
    }

    fn strength(&self, strength: &Strength) -> Doc {
        let list: Vec<&str> = strength.levels.iter().map(|l| l.as_str()).collect();
        Doc::text(format!("({})", list.join(", ")))
    }

    // --- types -----------------------------------------------------------

    /// The type without its packed dimensions.
    fn type_kind(&self, ty: &DataType) -> String {
        let mut out = match &ty.kind {
            DataTypeKind::Implicit => String::new(),
            DataTypeKind::Integer(i) => i.as_str().to_owned(),
            DataTypeKind::Real(r) => r.as_str().to_owned(),
            DataTypeKind::String => "string".to_owned(),
            DataTypeKind::Chandle => "chandle".to_owned(),
            DataTypeKind::Event => "event".to_owned(),
            DataTypeKind::Void => "void".to_owned(),
            DataTypeKind::Enum(_) | DataTypeKind::Struct(_) => {
                // Rendered by `data_type_body`, signing included.
                return self.data_type_body(ty).flat_text();
            }
            DataTypeKind::Named {
                package,
                name,
                member,
            } => {
                let mut text = String::new();
                if let Some(package) = package {
                    let _ = write!(text, "{}::", id(&package.name));
                }
                text.push_str(&id(&name.name));
                if let Some(member) = member {
                    let _ = write!(text, ".{}", id(&member.name));
                }
                text
            }
            DataTypeKind::Interface { modport } => match modport {
                Some(m) => format!("interface.{}", id(&m.name)),
                None => "interface".to_owned(),
            },
            DataTypeKind::TypeOf(expr) => format!("type({})", self.expr(expr).flat_text()),
        };
        if let Some(signing) = ty.signing {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(match signing {
                Signing::Signed => "signed",
                Signing::Unsigned => "unsigned",
            });
        }
        out
    }

    /// The packed dimensions of a type, as text.
    fn packed_dims(&self, ty: &DataType) -> String {
        self.dims(&ty.packed).flat_text()
    }

    /// A complete data type: kind, signing and packed dimensions.
    fn data_type(&self, ty: &DataType) -> Doc {
        if ty.is_empty() {
            return Doc::nil();
        }
        let body = match &ty.kind {
            DataTypeKind::Enum(_) | DataTypeKind::Struct(_) => self.data_type_body(ty),
            _ => Doc::text(self.type_kind(ty)),
        };
        let dims = self.dims(&ty.packed);
        match (body.is_nil(), dims.is_nil()) {
            (_, true) => body,
            (true, false) => dims,
            (false, false) => body.append(Doc::space()).append(dims),
        }
    }

    /// The `{ ... }` body of an `enum`, `struct` or `union` type.
    fn data_type_body(&self, ty: &DataType) -> Doc {
        match &ty.kind {
            DataTypeKind::Enum(e) => {
                let mut head = String::from("enum");
                if let Some(base) = &e.base {
                    let _ = write!(head, " {}", self.data_type(base).flat_text());
                }
                head.push_str(signing_text(ty.signing));
                let entries: Vec<Doc> = e
                    .variants
                    .iter()
                    .map(|v| {
                        let mut text = id(&v.name.name);
                        if let Some(range) = &v.range {
                            text.push_str(&self.dim(range).flat_text());
                        }
                        match &v.value {
                            Some(value) => {
                                Doc::concat([Doc::text(text), Doc::text(" = "), self.expr(value)])
                            }
                            None => Doc::text(text),
                        }
                    })
                    .collect();
                Doc::concat([
                    Doc::text(format!("{head} ")),
                    self.delimited("{", entries.clone(), entries, "}", head.len() + 1),
                ])
            }
            DataTypeKind::Struct(s) => {
                let mut head = String::from(if s.is_union { "union" } else { "struct" });
                if s.packed {
                    head.push_str(" packed");
                }
                if s.tagged {
                    head.push_str(" tagged");
                }
                head.push_str(signing_text(ty.signing));
                let members = self.struct_members(&s.members);
                Doc::concat([
                    Doc::text(format!("{head} {{")),
                    Doc::concat([Doc::hardline(), members]).indent(),
                    Doc::hardline(),
                    Doc::text("}"),
                ])
            }
            _ => Doc::nil(),
        }
    }

    fn struct_members(&self, members: &[StructMember]) -> Doc {
        let width = if self.opts.align_port_lists {
            members
                .iter()
                .map(|m| self.data_type(&m.data_type).flat_width())
                .max()
                .unwrap_or(0)
        } else {
            0
        };
        let mut lines = Lines::default();
        for member in members {
            let ty = self.data_type(&member.data_type);
            let padding = width.saturating_sub(ty.flat_width());
            lines.push(
                false,
                Doc::concat([
                    Self::pad(ty, padding),
                    Doc::space(),
                    self.declarators(&member.decls),
                    Doc::text(";"),
                ]),
            );
        }
        lines.finish()
    }

    fn dims(&self, dims: &[Dim]) -> Doc {
        Doc::concat(dims.iter().map(|d| self.dim(d)))
    }

    fn dim(&self, dim: &Dim) -> Doc {
        match &dim.kind {
            DimKind::Range(a, b) => Doc::concat([
                Doc::text("["),
                self.expr(a),
                Doc::text(":"),
                self.expr(b),
                Doc::text("]"),
            ]),
            DimKind::Size(e) => Doc::concat([Doc::text("["), self.expr(e), Doc::text("]")]),
            DimKind::Unsized => Doc::text("[]"),
            DimKind::Queue(None) => Doc::text("[$]"),
            DimKind::Queue(Some(e)) => {
                Doc::concat([Doc::text("[$:"), self.expr(e), Doc::text("]")])
            }
            DimKind::Assoc(None) => Doc::text("[*]"),
            DimKind::Assoc(Some(ty)) => {
                Doc::concat([Doc::text("["), self.data_type(ty), Doc::text("]")])
            }
        }
    }

    // --- expressions -----------------------------------------------------

    /// An expression in an ordinary position, where a bare assignment
    /// expression would not parse back.
    fn expr(&self, expr: &Expr) -> Doc {
        self.expr_prec(expr, prec::ASSIGN + 1)
    }

    /// An expression in a `for` header, where an assignment expression is
    /// exactly what is expected.
    fn expr_any(&self, expr: &Expr) -> Doc {
        self.expr_prec(expr, 0)
    }

    /// An expression, parenthesised when its precedence is below `min`.
    fn expr_prec(&self, expr: &Expr, min: u8) -> Doc {
        let doc = self.expr_inner(expr);
        if expr_prec(&expr.kind) < min {
            Doc::concat([Doc::text("("), doc, Doc::text(")")])
        } else {
            doc
        }
    }

    fn expr_inner(&self, expr: &Expr) -> Doc {
        match &expr.kind {
            ExprKind::Literal(lit) => Doc::text(self.literal(lit)),
            ExprKind::Ident(name) => Doc::text(id(&name.name)),
            ExprKind::SystemIdent(name) => Doc::text(format!("${}", id(&name.name))),
            ExprKind::Scoped { scope, name } => Doc::concat([
                self.expr_prec(scope, prec::PRIMARY),
                Doc::text(format!("::{}", id(&name.name))),
            ]),
            ExprKind::Member { base, name } => Doc::concat([
                self.expr_prec(base, prec::PRIMARY),
                Doc::text(format!(".{}", id(&name.name))),
            ]),
            ExprKind::Index { base, index } => Doc::concat([
                self.expr_prec(base, prec::PRIMARY),
                Doc::text("["),
                self.expr(index),
                Doc::text("]"),
            ]),
            ExprKind::Range {
                base,
                kind,
                left,
                right,
            } => Doc::concat([
                self.expr_prec(base, prec::PRIMARY),
                Doc::text("["),
                self.expr(left),
                Doc::text(kind.as_str()),
                self.expr(right),
                Doc::text("]"),
            ]),
            ExprKind::Unary { op, operand } => {
                let inner = self.expr_prec(operand, prec::UNARY);
                let text = op.as_str();
                let needs_space = inner.flat_text().starts_with(['+', '-', '~', '&', '|']);
                Doc::concat([
                    Doc::text(text),
                    if needs_space {
                        Doc::space()
                    } else {
                        Doc::nil()
                    },
                    inner,
                ])
            }
            ExprKind::Binary { op, .. } => self.binary_chain(expr, *op),
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => Doc::concat([
                self.expr_prec(cond, prec::TERNARY + 1),
                Doc::line(),
                Doc::text("? "),
                self.expr_prec(then_expr, 0),
                Doc::line(),
                Doc::text(": "),
                self.expr_prec(else_expr, prec::TERNARY),
            ])
            .indent()
            .group(),
            ExprKind::Concat(elems) => self.braced(elems),
            ExprKind::Replicate { count, elems } => Doc::concat([
                Doc::text("{"),
                self.expr(count),
                self.braced(elems),
                Doc::text("}"),
            ]),
            ExprKind::Streaming {
                right_to_left,
                slice,
                elems,
            } => {
                let mut parts = vec![Doc::text(if *right_to_left { "{<<" } else { "{>>" })];
                if let Some(slice) = slice {
                    parts.push(Doc::space());
                    parts.push(self.expr(slice));
                }
                parts.push(Doc::space());
                parts.push(self.braced(elems));
                parts.push(Doc::text("}"));
                Doc::concat(parts)
            }
            ExprKind::Pattern(items) => {
                let entries: Vec<Doc> = items
                    .iter()
                    .map(|item| match &item.key {
                        Some(key) => {
                            Doc::concat([self.expr(key), Doc::text(": "), self.expr(&item.value)])
                        }
                        None => self.expr(&item.value),
                    })
                    .collect();
                Doc::concat([
                    Doc::text("'{"),
                    Doc::concat([
                        Doc::softline(),
                        Doc::join(Doc::concat([Doc::text(","), Doc::line()]), entries),
                    ])
                    .indent(),
                    Doc::softline(),
                    Doc::text("}"),
                ])
                .group()
            }
            ExprKind::Call { callee, args } => {
                let entries: Vec<Doc> = args
                    .iter()
                    .map(|arg| match (&arg.name, &arg.value) {
                        (Some(name), Some(value)) => Doc::concat([
                            Doc::text(format!(".{}(", id(&name.name))),
                            self.expr(value),
                            Doc::text(")"),
                        ]),
                        (Some(name), None) => Doc::text(format!(".{}()", id(&name.name))),
                        (None, Some(value)) => self.expr(value),
                        (None, None) => Doc::nil(),
                    })
                    .collect();
                Doc::concat([
                    self.expr_prec(callee, prec::PRIMARY),
                    Doc::text("("),
                    Doc::concat([
                        Doc::softline(),
                        Doc::join(Doc::concat([Doc::text(","), Doc::line()]), entries),
                    ])
                    .indent(),
                    Doc::softline(),
                    Doc::text(")"),
                ])
                .group()
            }
            ExprKind::New(args) => {
                if args.is_empty() {
                    Doc::text("new")
                } else {
                    let entries: Vec<Doc> = args.iter().map(|a| self.expr(a)).collect();
                    Doc::concat([
                        Doc::text("new("),
                        Doc::join(Doc::text(", "), entries),
                        Doc::text(")"),
                    ])
                }
            }
            ExprKind::Cast { target, expr } => {
                let head = match target {
                    CastTarget::Type(ty) => self.data_type(ty),
                    CastTarget::Size(size) => self.expr_prec(size, prec::PRIMARY),
                    CastTarget::Signing(Signing::Signed) => Doc::text("signed"),
                    CastTarget::Signing(Signing::Unsigned) => Doc::text("unsigned"),
                    CastTarget::Const => Doc::text("const"),
                };
                Doc::concat([head, Doc::text("'("), self.expr(expr), Doc::text(")")])
            }
            ExprKind::Inside { expr, set } => {
                let entries: Vec<Doc> = set.iter().map(|e| self.expr(e)).collect();
                Doc::concat([
                    self.expr_prec(expr, prec::RELATIONAL + 1),
                    Doc::text(" inside {"),
                    Doc::join(Doc::text(", "), entries),
                    Doc::text("}"),
                ])
            }
            ExprKind::ValueRange { low, high } => Doc::concat([
                Doc::text("["),
                self.expr(low),
                Doc::text(":"),
                self.expr(high),
                Doc::text("]"),
            ]),
            ExprKind::MinTypMax { min, typ, max } => Doc::concat([
                self.expr_prec(min, prec::MIN_TYP_MAX + 1),
                Doc::text(":"),
                self.expr_prec(typ, prec::MIN_TYP_MAX + 1),
                Doc::text(":"),
                self.expr_prec(max, prec::MIN_TYP_MAX + 1),
            ]),
            ExprKind::Type(ty) => self.data_type(ty),
            ExprKind::Assign { lhs, op, rhs } => Doc::concat([
                self.expr_prec(lhs, prec::ASSIGN + 1),
                Doc::text(format!(" {} ", op.as_str())),
                self.expr_prec(rhs, prec::ASSIGN),
            ]),
            ExprKind::IncDec {
                increment,
                prefix,
                target,
            } => {
                let op = if *increment { "++" } else { "--" };
                let target = self.expr_prec(target, prec::UNARY);
                if *prefix {
                    Doc::text(op).append(target)
                } else {
                    target.append(Doc::text(op))
                }
            }
            ExprKind::Default => Doc::text("default"),
        }
    }

    /// `{a, b, c}`, broken after the commas when it does not fit.
    fn braced(&self, elems: &[Expr]) -> Doc {
        let entries: Vec<Doc> = elems.iter().map(|e| self.expr(e)).collect();
        Doc::concat([
            Doc::text("{"),
            Doc::concat([
                Doc::softline(),
                Doc::join(Doc::concat([Doc::text(","), Doc::line()]), entries),
            ])
            .indent(),
            Doc::softline(),
            Doc::text("}"),
        ])
        .group()
    }

    /// A run of binary operators of the same precedence, broken after the
    /// operator with a continuation indent.
    fn binary_chain(&self, expr: &Expr, op: BinaryOp) -> Doc {
        let level = binary_prec(op);
        if right_associative(op) {
            let ExprKind::Binary { op, lhs, rhs } = &expr.kind else {
                unreachable!("called on a binary expression")
            };
            return Doc::concat([
                self.expr_prec(lhs, level + 1),
                Doc::text(format!(" {}", op.as_str())),
                Doc::line(),
                self.expr_prec(rhs, level),
            ])
            .indent()
            .group();
        }
        let mut operands: Vec<&Expr> = Vec::new();
        let mut ops: Vec<BinaryOp> = Vec::new();
        flatten_binary(expr, level, &mut operands, &mut ops);
        let mut parts = vec![self.expr_prec(operands[0], level)];
        for (op, operand) in ops.iter().zip(&operands[1..]) {
            parts.push(Doc::text(format!(" {}", op.as_str())));
            parts.push(Doc::line());
            parts.push(self.expr_prec(operand, level + 1));
        }
        Doc::concat(parts).indent().group()
    }

    fn literal(&self, lit: &Literal) -> String {
        match lit {
            Literal::Number { text, .. } => text.clone(),
            Literal::Str { value, .. } => quote_string(value),
            Literal::Null(_) => "null".to_owned(),
            Literal::Unbounded(_) => "$".to_owned(),
        }
    }
}

/// The offset just past a module header, where a trailing comment on the
/// header line would start.
fn module_head_end(m: &Module, span: Span) -> u32 {
    let mut end = m.name.span.end;
    for import in &m.imports {
        end = end.max(import.span.end);
    }
    if let Some(params) = &m.params {
        for param in params {
            for decl in &param.decls {
                end = end.max(decl.span.end);
            }
        }
    }
    match &m.ports {
        Ports::None => {}
        Ports::NonAnsi(ports) => {
            for port in ports {
                end = end.max(port.span.end);
            }
        }
        Ports::Ansi(ports) => {
            for port in ports {
                end = end.max(port.span.end);
            }
        }
    }
    end.min(span.end)
}

/// Collects a left-associative chain of operators of precedence `level`.
fn flatten_binary<'e>(
    expr: &'e Expr,
    level: u8,
    operands: &mut Vec<&'e Expr>,
    ops: &mut Vec<BinaryOp>,
) {
    if let ExprKind::Binary { op, lhs, rhs } = &expr.kind
        && binary_prec(*op) == level
        && !right_associative(*op)
    {
        flatten_binary(lhs, level, operands, ops);
        ops.push(*op);
        operands.push(rhs);
    } else {
        operands.push(expr);
    }
}
