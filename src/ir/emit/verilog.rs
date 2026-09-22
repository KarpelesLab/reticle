//! Verilog-2005 emission.
//!
//! Every module becomes one `module ... endmodule`. The process form is
//! rendered as behavioural Verilog (`always @(posedge clk or negedge rst)`,
//! `always @*`, `initial`, blocking and non-blocking assignments, `if`,
//! `case` / `casez` / `casex`, loops, `wait`, `$display`-style system
//! tasks, delays under a `` `timescale `` derived from the module's), the
//! cell form as `assign` statements and `always` blocks, one per cell, and
//! instances as module instantiations with named connections.
//!
//! # Names
//!
//! Identifiers that are keywords or not plain `[A-Za-z_][A-Za-z0-9_$]*`
//! names are written as escaped identifiers (`\name `). Helper functions
//! the emitter generates are prefixed `reticle_`, so user names with that
//! prefix are escaped too.
//!
//! # Widths and signedness
//!
//! The IR requires operands of equal width, so the printer never invents
//! extension: `Resize` becomes an explicit concatenation (zero or sign
//! extension), a part-select (truncation) or a `$signed` / `$unsigned`
//! cast, and a generated function when the operand is not a name that can
//! be part-selected. Expression operands are parenthesised whenever their
//! precedence differs from the enclosing operator's, or when they sit on
//! the right of an operator of the same precedence, so the output reads
//! back with the IR's tree regardless of associativity.
//!
//! # Cells
//!
//! | Cell                      | Rendering                                                            |
//! |---------------------------|----------------------------------------------------------------------|
//! | bitwise, arithmetic, compare, reduce, `Buf` | `assign y = a op b;`                               |
//! | `Mux`                     | `assign y = s ? b : a;`                                              |
//! | `Pmux`                    | `assign y = reticle_pmux_W_N(a, b, s);` (a generated function)       |
//! | `Dff`                     | `always @(posedge clk [or posedge rst])` with reset before enable    |
//! | `Dlatch`                  | `always @* if (en) q = d;`                                           |
//! | `MemRdPort` / `MemWrPort` | `assign` or `always @(posedge clk)` reads and writes of the array    |
//! | `Lut`                     | `localparam` holding `init`, indexed by `a`                          |
//! | `Tristate`                | `assign y = en ? a : {W{1'bz}};`                                     |
//! | `Blackbox`                | a module instantiation with `#(params)`                              |
//!
//! `Assert` becomes `if (!(cond)) $error(...)`, with `$info`, `$warning`
//! and `$fatal` for the other severities. `unique` and `priority` cases
//! carry `(* parallel_case, full_case *)` / `(* full_case *)` attributes.
//! `break` and `continue` are lowered to `disable` of named blocks, since
//! Verilog-2005 has neither.
//!
//! Black-box modules (interface only) are omitted unless
//! [`VerilogOptions::blackboxes`] is set; the target flow supplies them.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::super::attr::{AttrValue, Attrs};
use super::super::cell::{Cell, CellKind};
use super::super::design::{Design, Memory, Module, ModuleId, ModuleRef, Net, NetId, PortDir};
use super::super::expr::{BinaryOp, ExprId, ExprKind, UnaryOp};
use super::super::process::{
    AssignKind, Block, CaseKind, CaseQualifier, Delay, Edge, Lvalue, Polarity, Process,
    ProcessKind, ReportSeverity, Stmt, StmtKind, TimeUnit, WaitKind,
};
use super::super::types::{Bit, Const, Type};
use super::{EmitError, Out, verilog_ident, verilog_string};
use crate::source::Span;

/// Options for the Verilog emitter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerilogOptions {
    /// Reject modules that still contain processes.
    pub structural_only: bool,
    /// Declare ports in the header (`module m (input wire clk, ...)`)
    /// rather than in the body (`module m (clk); input clk;`). A module
    /// whose port names differ from its net names always uses the body
    /// style, with `.port(net)` expressions.
    pub ansi_ports: bool,
    /// Emit IR attributes as `(* key = value *)`.
    pub keep_attrs: bool,
    /// Emit black-box modules as empty `(* blackbox *)` shells.
    pub blackboxes: bool,
}

impl Default for VerilogOptions {
    fn default() -> Self {
        VerilogOptions {
            structural_only: false,
            ansi_ports: true,
            keep_attrs: true,
            blackboxes: false,
        }
    }
}

/// Renders every module of `design` with default options.
pub fn emit_verilog(design: &Design) -> Result<String, EmitError> {
    emit_verilog_with(design, &VerilogOptions::default())
}

/// Renders every module of `design`.
pub fn emit_verilog_with(design: &Design, opts: &VerilogOptions) -> Result<String, EmitError> {
    let mut out = String::new();
    for (id, module) in design.modules.iter() {
        if module.blackbox && !opts.blackboxes {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&emit_verilog_module(design, id, opts)?);
    }
    Ok(out)
}

/// Renders one module.
pub fn emit_verilog_module(
    design: &Design,
    module: ModuleId,
    opts: &VerilogOptions,
) -> Result<String, EmitError> {
    let module = design.module(module);
    if opts.structural_only
        && let Some((_, p)) = module.processes.iter().next()
    {
        return Err(EmitError::new(
            p.span,
            format!(
                "module `{}` has a process but structural output was requested",
                module.name
            ),
        ));
    }
    let mut printer = Printer::new(design, module, opts);
    printer.module()?;
    Ok(printer.finish())
}

/// Operator precedence levels; higher binds tighter.
const PREC_TERNARY: u8 = 2;
const PREC_LOR: u8 = 3;
const PREC_LAND: u8 = 4;
const PREC_BOR: u8 = 5;
const PREC_XOR: u8 = 6;
const PREC_BAND: u8 = 7;
const PREC_EQ: u8 = 8;
const PREC_REL: u8 = 9;
const PREC_SHIFT: u8 = 10;
const PREC_ADD: u8 = 11;
const PREC_MUL: u8 = 12;
const PREC_POW: u8 = 13;
const PREC_UNARY: u8 = 14;
const PREC_ATOM: u8 = 20;

/// Verilog spelling and precedence of a binary operator.
fn binary_op(op: BinaryOp) -> (&'static str, u8) {
    match op {
        BinaryOp::And => ("&", PREC_BAND),
        BinaryOp::Or => ("|", PREC_BOR),
        BinaryOp::Xor => ("^", PREC_XOR),
        BinaryOp::Xnor => ("~^", PREC_XOR),
        BinaryOp::LogicAnd => ("&&", PREC_LAND),
        BinaryOp::LogicOr => ("||", PREC_LOR),
        BinaryOp::Add => ("+", PREC_ADD),
        BinaryOp::Sub => ("-", PREC_ADD),
        BinaryOp::Mul => ("*", PREC_MUL),
        BinaryOp::Div => ("/", PREC_MUL),
        BinaryOp::Mod => ("%", PREC_MUL),
        BinaryOp::Pow => ("**", PREC_POW),
        BinaryOp::Shl => ("<<", PREC_SHIFT),
        BinaryOp::Shr => (">>", PREC_SHIFT),
        BinaryOp::Sshr => (">>>", PREC_SHIFT),
        BinaryOp::Eq => ("==", PREC_EQ),
        BinaryOp::Ne => ("!=", PREC_EQ),
        BinaryOp::CaseEq => ("===", PREC_EQ),
        BinaryOp::CaseNe => ("!==", PREC_EQ),
        BinaryOp::WildEq => ("==?", PREC_EQ),
        BinaryOp::Lt => ("<", PREC_REL),
        BinaryOp::Le => ("<=", PREC_REL),
        BinaryOp::Gt => (">", PREC_REL),
        BinaryOp::Ge => (">=", PREC_REL),
    }
}

fn unary_op(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Not => "~",
        UnaryOp::Neg => "-",
        UnaryOp::ReduceAnd => "&",
        UnaryOp::ReduceOr => "|",
        UnaryOp::ReduceXor => "^",
        UnaryOp::ReduceNand => "~&",
        UnaryOp::ReduceNor => "~|",
        UnaryOp::ReduceXnor => "~^",
        UnaryOp::LogicNot => "!",
    }
}

/// Renders a name as an identifier, also escaping the emitter's own
/// `reticle_` prefix.
fn ident(name: &str) -> String {
    if name.starts_with("reticle_") {
        format!("\\{name} ")
    } else {
        verilog_ident(name)
    }
}

fn const_lit(c: &Const) -> String {
    match c.width() {
        0 => "1'b0".to_owned(),
        // Single bits read better in binary.
        1 => format!(
            "1'{}b{}",
            if c.is_signed() { "s" } else { "" },
            c.bit(0).to_char()
        ),
        _ => c.to_verilog_literal(),
    }
}

fn attr_value(v: &AttrValue) -> String {
    match v {
        AttrValue::Const(c) => const_lit(c),
        AttrValue::String(s) => verilog_string(s),
        AttrValue::Int(i) => i.to_string(),
    }
}

fn attrs_text(attrs: &Attrs) -> String {
    let items: Vec<String> = attrs
        .iter()
        .map(|(k, v)| format!("{} = {}", ident(k.as_str()), attr_value(v)))
        .collect();
    format!("(* {} *)", items.join(", "))
}

/// Renders a delay of `fs` femtoseconds in units of `unit_fs`, as an
/// integer when it divides and as a decimal fraction otherwise.
fn delay_text(fs: u64, unit_fs: u64) -> String {
    let whole = fs / unit_fs;
    let mut rem = fs % unit_fs;
    if rem == 0 {
        return whole.to_string();
    }
    let mut s = format!("{whole}.");
    while rem != 0 && s.len() < 40 {
        rem *= 10;
        s.push(char::from(b'0' + u8::try_from(rem / unit_fs).unwrap_or(0)));
        rem %= unit_fs;
    }
    s
}

/// True when `block` contains `kind` outside any nested loop.
fn contains_direct(block: &Block, pred: &dyn Fn(&StmtKind) -> bool) -> bool {
    block.iter().any(|stmt| {
        if pred(&stmt.kind) {
            return true;
        }
        match &stmt.kind {
            StmtKind::For { .. }
            | StmtKind::While { .. }
            | StmtKind::Repeat { .. }
            | StmtKind::Forever { .. } => false,
            _ => stmt.blocks().iter().any(|b| contains_direct(b, pred)),
        }
    })
}

/// Which of a module's nets are written where, to choose `reg` or `wire`.
struct NetUse {
    by_process: Vec<bool>,
    by_always_cell: Vec<bool>,
    structural: Vec<bool>,
}

impl NetUse {
    fn of(design: &Design, module: &Module) -> Self {
        let n = module.nets.len();
        let mut use_ = NetUse {
            by_process: vec![false; n],
            by_always_cell: vec![false; n],
            structural: vec![false; n],
        };
        module.for_each_stmt(|stmt| {
            let mut mark = |lv: &Lvalue| {
                for net in lv.nets() {
                    use_.by_process[net.index()] = true;
                }
            };
            match &stmt.kind {
                StmtKind::Assign { target, .. } => mark(target),
                StmtKind::For { init, step, .. } => {
                    if let Some((lv, _)) = init {
                        mark(lv);
                    }
                    if let Some((lv, _)) = step {
                        mark(lv);
                    }
                }
                _ => {}
            }
        });
        for assign in &module.assigns {
            for net in assign.target.nets() {
                use_.structural[net.index()] = true;
            }
        }
        for (_, cell) in module.cells.iter() {
            let always = matches!(
                cell.kind,
                CellKind::Dff { .. } | CellKind::Dlatch | CellKind::MemRdPort { clocked: true, .. }
            );
            for (_, net) in &cell.outputs {
                if always {
                    use_.by_always_cell[net.index()] = true;
                } else {
                    use_.structural[net.index()] = true;
                }
            }
        }
        for (_, inst) in module.instances.iter() {
            let target = inst.module.id().map(|id| design.module(id));
            for (port, id) in &inst.connections {
                let is_out = target.and_then(|t| t.port(port.as_str())).map(|p| p.dir);
                if is_out.is_some_and(|d| d != PortDir::In) || target.is_none() {
                    // Unresolved targets: treat any net connection as a possible driver.
                    collect_nets(module, *id, &mut |net| use_.structural[net.index()] = true);
                }
            }
        }
        use_
    }

    fn is_reg(
        &self,
        net: NetId,
        kind: super::super::design::NetKind,
        dir: Option<PortDir>,
    ) -> bool {
        if dir == Some(PortDir::InOut) {
            return false;
        }
        let i = net.index();
        if self.by_process[i] || self.by_always_cell[i] {
            return true;
        }
        if self.structural[i] {
            return false;
        }
        kind != super::super::design::NetKind::Wire
    }
}

fn collect_nets(module: &Module, id: ExprId, f: &mut dyn FnMut(NetId)) {
    match &module.exprs[id].kind {
        ExprKind::Net(n) => f(*n),
        ExprKind::Slice { base, .. } => collect_nets(module, *base, f),
        ExprKind::Concat(parts) => parts.iter().for_each(|p| collect_nets(module, *p, f)),
        _ => {}
    }
}

struct Printer<'a> {
    design: &'a Design,
    module: &'a Module,
    opts: &'a VerilogOptions,
    out: Out,
    /// Generated helper functions, by name.
    functions: BTreeMap<String, String>,
    use_: NetUse,
    /// The unit delays are written in, in femtoseconds.
    unit_fs: u64,
    /// Whether the process being printed is clocked (memory writes and
    /// latches use non-blocking assignments there).
    in_sequential: bool,
    /// Labels of the enclosing loops: (break block, continue block).
    loops: Vec<(Option<String>, Option<String>)>,
    next_label: u32,
}

impl<'a> Printer<'a> {
    fn new(design: &'a Design, module: &'a Module, opts: &'a VerilogOptions) -> Self {
        Printer {
            design,
            module,
            opts,
            out: Out::new("  "),
            functions: BTreeMap::new(),
            use_: NetUse::of(design, module),
            unit_fs: TimeUnit::Ns.in_fs(),
            in_sequential: false,
            loops: Vec::new(),
            next_label: 0,
        }
    }

    fn finish(self) -> String {
        self.out.finish()
    }

    fn err(&self, span: Span, message: impl Into<String>) -> EmitError {
        EmitError::new(span, message)
    }

    // --- module -------------------------------------------------------------

    fn module(&mut self) -> Result<(), EmitError> {
        let m = self.module;
        self.timescale();
        if self.opts.keep_attrs && !m.attrs.is_empty() {
            self.out.line(&attrs_text(&m.attrs));
        }
        if m.blackbox && !(self.opts.keep_attrs && m.attrs.contains("blackbox")) {
            self.out.line("(* blackbox *)");
        }
        let ansi = self.opts.ansi_ports && m.ports.iter().all(|p| p.name == m.nets[p.net].name);
        self.header(ansi)?;
        self.out.indent();
        for param in &m.params {
            if self.opts.keep_attrs && !param.attrs.is_empty() {
                self.out.line(&attrs_text(&param.attrs));
            }
            self.out.line(&format!(
                "parameter {} = {};",
                ident(param.name.as_str()),
                attr_value(&param.value)
            ));
        }
        if m.blackbox {
            self.out.dedent();
            self.out.line("endmodule");
            return Ok(());
        }
        if !ansi {
            for port in &m.ports {
                let net = &m.nets[port.net];
                let dir = match port.dir {
                    PortDir::In => "input",
                    PortDir::Out => "output",
                    PortDir::InOut => "inout",
                };
                let range = self.packed_range(&net.ty, net.span)?;
                self.out.line(&format!(
                    "{dir}{}{} {};",
                    if net.ty.is_signed() && net.ty.is_bits() {
                        " signed"
                    } else {
                        ""
                    },
                    range,
                    ident(net.name.as_str())
                ));
            }
        }
        // Declarations for nets that are not ANSI ports.
        for (id, net) in m.nets.iter() {
            let port = m.port_of_net(id);
            if ansi && port.is_some() {
                continue;
            }
            let decl = self.net_decl(id, net, port.map(|p| p.dir))?;
            if self.opts.keep_attrs && !net.attrs.is_empty() {
                self.out.line(&attrs_text(&net.attrs));
            }
            self.out.line(&format!("{decl};"));
        }
        for (_, mem) in m.memories.iter() {
            self.memory(mem)?;
        }
        // Body into a separate buffer so generated functions can precede it.
        let mut body = Out::new("  ");
        body.indent();
        std::mem::swap(&mut self.out, &mut body);
        for assign in &m.assigns {
            if self.opts.keep_attrs && !assign.attrs.is_empty() {
                self.out.line(&attrs_text(&assign.attrs));
            }
            let target = self.lvalue(&assign.target)?;
            let value = self.expr(assign.value)?;
            let delay = match assign.delay {
                Some(d) => format!("#{} ", self.delay(d)),
                None => String::new(),
            };
            self.out.line(&format!("assign {delay}{target} = {value};"));
        }
        for (_, process) in m.processes.iter() {
            self.process(process)?;
        }
        for (_, cell) in m.cells.iter() {
            self.cell(cell)?;
        }
        for (_, inst) in m.instances.iter() {
            let target = match &inst.module {
                ModuleRef::Resolved(id) => self.design.module(*id).name.as_str(),
                ModuleRef::Unresolved(name) => name.as_str(),
            };
            let conns: Vec<(String, String)> = inst
                .connections
                .iter()
                .map(|(port, e)| Ok((port.as_str().to_owned(), self.expr(*e)?)))
                .collect::<Result<_, EmitError>>()?;
            self.instantiation(
                target,
                inst.name.as_str(),
                &inst.params,
                &inst.attrs,
                &conns,
            );
        }
        std::mem::swap(&mut self.out, &mut body);
        for text in self.functions.values() {
            self.out.raw(text);
        }
        self.out.raw(&body.finish());
        self.out.dedent();
        self.out.line("endmodule");
        Ok(())
    }

    fn timescale(&mut self) {
        let m = self.module;
        let mut used: Option<TimeUnit> = None;
        let mut note = |d: Option<Delay>| {
            if let Some(d) = d {
                used = Some(used.map_or(d.unit, |u| u.min(d.unit)));
            }
        };
        for assign in &m.assigns {
            note(assign.delay);
        }
        let mut waits = false;
        m.for_each_stmt(|stmt| match &stmt.kind {
            StmtKind::Assign { delay, .. } => note(*delay),
            StmtKind::Wait(WaitKind::Delay(_)) => waits = true,
            _ => {}
        });
        match m.timescale {
            Some(ts) => {
                self.unit_fs = ts.unit.to_fs().max(1);
                self.out.line(&format!(
                    "`timescale {}{} / {}{}",
                    ts.unit.value,
                    ts.unit.unit.name(),
                    ts.precision.value,
                    ts.precision.unit.name()
                ));
            }
            None => {
                if used.is_some() || waits {
                    let unit = used.unwrap_or(TimeUnit::Ns);
                    self.unit_fs = unit.in_fs();
                    let name = unit.name();
                    self.out.line(&format!("`timescale 1{name} / 1{name}"));
                }
            }
        }
    }

    fn delay(&self, d: Delay) -> String {
        delay_text(d.to_fs(), self.unit_fs)
    }

    fn header(&mut self, ansi: bool) -> Result<(), EmitError> {
        let m = self.module;
        let name = ident(m.name.as_str());
        if m.ports.is_empty() {
            self.out.line(&format!("module {name};"));
            return Ok(());
        }
        if !ansi {
            let ports: Vec<String> = m
                .ports
                .iter()
                .map(|p| {
                    let net = &m.nets[p.net];
                    if p.name == net.name {
                        ident(p.name.as_str())
                    } else {
                        format!(".{}({})", ident(p.name.as_str()), ident(net.name.as_str()))
                    }
                })
                .collect();
            self.out
                .line(&format!("module {name} ({});", ports.join(", ")));
            return Ok(());
        }
        self.out.line(&format!("module {name} ("));
        self.out.indent();
        let count = m.ports.len();
        for (i, port) in m.ports.iter().enumerate() {
            let net = &m.nets[port.net];
            let dir = match port.dir {
                PortDir::In => "input",
                PortDir::Out => "output",
                PortDir::InOut => "inout",
            };
            let decl = self.net_decl(port.net, net, Some(port.dir))?;
            let attrs = if self.opts.keep_attrs && !net.attrs.is_empty() {
                format!("{} ", attrs_text(&net.attrs))
            } else {
                String::new()
            };
            let comma = if i + 1 < count { "," } else { "" };
            self.out.line(&format!("{attrs}{dir} {decl}{comma}"));
        }
        self.out.dedent();
        self.out.line(");");
        Ok(())
    }

    /// `[hi:0]` for vectors wider than one bit, empty otherwise.
    fn packed_range(&self, ty: &Type, span: Span) -> Result<String, EmitError> {
        let mut t = ty;
        while let Type::Array { elem, .. } = t {
            t = elem;
        }
        match t {
            Type::Bits { width, .. } => Ok(if *width == 1 {
                String::new()
            } else {
                format!(" [{}:0]", width.saturating_sub(1))
            }),
            Type::Integer | Type::Real => Ok(String::new()),
            Type::String => Err(self.err(span, "Verilog-2005 has no string variables")),
            Type::Array { .. } => unreachable!(),
        }
    }

    /// `wire [7:0] name [0:3]`, without the terminating semicolon.
    fn net_decl(&self, id: NetId, net: &Net, dir: Option<PortDir>) -> Result<String, EmitError> {
        let reg = self.use_.is_reg(id, net.kind, dir);
        let mut dims = String::new();
        let mut t = &net.ty;
        while let Type::Array { elem, len } = t {
            let _ = write!(dims, " [0:{}]", len.saturating_sub(1));
            t = elem;
        }
        let range = self.packed_range(&net.ty, net.span)?;
        let base = match t {
            Type::Bits { signed, .. } => {
                let kind = if reg { "reg" } else { "wire" };
                let signed = if *signed { " signed" } else { "" };
                format!("{kind}{signed}{range}")
            }
            Type::Integer => "integer".to_owned(),
            Type::Real => "real".to_owned(),
            Type::String | Type::Array { .. } => {
                return Err(self.err(net.span, "Verilog-2005 has no string variables"));
            }
        };
        Ok(format!("{base} {}{dims}", ident(net.name.as_str())))
    }

    fn memory(&mut self, mem: &Memory) -> Result<(), EmitError> {
        let range = self.packed_range(&mem.elem, mem.span)?;
        let mut dims = format!(" [0:{}]", mem.size.saturating_sub(1));
        let mut t = &mem.elem;
        while let Type::Array { elem, len } = t {
            let _ = write!(dims, " [0:{}]", len.saturating_sub(1));
            t = elem;
        }
        let signed = if t.is_signed() && t.is_bits() {
            " signed"
        } else {
            ""
        };
        let base = match t {
            Type::Integer => "integer".to_owned(),
            Type::Real => "real".to_owned(),
            _ => format!("reg{signed}{range}"),
        };
        if self.opts.keep_attrs && !mem.attrs.is_empty() {
            self.out.line(&attrs_text(&mem.attrs));
        }
        let name = ident(mem.name.as_str());
        self.out.line(&format!("{base} {name}{dims};"));
        if let Some(init) = &mem.init
            && !init.is_empty()
        {
            if !mem.elem.is_bits() {
                return Err(self.err(
                    mem.span,
                    "memory initialisation is only emitted for bit-vector elements",
                ));
            }
            self.out.line("initial begin");
            self.out.indent();
            for (i, value) in init.iter().enumerate() {
                self.out
                    .line(&format!("{name}[{i}] = {};", const_lit(value)));
            }
            self.out.dedent();
            self.out.line("end");
        }
        Ok(())
    }

    fn instantiation(
        &mut self,
        target: &str,
        name: &str,
        params: &Attrs,
        attrs: &Attrs,
        conns: &[(String, String)],
    ) {
        if self.opts.keep_attrs && !attrs.is_empty() {
            self.out.line(&attrs_text(attrs));
        }
        let mut head = ident(target);
        if !params.is_empty() {
            let items: Vec<String> = params
                .iter()
                .map(|(k, v)| format!(".{}({})", ident(k.as_str()), attr_value(v)))
                .collect();
            let _ = write!(head, " #({})", items.join(", "));
        }
        let _ = write!(head, " {}", ident(name));
        if conns.is_empty() {
            self.out.line(&format!("{head} ();"));
            return;
        }
        self.out.line(&format!("{head} ("));
        self.out.indent();
        let count = conns.len();
        for (i, (port, value)) in conns.iter().enumerate() {
            let comma = if i + 1 < count { "," } else { "" };
            self.out.line(&format!(".{}({value}){comma}", ident(port)));
        }
        self.out.dedent();
        self.out.line(");");
    }

    // --- processes ----------------------------------------------------------

    fn edge_list(&self, edges: &[Edge]) -> String {
        edges
            .iter()
            .map(|e| {
                let name = ident(self.module.nets[e.net].name.as_str());
                match e.polarity {
                    Polarity::Pos => format!("posedge {name}"),
                    Polarity::Neg => format!("negedge {name}"),
                    Polarity::Any => name,
                }
            })
            .collect::<Vec<_>>()
            .join(" or ")
    }

    fn process(&mut self, process: &Process) -> Result<(), EmitError> {
        if self.opts.keep_attrs && !process.attrs.is_empty() {
            self.out.line(&attrs_text(&process.attrs));
        }
        let head = match &process.kind {
            ProcessKind::Comb => "always @*".to_owned(),
            ProcessKind::Sequential { clocks, resets } => {
                let edges: Vec<Edge> = clocks.iter().chain(resets).copied().collect();
                format!("always @({})", self.edge_list(&edges))
            }
            ProcessKind::Initial => "initial".to_owned(),
            ProcessKind::Sensitive(nets) => {
                if nets.is_empty() {
                    "always @*".to_owned()
                } else {
                    let names: Vec<String> = nets
                        .iter()
                        .map(|n| ident(self.module.nets[*n].name.as_str()))
                        .collect();
                    format!("always @({})", names.join(" or "))
                }
            }
            ProcessKind::Free => "always".to_owned(),
        };
        self.in_sequential = matches!(process.kind, ProcessKind::Sequential { .. });
        let label = match &process.name {
            Some(name) => format!(" : {}", ident(name.as_str())),
            None => String::new(),
        };
        self.out.line(&format!("{head} begin{label}"));
        self.out.indent();
        self.block(&process.body)?;
        self.out.dedent();
        self.out.line("end");
        self.in_sequential = false;
        Ok(())
    }

    fn block(&mut self, block: &Block) -> Result<(), EmitError> {
        for stmt in block {
            self.stmt(stmt)?;
        }
        Ok(())
    }

    /// Renders a block as `begin ... end` on the current line prefix.
    fn begin_end(&mut self, head: &str, block: &Block, tail: &str) -> Result<(), EmitError> {
        self.out.line(&format!("{head} begin"));
        self.out.indent();
        self.block(block)?;
        self.out.dedent();
        self.out.line(&format!("end{tail}"));
        Ok(())
    }

    fn assign_op(&self, kind: AssignKind) -> &'static str {
        match kind {
            AssignKind::Blocking => "=",
            AssignKind::NonBlocking => "<=",
        }
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<(), EmitError> {
        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                kind,
                delay,
            } => {
                let target = self.lvalue(target)?;
                let value = self.expr(*value)?;
                let delay = match delay {
                    Some(d) => format!("#{} ", self.delay(*d)),
                    None => String::new(),
                };
                let op = self.assign_op(*kind);
                self.out.line(&format!("{target} {op} {delay}{value};"));
            }
            StmtKind::If { cond, then_, else_ } => {
                let cond = self.expr(*cond)?;
                self.if_chain(&format!("if ({cond})"), then_, else_)?;
            }
            StmtKind::Case {
                subject,
                kind,
                qualifier,
                arms,
                default,
            } => {
                let subject = self.expr(*subject)?;
                let keyword = match kind {
                    CaseKind::Plain => "case",
                    CaseKind::Z => "casez",
                    CaseKind::X => "casex",
                };
                match qualifier {
                    CaseQualifier::None => {}
                    CaseQualifier::Unique => self.out.line("(* parallel_case, full_case *)"),
                    CaseQualifier::Priority => self.out.line("(* full_case *)"),
                }
                self.out.line(&format!("{keyword} ({subject})"));
                self.out.indent();
                for arm in arms {
                    let values: Vec<String> = arm
                        .values
                        .iter()
                        .map(|v| self.expr(*v))
                        .collect::<Result<_, _>>()?;
                    self.begin_end(&format!("{}:", values.join(", ")), &arm.body, "")?;
                }
                if let Some(default) = default {
                    self.begin_end("default:", default, "")?;
                }
                self.out.dedent();
                self.out.line("endcase");
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                let init_text = match init {
                    Some((lv, e)) => Some(format!("{} = {}", self.lvalue(lv)?, self.expr(*e)?)),
                    None => None,
                };
                let cond_text = match cond {
                    Some(c) => Some(self.expr(*c)?),
                    None => None,
                };
                let step_text = match step {
                    Some((lv, e)) => Some(format!("{} = {}", self.lvalue(lv)?, self.expr(*e)?)),
                    None => None,
                };
                match (init_text, cond_text, step_text) {
                    (Some(i), Some(c), Some(s)) => {
                        self.loop_body(&format!("for ({i}; {c}; {s})"), body, None)?;
                    }
                    (i, c, s) => {
                        // Verilog-2005 needs all three parts: lower to `while`.
                        self.out.line("begin");
                        self.out.indent();
                        if let Some(i) = i {
                            self.out.line(&format!("{i};"));
                        }
                        let head = format!("while ({})", c.as_deref().unwrap_or("1'b1"));
                        self.loop_body(&head, body, s.as_deref())?;
                        self.out.dedent();
                        self.out.line("end");
                    }
                }
            }
            StmtKind::While { cond, body } => {
                let cond = self.expr(*cond)?;
                self.loop_body(&format!("while ({cond})"), body, None)?;
            }
            StmtKind::Repeat { count, body } => {
                let count = self.expr(*count)?;
                self.loop_body(&format!("repeat ({count})"), body, None)?;
            }
            StmtKind::Forever { body } => self.loop_body("forever", body, None)?,
            StmtKind::Block { name, body } => {
                let label = match name {
                    Some(n) => format!(" : {}", ident(n.as_str())),
                    None => String::new(),
                };
                self.out.line(&format!("begin{label}"));
                self.out.indent();
                self.block(body)?;
                self.out.dedent();
                self.out.line("end");
            }
            StmtKind::Wait(kind) => match kind {
                WaitKind::Delay(e) => {
                    let e = self.expr(*e)?;
                    self.out.line(&format!("#({e});"));
                }
                WaitKind::Event(edges) => {
                    let list = self.edge_list(edges);
                    self.out.line(&format!("@({list});"));
                }
                WaitKind::Until(e) => {
                    let e = self.expr(*e)?;
                    self.out.line(&format!("wait ({e});"));
                }
            },
            StmtKind::SysCall { name, args } => {
                let args: Vec<String> = args
                    .iter()
                    .map(|a| self.expr(*a))
                    .collect::<Result<_, _>>()?;
                let name = if name.as_str().starts_with('$') {
                    name.as_str().to_owned()
                } else {
                    ident(name.as_str())
                };
                if args.is_empty() {
                    self.out.line(&format!("{name};"));
                } else {
                    self.out.line(&format!("{name}({});", args.join(", ")));
                }
            }
            StmtKind::MemFile {
                op,
                mem,
                file,
                start,
                end,
                base,
            } => {
                let mut args = vec![
                    self.expr(*file)?,
                    ident(self.module.memories[*mem].name.as_str()),
                ];
                for addr in start.iter().chain(end.iter()) {
                    args.push(self.expr(*addr)?);
                }
                if *base != 0 {
                    // The memory is declared from 0 here, so `@` addresses
                    // in the file no longer line up with it.
                    self.out.line(&format!(
                        "// the file's `@` addresses count element 0 as {base}"
                    ));
                }
                self.out
                    .line(&format!("${}({});", op.keyword(), args.join(", ")));
            }
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                let name = ident(self.module.memories[*mem].name.as_str());
                let addr = self.expr(*addr)?;
                let value = self.expr(*value)?;
                let op = if self.in_sequential { "<=" } else { "=" };
                let text = format!("{name}[{addr}] {op} {value};");
                match enable {
                    Some(en) => {
                        let en = self.expr(*en)?;
                        self.out.line(&format!("if ({en}) {text}"));
                    }
                    None => self.out.line(&text),
                }
            }
            StmtKind::Assert {
                cond,
                severity,
                message,
            } => {
                let cond = self.expr(*cond)?;
                let mut args: Vec<String> = message
                    .iter()
                    .map(|a| self.expr(*a))
                    .collect::<Result<_, _>>()?;
                let task = match severity {
                    ReportSeverity::Note => "$info",
                    ReportSeverity::Warning => "$warning",
                    ReportSeverity::Error => "$error",
                    ReportSeverity::Failure => {
                        args.insert(0, "1".to_owned());
                        "$fatal"
                    }
                };
                let call = if args.is_empty() {
                    task.to_owned()
                } else {
                    format!("{task}({})", args.join(", "))
                };
                self.out.line(&format!("if (!({cond})) {call};"));
            }
            StmtKind::Finish => self.out.line("$finish;"),
            StmtKind::Stop => self.out.line("$stop;"),
            StmtKind::Break => match self.loops.last().and_then(|(b, _)| b.clone()) {
                Some(label) => self.out.line(&format!("disable {label};")),
                None => return Err(self.err(stmt.span, "`break` outside a loop")),
            },
            StmtKind::Continue => match self.loops.last().and_then(|(_, c)| c.clone()) {
                Some(label) => self.out.line(&format!("disable {label};")),
                None => return Err(self.err(stmt.span, "`continue` outside a loop")),
            },
        }
        Ok(())
    }

    fn if_chain(&mut self, head: &str, then_: &Block, else_: &Block) -> Result<(), EmitError> {
        self.out.line(&format!("{head} begin"));
        self.out.indent();
        self.block(then_)?;
        self.out.dedent();
        if else_.is_empty() {
            self.out.line("end");
            return Ok(());
        }
        if let [
            Stmt {
                kind: StmtKind::If { cond, then_, else_ },
                ..
            },
        ] = else_.as_slice()
        {
            let cond = self.expr(*cond)?;
            return self.if_chain(&format!("end else if ({cond})"), then_, else_);
        }
        self.out.line("end else begin");
        self.out.indent();
        self.block(else_)?;
        self.out.dedent();
        self.out.line("end");
        Ok(())
    }

    /// Renders a loop, wrapping it in named blocks when its body uses
    /// `break` or `continue`; `step` is appended to the body for loops
    /// lowered to `while`.
    fn loop_body(&mut self, head: &str, body: &Block, step: Option<&str>) -> Result<(), EmitError> {
        let needs_break = contains_direct(body, &|k| matches!(k, StmtKind::Break));
        let needs_continue = contains_direct(body, &|k| matches!(k, StmtKind::Continue));
        self.next_label += 1;
        let n = self.next_label;
        let brk = needs_break.then(|| format!("reticle_brk_{n}"));
        let cnt = needs_continue.then(|| format!("reticle_cnt_{n}"));
        if let Some(b) = &brk {
            self.out.line(&format!("begin : {b}"));
            self.out.indent();
        }
        self.loops.push((brk.clone(), cnt.clone()));
        match (&cnt, step) {
            (Some(c), Some(step)) => {
                self.out.line(&format!("{head} begin"));
                self.out.indent();
                self.out.line(&format!("begin : {c}"));
                self.out.indent();
                self.block(body)?;
                self.out.dedent();
                self.out.line("end");
                self.out.line(&format!("{step};"));
                self.out.dedent();
                self.out.line("end");
            }
            (Some(c), None) => {
                self.out.line(&format!("{head} begin : {c}"));
                self.out.indent();
                self.block(body)?;
                self.out.dedent();
                self.out.line("end");
            }
            (None, step) => {
                self.out.line(&format!("{head} begin"));
                self.out.indent();
                self.block(body)?;
                if let Some(step) = step {
                    self.out.line(&format!("{step};"));
                }
                self.out.dedent();
                self.out.line("end");
            }
        }
        self.loops.pop();
        if brk.is_some() {
            self.out.dedent();
            self.out.line("end");
        }
        Ok(())
    }

    // --- cells --------------------------------------------------------------

    fn cell_in(&mut self, cell: &Cell, port: &str) -> Result<String, EmitError> {
        match cell.input(port) {
            Some(id) => self.expr(id),
            None => Err(self.err(
                cell.span,
                format!("cell `{}` has no input `{port}`", cell.name),
            )),
        }
    }

    fn cell_out(&self, cell: &Cell, port: &str) -> Result<String, EmitError> {
        match cell.output(port) {
            Some(net) => Ok(ident(self.module.nets[net].name.as_str())),
            None => Err(self.err(
                cell.span,
                format!("cell `{}` has no output `{port}`", cell.name),
            )),
        }
    }

    fn cell_out_width(&self, cell: &Cell, port: &str) -> u32 {
        cell.output(port)
            .and_then(|n| self.module.nets[n].ty.width())
            .unwrap_or(1)
    }

    fn cell(&mut self, cell: &Cell) -> Result<(), EmitError> {
        if self.opts.keep_attrs && !cell.attrs.is_empty() {
            self.out.line(&attrs_text(&cell.attrs));
        }
        let simple = |op: &str| -> Result<String, EmitError> { Ok(op.to_owned()) };
        match &cell.kind {
            CellKind::Not => {
                let a = self.cell_in(cell, "a")?;
                let y = self.cell_out(cell, "y")?;
                self.out
                    .line(&format!("assign {y} = ~{};", paren_unless_atom(&a)));
            }
            CellKind::Buf => {
                let a = self.cell_in(cell, "a")?;
                let y = self.cell_out(cell, "y")?;
                self.out.line(&format!("assign {y} = {a};"));
            }
            CellKind::ReduceAnd | CellKind::ReduceOr | CellKind::ReduceXor => {
                let op = match cell.kind {
                    CellKind::ReduceAnd => "&",
                    CellKind::ReduceOr => "|",
                    _ => "^",
                };
                let a = self.cell_in(cell, "a")?;
                let y = self.cell_out(cell, "y")?;
                self.out
                    .line(&format!("assign {y} = {op}{};", paren_unless_atom(&a)));
            }
            CellKind::And
            | CellKind::Or
            | CellKind::Xor
            | CellKind::Add
            | CellKind::Sub
            | CellKind::Mul
            | CellKind::Div
            | CellKind::Mod
            | CellKind::Shl
            | CellKind::Shr
            | CellKind::Eq
            | CellKind::Ne
            | CellKind::Lt
            | CellKind::Le
            | CellKind::Gt
            | CellKind::Ge => {
                let op = simple(match cell.kind {
                    CellKind::And => "&",
                    CellKind::Or => "|",
                    CellKind::Xor => "^",
                    CellKind::Add => "+",
                    CellKind::Sub => "-",
                    CellKind::Mul => "*",
                    CellKind::Div => "/",
                    CellKind::Mod => "%",
                    CellKind::Shl => "<<",
                    CellKind::Shr => ">>",
                    CellKind::Eq => "==",
                    CellKind::Ne => "!=",
                    CellKind::Lt => "<",
                    CellKind::Le => "<=",
                    CellKind::Gt => ">",
                    _ => ">=",
                })?;
                let a = self.cell_in(cell, "a")?;
                let b = self.cell_in(cell, "b")?;
                let y = self.cell_out(cell, "y")?;
                self.out.line(&format!(
                    "assign {y} = {} {op} {};",
                    paren_unless_atom(&a),
                    paren_unless_atom(&b)
                ));
            }
            CellKind::Sshr => {
                let a = self.cell_in(cell, "a")?;
                let b = self.cell_in(cell, "b")?;
                let y = self.cell_out(cell, "y")?;
                let a_id = cell.input("a").expect("checked");
                let a_text = if self.module.exprs[a_id].ty.is_signed() {
                    paren_unless_atom(&a)
                } else {
                    format!("$signed({a})")
                };
                self.out.line(&format!(
                    "assign {y} = {a_text} >>> {};",
                    paren_unless_atom(&b)
                ));
            }
            CellKind::Mux => {
                let a = self.cell_in(cell, "a")?;
                let b = self.cell_in(cell, "b")?;
                let s = self.cell_in(cell, "s")?;
                let y = self.cell_out(cell, "y")?;
                self.out.line(&format!(
                    "assign {y} = {} ? {} : {};",
                    paren_unless_atom(&s),
                    paren_unless_atom(&b),
                    paren_unless_atom(&a)
                ));
            }
            CellKind::Pmux => {
                let a = self.cell_in(cell, "a")?;
                let b = self.cell_in(cell, "b")?;
                let s = self.cell_in(cell, "s")?;
                let y = self.cell_out(cell, "y")?;
                let w = self.cell_out_width(cell, "y");
                let n = cell
                    .input("s")
                    .and_then(|id| self.module.exprs[id].ty.width())
                    .unwrap_or(1);
                let f = self.pmux_function(w, n);
                self.out.line(&format!("assign {y} = {f}({a}, {b}, {s});"));
            }
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                let clk = self.cell_in(cell, "clk")?;
                let d = self.cell_in(cell, "d")?;
                let q = self.cell_out(cell, "q")?;
                let clk_edge = if *clk_pos { "posedge" } else { "negedge" };
                let mut sens = format!("{clk_edge} {}", paren_unless_atom(&clk));
                let mut body: Vec<String> = Vec::new();
                let mut else_ = "";
                if let Some(reset) = reset {
                    let rst = self.cell_in(cell, "rst")?;
                    if reset.asynchronous {
                        let edge = if reset.active_high {
                            "posedge"
                        } else {
                            "negedge"
                        };
                        let _ = write!(sens, " or {edge} {}", paren_unless_atom(&rst));
                    }
                    let cond = if reset.active_high {
                        rst
                    } else {
                        format!("!{}", paren_unless_atom(&rst))
                    };
                    body.push(format!("if ({cond}) {q} <= {};", const_lit(&reset.value)));
                    else_ = "else ";
                }
                if *has_enable {
                    let en = self.cell_in(cell, "en")?;
                    body.push(format!(
                        "{else_}if ({}) {q} <= {d};",
                        paren_unless_atom(&en)
                    ));
                } else {
                    body.push(format!("{else_}{q} <= {d};"));
                }
                self.out.line(&format!("always @({sens}) begin"));
                self.out.indent();
                for line in body {
                    self.out.line(&line);
                }
                self.out.dedent();
                self.out.line("end");
            }
            CellKind::Dlatch => {
                let en = self.cell_in(cell, "en")?;
                let d = self.cell_in(cell, "d")?;
                let q = self.cell_out(cell, "q")?;
                self.out.line("always @* begin");
                self.out.indent();
                self.out
                    .line(&format!("if ({}) {q} = {d};", paren_unless_atom(&en)));
                self.out.dedent();
                self.out.line("end");
            }
            CellKind::MemRdPort { mem, clocked } => {
                let name = ident(self.module.memories[*mem].name.as_str());
                let addr = self.cell_in(cell, "addr")?;
                let data = self.cell_out(cell, "data")?;
                if *clocked {
                    let clk = self.cell_in(cell, "clk")?;
                    let en = self.cell_in(cell, "en")?;
                    self.out.line(&format!(
                        "always @(posedge {}) begin",
                        paren_unless_atom(&clk)
                    ));
                    self.out.indent();
                    self.out.line(&format!(
                        "if ({}) {data} <= {name}[{addr}];",
                        paren_unless_atom(&en)
                    ));
                    self.out.dedent();
                    self.out.line("end");
                } else {
                    self.out.line(&format!("assign {data} = {name}[{addr}];"));
                }
            }
            CellKind::MemWrPort { mem, clocked } => {
                let name = ident(self.module.memories[*mem].name.as_str());
                let addr = self.cell_in(cell, "addr")?;
                let data = self.cell_in(cell, "data")?;
                let en = self.cell_in(cell, "en")?;
                if *clocked {
                    let clk = self.cell_in(cell, "clk")?;
                    self.out.line(&format!(
                        "always @(posedge {}) begin",
                        paren_unless_atom(&clk)
                    ));
                    self.out.indent();
                    self.out.line(&format!(
                        "if ({}) {name}[{addr}] <= {data};",
                        paren_unless_atom(&en)
                    ));
                } else {
                    self.out.line("always @* begin");
                    self.out.indent();
                    self.out.line(&format!(
                        "if ({}) {name}[{addr}] = {data};",
                        paren_unless_atom(&en)
                    ));
                }
                self.out.dedent();
                self.out.line("end");
            }
            CellKind::Lut { init, .. } => {
                let a = self.cell_in(cell, "a")?;
                let y = self.cell_out(cell, "y")?;
                let param = ident(&format!("{}_INIT", cell.name));
                self.out.line(&format!(
                    "localparam [{}:0] {param} = {};",
                    init.width().saturating_sub(1),
                    const_lit(init)
                ));
                self.out.line(&format!("assign {y} = {param}[{a}];"));
            }
            CellKind::Tristate => {
                let a = self.cell_in(cell, "a")?;
                let en = self.cell_in(cell, "en")?;
                let y = self.cell_out(cell, "y")?;
                let w = self.cell_out_width(cell, "y");
                let z = if w == 1 {
                    "1'bz".to_owned()
                } else {
                    format!("{{{w}{{1'bz}}}}")
                };
                self.out.line(&format!(
                    "assign {y} = {} ? {} : {z};",
                    paren_unless_atom(&en),
                    paren_unless_atom(&a)
                ));
            }
            CellKind::Blackbox(target) => {
                let mut conns: Vec<(String, String)> = Vec::new();
                for (port, e) in &cell.inputs {
                    conns.push((port.as_str().to_owned(), self.expr(*e)?));
                }
                for (port, net) in &cell.outputs {
                    conns.push((
                        port.as_str().to_owned(),
                        ident(self.module.nets[*net].name.as_str()),
                    ));
                }
                let attrs = Attrs::new();
                self.instantiation(
                    target.as_str(),
                    cell.name.as_str(),
                    &cell.params,
                    &attrs,
                    &conns,
                );
            }
        }
        Ok(())
    }

    fn pmux_function(&mut self, w: u32, n: u32) -> String {
        let name = format!("reticle_pmux_{w}_{n}");
        if !self.functions.contains_key(&name) {
            let mut f = Out::new("  ");
            f.indent();
            f.line(&format!("function [{}:0] {name};", w.saturating_sub(1)));
            f.indent();
            f.line(&format!("input [{}:0] a;", w.saturating_sub(1)));
            f.line(&format!("input [{}:0] b;", u64::from(w) * u64::from(n) - 1));
            f.line(&format!("input [{}:0] s;", n.saturating_sub(1)));
            f.line("integer i;");
            f.line("begin");
            f.indent();
            f.line(&format!("{name} = a;"));
            f.line(&format!("for (i = 0; i < {n}; i = i + 1)"));
            f.indent();
            f.line(&format!("if (s[i]) {name} = b[i * {w} +: {w}];"));
            f.dedent();
            f.dedent();
            f.line("end");
            f.dedent();
            f.line("endfunction");
            self.functions.insert(name.clone(), f.finish());
        }
        name
    }

    /// A one-input helper function `name` of return width `ret` taking
    /// `x` of width `w`, whose body assigns `body` to the result.
    fn function1(&mut self, name: &str, ret: u32, signed: bool, w: u32, body: &str) {
        if self.functions.contains_key(name) {
            return;
        }
        let mut f = Out::new("  ");
        f.indent();
        let s = if signed { " signed" } else { "" };
        f.line(&format!(
            "function{s} [{}:0] {name};",
            ret.saturating_sub(1)
        ));
        f.indent();
        f.line(&format!("input [{}:0] x;", w.saturating_sub(1)));
        f.line(&format!("{name} = {body};"));
        f.dedent();
        f.line("endfunction");
        self.functions.insert(name.to_owned(), f.finish());
    }

    fn function2(&mut self, name: &str, ret: u32, w: u32, iw: u32, body: &str) {
        if self.functions.contains_key(name) {
            return;
        }
        let mut f = Out::new("  ");
        f.indent();
        f.line(&format!("function [{}:0] {name};", ret.saturating_sub(1)));
        f.indent();
        f.line(&format!("input [{}:0] x;", w.saturating_sub(1)));
        f.line(&format!("input [{}:0] i;", iw.saturating_sub(1)));
        f.line(&format!("{name} = {body};"));
        f.dedent();
        f.line("endfunction");
        self.functions.insert(name.to_owned(), f.finish());
    }

    // --- expressions --------------------------------------------------------

    fn lvalue(&mut self, lv: &Lvalue) -> Result<String, EmitError> {
        Ok(match lv {
            Lvalue::Net(net) => ident(self.module.nets[*net].name.as_str()),
            Lvalue::Slice { net, hi, lo } if hi == lo => {
                format!("{}[{lo}]", ident(self.module.nets[*net].name.as_str()))
            }
            Lvalue::Slice { net, hi, lo } => {
                format!("{}[{hi}:{lo}]", ident(self.module.nets[*net].name.as_str()))
            }
            Lvalue::Index { net, index } => {
                let index = self.expr(*index)?;
                format!("{}[{index}]", ident(self.module.nets[*net].name.as_str()))
            }
            Lvalue::Concat(parts) => {
                let parts: Vec<String> = parts
                    .iter()
                    .map(|p| self.lvalue(p))
                    .collect::<Result<_, _>>()?;
                format!("{{{}}}", parts.join(", "))
            }
            Lvalue::MemElem { mem, addr } => {
                let addr = self.expr(*addr)?;
                format!(
                    "{}[{addr}]",
                    ident(self.module.memories[*mem].name.as_str())
                )
            }
        })
    }

    /// The precedence of the rendering of `id`.
    fn prec(&self, id: ExprId) -> u8 {
        match &self.module.exprs[id].kind {
            ExprKind::Unary { .. } => PREC_UNARY,
            ExprKind::Binary { op, .. } => match op {
                BinaryOp::Sshr if !self.module.exprs[id].ty.is_signed() => PREC_ATOM,
                _ => binary_op(*op).1,
            },
            ExprKind::Ternary { .. } => PREC_TERNARY,
            _ => PREC_ATOM,
        }
    }

    /// Renders an operand, parenthesised when its precedence differs from
    /// `parent` or when it is a right operand of equal precedence.
    fn operand(&mut self, id: ExprId, parent: u8, right: bool) -> Result<String, EmitError> {
        let text = self.expr(id)?;
        let p = self.prec(id);
        if p == PREC_ATOM || (p == parent && !right) {
            Ok(text)
        } else {
            Ok(format!("({text})"))
        }
    }

    /// `name[...]` access to a bit-vector expression that is a net, a
    /// constant slice of one, or an element of an array net: the name and
    /// the bit offset of the expression's bit 0 inside it.
    fn select_base(&mut self, id: ExprId) -> Result<Option<(String, u32)>, EmitError> {
        let expr = &self.module.exprs[id];
        Ok(match &expr.kind {
            ExprKind::Net(net) => {
                let net = &self.module.nets[*net];
                if net.ty.is_bits() || net.ty == Type::Integer {
                    Some((ident(net.name.as_str()), 0))
                } else {
                    None
                }
            }
            ExprKind::Slice { base, lo, .. } if self.module.exprs[*base].ty.is_bits() => {
                let lo = *lo;
                self.select_base(*base)?.map(|(name, off)| (name, off + lo))
            }
            ExprKind::Index { base, index } => {
                let base_ty = &self.module.exprs[*base].ty;
                match base_ty {
                    Type::Array { elem, .. } if elem.is_bits() => {
                        let index = *index;
                        let name = self.array_name(*base)?;
                        let idx = self.expr(index)?;
                        Some((format!("{name}[{idx}]"), 0))
                    }
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// The name of an array-typed expression: a net or an element of a
    /// higher-dimensional array net.
    fn array_name(&mut self, id: ExprId) -> Result<String, EmitError> {
        let expr = &self.module.exprs[id];
        match &expr.kind {
            ExprKind::Net(net) => Ok(ident(self.module.nets[*net].name.as_str())),
            ExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                let name = self.array_name(base)?;
                let idx = self.expr(index)?;
                Ok(format!("{name}[{idx}]"))
            }
            _ => Err(self.err(expr.span, "Verilog can only select elements of array nets")),
        }
    }

    fn expr(&mut self, id: ExprId) -> Result<String, EmitError> {
        let expr = &self.module.exprs[id];
        let span = expr.span;
        match &expr.kind {
            ExprKind::Const(c) => Ok(const_lit(c)),
            ExprKind::String(s) => Ok(verilog_string(s)),
            ExprKind::Net(net) => Ok(ident(self.module.nets[*net].name.as_str())),
            ExprKind::Slice { base, hi, lo } => {
                let (base, hi, lo) = (*base, *hi, *lo);
                let base_ty = self.module.exprs[base].ty.clone();
                if !base_ty.is_bits() {
                    let name = self.array_name(base)?;
                    return Ok(format!("{name}[{hi}:{lo}]"));
                }
                if let Some((name, off)) = self.select_base(base)? {
                    return Ok(if hi == lo {
                        format!("{name}[{}]", off + lo)
                    } else {
                        format!("{name}[{}:{}]", off + hi, off + lo)
                    });
                }
                let w = base_ty.width().unwrap_or(1);
                let name = format!("reticle_bits_{w}_{hi}_{lo}");
                self.function1(&name, hi - lo + 1, false, w, &format!("x[{hi}:{lo}]"));
                let inner = self.expr(base)?;
                Ok(format!("{name}({inner})"))
            }
            ExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                let base_ty = self.module.exprs[base].ty.clone();
                if !base_ty.is_bits() {
                    let name = self.array_name(base)?;
                    let idx = self.expr(index)?;
                    return Ok(format!("{name}[{idx}]"));
                }
                let idx = self.expr(index)?;
                if let Some((name, off)) = self.select_base(base)? {
                    return Ok(if off == 0 {
                        format!("{name}[{idx}]")
                    } else {
                        format!("{name}[{off} + {idx}]")
                    });
                }
                let w = base_ty.width().unwrap_or(1);
                let iw = self.module.exprs[index].ty.width().unwrap_or(32);
                let name = format!("reticle_bit_{w}_{iw}");
                self.function2(&name, 1, w, iw, "x[i]");
                let inner = self.expr(base)?;
                Ok(format!("{name}({inner}, {idx})"))
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => {
                let (base, offset, width, up) = (*base, *offset, *width, *up);
                let op = if up { "+:" } else { "-:" };
                let off = self.expr(offset)?;
                if let Some((name, base_off)) = self.select_base(base)? {
                    return Ok(if base_off == 0 {
                        format!("{name}[{off} {op} {width}]")
                    } else {
                        format!("{name}[{base_off} + {off} {op} {width}]")
                    });
                }
                let w = self.module.exprs[base].ty.width().unwrap_or(1);
                let iw = self.module.exprs[offset].ty.width().unwrap_or(32);
                let kind = if up { "pslice" } else { "mslice" };
                let name = format!("reticle_{kind}_{w}_{iw}_{width}");
                self.function2(&name, width, w, iw, &format!("x[i {op} {width}]"));
                let inner = self.expr(base)?;
                Ok(format!("{name}({inner}, {off})"))
            }
            ExprKind::Concat(parts) => {
                let parts: Vec<String> = parts
                    .clone()
                    .into_iter()
                    .map(|p| self.expr(p))
                    .collect::<Result<_, _>>()?;
                Ok(format!("{{{}}}", parts.join(", ")))
            }
            ExprKind::Replicate { count, expr } => {
                let (count, expr) = (*count, *expr);
                let inner = self.expr(expr)?;
                Ok(format!("{{{count}{{{inner}}}}}"))
            }
            ExprKind::Unary { op, expr } => {
                let (op, expr) = (*op, *expr);
                let inner = self.operand(expr, PREC_UNARY, true)?;
                Ok(format!("{}{inner}", unary_op(op)))
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let (op, lhs, rhs) = (*op, *lhs, *rhs);
                let (text, prec) = binary_op(op);
                let l = self.operand(lhs, prec, false)?;
                let r = self.operand(rhs, prec, true)?;
                if op == BinaryOp::WildEq
                    && let Some(pattern) = self.module.exprs[rhs].as_const().cloned()
                {
                    // `==?` is SystemVerilog; against a constant pattern the
                    // wildcard compare is an equality on the known bits.
                    let known: Vec<Bit> = pattern
                        .bits()
                        .iter()
                        .map(|b| Bit::from_bool(b.is_known()))
                        .collect();
                    let mask = Const::from_bits(&known).with_signed(pattern.is_signed());
                    let value = pattern.and(&mask).with_signed(pattern.is_signed());
                    let l = self.operand(lhs, PREC_BAND, false)?;
                    return Ok(format!(
                        "({l} & {}) == {}",
                        const_lit(&mask),
                        const_lit(&value)
                    ));
                }
                if op == BinaryOp::Sshr && !self.module.exprs[lhs].ty.is_signed() {
                    // Arithmetic shift of an unsigned value: Verilog only sign
                    // fills a signed left operand, so cast and wrap back into
                    // an unsigned, self-determined concatenation.
                    let l = self.expr(lhs)?;
                    return Ok(format!("{{$signed({l}) >>> {r}}}"));
                }
                Ok(format!("{l} {text} {r}"))
            }
            ExprKind::Ternary { cond, then_, else_ } => {
                let (cond, then_, else_) = (*cond, *then_, *else_);
                let c = self.operand(cond, PREC_TERNARY, true)?;
                let t = self.operand(then_, PREC_TERNARY, true)?;
                let e = self.operand(else_, PREC_TERNARY, true)?;
                Ok(format!("{c} ? {t} : {e}"))
            }
            ExprKind::Resize {
                expr,
                width,
                signed,
            } => {
                let (expr, width, signed) = (*expr, *width, *signed);
                self.resize(expr, width, signed, span)
            }
            ExprKind::MemRead { mem, addr } => {
                let (mem, addr) = (*mem, *addr);
                let name = ident(self.module.memories[mem].name.as_str());
                let addr = self.expr(addr)?;
                Ok(format!("{name}[{addr}]"))
            }
            ExprKind::Call { name, args } => {
                let args: Vec<String> = args
                    .clone()
                    .into_iter()
                    .map(|a| self.expr(a))
                    .collect::<Result<_, _>>()?;
                let name = if name.as_str().starts_with('$') {
                    name.as_str().to_owned()
                } else {
                    ident(name.as_str())
                };
                Ok(format!("{name}({})", args.join(", ")))
            }
        }
    }

    fn resize(
        &mut self,
        inner: ExprId,
        width: u32,
        signed: bool,
        span: Span,
    ) -> Result<String, EmitError> {
        let operand = &self.module.exprs[inner];
        let (w, operand_signed) = match &operand.ty {
            Type::Bits { width, signed } => (*width, *signed),
            Type::Integer => (32, true),
            other => {
                return Err(self.err(
                    span,
                    format!("cannot resize a value of type `{other}` in Verilog"),
                ));
            }
        };
        let sext = signed && operand_signed;
        // (text, natural signedness of the text); every form but the
        // same-width one is self-determined (a select, concatenation or
        // call), so only that one may need `{}` to pin its width.
        let same_width = width == w;
        let (text, natural) = if same_width {
            (self.expr(inner)?, operand_signed)
        } else if width > w {
            let n = width - w;
            let inner_text = self.expr(inner)?;
            if sext {
                match self.select_base(inner)? {
                    Some((name, off)) => (
                        format!(
                            "{{{{{n}{{{name}[{}]}}}}, {inner_text}}}",
                            off + w.saturating_sub(1)
                        ),
                        false,
                    ),
                    None => {
                        let name = format!("reticle_sext_{w}_{width}");
                        self.function1(
                            &name,
                            width,
                            true,
                            w,
                            &format!("{{{{{n}{{x[{}]}}}}, x}}", w.saturating_sub(1)),
                        );
                        (format!("{name}({inner_text})"), true)
                    }
                }
            } else {
                (format!("{{{{{n}{{1'b0}}}}, {inner_text}}}"), false)
            }
        } else {
            match self.select_base(inner)? {
                Some((name, off)) => (
                    if width == 1 {
                        format!("{name}[{off}]")
                    } else {
                        format!("{name}[{}:{off}]", off + width.saturating_sub(1))
                    },
                    false,
                ),
                None => {
                    let name = format!("reticle_trunc_{w}_{width}");
                    self.function1(
                        &name,
                        width,
                        false,
                        w,
                        &format!("x[{}:0]", width.saturating_sub(1)),
                    );
                    let inner_text = self.expr(inner)?;
                    (format!("{name}({inner_text})"), false)
                }
            }
        };
        if natural == signed {
            return Ok(text);
        }
        let atom = !same_width || self.prec(inner) == PREC_ATOM;
        if signed {
            // `{text}` keeps the width self-determined inside `$signed`.
            Ok(if atom {
                format!("$signed({text})")
            } else {
                format!("$signed({{{text}}})")
            })
        } else if atom {
            Ok(format!("$unsigned({text})"))
        } else {
            Ok(format!("{{{text}}}"))
        }
    }
}

/// Wraps `text` in parentheses unless it is a single identifier, literal,
/// concatenation or call.
fn paren_unless_atom(text: &str) -> String {
    let simple = text.starts_with('\\')
        || text.starts_with('{')
        || text.starts_with('$')
        || text.starts_with('"')
        || text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '[' | ']' | ':'));
    if simple || (text.ends_with(')') && !text.contains(' ')) {
        text.to_owned()
    } else {
        format!("({text})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::process::{AssignKind, TimeUnit};
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// Renders one expression through a module with nets `a`, `b`, `c`
    /// (unsigned 8-bit) and `sa`, `sb` (signed 8-bit).
    fn render(build: impl FnOnce(&mut ModuleBuilder, [ExprId; 5]) -> ExprId) -> String {
        let span = span();
        let mut b = ModuleBuilder::new("t", span);
        let a = b.input("a", Type::bits(8));
        let bb = b.input("b", Type::bits(8));
        let c = b.input("c", Type::bits(8));
        let sa = b.input("sa", Type::sbits(8));
        let sb = b.input("sb", Type::sbits(8));
        let nets = [b.net(a), b.net(bb), b.net(c), b.net(sa), b.net(sb)];
        let e = build(&mut b, nets);
        let module = b.finish();
        let mut design = Design::new();
        design.add_module(module);
        let opts = VerilogOptions::default();
        let module = design.module(ModuleId(0));
        let mut p = Printer::new(&design, module, &opts);
        p.expr(e).unwrap()
    }

    #[test]
    fn precedence_and_parentheses() {
        assert_eq!(
            render(|b, [a, bb, c, ..]| {
                let s = b.add(a, bb);
                b.add(s, c)
            }),
            "a + b + c"
        );
        assert_eq!(
            render(|b, [a, bb, c, ..]| {
                let s = b.add(bb, c);
                b.sub(a, s)
            }),
            "a - (b + c)"
        );
        assert_eq!(
            render(|b, [a, bb, c, ..]| {
                let m = b.mul(bb, c);
                b.add(a, m)
            }),
            "a + (b * c)"
        );
        assert_eq!(
            render(|b, [a, bb, c, ..]| {
                let s = b.add(a, bb);
                b.and(s, c)
            }),
            "(a + b) & c"
        );
        assert_eq!(
            render(|b, [a, bb, ..]| {
                let s = b.add(a, bb);
                b.not(s)
            }),
            "~(a + b)"
        );
        assert_eq!(
            render(|b, [a, ..]| {
                let n = b.neg(a);
                b.neg(n)
            }),
            "-(-a)"
        );
        assert_eq!(
            render(|b, [a, bb, c, ..]| {
                let e = b.eq(a, bb);
                b.mux(e, bb, c)
            }),
            "(a == b) ? b : c"
        );
        assert_eq!(
            render(|b, [a, bb, ..]| {
                let x = b.const_u64(8, 3);
                let s = b.binary(BinaryOp::Sshr, a, x);
                b.or(s, bb)
            }),
            "{$signed(a) >>> 8'h03} | b"
        );
        assert_eq!(
            render(|b, [_, _, _, sa, sb]| { b.binary(BinaryOp::Sshr, sa, sb) }),
            "sa >>> sb"
        );
    }

    #[test]
    fn resize_forms() {
        assert_eq!(render(|b, [a, ..]| b.zext(a, 12)), "{{4{1'b0}}, a}");
        assert_eq!(
            render(|b, [_, _, _, sa, _]| b.sext(sa, 12)),
            "$signed({{4{sa[7]}}, sa})"
        );
        assert_eq!(render(|b, [a, ..]| b.zext(a, 4)), "a[3:0]");
        assert_eq!(render(|b, [a, ..]| b.sext(a, 8)), "$signed(a)");
        assert_eq!(render(|b, [_, _, _, sa, _]| b.zext(sa, 8)), "$unsigned(sa)");
        assert_eq!(
            render(|b, [a, bb, ..]| {
                let s = b.add(a, bb);
                b.zext(s, 16)
            }),
            "{{8{1'b0}}, a + b}"
        );
        assert_eq!(
            render(|b, [_, _, _, sa, sb]| {
                let s = b.add(sa, sb);
                b.sext(s, 16)
            }),
            "reticle_sext_8_16(sa + sb)"
        );
        assert_eq!(
            render(|b, [a, bb, ..]| {
                let s = b.add(a, bb);
                b.zext(s, 4)
            }),
            "reticle_trunc_8_4(a + b)"
        );
        assert_eq!(
            render(|b, [a, ..]| {
                let s = b.slice(a, 6, 2);
                b.zext(s, 3)
            }),
            "a[4:2]"
        );
        assert_eq!(
            render(|b, [a, ..]| {
                let s = b.slice(a, 6, 2);
                b.sext(s, 3)
            }),
            "$signed(a[4:2])"
        );
    }

    #[test]
    fn selects_of_expressions_use_functions() {
        assert_eq!(
            render(|b, [a, bb, ..]| {
                let s = b.add(a, bb);
                b.slice(s, 3, 1)
            }),
            "reticle_bits_8_3_1(a + b)"
        );
        assert_eq!(
            render(|b, [a, bb, c, ..]| {
                let s = b.add(a, bb);
                b.index(s, c)
            }),
            "reticle_bit_8_8(a + b, c)"
        );
        assert_eq!(
            render(|b, [a, bb, c, ..]| {
                let s = b.slice(a, 7, 4);
                let s = b.slice(s, 2, 1);
                let i = b.index(s, c);
                let p = b.indexed_slice(a, bb, 2, true);
                b.concat(vec![i, p])
            }),
            "{a[5 + c], a[b +: 2]}"
        );
        assert_eq!(
            render(|b, [a, bb, ..]| {
                let s = b.add(a, a);
                b.indexed_slice(s, bb, 2, false)
            }),
            "reticle_mslice_8_8_2(a + a, b)"
        );
    }

    #[test]
    fn delays_and_helpers() {
        assert_eq!(delay_text(3_000_000, 1_000_000), "3");
        assert_eq!(delay_text(1_500_000, 1_000_000), "1.5");
        assert_eq!(delay_text(1, 1_000_000), "0.000001");
        assert_eq!(paren_unless_atom("a"), "a");
        assert_eq!(paren_unless_atom("a[3:0]"), "a[3:0]");
        assert_eq!(paren_unless_atom("8'h0f"), "8'h0f");
        assert_eq!(paren_unless_atom("{a, b}"), "{a, b}");
        assert_eq!(paren_unless_atom("a + b"), "(a + b)");
        assert_eq!(paren_unless_atom("f(a)"), "f(a)");
        assert_eq!(paren_unless_atom("\\a b "), "\\a b ");
        assert_eq!(ident("reticle_x"), "\\reticle_x ");
        assert_eq!(ident("x"), "x");
        assert_eq!(const_lit(&Const::zero(0)), "1'b0");
    }

    #[test]
    fn structural_only_rejects_processes() {
        let span = span();
        let mut b = ModuleBuilder::new("t", span);
        let clk = b.input("clk", Type::bit());
        let q = b.output_reg("q", Type::bit());
        let one = b.const_bit(true);
        let mut p = b.process(None, ProcessKind::posedge(clk));
        p.assign(q, one, AssignKind::NonBlocking);
        b.end_process(p);
        let mut design = Design::new();
        design.add_module(b.finish());
        let opts = VerilogOptions {
            structural_only: true,
            ..VerilogOptions::default()
        };
        assert!(emit_verilog_with(&design, &opts).is_err());
        let text = emit_verilog(&design).unwrap();
        assert!(text.contains("always @(posedge clk) begin"));
        assert!(text.contains("output reg q"));
    }

    #[test]
    fn non_ansi_and_port_expressions() {
        let span = span();
        let mut b = ModuleBuilder::new("t", span);
        let a = b.add_net("a_int", Type::bits(2));
        b.add_port("a", PortDir::In, a);
        let y = b.output("y", Type::sbits(2));
        let av = b.net(a);
        let s = b.resize(av, 2, true);
        b.assign_after(y, s, Some(Delay::new(500, TimeUnit::Ps)));
        let mut design = Design::new();
        design.add_module(b.finish());
        let text = emit_verilog(&design).unwrap();
        assert!(text.contains("module t (.a(a_int), y);"));
        assert!(text.contains("input [1:0] a_int;"));
        assert!(text.contains("output signed [1:0] y;"));
        assert!(text.contains("`timescale 1ps / 1ps"));
        assert!(text.contains("assign #500 y = $signed(a_int);"));
    }
}
