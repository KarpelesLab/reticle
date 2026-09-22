//! The `.rtl` text format: a round-tripping rendering of a [`Design`].
//!
//! The format exists for golden tests and for debugging, so it is
//! line-oriented, diff-friendly and unambiguous rather than compact. One
//! line per object; nested constructs (modules, processes, `if`, `case`,
//! loops) open on their own line and close with `end`; expressions are
//! prefix calls (`add(%a, 8'd1)`) with a few Verilog-flavoured pieces of
//! notation for slices, concatenations and literals. `docs/ir.md` has the
//! full description; the summary is:
//!
//! ```text
//! top counter
//!
//! module counter
//!   port clk in %clk
//!   port q out %q
//!   net %clk u1 wire
//!   net %q u8 reg
//!   process seq posedge %clk
//!     %q <= add(%q, 8'd1)
//!   end
//! end
//! ```
//!
//! - Objects are referred to by name: nets as `%name`, memories as
//!   `@name`, everything else bare. A name that is not of the form
//!   `[A-Za-z_$][A-Za-z0-9_$]*`, or that is one of the process kind words
//!   (`comb`, `seq`, `initial`, `sensitive`, `free`), is written as a quoted
//!   string (`%"a b"`).
//! - `attr key = value` lines precede the object they annotate.
//! - Nets and memories must be declared before they are used; the writer
//!   always emits them first.
//! - Types are `u8` / `s8` (unsigned / signed bit vectors), `[4]u8`
//!   (unpacked arrays), `int`, `real`, `string`. Constants are Verilog
//!   sized literals; the writer emits decimal, hexadecimal for values wider
//!   than 64 bits, and binary when any bit is `x` or `z`.
//! - An expression whose cached type is not what the operator rules give
//!   (which is always the case for `call`) carries an `as <type>`
//!   ascription so the design loads back unchanged.
//!
//! Spans are not part of the text. Parsing gives every object a span into
//! the `.rtl` source itself (the caller adds the text to a [`SourceMap`]
//! and passes the [`SourceId`]), so syntax errors and later validation
//! errors on a hand-written file render with excerpts.
//!
//! [`SourceMap`]: crate::source::SourceMap

use std::collections::HashMap;
use std::fmt::Write as _;

use super::Name;
use super::attr::{AttrValue, Attrs};
use super::cell::{Cell, CellKind, Reset};
use super::design::{
    Assign, Design, Instance, Memory, MemoryId, Module, ModuleRef, Net, NetId, NetKind, Param,
    Port, PortDir,
};
use super::expr::{BinaryOp, Expr, ExprId, ExprKind, UnaryOp, infer_type, operands};
use super::process::{
    AssignKind, Block, CaseArm, CaseKind, CaseQualifier, Delay, Edge, Lvalue, Polarity, Process,
    ProcessKind, ReportSeverity, Stmt, StmtKind, TimeUnit, Timescale, WaitKind,
};
use super::types::{Const, Type};
use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, Span};

/// Words that cannot be written as bare names: the process kinds, because
/// a process name is optional and precedes the kind. Every other name
/// position is either mandatory or followed by a line end, so no other
/// keyword can be mistaken for a name.
const KEYWORDS: &[&str] = &["comb", "seq", "initial", "sensitive", "free"];

/// Error code for syntax errors in `.rtl` text.
const SYNTAX: &str = "I0100";
/// Error code for references to names the text does not define.
const UNKNOWN: &str = "I0101";
/// Error code for malformed literals.
const LITERAL: &str = "I0102";

impl Design {
    /// Renders the design in the `.rtl` text format.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(top) = self.top
            && let Some(module) = self.modules.get(top)
        {
            let _ = writeln!(out, "top {}", name(&module.name));
        }
        for (i, (_, module)) in self.modules.iter().enumerate() {
            if i > 0 || self.top.is_some() {
                out.push('\n');
            }
            Printer::new(&mut out, Some(self)).module(module);
        }
        out
    }

    /// Parses `.rtl` text. `file` is the id under which the caller added
    /// `text` to its source map; every span of the result points into it.
    pub fn parse_text(text: &str, file: SourceId) -> Result<Design, Diagnostics> {
        Parser::new(text, file).design()
    }
}

impl Module {
    /// Renders this module alone in the `.rtl` text format. Without the
    /// owning design, resolved instance targets print as `"?m<id>"`.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        Printer::new(&mut out, None).module(self);
        out
    }
}

// ---------------------------------------------------------------------------
// Printing
// ---------------------------------------------------------------------------

/// True when `s` can be written without quotes.
fn is_bare(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_word_start(first) && chars.all(is_word_char) && !KEYWORDS.contains(&s)
}

/// Renders a string as a quoted literal.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Renders a name, quoting it when needed.
fn name(n: &Name) -> String {
    if is_bare(n.as_str()) {
        n.as_str().to_owned()
    } else {
        quoted(n.as_str())
    }
}

fn attr_value(v: &AttrValue) -> String {
    match v {
        AttrValue::Const(c) => const_text(c),
        AttrValue::String(s) => quoted(s),
        AttrValue::Int(i) => i.to_string(),
    }
}

fn delay(d: Delay) -> String {
    format!("{} {}", d.value, d.unit.name())
}

struct Printer<'a> {
    out: &'a mut String,
    indent: usize,
    design: Option<&'a Design>,
    module: Option<&'a Module>,
}

impl<'a> Printer<'a> {
    fn new(out: &'a mut String, design: Option<&'a Design>) -> Self {
        Printer {
            out,
            indent: 0,
            design,
            module: None,
        }
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn attrs(&mut self, attrs: &Attrs) {
        for (k, v) in attrs.iter() {
            self.line(&format!("attr {} = {}", name(k), attr_value(v)));
        }
    }

    fn params(&self, params: &Attrs) -> String {
        if params.is_empty() {
            return String::new();
        }
        let items: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{}={}", name(k), attr_value(v)))
            .collect();
        format!(" #({})", items.join(", "))
    }

    fn m(&self) -> &'a Module {
        self.module.expect("printer used without a module")
    }

    fn net_ref(&self, id: NetId) -> String {
        match self.m().nets.get(id) {
            Some(net) => format!("%{}", name(&net.name)),
            None => format!("%{}", quoted(&format!("?{id}"))),
        }
    }

    fn mem_ref(&self, id: MemoryId) -> String {
        match self.m().memories.get(id) {
            Some(mem) => format!("@{}", name(&mem.name)),
            None => format!("@{}", quoted(&format!("?{id}"))),
        }
    }

    fn module(&mut self, module: &'a Module) {
        self.module = Some(module);
        self.attrs(&module.attrs);
        let mut header = format!("module {}", name(&module.name));
        if module.blackbox {
            header.push_str(" blackbox");
        }
        self.line(&header);
        self.indent += 1;
        if let Some(ts) = module.timescale {
            self.line(&format!(
                "timescale {} / {}",
                delay(ts.unit),
                delay(ts.precision)
            ));
        }
        for param in &module.params {
            self.param(param);
        }
        for (_, net) in module.nets.iter() {
            self.net(net);
        }
        for port in &module.ports {
            self.port(port);
        }
        for (_, mem) in module.memories.iter() {
            self.memory(mem);
        }
        for (_, inst) in module.instances.iter() {
            self.instance(inst);
        }
        for assign in &module.assigns {
            self.assign(assign);
        }
        for (_, process) in module.processes.iter() {
            self.process(process);
        }
        for (_, cell) in module.cells.iter() {
            self.cell(cell);
        }
        self.indent -= 1;
        self.line("end");
        self.module = None;
    }

    fn param(&mut self, param: &Param) {
        self.attrs(&param.attrs);
        self.line(&format!(
            "param {} = {}",
            name(&param.name),
            attr_value(&param.value)
        ));
    }

    fn net(&mut self, net: &Net) {
        self.attrs(&net.attrs);
        self.line(&format!(
            "net %{} {} {}",
            name(&net.name),
            net.ty,
            net.kind.keyword()
        ));
    }

    fn port(&mut self, port: &Port) {
        let net = self.net_ref(port.net);
        self.line(&format!(
            "port {} {} {net}",
            name(&port.name),
            port.dir.keyword()
        ));
    }

    fn memory(&mut self, mem: &Memory) {
        self.attrs(&mem.attrs);
        self.line(&format!(
            "memory @{} {} x {}",
            name(&mem.name),
            mem.size,
            mem.elem
        ));
        if let Some(init) = &mem.init {
            self.indent += 1;
            if init.is_empty() {
                self.line("init");
            }
            for chunk in init.chunks(8) {
                let items: Vec<String> = chunk.iter().map(const_text).collect();
                self.line(&format!("init {}", items.join(" ")));
            }
            self.indent -= 1;
        }
    }

    fn connections(&self, conns: &[(Name, ExprId)]) -> String {
        let items: Vec<String> = conns
            .iter()
            .map(|(port, e)| format!("{}={}", name(port), self.expr(*e)))
            .collect();
        format!("({})", items.join(", "))
    }

    fn instance(&mut self, inst: &Instance) {
        self.attrs(&inst.attrs);
        let target_name = match &inst.module {
            ModuleRef::Unresolved(n) => name(n),
            ModuleRef::Resolved(id) => match self.design.and_then(|d| d.modules.get(*id)) {
                Some(target) => name(&target.name),
                None => quoted(&format!("?{id}")),
            },
        };
        let line = format!(
            "instance {} of {target_name}{} {}",
            name(&inst.name),
            self.params(&inst.params),
            self.connections(&inst.connections),
        );
        self.line(&line);
    }

    fn assign(&mut self, assign: &Assign) {
        self.attrs(&assign.attrs);
        let mut line = format!(
            "assign {} = {}",
            self.lvalue(&assign.target),
            self.expr(assign.value)
        );
        if let Some(d) = assign.delay {
            let _ = write!(line, " after {}", delay(d));
        }
        self.line(&line);
    }

    fn edge(&self, edge: &Edge) -> String {
        let net = self.net_ref(edge.net);
        match edge.polarity {
            Polarity::Pos => format!("posedge {net}"),
            Polarity::Neg => format!("negedge {net}"),
            Polarity::Any => net,
        }
    }

    fn edges(&self, edges: &[Edge]) -> String {
        edges
            .iter()
            .map(|e| self.edge(e))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn process(&mut self, process: &Process) {
        self.attrs(&process.attrs);
        let mut header = String::from("process");
        if let Some(n) = &process.name {
            let _ = write!(header, " {}", name(n));
        }
        let _ = write!(header, " {}", process.kind.keyword());
        match &process.kind {
            ProcessKind::Sequential { clocks, resets } => {
                if !clocks.is_empty() {
                    let _ = write!(header, " {}", self.edges(clocks));
                }
                if !resets.is_empty() {
                    let _ = write!(header, " async {}", self.edges(resets));
                }
            }
            ProcessKind::Sensitive(nets) => {
                let refs: Vec<String> = nets.iter().map(|n| self.net_ref(*n)).collect();
                let _ = write!(header, " {}", refs.join(", "));
            }
            ProcessKind::Comb | ProcessKind::Initial | ProcessKind::Free => {}
        }
        self.line(&header);
        self.block(&process.body);
        self.line("end");
    }

    fn block(&mut self, block: &Block) {
        self.indent += 1;
        for stmt in block {
            self.stmt(stmt);
        }
        self.indent -= 1;
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                kind,
                delay: d,
            } => {
                let op = match kind {
                    AssignKind::Blocking => "=",
                    AssignKind::NonBlocking => "<=",
                };
                let mut line = format!("{} {op} {}", self.lvalue(target), self.expr(*value));
                if let Some(d) = d {
                    let _ = write!(line, " after {}", delay(*d));
                }
                self.line(&line);
            }
            StmtKind::If { cond, then_, else_ } => {
                self.line(&format!("if {}", self.expr(*cond)));
                self.block(then_);
                if !else_.is_empty() {
                    self.line("else");
                    self.block(else_);
                }
                self.line("end");
            }
            StmtKind::Case {
                subject,
                kind,
                qualifier,
                arms,
                default,
            } => {
                let mut header = format!("{} {}", kind.keyword(), self.expr(*subject));
                match qualifier {
                    CaseQualifier::None => {}
                    CaseQualifier::Unique => header.push_str(" unique"),
                    CaseQualifier::Priority => header.push_str(" priority"),
                }
                self.line(&header);
                self.indent += 1;
                for arm in arms {
                    let values: Vec<String> = arm.values.iter().map(|v| self.expr(*v)).collect();
                    self.line(&format!("when {}", values.join(", ")));
                    self.block(&arm.body);
                    self.line("end");
                }
                if let Some(d) = default {
                    self.line("default");
                    self.block(d);
                    self.line("end");
                }
                self.indent -= 1;
                self.line("end");
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                let part = |p: &Option<(Lvalue, ExprId)>| match p {
                    Some((lv, e)) => format!(" {} = {}", self.lvalue(lv), self.expr(*e)),
                    None => String::new(),
                };
                let cond = cond
                    .map(|c| format!(" {}", self.expr(c)))
                    .unwrap_or_default();
                self.line(&format!("for{};{cond};{}", part(init), part(step)));
                self.block(body);
                self.line("end");
            }
            StmtKind::While { cond, body } => {
                self.line(&format!("while {}", self.expr(*cond)));
                self.block(body);
                self.line("end");
            }
            StmtKind::Repeat { count, body } => {
                self.line(&format!("repeat {}", self.expr(*count)));
                self.block(body);
                self.line("end");
            }
            StmtKind::Forever { body } => {
                self.line("forever");
                self.block(body);
                self.line("end");
            }
            StmtKind::Block { name: n, body } => {
                match n {
                    Some(n) => self.line(&format!("block {}", name(n))),
                    None => self.line("block"),
                }
                self.block(body);
                self.line("end");
            }
            StmtKind::Wait(WaitKind::Delay(e)) => self.line(&format!("wait for {}", self.expr(*e))),
            StmtKind::Wait(WaitKind::Event(edges)) => {
                self.line(&format!("wait on {}", self.edges(edges)));
            }
            StmtKind::Wait(WaitKind::Until(e)) => {
                self.line(&format!("wait until {}", self.expr(*e)));
            }
            StmtKind::SysCall { name: n, args } => {
                self.line(&format!("sys {}({})", name(n), self.args(args)));
            }
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                let mut line = format!(
                    "memwrite {}[{}] = {}",
                    self.mem_ref(*mem),
                    self.expr(*addr),
                    self.expr(*value)
                );
                if let Some(en) = enable {
                    let _ = write!(line, " enable {}", self.expr(*en));
                }
                self.line(&line);
            }
            StmtKind::Assert {
                cond,
                severity,
                message,
            } => {
                let mut line = format!("assert {} {}", severity.keyword(), self.expr(*cond));
                if !message.is_empty() {
                    let _ = write!(line, " report {}", self.args(message));
                }
                self.line(&line);
            }
            StmtKind::Finish => self.line("finish"),
            StmtKind::Stop => self.line("stop"),
            StmtKind::Break => self.line("break"),
            StmtKind::Continue => self.line("continue"),
        }
    }

    fn args(&self, args: &[ExprId]) -> String {
        args.iter()
            .map(|a| self.expr(*a))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn lvalue(&self, lv: &Lvalue) -> String {
        match lv {
            Lvalue::Net(net) => self.net_ref(*net),
            Lvalue::Slice { net, hi, lo } => format!("{}[{hi}:{lo}]", self.net_ref(*net)),
            Lvalue::Index { net, index } => {
                format!("{}[{}]", self.net_ref(*net), self.expr(*index))
            }
            Lvalue::Concat(parts) => {
                let items: Vec<String> = parts.iter().map(|p| self.lvalue(p)).collect();
                format!("{{{}}}", items.join(", "))
            }
            Lvalue::MemElem { mem, addr } => {
                format!("{}[{}]", self.mem_ref(*mem), self.expr(*addr))
            }
        }
    }

    fn cell(&mut self, cell: &Cell) {
        self.attrs(&cell.attrs);
        let mut kind = String::from(cell.kind.keyword());
        match &cell.kind {
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                kind.push_str(if *clk_pos { " pos" } else { " neg" });
                if *has_enable {
                    kind.push_str(" en");
                }
                if let Some(Reset {
                    asynchronous,
                    active_high,
                    value,
                }) = reset
                {
                    let _ = write!(
                        kind,
                        " {} {} {}",
                        if *asynchronous { "arst" } else { "srst" },
                        if *active_high { "pos" } else { "neg" },
                        const_text(value)
                    );
                }
            }
            CellKind::MemRdPort { mem, clocked } | CellKind::MemWrPort { mem, clocked } => {
                let _ = write!(kind, " {}", self.mem_ref(*mem));
                if *clocked {
                    kind.push_str(" clocked");
                }
            }
            CellKind::Lut { k, init } => {
                let _ = write!(kind, " {k} {}", const_text(init));
            }
            CellKind::Blackbox(n) => {
                let _ = write!(kind, " {}", name(n));
            }
            _ => {}
        }
        let outputs: Vec<String> = cell
            .outputs
            .iter()
            .map(|(port, n)| format!("{}={}", name(port), self.net_ref(*n)))
            .collect();
        let line = format!(
            "cell {} {kind}{} {} -> ({})",
            name(&cell.name),
            self.params(&cell.params),
            self.connections(&cell.inputs),
            outputs.join(", ")
        );
        self.line(&line);
    }

    /// True when the node's cached type must be written explicitly.
    fn needs_ascription(&self, expr: &Expr) -> bool {
        infer_type(self.m(), &expr.kind).ok().as_ref() != Some(&expr.ty)
    }

    fn expr(&self, id: ExprId) -> String {
        let Some(expr) = self.m().exprs.get(id) else {
            return quoted(&format!("?{id}"));
        };
        let text = self.expr_inner(expr);
        if self.needs_ascription(expr) {
            format!("{text} as {}", expr.ty)
        } else {
            text
        }
    }

    /// The operand of a postfix selection, parenthesised when it carries an
    /// ascription so the `as` does not swallow the selection.
    fn base(&self, id: ExprId) -> String {
        let text = self.expr(id);
        match self.m().exprs.get(id) {
            Some(e) if self.needs_ascription(e) => format!("({text})"),
            _ => text,
        }
    }

    fn expr_inner(&self, expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Const(c) => const_text(c),
            ExprKind::String(s) => quoted(s),
            ExprKind::Net(n) => self.net_ref(*n),
            ExprKind::Slice { base, hi, lo } => format!("{}[{hi}:{lo}]", self.base(*base)),
            ExprKind::Index { base, index } => {
                format!("{}[{}]", self.base(*base), self.expr(*index))
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => format!(
                "{}[{} {} {width}]",
                self.base(*base),
                self.expr(*offset),
                if *up { "+:" } else { "-:" }
            ),
            ExprKind::Concat(parts) => format!("{{{}}}", self.args(parts)),
            ExprKind::Replicate { count, expr } => {
                format!("{{{count}{{{}}}}}", self.expr(*expr))
            }
            ExprKind::Unary { op, expr } => format!("{}({})", op.name(), self.expr(*expr)),
            ExprKind::Binary { op, lhs, rhs } => {
                format!("{}({}, {})", op.name(), self.expr(*lhs), self.expr(*rhs))
            }
            ExprKind::Ternary { cond, then_, else_ } => format!(
                "mux({}, {}, {})",
                self.expr(*cond),
                self.expr(*then_),
                self.expr(*else_)
            ),
            ExprKind::Resize {
                expr,
                width,
                signed,
            } => format!(
                "resize({}, {})",
                self.expr(*expr),
                Type::Bits {
                    width: *width,
                    signed: *signed
                }
            ),
            ExprKind::MemRead { mem, addr } => {
                format!("{}[{}]", self.mem_ref(*mem), self.expr(*addr))
            }
            ExprKind::Call { name: n, args } => format!("call {}({})", name(n), self.args(args)),
        }
    }
}

// ---------------------------------------------------------------------------
// Lexing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    /// A bare word: keyword, identifier, type or operator name.
    Word(String),
    /// A quoted string.
    Str(String),
    /// A plain unsigned integer.
    Int(u64),
    /// A sized literal, raw (`8'd255`), interpreted by the parser.
    Lit(String),
    /// `%name`.
    Net(String),
    /// `@name`.
    Mem(String),
    /// Punctuation.
    Punct(&'static str),
    /// End of line (consecutive newlines are merged).
    Newline,
    /// End of input.
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    span: Span,
}

const PUNCTS: &[&str] = &[
    "->", "<=", "+:", "-:", "(", ")", "[", "]", "{", "}", ",", ":", ";", "=", "#", "/", "-",
];

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

struct Lexer<'a> {
    text: &'a str,
    pos: usize,
    file: SourceId,
    diags: Diagnostics,
}

impl<'a> Lexer<'a> {
    fn span(&self, start: usize, end: usize) -> Span {
        let n = |v: usize| u32::try_from(v).expect("rtl text exceeds u32");
        Span::new(self.file, n(start), n(end))
    }

    fn peek_char(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn take_while(&mut self, pred: impl Fn(char) -> bool) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if !pred(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        &self.text[start..self.pos]
    }

    fn tokens(mut self) -> (Vec<Token>, Diagnostics) {
        let mut tokens = Vec::new();
        loop {
            self.take_while(|c| c == ' ' || c == '\t' || c == '\r');
            let start = self.pos;
            let Some(c) = self.peek_char() else {
                tokens.push(Token {
                    tok: Tok::Eof,
                    span: self.span(start, start),
                });
                break;
            };
            if c == '\n' {
                self.pos += 1;
                if !matches!(
                    tokens.last(),
                    Some(Token {
                        tok: Tok::Newline,
                        ..
                    }) | None
                ) {
                    tokens.push(Token {
                        tok: Tok::Newline,
                        span: self.span(start, self.pos),
                    });
                }
                continue;
            }
            if self.text[self.pos..].starts_with("//") {
                self.take_while(|c| c != '\n');
                continue;
            }
            let tok = if is_word_start(c) {
                Tok::Word(self.take_while(is_word_char).to_owned())
            } else if c.is_ascii_digit() {
                let digits = self.take_while(|c| c.is_ascii_digit() || c == '_');
                if self.peek_char() == Some('\'') {
                    self.pos += 1;
                    self.take_while(|c| c.is_ascii_alphanumeric() || c == '_' || c == '?');
                    Tok::Lit(self.text[start..self.pos].to_owned())
                } else {
                    match digits.replace('_', "").parse::<u64>() {
                        Ok(v) => Tok::Int(v),
                        Err(_) => {
                            self.diags.push(
                                Diagnostic::error("integer literal is too large")
                                    .with_code(LITERAL)
                                    .with_span(self.span(start, self.pos)),
                            );
                            Tok::Int(0)
                        }
                    }
                }
            } else if c == '"' {
                Tok::Str(self.string())
            } else if c == '%' || c == '@' {
                self.pos += 1;
                let name = match self.peek_char() {
                    Some('"') => self.string(),
                    Some(n) if is_word_char(n) => self.take_while(is_word_char).to_owned(),
                    _ => {
                        self.diags.push(
                            Diagnostic::error(format!("expected a name after `{c}`"))
                                .with_code(SYNTAX)
                                .with_span(self.span(start, self.pos)),
                        );
                        String::new()
                    }
                };
                if c == '%' {
                    Tok::Net(name)
                } else {
                    Tok::Mem(name)
                }
            } else if let Some(p) = PUNCTS
                .iter()
                .find(|p| self.text[self.pos..].starts_with(**p))
            {
                self.pos += p.len();
                Tok::Punct(p)
            } else {
                self.pos += c.len_utf8();
                self.diags.push(
                    Diagnostic::error(format!("unexpected character `{c}`"))
                        .with_code(SYNTAX)
                        .with_span(self.span(start, self.pos)),
                );
                continue;
            };
            tokens.push(Token {
                tok,
                span: self.span(start, self.pos),
            });
        }
        (tokens, self.diags)
    }

    /// Lexes a quoted string starting at the opening quote.
    fn string(&mut self) -> String {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek_char() else {
                self.diags.push(
                    Diagnostic::error("unterminated string")
                        .with_code(SYNTAX)
                        .with_span(self.span(start, self.pos)),
                );
                return out;
            };
            self.pos += c.len_utf8();
            match c {
                '"' => return out,
                '\n' => {
                    self.diags.push(
                        Diagnostic::error("unterminated string")
                            .with_code(SYNTAX)
                            .with_span(self.span(start, self.pos)),
                    );
                    return out;
                }
                '\\' => {
                    let Some(e) = self.peek_char() else {
                        continue;
                    };
                    self.pos += e.len_utf8();
                    match e {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' if self.peek_char() == Some('{') => {
                            self.pos += 1;
                            let hex = self.take_while(|c| c.is_ascii_hexdigit());
                            let value = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                            if self.peek_char() == Some('}') {
                                self.pos += 1;
                            }
                            match value {
                                Some(ch) => out.push(ch),
                                None => self.diags.push(
                                    Diagnostic::error("invalid `\\u{...}` escape")
                                        .with_code(SYNTAX)
                                        .with_span(self.span(start, self.pos)),
                                ),
                            }
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// A parse failure; the diagnostic has already been recorded.
struct Fail;

type PResult<T> = Result<T, Fail>;

/// A port line, kept until the module's nets are all known.
struct PendingPort {
    name: Name,
    dir: PortDir,
    net: String,
    span: Span,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diags: Diagnostics,
    design: Design,
    module: Module,
    nets: HashMap<String, NetId>,
    mems: HashMap<String, MemoryId>,
    ports: Vec<PendingPort>,
    pending_attrs: Attrs,
    top: Option<(Name, Span)>,
}

impl Parser {
    fn new(text: &str, file: SourceId) -> Self {
        let (tokens, diags) = Lexer {
            text,
            pos: 0,
            file,
            diags: Diagnostics::new(),
        }
        .tokens();
        let dummy = Span::new(file, 0, 0);
        Parser {
            tokens,
            pos: 0,
            diags,
            design: Design::new(),
            module: Module::new("", dummy),
            nets: HashMap::new(),
            mems: HashMap::new(),
            ports: Vec::new(),
            pending_attrs: Attrs::new(),
            top: None,
        }
    }

    // --- token helpers -----------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// The span from `start` to the end of the last consumed token.
    fn span_from(&self, start: Span) -> Span {
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        if end.start < start.start {
            start
        } else {
            start.to(end)
        }
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn fail<T>(&mut self, span: Span, message: impl Into<String>) -> PResult<T> {
        self.diags
            .push(Diagnostic::error(message).with_code(SYNTAX).with_span(span));
        Err(Fail)
    }

    fn describe(tok: &Tok) -> String {
        match tok {
            Tok::Word(w) => format!("`{w}`"),
            Tok::Str(_) => "a string".into(),
            Tok::Int(v) => format!("`{v}`"),
            Tok::Lit(l) => format!("`{l}`"),
            Tok::Net(n) => format!("`%{n}`"),
            Tok::Mem(n) => format!("`@{n}`"),
            Tok::Punct(p) => format!("`{p}`"),
            Tok::Newline => "end of line".into(),
            Tok::Eof => "end of input".into(),
        }
    }

    fn unexpected<T>(&mut self, expected: &str) -> PResult<T> {
        let found = Self::describe(self.peek());
        let span = self.peek_span();
        self.fail(span, format!("expected {expected}, found {found}"))
    }

    fn at_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Word(x) if x == w)
    }

    fn at_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(x) if *x == p)
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.at_word(w) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.at_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_word(&mut self, w: &str) -> PResult<()> {
        if self.eat_word(w) {
            Ok(())
        } else {
            self.unexpected(&format!("`{w}`"))
        }
    }

    fn expect_punct(&mut self, p: &str) -> PResult<()> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            self.unexpected(&format!("`{p}`"))
        }
    }

    fn expect_int(&mut self) -> PResult<u64> {
        match self.peek() {
            Tok::Int(v) => {
                let v = *v;
                self.bump();
                Ok(v)
            }
            _ => self.unexpected("an integer"),
        }
    }

    fn expect_u32(&mut self) -> PResult<u32> {
        let span = self.peek_span();
        let v = self.expect_int()?;
        match u32::try_from(v) {
            Ok(v) => Ok(v),
            Err(_) => self.fail(span, "value does not fit in 32 bits"),
        }
    }

    /// A bare word or quoted string used as a name.
    fn expect_name(&mut self) -> PResult<Name> {
        match self.peek() {
            Tok::Word(w) => {
                let n = Name::new(w.as_str());
                self.bump();
                Ok(n)
            }
            Tok::Str(s) => {
                let n = Name::new(s.as_str());
                self.bump();
                Ok(n)
            }
            _ => self.unexpected("a name"),
        }
    }

    fn expect_newline(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline => {
                self.bump();
                Ok(())
            }
            Tok::Eof => Ok(()),
            _ => self.unexpected("end of line"),
        }
    }

    fn skip_newlines(&mut self) {
        while *self.peek() == Tok::Newline {
            self.bump();
        }
    }

    /// Error recovery: skips to the start of the next line.
    fn skip_line(&mut self) {
        while !matches!(self.peek(), Tok::Newline | Tok::Eof) {
            self.bump();
        }
        self.skip_newlines();
    }

    fn take_attrs(&mut self) -> Attrs {
        std::mem::take(&mut self.pending_attrs)
    }

    /// Reports `attr` lines that no object followed.
    fn check_no_pending_attrs(&mut self) {
        if !self.pending_attrs.is_empty() {
            let span = self.tokens[self.pos.saturating_sub(1)].span;
            self.diags.push(
                Diagnostic::error("attributes are not followed by an object")
                    .with_code(SYNTAX)
                    .with_span(span),
            );
            self.pending_attrs = Attrs::new();
        }
    }

    // --- literals and types ------------------------------------------------

    fn parse_const(&mut self, raw: &str, span: Span) -> PResult<Const> {
        match Const::parse_verilog(raw) {
            Ok(c) => Ok(c),
            Err(err) => self.fail(span, format!("malformed literal: {err}")),
        }
    }

    fn expect_const(&mut self) -> PResult<Const> {
        match self.peek().clone() {
            Tok::Lit(raw) => {
                let span = self.peek_span();
                self.bump();
                self.parse_const(&raw, span)
            }
            _ => self.unexpected("a sized literal"),
        }
    }

    fn expect_type(&mut self) -> PResult<Type> {
        if self.eat_punct("[") {
            let len = self.expect_int()?;
            self.expect_punct("]")?;
            let elem = self.expect_type()?;
            return Ok(Type::array(elem, len));
        }
        let span = self.peek_span();
        let Tok::Word(w) = self.peek().clone() else {
            return self.unexpected("a type");
        };
        let ty = match w.as_str() {
            "int" => Type::Integer,
            "real" => Type::Real,
            "string" => Type::String,
            _ => {
                let (signed, digits) = match w.split_at(1) {
                    ("u", d) => (false, d),
                    ("s", d) => (true, d),
                    _ => return self.fail(span, format!("unknown type `{w}`")),
                };
                match digits.parse::<u32>() {
                    Ok(width) if !digits.is_empty() && !digits.starts_with('+') => {
                        Type::Bits { width, signed }
                    }
                    _ => return self.fail(span, format!("unknown type `{w}`")),
                }
            }
        };
        self.bump();
        Ok(ty)
    }

    fn expect_attr_value(&mut self) -> PResult<AttrValue> {
        match self.peek().clone() {
            Tok::Lit(_) => Ok(AttrValue::Const(self.expect_const()?)),
            Tok::Str(s) => {
                self.bump();
                Ok(AttrValue::String(s))
            }
            Tok::Int(_) => {
                let span = self.peek_span();
                let v = self.expect_int()?;
                match i64::try_from(v) {
                    Ok(v) => Ok(AttrValue::Int(v)),
                    Err(_) => self.fail(span, "integer attribute value is too large"),
                }
            }
            Tok::Punct("-") => {
                self.bump();
                let span = self.peek_span();
                let v = self.expect_int()?;
                match i64::try_from(v) {
                    Ok(v) => Ok(AttrValue::Int(-v)),
                    Err(_) => self.fail(span, "integer attribute value is too large"),
                }
            }
            _ => self.unexpected("an attribute value"),
        }
    }

    fn expect_unit(&mut self) -> PResult<TimeUnit> {
        let span = self.peek_span();
        match self.peek().clone() {
            Tok::Word(w) => match TimeUnit::from_name(&w) {
                Some(u) => {
                    self.bump();
                    Ok(u)
                }
                None => self.fail(span, format!("unknown time unit `{w}`")),
            },
            _ => self.unexpected("a time unit"),
        }
    }

    fn expect_delay(&mut self) -> PResult<Delay> {
        let value = self.expect_int()?;
        let unit = self.expect_unit()?;
        Ok(Delay { value, unit })
    }

    /// `after <delay>` when present.
    fn optional_after(&mut self) -> PResult<Option<Delay>> {
        if self.eat_word("after") {
            Ok(Some(self.expect_delay()?))
        } else {
            Ok(None)
        }
    }

    // --- references --------------------------------------------------------

    fn lookup_net(&mut self, n: &str, span: Span) -> PResult<NetId> {
        match self.nets.get(n) {
            Some(id) => Ok(*id),
            None => {
                self.diags.push(
                    Diagnostic::error(format!("unknown net `%{n}`"))
                        .with_code(UNKNOWN)
                        .with_span(span),
                );
                Err(Fail)
            }
        }
    }

    fn lookup_mem(&mut self, n: &str, span: Span) -> PResult<MemoryId> {
        match self.mems.get(n) {
            Some(id) => Ok(*id),
            None => {
                self.diags.push(
                    Diagnostic::error(format!("unknown memory `@{n}`"))
                        .with_code(UNKNOWN)
                        .with_span(span),
                );
                Err(Fail)
            }
        }
    }

    fn expect_net(&mut self) -> PResult<NetId> {
        match self.peek().clone() {
            Tok::Net(n) => {
                let span = self.peek_span();
                self.bump();
                self.lookup_net(&n, span)
            }
            _ => self.unexpected("a net reference (`%name`)"),
        }
    }

    // --- top level -----------------------------------------------------------

    fn design(mut self) -> Result<Design, Diagnostics> {
        self.skip_newlines();
        while *self.peek() != Tok::Eof {
            let result = match self.peek().clone() {
                Tok::Word(w) if w == "top" => self.top_line(),
                Tok::Word(w) if w == "attr" => self.attr_line(),
                Tok::Word(w) if w == "module" => self.module_decl(),
                _ => self.unexpected("`module`, `top` or `attr`"),
            };
            if result.is_err() {
                self.skip_line();
            }
            self.skip_newlines();
        }
        self.check_no_pending_attrs();
        if let Some((top, span)) = self.top.take() {
            match self.design.module_by_name(top.as_str()) {
                Some(id) => self.design.top = Some(id),
                None => self.diags.push(
                    Diagnostic::error(format!("top module `{top}` is not defined"))
                        .with_code(UNKNOWN)
                        .with_span(span),
                ),
            }
        }
        self.design.resolve_instances();
        if self.diags.has_errors() {
            Err(self.diags)
        } else {
            Ok(self.design)
        }
    }

    fn top_line(&mut self) -> PResult<()> {
        let start = self.peek_span();
        self.expect_word("top")?;
        let n = self.expect_name()?;
        self.expect_newline()?;
        self.top = Some((n, self.span_from(start)));
        Ok(())
    }

    fn attr_line(&mut self) -> PResult<()> {
        self.expect_word("attr")?;
        let key = self.expect_name()?;
        self.expect_punct("=")?;
        let value = self.expect_attr_value()?;
        self.expect_newline()?;
        self.pending_attrs.set(key, value);
        Ok(())
    }

    fn module_decl(&mut self) -> PResult<()> {
        let start = self.peek_span();
        self.expect_word("module")?;
        let n = self.expect_name()?;
        let blackbox = self.eat_word("blackbox");
        self.expect_newline()?;
        self.module = Module::new(n, start);
        self.module.blackbox = blackbox;
        self.module.attrs = self.take_attrs();
        self.nets.clear();
        self.mems.clear();
        self.ports.clear();
        loop {
            self.skip_newlines();
            if self.eat_word("end") {
                break;
            }
            if *self.peek() == Tok::Eof {
                return self.unexpected("`end`");
            }
            if self.module_item().is_err() {
                self.skip_line();
            }
        }
        self.check_no_pending_attrs();
        self.module.span = self.span_from(start);
        let ports = std::mem::take(&mut self.ports);
        for port in ports {
            if let Ok(net) = self.lookup_net(&port.net, port.span) {
                self.module.ports.push(Port {
                    name: port.name,
                    dir: port.dir,
                    net,
                    span: port.span,
                });
            }
        }
        let module = std::mem::replace(&mut self.module, Module::new("", start));
        self.design.add_module(module);
        Ok(())
    }

    fn module_item(&mut self) -> PResult<()> {
        let start = self.peek_span();
        let Tok::Word(w) = self.peek().clone() else {
            return self.unexpected("a module item");
        };
        match w.as_str() {
            "attr" => self.attr_line(),
            "timescale" => {
                self.bump();
                let unit = self.expect_delay()?;
                self.expect_punct("/")?;
                let precision = self.expect_delay()?;
                self.expect_newline()?;
                self.module.timescale = Some(Timescale { unit, precision });
                Ok(())
            }
            "param" => {
                self.bump();
                let n = self.expect_name()?;
                self.expect_punct("=")?;
                let value = self.expect_attr_value()?;
                self.expect_newline()?;
                let attrs = self.take_attrs();
                self.module.params.push(Param {
                    name: n,
                    value,
                    attrs,
                    span: self.span_from(start),
                });
                Ok(())
            }
            "port" => {
                self.bump();
                let n = self.expect_name()?;
                let dir_span = self.peek_span();
                let dir = match self.peek().clone() {
                    Tok::Word(d) => match PortDir::from_keyword(&d) {
                        Some(d) => {
                            self.bump();
                            d
                        }
                        None => return self.fail(dir_span, format!("unknown direction `{d}`")),
                    },
                    _ => return self.unexpected("`in`, `out` or `inout`"),
                };
                let Tok::Net(net) = self.peek().clone() else {
                    return self.unexpected("a net reference (`%name`)");
                };
                self.bump();
                self.expect_newline()?;
                self.ports.push(PendingPort {
                    name: n,
                    dir,
                    net,
                    span: self.span_from(start),
                });
                Ok(())
            }
            "net" => {
                self.bump();
                let Tok::Net(n) = self.peek().clone() else {
                    return self.unexpected("a net name (`%name`)");
                };
                self.bump();
                let ty = self.expect_type()?;
                let kind_span = self.peek_span();
                let kind = match self.peek().clone() {
                    Tok::Word(k) => match NetKind::from_keyword(&k) {
                        Some(k) => {
                            self.bump();
                            k
                        }
                        None => return self.fail(kind_span, format!("unknown net kind `{k}`")),
                    },
                    _ => return self.unexpected("`wire`, `reg` or `var`"),
                };
                self.expect_newline()?;
                let attrs = self.take_attrs();
                let id = self.module.nets.push(Net {
                    name: Name::new(n.as_str()),
                    ty,
                    kind,
                    attrs,
                    span: self.span_from(start),
                });
                self.nets.insert(n, id);
                Ok(())
            }
            "memory" => {
                self.bump();
                let Tok::Mem(n) = self.peek().clone() else {
                    return self.unexpected("a memory name (`@name`)");
                };
                self.bump();
                let size = self.expect_int()?;
                self.expect_word("x")?;
                let elem = self.expect_type()?;
                self.expect_newline()?;
                let attrs = self.take_attrs();
                let id = self.module.memories.push(Memory {
                    name: Name::new(n.as_str()),
                    elem,
                    size,
                    init: None,
                    attrs,
                    span: self.span_from(start),
                });
                self.mems.insert(n, id);
                Ok(())
            }
            "init" => {
                self.bump();
                let Some(mem) = self.module.memories.iter().next_back().map(|(id, _)| id) else {
                    return self.fail(start, "`init` must follow a memory");
                };
                let mut values = Vec::new();
                while matches!(self.peek(), Tok::Lit(_)) {
                    values.push(self.expect_const()?);
                }
                self.expect_newline()?;
                self.module.memories[mem]
                    .init
                    .get_or_insert_with(Vec::new)
                    .extend(values);
                Ok(())
            }
            "assign" => {
                self.bump();
                let target = self.lvalue()?;
                self.expect_punct("=")?;
                let value = self.expr()?;
                let d = self.optional_after()?;
                self.expect_newline()?;
                let attrs = self.take_attrs();
                self.module.assigns.push(Assign {
                    target,
                    value,
                    delay: d,
                    attrs,
                    span: self.span_from(start),
                });
                Ok(())
            }
            "instance" => {
                self.bump();
                let n = self.expect_name()?;
                self.expect_word("of")?;
                let target = self.expect_name()?;
                let params = self.optional_params()?;
                let connections = self.connections()?;
                self.expect_newline()?;
                let attrs = self.take_attrs();
                self.module.instances.push(Instance {
                    name: n,
                    module: ModuleRef::Unresolved(target),
                    connections,
                    params,
                    attrs,
                    span: self.span_from(start),
                });
                Ok(())
            }
            "cell" => self.cell_line(),
            "process" => self.process_decl(),
            _ => self.unexpected("a module item"),
        }
    }

    /// `#(name=value, ...)` when present.
    fn optional_params(&mut self) -> PResult<Attrs> {
        let mut params = Attrs::new();
        if !self.eat_punct("#") {
            return Ok(params);
        }
        self.expect_punct("(")?;
        if !self.at_punct(")") {
            loop {
                let key = self.expect_name()?;
                self.expect_punct("=")?;
                let value = self.expect_attr_value()?;
                params.set(key, value);
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        Ok(params)
    }

    /// `(port=expr, ...)`.
    fn connections(&mut self) -> PResult<Vec<(Name, ExprId)>> {
        self.expect_punct("(")?;
        let mut conns = Vec::new();
        if !self.at_punct(")") {
            loop {
                let port = self.expect_name()?;
                self.expect_punct("=")?;
                let e = self.expr()?;
                conns.push((port, e));
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        Ok(conns)
    }

    fn cell_line(&mut self) -> PResult<()> {
        let start = self.peek_span();
        self.expect_word("cell")?;
        let n = self.expect_name()?;
        let kind_span = self.peek_span();
        let Tok::Word(kw) = self.peek().clone() else {
            return self.unexpected("a cell kind");
        };
        self.bump();
        let kind = match kw.as_str() {
            "dff" => {
                let clk_pos = self.expect_polarity_word()?;
                let has_enable = self.eat_word("en");
                let reset = if self.at_word("srst") || self.at_word("arst") {
                    let asynchronous = self.eat_word("arst");
                    if !asynchronous {
                        self.bump();
                    }
                    let active_high = self.expect_polarity_word()?;
                    let value = self.expect_const()?;
                    Some(Reset {
                        asynchronous,
                        active_high,
                        value,
                    })
                } else {
                    None
                };
                CellKind::Dff {
                    clk_pos,
                    has_enable,
                    reset,
                }
            }
            "memrd" | "memwr" => {
                let Tok::Mem(m) = self.peek().clone() else {
                    return self.unexpected("a memory reference (`@name`)");
                };
                let span = self.peek_span();
                self.bump();
                let mem = self.lookup_mem(&m, span)?;
                let clocked = self.eat_word("clocked");
                if kw == "memrd" {
                    CellKind::MemRdPort { mem, clocked }
                } else {
                    CellKind::MemWrPort { mem, clocked }
                }
            }
            "lut" => {
                let k = self.expect_u32()?;
                let init = self.expect_const()?;
                CellKind::Lut { k, init }
            }
            "blackbox" => CellKind::Blackbox(self.expect_name()?),
            other => match CellKind::simple_from_keyword(other) {
                Some(k) => k,
                None => return self.fail(kind_span, format!("unknown cell kind `{other}`")),
            },
        };
        let params = self.optional_params()?;
        let inputs = self.connections()?;
        self.expect_punct("->")?;
        self.expect_punct("(")?;
        let mut outputs = Vec::new();
        if !self.at_punct(")") {
            loop {
                let port = self.expect_name()?;
                self.expect_punct("=")?;
                let net = self.expect_net()?;
                outputs.push((port, net));
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        self.expect_newline()?;
        let attrs = self.take_attrs();
        self.module.cells.push(Cell {
            name: n,
            kind,
            inputs,
            outputs,
            params,
            attrs,
            span: self.span_from(start),
        });
        Ok(())
    }

    /// `pos` or `neg`, as a boolean.
    fn expect_polarity_word(&mut self) -> PResult<bool> {
        if self.eat_word("pos") {
            Ok(true)
        } else if self.eat_word("neg") {
            Ok(false)
        } else {
            self.unexpected("`pos` or `neg`")
        }
    }

    /// A comma-separated list of `posedge %n`, `negedge %n` or `%n`.
    fn edges(&mut self) -> PResult<Vec<Edge>> {
        let mut edges = Vec::new();
        loop {
            let polarity = if self.eat_word("posedge") {
                Polarity::Pos
            } else if self.eat_word("negedge") {
                Polarity::Neg
            } else {
                Polarity::Any
            };
            let net = self.expect_net()?;
            edges.push(Edge { net, polarity });
            if !self.eat_punct(",") {
                break;
            }
        }
        Ok(edges)
    }

    fn process_decl(&mut self) -> PResult<()> {
        let start = self.peek_span();
        self.expect_word("process")?;
        const KINDS: [&str; 5] = ["comb", "seq", "initial", "sensitive", "free"];
        let n = match self.peek() {
            Tok::Word(w) if KINDS.contains(&w.as_str()) => None,
            _ => Some(self.expect_name()?),
        };
        let kind_span = self.peek_span();
        let Tok::Word(kw) = self.peek().clone() else {
            return self.unexpected("a process kind");
        };
        self.bump();
        let kind = match kw.as_str() {
            "comb" => ProcessKind::Comb,
            "initial" => ProcessKind::Initial,
            "free" => ProcessKind::Free,
            "seq" => {
                let clocks =
                    if matches!(self.peek(), Tok::Newline | Tok::Eof) || self.at_word("async") {
                        Vec::new()
                    } else {
                        self.edges()?
                    };
                let resets = if self.eat_word("async") {
                    self.edges()?
                } else {
                    Vec::new()
                };
                ProcessKind::Sequential { clocks, resets }
            }
            "sensitive" => {
                let mut nets = Vec::new();
                if !matches!(self.peek(), Tok::Newline | Tok::Eof) {
                    loop {
                        nets.push(self.expect_net()?);
                        if !self.eat_punct(",") {
                            break;
                        }
                    }
                }
                ProcessKind::Sensitive(nets)
            }
            other => return self.fail(kind_span, format!("unknown process kind `{other}`")),
        };
        self.expect_newline()?;
        let attrs = self.take_attrs();
        let body = self.block(&["end"])?;
        self.expect_word("end")?;
        self.expect_newline()?;
        self.module.processes.push(Process {
            name: n,
            kind,
            body,
            attrs,
            span: self.span_from(start),
        });
        Ok(())
    }

    /// Statements up to (not including) one of the `terminators` at the
    /// start of a line.
    fn block(&mut self, terminators: &[&str]) -> PResult<Block> {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::Word(w) if terminators.contains(&w.as_str()) => return Ok(stmts),
                Tok::Eof => return self.unexpected(&format!("`{}`", terminators.join("` or `"))),
                _ => {}
            }
            match self.stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(Fail) => self.skip_line(),
            }
        }
    }

    /// A block followed by the `end` line.
    fn block_end(&mut self) -> PResult<Block> {
        let body = self.block(&["end"])?;
        self.expect_word("end")?;
        self.expect_newline()?;
        Ok(body)
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let start = self.peek_span();
        let kind = match self.peek().clone() {
            Tok::Net(_) | Tok::Mem(_) | Tok::Punct("{") => {
                let target = self.lvalue()?;
                let kind = if self.eat_punct("=") {
                    AssignKind::Blocking
                } else if self.eat_punct("<=") {
                    AssignKind::NonBlocking
                } else {
                    return self.unexpected("`=` or `<=`");
                };
                let value = self.expr()?;
                let d = self.optional_after()?;
                self.expect_newline()?;
                StmtKind::Assign {
                    target,
                    value,
                    kind,
                    delay: d,
                }
            }
            Tok::Word(w) => {
                self.bump();
                match w.as_str() {
                    "if" => {
                        let cond = self.expr()?;
                        self.expect_newline()?;
                        let then_ = self.block(&["else", "end"])?;
                        let else_ = if self.eat_word("else") {
                            self.expect_newline()?;
                            self.block(&["end"])?
                        } else {
                            Vec::new()
                        };
                        self.expect_word("end")?;
                        self.expect_newline()?;
                        StmtKind::If { cond, then_, else_ }
                    }
                    "case" | "casez" | "casex" => {
                        let kind = match w.as_str() {
                            "casez" => CaseKind::Z,
                            "casex" => CaseKind::X,
                            _ => CaseKind::Plain,
                        };
                        let subject = self.expr()?;
                        let qualifier = if self.eat_word("unique") {
                            CaseQualifier::Unique
                        } else if self.eat_word("priority") {
                            CaseQualifier::Priority
                        } else {
                            CaseQualifier::None
                        };
                        self.expect_newline()?;
                        let mut arms = Vec::new();
                        let mut default = None;
                        loop {
                            self.skip_newlines();
                            if self.eat_word("when") {
                                let mut values = Vec::new();
                                loop {
                                    values.push(self.expr()?);
                                    if !self.eat_punct(",") {
                                        break;
                                    }
                                }
                                self.expect_newline()?;
                                let body = self.block_end()?;
                                arms.push(CaseArm { values, body });
                            } else if self.eat_word("default") {
                                self.expect_newline()?;
                                default = Some(self.block_end()?);
                            } else if self.eat_word("end") {
                                self.expect_newline()?;
                                break;
                            } else {
                                return self.unexpected("`when`, `default` or `end`");
                            }
                        }
                        StmtKind::Case {
                            subject,
                            kind,
                            qualifier,
                            arms,
                            default,
                        }
                    }
                    "for" => {
                        let init = self.optional_loop_assign()?;
                        self.expect_punct(";")?;
                        let cond = if self.at_punct(";") {
                            None
                        } else {
                            Some(self.expr()?)
                        };
                        self.expect_punct(";")?;
                        let step = self.optional_loop_assign()?;
                        self.expect_newline()?;
                        let body = self.block_end()?;
                        StmtKind::For {
                            init,
                            cond,
                            step,
                            body,
                        }
                    }
                    "while" => {
                        let cond = self.expr()?;
                        self.expect_newline()?;
                        let body = self.block_end()?;
                        StmtKind::While { cond, body }
                    }
                    "repeat" => {
                        let count = self.expr()?;
                        self.expect_newline()?;
                        let body = self.block_end()?;
                        StmtKind::Repeat { count, body }
                    }
                    "forever" => {
                        self.expect_newline()?;
                        let body = self.block_end()?;
                        StmtKind::Forever { body }
                    }
                    "block" => {
                        let n = if matches!(self.peek(), Tok::Newline | Tok::Eof) {
                            None
                        } else {
                            Some(self.expect_name()?)
                        };
                        self.expect_newline()?;
                        let body = self.block_end()?;
                        StmtKind::Block { name: n, body }
                    }
                    "wait" => {
                        let kind = if self.eat_word("for") {
                            WaitKind::Delay(self.expr()?)
                        } else if self.eat_word("on") {
                            WaitKind::Event(self.edges()?)
                        } else if self.eat_word("until") {
                            WaitKind::Until(self.expr()?)
                        } else {
                            return self.unexpected("`for`, `on` or `until`");
                        };
                        self.expect_newline()?;
                        StmtKind::Wait(kind)
                    }
                    "sys" => {
                        let n = self.expect_name()?;
                        self.expect_punct("(")?;
                        let args = self.args_until(")")?;
                        self.expect_newline()?;
                        StmtKind::SysCall { name: n, args }
                    }
                    "memwrite" => {
                        let Tok::Mem(m) = self.peek().clone() else {
                            return self.unexpected("a memory reference (`@name`)");
                        };
                        let span = self.peek_span();
                        self.bump();
                        let mem = self.lookup_mem(&m, span)?;
                        self.expect_punct("[")?;
                        let addr = self.expr()?;
                        self.expect_punct("]")?;
                        self.expect_punct("=")?;
                        let value = self.expr()?;
                        let enable = if self.eat_word("enable") {
                            Some(self.expr()?)
                        } else {
                            None
                        };
                        self.expect_newline()?;
                        StmtKind::MemWrite {
                            mem,
                            addr,
                            value,
                            enable,
                        }
                    }
                    "assert" => {
                        let sev_span = self.peek_span();
                        let severity = match self.peek().clone() {
                            Tok::Word(s) => match ReportSeverity::from_keyword(&s) {
                                Some(s) => {
                                    self.bump();
                                    s
                                }
                                None => {
                                    return self.fail(sev_span, format!("unknown severity `{s}`"));
                                }
                            },
                            _ => return self.unexpected("a severity"),
                        };
                        let cond = self.expr()?;
                        let mut message = Vec::new();
                        if self.eat_word("report") {
                            loop {
                                message.push(self.expr()?);
                                if !self.eat_punct(",") {
                                    break;
                                }
                            }
                        }
                        self.expect_newline()?;
                        StmtKind::Assert {
                            cond,
                            severity,
                            message,
                        }
                    }
                    "finish" => {
                        self.expect_newline()?;
                        StmtKind::Finish
                    }
                    "stop" => {
                        self.expect_newline()?;
                        StmtKind::Stop
                    }
                    "break" => {
                        self.expect_newline()?;
                        StmtKind::Break
                    }
                    "continue" => {
                        self.expect_newline()?;
                        StmtKind::Continue
                    }
                    other => return self.fail(start, format!("unknown statement `{other}`")),
                }
            }
            _ => return self.unexpected("a statement"),
        };
        Ok(Stmt::new(kind, self.span_from(start)))
    }

    /// `lvalue = expr` when the next token starts an lvalue.
    fn optional_loop_assign(&mut self) -> PResult<Option<(Lvalue, ExprId)>> {
        if !matches!(self.peek(), Tok::Net(_) | Tok::Mem(_) | Tok::Punct("{")) {
            return Ok(None);
        }
        let lv = self.lvalue()?;
        self.expect_punct("=")?;
        let e = self.expr()?;
        Ok(Some((lv, e)))
    }

    fn lvalue(&mut self) -> PResult<Lvalue> {
        match self.peek().clone() {
            Tok::Net(n) => {
                let span = self.peek_span();
                self.bump();
                let net = self.lookup_net(&n, span)?;
                if !self.eat_punct("[") {
                    return Ok(Lvalue::Net(net));
                }
                if let Tok::Int(_) = self.peek() {
                    let hi = self.expect_u32()?;
                    self.expect_punct(":")?;
                    let lo = self.expect_u32()?;
                    self.expect_punct("]")?;
                    return Ok(Lvalue::Slice { net, hi, lo });
                }
                let index = self.expr()?;
                self.expect_punct("]")?;
                Ok(Lvalue::Index { net, index })
            }
            Tok::Mem(m) => {
                let span = self.peek_span();
                self.bump();
                let mem = self.lookup_mem(&m, span)?;
                self.expect_punct("[")?;
                let addr = self.expr()?;
                self.expect_punct("]")?;
                Ok(Lvalue::MemElem { mem, addr })
            }
            Tok::Punct("{") => {
                self.bump();
                let mut parts = Vec::new();
                if !self.at_punct("}") {
                    loop {
                        parts.push(self.lvalue()?);
                        if !self.eat_punct(",") {
                            break;
                        }
                    }
                }
                self.expect_punct("}")?;
                Ok(Lvalue::Concat(parts))
            }
            _ => self.unexpected("an assignment target"),
        }
    }

    // --- expressions ---------------------------------------------------------

    /// Adds a node, typing it by inference or, failing that, after its
    /// first operand (the validator reports the mismatch later).
    fn add_expr(&mut self, kind: ExprKind, span: Span) -> ExprId {
        let ty = match infer_type(&self.module, &kind) {
            Ok(ty) => ty,
            Err(_) => operands(&kind)
                .first()
                .and_then(|id| self.module.exprs.get(*id))
                .map_or(Type::bit(), |e| e.ty.clone()),
        };
        self.module.add_expr(Expr::new(kind, ty, span))
    }

    /// Comma-separated expressions up to and including `close`.
    fn args_until(&mut self, close: &str) -> PResult<Vec<ExprId>> {
        let mut args = Vec::new();
        if !self.at_punct(close) {
            loop {
                args.push(self.expr()?);
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct(close)?;
        Ok(args)
    }

    fn expr(&mut self) -> PResult<ExprId> {
        let start = self.peek_span();
        let mut id = self.primary()?;
        while self.eat_punct("[") {
            let kind = if let Tok::Int(_) = self.peek() {
                let hi = self.expect_u32()?;
                self.expect_punct(":")?;
                let lo = self.expect_u32()?;
                ExprKind::Slice { base: id, hi, lo }
            } else {
                let index = self.expr()?;
                if self.at_punct("+:") || self.at_punct("-:") {
                    let up = self.eat_punct("+:");
                    if !up {
                        self.bump();
                    }
                    let width = self.expect_u32()?;
                    ExprKind::IndexedSlice {
                        base: id,
                        offset: index,
                        width,
                        up,
                    }
                } else {
                    ExprKind::Index { base: id, index }
                }
            };
            self.expect_punct("]")?;
            id = self.add_expr(kind, self.span_from(start));
        }
        if self.eat_word("as") {
            let ty = self.expect_type()?;
            self.module.exprs[id].ty = ty;
            self.module.exprs[id].span = self.span_from(start);
        }
        Ok(id)
    }

    fn primary(&mut self) -> PResult<ExprId> {
        let start = self.peek_span();
        let tok = self.peek().clone();
        let kind = match tok {
            Tok::Lit(_) => ExprKind::Const(self.expect_const()?),
            Tok::Str(s) => {
                self.bump();
                ExprKind::String(s)
            }
            Tok::Net(n) => {
                self.bump();
                ExprKind::Net(self.lookup_net(&n, start)?)
            }
            Tok::Mem(m) => {
                self.bump();
                let mem = self.lookup_mem(&m, start)?;
                self.expect_punct("[")?;
                let addr = self.expr()?;
                self.expect_punct("]")?;
                ExprKind::MemRead { mem, addr }
            }
            Tok::Punct("(") => {
                self.bump();
                let inner = self.expr()?;
                self.expect_punct(")")?;
                return Ok(inner);
            }
            Tok::Punct("{") => {
                self.bump();
                if let Tok::Int(_) = self.peek() {
                    let count = self.expect_u32()?;
                    self.expect_punct("{")?;
                    let expr = self.expr()?;
                    self.expect_punct("}")?;
                    self.expect_punct("}")?;
                    ExprKind::Replicate { count, expr }
                } else {
                    ExprKind::Concat(self.args_until("}")?)
                }
            }
            Tok::Word(w) => {
                self.bump();
                match w.as_str() {
                    "call" => {
                        let n = self.expect_name()?;
                        self.expect_punct("(")?;
                        let args = self.args_until(")")?;
                        ExprKind::Call { name: n, args }
                    }
                    "mux" => {
                        self.expect_punct("(")?;
                        let cond = self.expr()?;
                        self.expect_punct(",")?;
                        let then_ = self.expr()?;
                        self.expect_punct(",")?;
                        let else_ = self.expr()?;
                        self.expect_punct(")")?;
                        ExprKind::Ternary { cond, then_, else_ }
                    }
                    "resize" => {
                        self.expect_punct("(")?;
                        let expr = self.expr()?;
                        self.expect_punct(",")?;
                        let ty_span = self.peek_span();
                        let ty = self.expect_type()?;
                        self.expect_punct(")")?;
                        let Type::Bits { width, signed } = ty else {
                            return self.fail(ty_span, "resize needs a bit vector type");
                        };
                        ExprKind::Resize {
                            expr,
                            width,
                            signed,
                        }
                    }
                    other => {
                        if let Some(op) = UnaryOp::from_name(other) {
                            self.expect_punct("(")?;
                            let expr = self.expr()?;
                            self.expect_punct(")")?;
                            ExprKind::Unary { op, expr }
                        } else if let Some(op) = BinaryOp::from_name(other) {
                            self.expect_punct("(")?;
                            let lhs = self.expr()?;
                            self.expect_punct(",")?;
                            let rhs = self.expr()?;
                            self.expect_punct(")")?;
                            ExprKind::Binary { op, lhs, rhs }
                        } else {
                            return self.fail(start, format!("unknown operator `{other}`"));
                        }
                    }
                }
            }
            _ => return self.unexpected("an expression"),
        };
        Ok(self.add_expr(kind, self.span_from(start)))
    }
}

/// Renders a constant in the text format's canonical form: decimal for
/// two-state values up to 64 bits (`8'd255`, `8'sd255`), hexadecimal for
/// wider two-state values, binary when any bit is `x` or `z` (`4'b10xz`).
fn const_text(c: &Const) -> String {
    let s = if c.is_signed() { "s" } else { "" };
    let width = c.width();
    if width <= 64
        && let Some(v) = c.to_u64()
    {
        return format!("{width}'{s}d{v}");
    }
    if c.is_fully_known() {
        let mut digits = String::new();
        let mut lo = 0;
        while lo < width {
            let hi = (lo + 3).min(width - 1);
            let nibble = c.slice(hi, lo).to_u64().unwrap_or(0);
            digits.push(char::from_digit(u32::try_from(nibble).unwrap_or(0), 16).unwrap_or('0'));
            lo += 4;
        }
        let digits: String = digits.chars().rev().collect();
        return format!("{width}'{s}h{digits}");
    }
    format!("{width}'{s}b{}", c.to_binary_string())
}

#[cfg(test)]
mod tests {
    use super::const_text;
    use crate::ir::Bit;

    #[test]
    fn const_text_is_canonical() {
        assert_eq!(const_text(&Const::from_u64(255, 8)), "8'd255");
        assert_eq!(const_text(&Const::from_i64(-1, 8)), "8'sd255");
        assert_eq!(const_text(&Const::x(4)), "4'bxxxx");
        assert_eq!(
            const_text(&Const::from_bits(&[Bit::Z, Bit::X, Bit::One, Bit::Zero])),
            "4'b01xz"
        );
        assert_eq!(const_text(&Const::from_u64(1, 1)), "1'd1");
        let wide = Const::from_u64(u64::MAX, 72);
        assert_eq!(const_text(&wide), "72'h00ffffffffffffffff");
        let mut top = Const::zero(65);
        top.set_bit(64, Bit::One);
        assert_eq!(const_text(&top), "65'h10000000000000000");
    }

    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::validate::validate;
    use crate::source::SourceMap;

    fn parse(text: &str) -> Result<Design, String> {
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        Design::parse_text(text, file).map_err(|d| d.render(&map))
    }

    fn round_trip(text: &str) {
        let design = parse(text).unwrap_or_else(|e| panic!("parse failed:\n{e}"));
        assert_eq!(design.to_text(), text);
        let again = parse(&design.to_text()).unwrap();
        assert_eq!(again.to_text(), text);
    }

    #[test]
    fn builder_module_round_trips() {
        let mut map = SourceMap::new();
        let file = map.add("t", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("m", span);
        b.attr("keep_hierarchy", 1);
        let clk = b.input("clk", Type::bit());
        let a = b.input("a", Type::bits(8));
        let y = b.output_reg("y", Type::bits(8));
        b.net_attr(y, "keep", "yes");
        let an = b.net(a);
        let one = b.const_u64(8, 1);
        let sum = b.add(an, one);
        let call = b.call("$clog2", vec![an], Type::Integer);
        let sized = b.zext(call, 8);
        let mut p = b.process(Some("p"), ProcessKind::posedge(clk));
        p.nonblocking(y, sum);
        p.nonblocking(y, sized);
        b.end_process(p);
        let mut design = Design::new();
        let id = design.add_module(b.finish());
        design.top = Some(id);
        let text = design.to_text();
        let expected = "\
top m

attr keep_hierarchy = 1
module m
  net %clk u1 wire
  net %a u8 wire
  attr keep = \"yes\"
  net %y u8 reg
  port clk in %clk
  port a in %a
  port y out %y
  process p seq posedge %clk
    %y <= add(%a, 8'd1)
    %y <= resize(call $clog2(%a) as int, u8)
  end
end
";
        assert_eq!(text, expected);
        round_trip(&text);
        assert!(validate(&parse(&text).unwrap()).is_empty());
    }

    #[test]
    fn every_construct_round_trips() {
        let text = "\
top \"a b\"

attr \"weird key\" = -3
attr c = 4'b10xz
module \"a b\"
  timescale 1 ns / 10 ps
  attr doc = \"line\\nbreak\"
  param WIDTH = 8
  param NAME = \"x\"
  net %clk u1 wire
  net %rst u1 wire
  net %a u8 wire
  net %s s8 wire
  net %arr [4]u8 var
  net %t u8 reg
  net %i int var
  net %io u1 wire
  net %end u1 wire
  port clk in %clk
  port rst in %rst
  port a in %a
  port io inout %io
  attr ram_style = \"block\"
  memory @m 16 x u8
    init 8'd0 8'd1 8'd2 8'd3 8'd4 8'd5 8'd6 8'd7
    init 8'd8
  memory @rom 4 x u8
    init
  memory @empty 2 x u8
  attr keep = 1
  instance u0 of sub #(W=8, S=\"y\") (a=%a, y=%t)
  instance u1 of sub ()
  assign %t = and(%a, %a)
  assign %t[3:0] = %a[7:4] after 2 ns
  assign {%t[7:4], %t[1:0]} = {%a[0:0], {5{%a[1:1]}}}
  assign @m[%a[3:0]] = %a
  assign %io = mux(%a[%a[2:0]], %a[%a[2:0] +: 1][1'd0], %a[%a[2:0] -: 1][1'd0])
  assign %t = resize(neg(%s), u8)
  assign %t = (call f(%a, \"s\") as u4)[2:0] as u8
  assign %arr[%a] = %a
  process p0 comb
    %t = %a
  end
  process seq posedge %clk, negedge %rst async negedge %rst
    if eq(%rst, 1'd0)
      %t <= 8'd0
    else
      %t <= add(%t, 8'd1) after 1 ns
    end
  end
  process seq async posedge %rst
  end
  process p2 sensitive %a, %clk
    casez %a unique
      when 8'b1zzzzzzz, 8'b01zzzzzz
        %t = 8'd1
      end
      default
        %t = 8'd0
      end
    end
    case %a priority
    end
    casex %a
      when 8'd3
      end
    end
  end
  process p3 initial
    for %i = resize(8'd0, s64) as int; lt(%i, 8'd4 as int); %i = %i
      break
    end
    for;;
      continue
    end
    while ror(%a)
    end
    repeat %a
      stop
    end
    forever
      finish
    end
    block named
      block
      end
    end
    wait for %a
    wait on posedge %clk, %a
    wait until rand(%a)
    sys $display(\"v=%d\", %a)
    sys $finish()
    memwrite @m[%a[3:0]] = %a enable %clk
    memwrite @m[%a[3:0]] = %a
    assert error lnot(%clk) report \"bad\", %a
    assert note %clk
    %i = %i
  end
  process \"comb\" free
    %t[%a] = 1'd1
  end
  cell c0 and (a=%a, b=%a) -> (y=%t)
  cell c1 dff pos en arst neg 8'd0 (clk=%clk, d=%a, en=%clk, rst=%rst) -> (q=%t)
  cell c2 dff neg srst pos 8'd255 (clk=%clk, d=%a, rst=%rst) -> (q=%t)
  cell c3 dff pos (clk=%clk, d=%a) -> (q=%t)
  cell c4 memrd @m clocked (addr=%a[3:0], clk=%clk, en=%clk) -> (data=%t)
  cell c5 memwr @m (addr=%a[3:0], data=%a, en=%clk) -> ()
  cell c6 lut 2 4'd6 (a=%a[1:0]) -> (y=%io)
  cell c7 blackbox SB_IO #(PIN_TYPE=6'd1) (PACKAGE_PIN=%io) -> (D_IN_0=%clk)
  cell if not () -> ()
end

module sub blackbox
  net %a u8 wire
  net %y u8 wire
  port a in %a
  port y out %y
end
";
        round_trip(text);
    }

    #[test]
    fn literals_and_types() {
        let text = "\
module m
  net %a u72 wire
  net %b u4 wire
  assign %a = 72'h00ffffffffffffffff
  assign %a = 72'h10000000000000000f
  assign %b = 4'b1x0z
  assign %b = 4'sd15
  assign %b = 4'bzzzz
end
";
        round_trip(text);
        let d = parse("module m\n  net %a u4 wire\n  assign %a = 4'o17\n  assign %a = 4'hx\n  assign %a = 4'dz\n  assign %a = 4'b1?\n  assign %a = 3'd5 as u4\n  assign %a = 4'bzz\n  assign %a = 4'd1_5\n  assign %a = 4'D5\nend\n").unwrap();
        assert_eq!(
            d.to_text(),
            "module m\n  net %a u4 wire\n  assign %a = 4'd15\n  assign %a = 4'bxxxx\n  assign %a = 4'bzzzz\n  assign %a = 4'b001z\n  assign %a = 3'd5 as u4\n  assign %a = 4'bzzzz\n  assign %a = 4'd15\n  assign %a = 4'd5\nend\n"
        );
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let d = parse("// header\n\n\nmodule m // trailing\n\n  net %a u1 wire\n\nend\n").unwrap();
        assert_eq!(d.to_text(), "module m\n  net %a u1 wire\nend\n");
        assert!(d.top.is_none());
    }

    #[test]
    fn instances_resolve_to_defined_modules() {
        let d = parse(
            "module top\n  instance u of leaf ()\n  instance v of ext ()\nend\nmodule leaf\nend\n",
        )
        .unwrap();
        let top = d.module_by_name("top").unwrap();
        let leaf = d.module_by_name("leaf").unwrap();
        let insts: Vec<_> = d.modules[top]
            .instances
            .values()
            .map(|i| i.module.clone())
            .collect();
        assert_eq!(
            insts,
            [
                ModuleRef::Resolved(leaf),
                ModuleRef::Unresolved(Name::new("ext"))
            ]
        );
    }

    #[test]
    fn reports_errors_with_excerpts() {
        let cases: Vec<(&str, &str)> = vec![
            (
                "module m\n  net %a u1 wire\n  assign %b = %a\nend\n",
                "unknown net `%b`",
            ),
            (
                "module m\n  assign @x[1'd0] = 1'd0\nend\n",
                "unknown memory `@x`",
            ),
            (
                "module m\n  net %a u1 wire\n  assign %a = 1'd0\n",
                "expected `end`",
            ),
            (
                "module m\n  bogus\nend\n",
                "expected a module item, found `bogus`",
            ),
            (
                "module m\n  net %a u1 wire\n  assign %a = 1'q0\nend\n",
                "invalid base `q`",
            ),
            (
                "module m\n  net %a u1 wire\n  assign %a = frob(%a)\nend\n",
                "unknown operator `frob`",
            ),
            ("module m\n  net %a v1 wire\nend\n", "unknown type `v1`"),
            (
                "module m\n  net %a u1 wire\n  process p seq\n    frob\n  end\nend\n",
                "unknown statement `frob`",
            ),
            (
                "top nope\nmodule m\nend\n",
                "top module `nope` is not defined",
            ),
            (
                "module m\n  net %a u1 wire\n  assign %a = \"abc\nend\n",
                "unterminated string",
            ),
            ("module m\n  ~\nend\n", "unexpected character `~`"),
            (
                "module m\n  net %a u1 wire\n  assign %a = 9999999999999999999999'd0\nend\n",
                "invalid size",
            ),
            (
                "module m\n  timescale 1 xs / 1 ps\nend\n",
                "unknown time unit `xs`",
            ),
            (
                "module m\n  init 1'd0\nend\n",
                "`init` must follow a memory",
            ),
            (
                "module m\n  cell c frob () -> ()\nend\n",
                "unknown cell kind `frob`",
            ),
            (
                "module m\n  process p bogus\n  end\nend\n",
                "unknown process kind `bogus`",
            ),
            (
                "module m\n  net %a u1 wire\n  assign %a = resize(%a, int)\nend\n",
                "resize needs a bit vector type",
            ),
            (
                "module m\n  port a sideways %a\nend\n",
                "unknown direction `sideways`",
            ),
            (
                "module m\n  net %a u1 twisted\nend\n",
                "unknown net kind `twisted`",
            ),
            (
                "module m\n  attr a = %x\nend\n",
                "expected an attribute value",
            ),
            (
                "module m\n  attr a = 99999999999999999999\nend\n",
                "too large",
            ),
            (
                "module m\n  net %a u1 wire\n  process p initial\n    assert loud %a\n  end\nend\n",
                "unknown severity `loud`",
            ),
            (
                "module m\n  net %a u1 wire\n  process p initial\n    %a = 1'd0 after 3 minutes\n  end\nend\n",
                "unknown time unit",
            ),
            (
                "module m\n  net %a u1 wire\n  assign %a = \"\\u{zz}\"\nend\n",
                "invalid `\\u{...}` escape",
            ),
            ("module m\n  net %\nend\n", "expected a name after `%`"),
            (
                "module m\n  attr a = 1\nend\n",
                "attributes are not followed by an object",
            ),
            (
                "module m\nend\nattr a = 1\n",
                "attributes are not followed by an object",
            ),
        ];
        for (text, expected) in cases {
            let err = parse(text).expect_err(text);
            assert!(
                err.contains(expected),
                "for {text:?}\nexpected {expected:?} in:\n{err}"
            );
            assert!(err.contains("-->"), "no excerpt for {text:?}:\n{err}");
        }
    }

    #[test]
    fn recovery_reports_multiple_errors() {
        let err = parse("module m\n  net %a u1 wire\n  assign %b = %a\n  assign %c = %a\nend\n")
            .unwrap_err();
        assert!(err.contains("%b") && err.contains("%c"));
    }

    #[test]
    fn string_escapes_round_trip() {
        let d = parse("attr s = \"q\\\"b\\\\s\\tt\\r\\u{1}\\u{e9}\"\nmodule m\nend\n").unwrap();
        assert_eq!(
            d.modules
                .values()
                .next()
                .unwrap()
                .attrs
                .get("s")
                .and_then(AttrValue::as_str),
            Some("q\"b\\s\tt\r\u{1}é")
        );
        round_trip(&d.to_text());
        assert_eq!(quoted(""), "\"\"");
        assert!(!is_bare(""));
        assert!(!is_bare("9a"));
        assert!(!is_bare("seq"));
        assert!(!is_bare("a b"));
        assert!(is_bare("a$9"));
        assert!(is_bare("$display"));
        assert!(is_bare("end"));
    }

    #[test]
    fn module_to_text_alone() {
        let d = parse("top m\nmodule m\nend\n").unwrap();
        assert_eq!(
            d.modules.values().next().unwrap().to_text(),
            "module m\nend\n"
        );
    }

    #[test]
    fn dangling_ids_print_placeholders() {
        let mut map = SourceMap::new();
        let file = map.add("t", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("m", span);
        b.add_port("p", PortDir::In, NetId(5));
        let y = b.add_net("y", Type::bit());
        b.assign(y, ExprId(9));
        b.instance(
            "u",
            ModuleRef::Resolved(super::super::design::ModuleId(3)),
            vec![],
        );
        let text = b.finish().to_text();
        assert!(text.contains("port p in %\"?n5\""));
        assert!(text.contains("assign %y = \"?e9\""));
        assert!(text.contains("instance u of \"?m3\" ()"));
    }
}
