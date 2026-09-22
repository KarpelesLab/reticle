//! Structural validation of a design.
//!
//! [`validate`] checks the invariants every consumer of the IR relies on
//! and reports each violation as an error diagnostic with a stable code, so
//! a pass can assert a clean design in tests and a frontend bug shows up
//! with a span instead of a panic three stages later. [`validate_module`]
//! runs the per-module subset that needs no knowledge of other modules.
//!
//! | Code    | Check                                                                 |
//! |---------|-----------------------------------------------------------------------|
//! | `I0001` | Duplicate name among modules, or among a module's nets, memories, instances, cells, named processes, ports or parameters |
//! | `I0002` | A port refers to a net that does not exist                            |
//! | `I0003` | A reference to an expression, net or memory that does not exist       |
//! | `I0004` | An expression operand has a type its operator does not accept         |
//! | `I0005` | A constant slice lies outside its operand                             |
//! | `I0006` | An expression's cached type differs from the inferred one             |
//! | `I0007` | An instance refers to a module id that is not in the design           |
//! | `I0008` | An instance connects a port the target module does not have           |
//! | `I0009` | An instance connects an output or inout port to something that is not a net, slice or concatenation of nets |
//! | `I0010` | An instance connection's width differs from the target port's         |
//! | `I0011` | A net has more than one whole-net driver (assigns, cell outputs, instance outputs) |
//! | `I0012` | A constant memory address is out of range, or an initialiser is longer than the memory |
//! | `I0013` | A memory access's data type differs from the element type             |
//! | `I0014` | The widths of an assignment target and value, or of a case subject and item, differ |
//! | `I0015` | A cell is missing a required port or has one its kind does not define |
//! | `I0016` | A cell port's width violates the kind's rule                          |
//! | `I0017` | A sequential process has no clock                                     |
//! | `I0018` | A condition, enable, clock or edge net is not a single bit            |
//! | `I0019` | The top module id is not in the design                                |
//! | `I0020` | A cell parameter is inconsistent (LUT table size, reset value width)  |

use std::collections::HashMap;

use super::cell::{Cell, CellKind};
use super::design::{Design, Instance, Module, ModuleRef, PortDir};
use super::expr::{ExprId, ExprKind, TypeError, infer_type};
use super::process::{Edge, Lvalue, Polarity, ProcessKind, Stmt, StmtKind, WaitKind};
use super::types::Type;
use super::walk::walk_block;
use crate::diag::{Diagnostic, Diagnostics};
use crate::source::Span;

/// Checks every module of `design` and the design-level invariants.
pub fn validate(design: &Design) -> Diagnostics {
    let mut diags = Diagnostics::new();
    if let Some(top) = design.top
        && !design.modules.contains(top)
    {
        diags.push(
            Diagnostic::error(format!("top module {top} does not exist"))
                .with_code("I0019")
                .with_note("the design has no such module id"),
        );
    }
    let mut seen: HashMap<&str, Span> = HashMap::new();
    for (_, module) in design.modules.iter() {
        if let Some(first) = seen.insert(module.name.as_str(), module.span) {
            diags.push(
                Diagnostic::error(format!("duplicate module name `{}`", module.name))
                    .with_code("I0001")
                    .with_label(module.span, "second definition")
                    .with_secondary(first, "first definition"),
            );
        }
    }
    for (_, module) in design.modules.iter() {
        let mut checker = Checker::new(module, Some(design));
        checker.run();
        diags.append(&mut checker.diags);
    }
    diags
}

/// Checks one module in isolation; instance targets are not verified.
pub fn validate_module(module: &Module) -> Diagnostics {
    let mut checker = Checker::new(module, None);
    checker.run();
    checker.diags
}

/// Per-module checking state.
struct Checker<'a> {
    module: &'a Module,
    design: Option<&'a Design>,
    diags: Diagnostics,
}

impl<'a> Checker<'a> {
    fn new(module: &'a Module, design: Option<&'a Design>) -> Self {
        Checker {
            module,
            design,
            diags: Diagnostics::new(),
        }
    }

    fn error(&mut self, code: &'static str, span: Span, message: String) {
        self.diags.push(
            Diagnostic::error(format!("{message} in module `{}`", self.module.name))
                .with_code(code)
                .with_span(span),
        );
    }

    fn run(&mut self) {
        self.check_names();
        self.check_ports();
        self.check_memories();
        self.check_exprs();
        self.check_assigns();
        self.check_processes();
        self.check_cells();
        self.check_instances();
        self.check_drivers();
    }

    fn check_duplicates<'b>(&mut self, what: &str, items: impl Iterator<Item = (&'b str, Span)>) {
        let mut seen: HashMap<&'b str, Span> = HashMap::new();
        for (name, span) in items {
            if let Some(first) = seen.insert(name, span) {
                self.diags.push(
                    Diagnostic::error(format!(
                        "duplicate {what} name `{name}` in module `{}`",
                        self.module.name
                    ))
                    .with_code("I0001")
                    .with_label(span, "second definition")
                    .with_secondary(first, "first definition"),
                );
            }
        }
    }

    fn check_names(&mut self) {
        let m = self.module;
        self.check_duplicates("net", m.nets.values().map(|n| (n.name.as_str(), n.span)));
        self.check_duplicates(
            "memory",
            m.memories.values().map(|n| (n.name.as_str(), n.span)),
        );
        self.check_duplicates(
            "instance",
            m.instances.values().map(|n| (n.name.as_str(), n.span)),
        );
        self.check_duplicates("cell", m.cells.values().map(|n| (n.name.as_str(), n.span)));
        self.check_duplicates(
            "process",
            m.processes
                .values()
                .filter_map(|p| p.name.as_ref().map(|n| (n.as_str(), p.span))),
        );
        self.check_duplicates("port", m.ports.iter().map(|p| (p.name.as_str(), p.span)));
        self.check_duplicates(
            "parameter",
            m.params.iter().map(|p| (p.name.as_str(), p.span)),
        );
    }

    fn check_ports(&mut self) {
        for port in &self.module.ports {
            if !self.module.nets.contains(port.net) {
                self.error(
                    "I0002",
                    port.span,
                    format!("port `{}` refers to missing net {}", port.name, port.net),
                );
            }
        }
    }

    fn check_memories(&mut self) {
        for (_, mem) in self.module.memories.iter() {
            if let Some(init) = &mem.init {
                let len = u64::try_from(init.len()).unwrap_or(u64::MAX);
                if len > mem.size {
                    self.error(
                        "I0012",
                        mem.span,
                        format!(
                            "memory `{}` has {len} initial values but {} elements",
                            mem.name, mem.size
                        ),
                    );
                }
                for c in init {
                    if c.ty().width() != mem.elem.width() {
                        self.error(
                            "I0013",
                            mem.span,
                            format!(
                                "memory `{}` initial value `{c}` does not match element type `{}`",
                                mem.name, mem.elem
                            ),
                        );
                        break;
                    }
                }
            }
        }
    }

    /// True when `id` is a valid expression; reports `I0003` otherwise.
    fn expr_exists(&mut self, id: ExprId, span: Span) -> bool {
        if self.module.exprs.contains(id) {
            true
        } else {
            self.error(
                "I0003",
                span,
                format!("reference to missing expression {id}"),
            );
            false
        }
    }

    fn check_exprs(&mut self) {
        for (id, expr) in self.module.exprs.iter() {
            match infer_type(self.module, &expr.kind) {
                Ok(ty) => {
                    if ty != expr.ty {
                        self.error(
                            "I0006",
                            expr.span,
                            format!(
                                "expression {id} is typed `{}` but its operands give `{ty}`",
                                expr.ty
                            ),
                        );
                    }
                }
                Err(TypeError::NoRule) => {}
                Err(
                    e @ (TypeError::DanglingExpr(_)
                    | TypeError::DanglingNet(_)
                    | TypeError::DanglingMemory(_)),
                ) => self.error("I0003", expr.span, format!("expression {id}: {e}")),
                Err(e @ TypeError::OutOfRange { .. }) => {
                    self.error("I0005", expr.span, format!("expression {id}: {e}"));
                }
                Err(e @ (TypeError::BadOperand { .. } | TypeError::Mismatch { .. })) => {
                    self.error("I0004", expr.span, format!("expression {id}: {e}"));
                }
            }
            if let ExprKind::MemRead { mem, addr } = &expr.kind {
                self.check_mem_addr(*mem, *addr, expr.span);
            }
        }
    }

    /// Reports `I0012` when `addr` is a constant outside memory `mem`.
    fn check_mem_addr(&mut self, mem: super::design::MemoryId, addr: ExprId, span: Span) {
        let Some(memory) = self.module.memories.get(mem) else {
            return;
        };
        if let Some(a) = self
            .module
            .exprs
            .get(addr)
            .and_then(|e| e.as_const())
            .and_then(|c| c.to_u64())
            && a >= memory.size
        {
            self.error(
                "I0012",
                span,
                format!(
                    "address {a} is outside memory `{}` of {} elements",
                    memory.name, memory.size
                ),
            );
        }
    }

    /// The type an lvalue receives, reporting dangling ids and bad slices.
    fn lvalue_type(&mut self, lv: &Lvalue, span: Span) -> Option<Type> {
        match lv {
            Lvalue::Net(net) => match self.module.nets.get(*net) {
                Some(n) => Some(n.ty.clone()),
                None => {
                    self.error("I0003", span, format!("assignment to missing net {net}"));
                    None
                }
            },
            Lvalue::Slice { net, hi, lo } => {
                let Some(n) = self.module.nets.get(*net) else {
                    self.error("I0003", span, format!("assignment to missing net {net}"));
                    return None;
                };
                match &n.ty {
                    Type::Bits { width, .. } if hi >= lo && hi < width => {
                        Some(Type::bits(hi - lo + 1))
                    }
                    Type::Array { elem, len } if hi >= lo && u64::from(*hi) < *len => {
                        Some(Type::array((**elem).clone(), u64::from(hi - lo) + 1))
                    }
                    ty => {
                        self.error(
                            "I0005",
                            span,
                            format!(
                                "slice [{hi}:{lo}] is outside net `{}` of type `{ty}`",
                                n.name
                            ),
                        );
                        None
                    }
                }
            }
            Lvalue::Index { net, index } => {
                self.expr_exists(*index, span);
                let Some(n) = self.module.nets.get(*net) else {
                    self.error("I0003", span, format!("assignment to missing net {net}"));
                    return None;
                };
                match &n.ty {
                    Type::Bits { .. } => Some(Type::bit()),
                    Type::Array { elem, .. } => Some((**elem).clone()),
                    ty => {
                        self.error(
                            "I0004",
                            span,
                            format!("net `{}` of type `{ty}` cannot be indexed", n.name),
                        );
                        None
                    }
                }
            }
            Lvalue::Concat(parts) => {
                let mut total = 0u32;
                for part in parts {
                    let ty = self.lvalue_type(part, span)?;
                    match ty.width() {
                        Some(w) => total = total.saturating_add(w),
                        None => {
                            self.error(
                                "I0004",
                                span,
                                format!("concatenated target has non-vector type `{ty}`"),
                            );
                            return None;
                        }
                    }
                }
                Some(Type::bits(total))
            }
            Lvalue::MemElem { mem, addr } => {
                self.expr_exists(*addr, span);
                self.check_mem_addr(*mem, *addr, span);
                match self.module.memories.get(*mem) {
                    Some(m) => Some(m.elem.clone()),
                    None => {
                        self.error("I0003", span, format!("write to missing memory {mem}"));
                        None
                    }
                }
            }
        }
    }

    /// Reports `I0014` when the widths of `target` and `value` differ.
    fn check_assignment(&mut self, target: &Lvalue, value: ExprId, span: Span) {
        let target_ty = self.lvalue_type(target, span);
        if !self.expr_exists(value, span) {
            return;
        }
        let value_ty = &self.module.exprs[value].ty;
        if let Some(t) = target_ty
            && !same_shape(&t, value_ty)
        {
            self.error(
                "I0014",
                span,
                format!("assignment of a `{value_ty}` value to a `{t}` target"),
            );
        }
    }

    fn check_assigns(&mut self) {
        for assign in &self.module.assigns {
            self.check_assignment(&assign.target, assign.value, assign.span);
        }
    }

    /// Reports `I0018` unless `id` is a valid single-bit expression.
    fn check_bit(&mut self, id: ExprId, what: &str, span: Span) {
        if !self.expr_exists(id, span) {
            return;
        }
        let ty = &self.module.exprs[id].ty;
        if !ty.is_bit() {
            self.error(
                "I0018",
                span,
                format!("{what} has type `{ty}`, expected `u1`"),
            );
        }
    }

    fn check_edge(&mut self, edge: &Edge, span: Span) {
        match self.module.nets.get(edge.net) {
            None => self.error("I0003", span, format!("edge on missing net {}", edge.net)),
            Some(n) if edge.polarity != Polarity::Any && !n.ty.is_bit() => self.error(
                "I0018",
                span,
                format!("edge on net `{}` of type `{}`, expected `u1`", n.name, n.ty),
            ),
            Some(_) => {}
        }
    }

    fn check_processes(&mut self) {
        for (_, process) in self.module.processes.iter() {
            match &process.kind {
                ProcessKind::Sequential { clocks, resets } => {
                    if clocks.is_empty() {
                        self.error(
                            "I0017",
                            process.span,
                            "sequential process has no clock".into(),
                        );
                    }
                    for edge in clocks.iter().chain(resets) {
                        self.check_edge(edge, process.span);
                    }
                }
                ProcessKind::Sensitive(nets) => {
                    for net in nets {
                        if !self.module.nets.contains(*net) {
                            self.error(
                                "I0003",
                                process.span,
                                format!("sensitivity on missing net {net}"),
                            );
                        }
                    }
                }
                ProcessKind::Comb | ProcessKind::Initial | ProcessKind::Free => {}
            }
            let mut stmts: Vec<&Stmt> = Vec::new();
            walk_block(&process.body, &mut |s| stmts.push(s));
            for stmt in stmts {
                self.check_stmt(stmt);
            }
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        let span = stmt.span;
        match &stmt.kind {
            StmtKind::Assign { target, value, .. } => self.check_assignment(target, *value, span),
            StmtKind::If { cond, .. } => self.check_bit(*cond, "if condition", span),
            StmtKind::Case { subject, arms, .. } => {
                if !self.expr_exists(*subject, span) {
                    return;
                }
                let subject_ty = self.module.exprs[*subject].ty.clone();
                for arm in arms {
                    for value in &arm.values {
                        if !self.expr_exists(*value, span) {
                            continue;
                        }
                        let ty = &self.module.exprs[*value].ty;
                        if !same_shape(ty, &subject_ty) {
                            self.error(
                                "I0014",
                                span,
                                format!("case item of type `{ty}` against subject `{subject_ty}`"),
                            );
                        }
                    }
                }
            }
            StmtKind::For {
                init, cond, step, ..
            } => {
                for (lv, e) in init.iter().chain(step.iter()) {
                    self.check_assignment(lv, *e, span);
                }
                if let Some(c) = cond {
                    self.check_bit(*c, "for condition", span);
                }
            }
            StmtKind::While { cond, .. } => self.check_bit(*cond, "while condition", span),
            StmtKind::Repeat { count, .. } => {
                self.expr_exists(*count, span);
            }
            StmtKind::Wait(WaitKind::Delay(e)) => {
                self.expr_exists(*e, span);
            }
            StmtKind::Wait(WaitKind::Until(e)) => self.check_bit(*e, "wait condition", span),
            StmtKind::Wait(WaitKind::Event(edges)) => {
                for edge in edges {
                    self.check_edge(edge, span);
                }
            }
            StmtKind::SysCall { args, .. } => {
                for arg in args {
                    self.expr_exists(*arg, span);
                }
            }
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                self.expr_exists(*addr, span);
                self.check_mem_addr(*mem, *addr, span);
                if let Some(en) = enable {
                    self.check_bit(*en, "write enable", span);
                }
                if !self.expr_exists(*value, span) {
                    return;
                }
                match self.module.memories.get(*mem) {
                    None => self.error("I0003", span, format!("write to missing memory {mem}")),
                    Some(m) => {
                        let ty = &self.module.exprs[*value].ty;
                        if !same_shape(ty, &m.elem) {
                            self.error(
                                "I0013",
                                span,
                                format!(
                                    "write of a `{ty}` value to memory `{}` of `{}` elements",
                                    m.name, m.elem
                                ),
                            );
                        }
                    }
                }
            }
            StmtKind::Assert { cond, message, .. } => {
                self.check_bit(*cond, "assertion condition", span);
                for arg in message {
                    self.expr_exists(*arg, span);
                }
            }
            StmtKind::Forever { .. }
            | StmtKind::Block { .. }
            | StmtKind::Finish
            | StmtKind::Stop
            | StmtKind::Break
            | StmtKind::Continue => {}
        }
    }

    fn check_cells(&mut self) {
        for (_, cell) in self.module.cells.iter() {
            self.check_cell(cell);
        }
    }

    /// The width of a cell input, if the expression exists and is a bit
    /// vector.
    fn cell_input_width(&mut self, cell: &Cell, port: &str) -> Option<u32> {
        let id = cell.input(port)?;
        if !self.expr_exists(id, cell.span) {
            return None;
        }
        self.module.exprs[id].ty.width()
    }

    /// The type of the net driven by a cell output, if it exists.
    fn cell_output_type(&mut self, cell: &Cell, port: &str) -> Option<Type> {
        let net = cell.output(port)?;
        match self.module.nets.get(net) {
            Some(n) => Some(n.ty.clone()),
            None => {
                self.error(
                    "I0003",
                    cell.span,
                    format!(
                        "cell `{}` output `{port}` drives missing net {net}",
                        cell.name
                    ),
                );
                None
            }
        }
    }

    fn check_cell(&mut self, cell: &Cell) {
        let span = cell.span;
        for (_, id) in &cell.inputs {
            self.expr_exists(*id, span);
        }
        for (_, net) in &cell.outputs {
            if !self.module.nets.contains(*net) {
                self.error(
                    "I0003",
                    span,
                    format!("cell `{}` drives missing net {net}", cell.name),
                );
            }
        }
        if matches!(cell.kind, CellKind::Blackbox(_)) {
            return;
        }
        let inputs = cell.kind.input_ports();
        let outputs = cell.kind.output_ports();
        for port in &inputs {
            if cell.input(port).is_none() {
                self.error(
                    "I0015",
                    span,
                    format!(
                        "cell `{}` ({}) lacks input `{port}`",
                        cell.name,
                        cell.kind.keyword()
                    ),
                );
            }
        }
        for port in &outputs {
            if cell.output(port).is_none() {
                self.error(
                    "I0015",
                    span,
                    format!(
                        "cell `{}` ({}) lacks output `{port}`",
                        cell.name,
                        cell.kind.keyword()
                    ),
                );
            }
        }
        for (name, _) in &cell.inputs {
            if !inputs.contains(&name.as_str()) {
                self.error(
                    "I0015",
                    span,
                    format!(
                        "cell `{}` ({}) has no input `{name}`",
                        cell.name,
                        cell.kind.keyword()
                    ),
                );
            }
        }
        for (name, _) in &cell.outputs {
            if !outputs.contains(&name.as_str()) {
                self.error(
                    "I0015",
                    span,
                    format!(
                        "cell `{}` ({}) has no output `{name}`",
                        cell.name,
                        cell.kind.keyword()
                    ),
                );
            }
        }

        // Width rules. A missing port was reported above; `None` widths
        // from non-vector types are reported here.
        let a = self.cell_input_width(cell, "a");
        let b = self.cell_input_width(cell, "b");
        let y = self.cell_output_type(cell, "y").and_then(|t| t.width());
        let rule = |checker: &mut Self, ok: bool, what: &str| {
            if !ok {
                checker.error(
                    "I0016",
                    span,
                    format!("cell `{}` ({}): {what}", cell.name, cell.kind.keyword()),
                );
            }
        };
        let eq = |x: Option<u32>, y: Option<u32>| x.is_some() && x == y;
        let one = |x: Option<u32>| x == Some(1);
        match &cell.kind {
            CellKind::Not | CellKind::Buf => {
                rule(self, eq(a, y), "`a` and `y` must have the same width")
            }
            CellKind::And
            | CellKind::Or
            | CellKind::Xor
            | CellKind::Add
            | CellKind::Sub
            | CellKind::Mul
            | CellKind::Div
            | CellKind::Mod => rule(
                self,
                eq(a, b) && eq(a, y),
                "`a`, `b` and `y` must have the same width",
            ),
            CellKind::Shl | CellKind::Shr | CellKind::Sshr => {
                rule(
                    self,
                    eq(a, y) && b.is_some(),
                    "`a` and `y` must have the same width",
                );
            }
            CellKind::Eq
            | CellKind::Ne
            | CellKind::Lt
            | CellKind::Le
            | CellKind::Gt
            | CellKind::Ge => {
                rule(self, eq(a, b), "`a` and `b` must have the same width");
                rule(self, one(y), "`y` must be one bit");
            }
            CellKind::ReduceAnd | CellKind::ReduceOr | CellKind::ReduceXor => {
                rule(self, a.is_some(), "`a` must be a bit vector");
                rule(self, one(y), "`y` must be one bit");
            }
            CellKind::Mux => {
                let s = self.cell_input_width(cell, "s");
                rule(
                    self,
                    eq(a, b) && eq(a, y),
                    "`a`, `b` and `y` must have the same width",
                );
                rule(self, one(s), "`s` must be one bit");
            }
            CellKind::Pmux => {
                let s = self.cell_input_width(cell, "s");
                rule(self, eq(a, y), "`a` and `y` must have the same width");
                let ok = match (s, b, y) {
                    (Some(s), Some(b), Some(y)) => u64::from(s) * u64::from(y) == u64::from(b),
                    _ => false,
                };
                rule(self, ok, "`b` must be width(`s`) × width(`y`) bits");
            }
            CellKind::Dff {
                has_enable, reset, ..
            } => {
                let d = self.cell_input_width(cell, "d");
                let q = self.cell_output_type(cell, "q").and_then(|t| t.width());
                let clk = self.cell_input_width(cell, "clk");
                rule(self, eq(d, q), "`d` and `q` must have the same width");
                rule(self, one(clk), "`clk` must be one bit");
                if *has_enable {
                    let en = self.cell_input_width(cell, "en");
                    rule(self, one(en), "`en` must be one bit");
                }
                if let Some(reset) = reset {
                    let rst = self.cell_input_width(cell, "rst");
                    rule(self, one(rst), "`rst` must be one bit");
                    if q.is_some() && Some(reset.value.width) != q {
                        self.error(
                            "I0020",
                            span,
                            format!(
                                "cell `{}` (dff): reset value `{}` does not match the width of `q`",
                                cell.name, reset.value
                            ),
                        );
                    }
                }
            }
            CellKind::Dlatch => {
                let d = self.cell_input_width(cell, "d");
                let q = self.cell_output_type(cell, "q").and_then(|t| t.width());
                let en = self.cell_input_width(cell, "en");
                rule(self, eq(d, q), "`d` and `q` must have the same width");
                rule(self, one(en), "`en` must be one bit");
            }
            CellKind::MemRdPort { mem, clocked } => {
                let addr = self.cell_input_width(cell, "addr");
                rule(self, addr.is_some(), "`addr` must be a bit vector");
                if *clocked {
                    let clk = self.cell_input_width(cell, "clk");
                    let en = self.cell_input_width(cell, "en");
                    rule(self, one(clk) && one(en), "`clk` and `en` must be one bit");
                }
                let data = self.cell_output_type(cell, "data");
                self.check_mem_port_type(cell, *mem, data.as_ref());
            }
            CellKind::MemWrPort { mem, clocked } => {
                let addr = self.cell_input_width(cell, "addr");
                let en = self.cell_input_width(cell, "en");
                rule(self, addr.is_some(), "`addr` must be a bit vector");
                rule(self, one(en), "`en` must be one bit");
                if *clocked {
                    let clk = self.cell_input_width(cell, "clk");
                    rule(self, one(clk), "`clk` must be one bit");
                }
                let data = cell
                    .input("data")
                    .and_then(|id| self.module.exprs.get(id))
                    .map(|e| e.ty.clone());
                self.check_mem_port_type(cell, *mem, data.as_ref());
            }
            CellKind::Lut { k, init } => {
                rule(self, a == Some(*k), "`a` must have `k` bits");
                rule(self, one(y), "`y` must be one bit");
                let expected = if *k < 32 { Some(1u32 << k) } else { None };
                if expected != Some(init.width) {
                    self.error(
                        "I0020",
                        span,
                        format!(
                            "cell `{}` (lut): table has {} bits, expected 2^{k}",
                            cell.name, init.width
                        ),
                    );
                }
            }
            CellKind::Tristate => {
                let en = self.cell_input_width(cell, "en");
                rule(self, eq(a, y), "`a` and `y` must have the same width");
                rule(self, one(en), "`en` must be one bit");
            }
            CellKind::Blackbox(_) => {}
        }
    }

    fn check_mem_port_type(
        &mut self,
        cell: &Cell,
        mem: super::design::MemoryId,
        data: Option<&Type>,
    ) {
        let Some(memory) = self.module.memories.get(mem) else {
            self.error(
                "I0003",
                cell.span,
                format!("cell `{}` refers to missing memory {mem}", cell.name),
            );
            return;
        };
        if let Some(data) = data
            && !same_shape(data, &memory.elem)
        {
            self.error(
                "I0013",
                cell.span,
                format!(
                    "cell `{}` data port is `{data}` but memory `{}` holds `{}`",
                    cell.name, memory.name, memory.elem
                ),
            );
        }
    }

    fn check_instances(&mut self) {
        for (_, inst) in self.module.instances.iter() {
            self.check_instance(inst);
        }
    }

    fn check_instance(&mut self, inst: &Instance) {
        let span = inst.span;
        for (_, id) in &inst.connections {
            self.expr_exists(*id, span);
        }
        let ModuleRef::Resolved(target_id) = &inst.module else {
            return;
        };
        let Some(design) = self.design else {
            return;
        };
        let Some(target) = design.modules.get(*target_id) else {
            self.error(
                "I0007",
                span,
                format!(
                    "instance `{}` refers to missing module {target_id}",
                    inst.name
                ),
            );
            return;
        };
        for (port_name, id) in &inst.connections {
            let Some(port) = target.port(port_name.as_str()) else {
                self.error(
                    "I0008",
                    span,
                    format!(
                        "instance `{}` connects port `{port_name}` that module `{}` does not have",
                        inst.name, target.name
                    ),
                );
                continue;
            };
            let Some(expr) = self.module.exprs.get(*id) else {
                continue;
            };
            if port.dir != PortDir::In && !self.is_assignable(*id) {
                self.error(
                    "I0009",
                    span,
                    format!(
                        "instance `{}` connects {} port `{port_name}` to an expression that cannot be driven",
                        inst.name,
                        port.dir.keyword()
                    ),
                );
            }
            if let Some(net) = target.nets.get(port.net)
                && !same_shape(&expr.ty, &net.ty)
            {
                self.error(
                    "I0010",
                    span,
                    format!(
                        "instance `{}` connects `{}` to port `{port_name}` of type `{}`",
                        inst.name, expr.ty, net.ty
                    ),
                );
            }
        }
    }

    /// True when `id` is a net, a constant slice of an assignable, or a
    /// concatenation of assignables: something an output can drive.
    fn is_assignable(&self, id: ExprId) -> bool {
        match self.module.exprs.get(id).map(|e| &e.kind) {
            Some(ExprKind::Net(_)) => true,
            Some(ExprKind::Slice { base, .. }) => self.is_assignable(*base),
            Some(ExprKind::Concat(parts)) => parts.iter().all(|p| self.is_assignable(*p)),
            _ => false,
        }
    }

    /// Every net driven as a whole by assigns, cell outputs and instance
    /// outputs must have exactly one such driver.
    fn check_drivers(&mut self) {
        let mut drivers: Vec<Vec<Span>> = vec![Vec::new(); self.module.nets.len()];
        let mut record = |net: super::design::NetId, span: Span| {
            if let Some(slot) = drivers.get_mut(net.index()) {
                slot.push(span);
            }
        };
        for assign in &self.module.assigns {
            if let Lvalue::Net(net) = assign.target {
                record(net, assign.span);
            }
        }
        for (_, cell) in self.module.cells.iter() {
            for (_, net) in &cell.outputs {
                record(*net, cell.span);
            }
        }
        for (_, inst) in self.module.instances.iter() {
            let Some(target) = self
                .design
                .zip(inst.module.id())
                .and_then(|(d, id)| d.modules.get(id))
            else {
                continue;
            };
            for (port_name, id) in &inst.connections {
                if target
                    .port(port_name.as_str())
                    .is_some_and(|p| p.dir == PortDir::Out)
                    && let Some(net) = self.module.exprs.get(*id).and_then(|e| e.as_net())
                {
                    record(net, inst.span);
                }
            }
        }
        for (net, spans) in self.module.nets.iter().zip(drivers) {
            if spans.len() > 1 {
                let mut d = Diagnostic::error(format!(
                    "net `{}` has {} drivers in module `{}`",
                    net.1.name,
                    spans.len(),
                    self.module.name
                ))
                .with_code("I0011")
                .with_label(spans[0], "first driver");
                for span in &spans[1..] {
                    d = d.with_secondary(*span, "another driver");
                }
                self.diags.push(d);
            }
        }
    }
}

/// True when two types carry the same shape of value: equal widths for bit
/// vectors regardless of signedness, equal element shape and length for
/// arrays, identity otherwise.
fn same_shape(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Bits { width: wa, .. }, Type::Bits { width: wb, .. }) => wa == wb,
        (Type::Array { elem: ea, len: la }, Type::Array { elem: eb, len: lb }) => {
            la == lb && same_shape(ea, eb)
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Name;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::cell::Reset;
    use crate::ir::design::{Module, Port};
    use crate::ir::process::{AssignKind, CaseArm, CaseKind, CaseQualifier, ReportSeverity};
    use crate::ir::types::Const;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    fn codes(diags: &Diagnostics) -> Vec<&'static str> {
        let mut codes: Vec<_> = diags.iter().filter_map(|d| d.code).collect();
        codes.sort_unstable();
        codes.dedup();
        codes
    }

    fn design_with(module: Module) -> Design {
        let mut d = Design::new();
        let id = d.add_module(module);
        d.top = Some(id);
        d
    }

    #[test]
    fn clean_module_passes() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let an = b.net(a);
        let one = b.const_u64(4, 1);
        let sum = b.add(an, one);
        b.assign(y, sum);
        let d = design_with(b.finish());
        assert!(validate(&d).is_empty());
    }

    #[test]
    fn i0001_duplicate_names() {
        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        b.add_net("a", Type::bit());
        b.add_net("a", Type::bit());
        let mut d = design_with(b.finish());
        d.add_module(Module::new("m", span));
        assert_eq!(codes(&validate(&d)), ["I0001"]);
        assert_eq!(validate(&d).len(), 2);
    }

    #[test]
    fn i0002_port_without_net() {
        let span = span();
        let mut m = Module::new("m", span);
        m.ports.push(Port {
            name: Name::new("p"),
            dir: PortDir::In,
            net: super::super::design::NetId(3),
            span,
        });
        assert_eq!(codes(&validate_module(&m)), ["I0002"]);
    }

    #[test]
    fn i0003_dangling_references() {
        let mut b = ModuleBuilder::new("m", span());
        let y = b.add_net("y", Type::bit());
        let bad = ExprId(99);
        b.assign(y, bad);
        b.expr_typed(ExprKind::Net(super::super::design::NetId(9)), Type::bit());
        let m = b.finish();
        assert_eq!(codes(&validate_module(&m)), ["I0003"]);
    }

    #[test]
    fn i0004_bad_operand_types() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(8));
        let an = b.net(a);
        let cn = b.net(c);
        b.and(an, cn);
        assert_eq!(codes(&validate_module(&b.finish())), ["I0004"]);
    }

    #[test]
    fn i0005_slice_out_of_range() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let an = b.net(a);
        b.slice(an, 7, 0);
        assert_eq!(codes(&validate_module(&b.finish())), ["I0005"]);
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let one = b.const_bit(true);
        b.assign(
            Lvalue::Slice {
                net: a,
                hi: 9,
                lo: 9,
            },
            one,
        );
        assert_eq!(codes(&validate_module(&b.finish())), ["I0005"]);
    }

    #[test]
    fn i0006_cached_type_mismatch() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        b.expr_typed(ExprKind::Net(a), Type::bits(5));
        assert_eq!(codes(&validate_module(&b.finish())), ["I0006"]);
    }

    #[test]
    fn i0007_to_i0010_instances() {
        let span = span();
        let mut leaf = ModuleBuilder::new("leaf", span);
        leaf.input("i", Type::bits(4));
        leaf.output("o", Type::bits(4));
        let leaf = leaf.finish();

        let mut top = ModuleBuilder::new("top", span);
        let x = top.input("x", Type::bits(4));
        let y = top.output("y", Type::bits(4));
        let xn = top.net(x);
        let yn = top.net(y);
        let sum = top.add(xn, xn);
        let wide = top.zext(xn, 8);
        top.instance(
            "u0",
            ModuleRef::Resolved(super::super::design::ModuleId(7)),
            vec![],
        );
        top.instance(
            "u1",
            ModuleRef::Resolved(super::super::design::ModuleId(0)),
            vec![
                (Name::new("nope"), xn),
                (Name::new("o"), sum),
                (Name::new("i"), wide),
            ],
        );
        top.instance(
            "u2",
            ModuleRef::Resolved(super::super::design::ModuleId(0)),
            vec![(Name::new("o"), yn), (Name::new("i"), xn)],
        );
        let mut d = Design::new();
        d.add_module(leaf);
        d.add_module(top.finish());
        assert_eq!(codes(&validate(&d)), ["I0007", "I0008", "I0009", "I0010"]);
    }

    #[test]
    fn i0011_multiple_drivers() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bit());
        let y = b.output("y", Type::bit());
        let an = b.net(a);
        b.assign(y, an);
        b.cell(
            "c",
            CellKind::Not,
            vec![(Name::new("a"), an)],
            vec![(Name::new("y"), y)],
        );
        assert_eq!(codes(&validate_module(&b.finish())), ["I0011"]);
    }

    #[test]
    fn i0012_memory_out_of_range() {
        let mut b = ModuleBuilder::new("m", span());
        let mem = b.memory("mem", Type::bits(8), 4);
        let addr = b.const_u64(3, 4);
        let y = b.add_net("y", Type::bits(8));
        let rd = b.mem_read(mem, addr);
        b.assign(y, rd);
        let mut m = b.finish();
        m.memories[mem].init = Some(vec![Const::from_u64(8, 0); 5]);
        assert_eq!(codes(&validate_module(&m)), ["I0012"]);
    }

    #[test]
    fn i0013_memory_type_mismatch() {
        let mut b = ModuleBuilder::new("m", span());
        let mem = b.memory("mem", Type::bits(8), 4);
        let addr = b.const_u64(2, 0);
        let v = b.const_u64(4, 0);
        let mut p = b.process(None, ProcessKind::Initial);
        p.mem_write(mem, addr, v, None);
        b.end_process(p);
        let mut m = b.finish();
        m.memories[mem].init = Some(vec![Const::from_u64(4, 0)]);
        assert_eq!(codes(&validate_module(&m)), ["I0013"]);
    }

    #[test]
    fn i0014_width_mismatch() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(8));
        let an = b.net(a);
        b.assign(y, an);
        let mut p = b.process(None, ProcessKind::Comb);
        let item = b.const_u64(3, 1);
        p.case(
            an,
            CaseKind::Plain,
            CaseQualifier::None,
            vec![CaseArm {
                values: vec![item],
                body: vec![],
            }],
            None,
        );
        b.end_process(p);
        assert_eq!(codes(&validate_module(&b.finish())), ["I0014"]);
    }

    #[test]
    fn i0015_i0016_i0020_cells() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let clk = b.input("clk", Type::bit());
        let y = b.output("y", Type::bits(4));
        let q = b.output("q", Type::bits(4));
        let an = b.net(a);
        let clkn = b.net(clk);
        // Missing `b`, unknown `z`.
        b.cell(
            "c0",
            CellKind::And,
            vec![(Name::new("a"), an), (Name::new("z"), an)],
            vec![(Name::new("y"), y)],
        );
        let m = b.finish();
        assert_eq!(codes(&validate_module(&m)), ["I0015", "I0016"]);

        let mut b = ModuleBuilder::from_module(m, span());
        b.module_mut().cells = Default::default();
        // Width rule: comparing 4 bits into a 4-bit output.
        b.cell2("c1", CellKind::Eq, an, an, y);
        b.cell(
            "c2",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: true,
                    active_high: false,
                    value: Const::from_u64(3, 0),
                }),
            },
            vec![
                (Name::new("clk"), clkn),
                (Name::new("d"), an),
                (Name::new("rst"), clkn),
            ],
            vec![(Name::new("q"), q)],
        );
        assert_eq!(codes(&validate_module(&b.finish())), ["I0016", "I0020"]);
    }

    #[test]
    fn i0017_i0018_processes() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let an = b.net(a);
        let mut p = b.process(
            None,
            ProcessKind::Sequential {
                clocks: vec![],
                resets: vec![Edge::pos(a)],
            },
        );
        p.if_(an, vec![], vec![]);
        p.assert(an, ReportSeverity::Error, vec![]);
        b.end_process(p);
        assert_eq!(codes(&validate_module(&b.finish())), ["I0017", "I0018"]);
    }

    #[test]
    fn i0019_missing_top() {
        let mut d = Design::new();
        d.top = Some(super::super::design::ModuleId(4));
        assert_eq!(codes(&validate(&d)), ["I0019"]);
    }

    #[test]
    fn lut_and_process_statements_are_checked() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bit());
        let an = b.net(a);
        b.cell(
            "l",
            CellKind::Lut {
                k: 4,
                init: Const::from_u64(16, 0x8000),
            },
            vec![(Name::new("a"), an)],
            vec![(Name::new("y"), y)],
        );
        let t = b.add_reg("t", Type::bits(4));
        let mut p = b.process(None, ProcessKind::Free);
        let cond = b.reduce_or(an);
        p.while_(cond, vec![]);
        p.wait_until(cond);
        p.wait_event(vec![Edge::any(a)]);
        p.for_(
            Some((Lvalue::Net(t), an)),
            Some(cond),
            Some((Lvalue::Net(t), an)),
            vec![],
        );
        p.assign(
            Lvalue::Concat(vec![
                Lvalue::Slice {
                    net: t,
                    hi: 1,
                    lo: 0,
                },
                Lvalue::Slice {
                    net: t,
                    hi: 3,
                    lo: 2,
                },
            ]),
            an,
            AssignKind::Blocking,
        );
        p.assign(
            Lvalue::Index { net: t, index: an },
            cond,
            AssignKind::Blocking,
        );
        b.end_process(p);
        assert!(validate_module(&b.finish()).is_empty());
    }
}
