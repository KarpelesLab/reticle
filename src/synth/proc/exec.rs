//! Symbolic execution of a process body.
//!
//! [`Exec`] walks the statements of one process and builds, for every net
//! the process assigns, a *symbolic value*: a tree of [`SNode`]s whose
//! leaves are expressions or [`SNode::Hold`] ("not assigned on this path,
//! keeps its previous value"), and whose inner nodes are the `if` / `case`
//! decisions the assignment sits under. Blocking assignments update the
//! value seen by later reads in the same process (SSA-style substitution
//! through [`Exec::subst`]); non-blocking ones only feed the final value.
//!
//! Constant conditions select a branch statically, loops with constant
//! bounds are unrolled, and memory reads become read-port cells on the
//! spot. The driver in the parent module turns the finished trees into
//! flip-flops, latches or assigns through [`Exec::extract`], which splits a
//! tree into an enable condition and a data value.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{
    AssignKind, BinaryOp, Block, CaseKind, CaseQualifier, CellId, CellKind, Const, ExprId,
    ExprKind, Lvalue, MemoryId, Module, NetId, Stmt, StmtKind, Type, UnaryOp, expr::operands,
};
use crate::source::Span;
use crate::synth::PassStats;
use crate::synth::eval::{Env, eval};
use crate::synth::opt::const_fold::simplify;
use crate::synth::opt::replace_operands;
use crate::synth::util::{
    add_cell, add_wire, mk, mk_and, mk_binary, mk_bit, mk_concat, mk_const, mk_mux, mk_net, mk_not,
    mk_or, mk_slice, mk_unary, mk_undef,
};

/// Index of a node in the symbolic-value arena.
pub(super) type SId = usize;

/// The id of the shared [`SNode::Hold`] node.
pub(super) const HOLD: SId = 0;

/// One node of a symbolic value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum SNode {
    /// Not assigned on this path: the net keeps its previous value.
    Hold,
    /// An expression (already substituted).
    Expr(ExprId),
    /// The value of an `if`.
    Mux {
        /// Single-bit condition.
        cond: ExprId,
        /// Value when the condition holds.
        then_: SId,
        /// Value otherwise.
        else_: SId,
    },
    /// The value of a `case`.
    Case {
        /// One match expression per arm, in source order.
        matches: Vec<ExprId>,
        /// One value per arm.
        arms: Vec<SId>,
        /// The value when no arm matches.
        default: SId,
        /// True when the arms are provably mutually exclusive (a `pmux`
        /// is safe); false means priority order must be kept.
        parallel: bool,
    },
}

/// How the process is interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// Combinational: reads of a net not yet assigned see the driven value.
    Comb,
    /// Clocked: non-blocking reads see the register's current value.
    Seq,
    /// `initial`: everything must fold to constants.
    Initial,
}

/// A memory write collected from the body, with its path condition folded
/// into `en`.
#[derive(Clone, Debug)]
pub(super) struct MemWrite {
    /// The memory.
    pub mem: MemoryId,
    /// Element address.
    pub addr: ExprId,
    /// Data written.
    pub data: ExprId,
    /// Single-bit enable.
    pub en: ExprId,
    /// Where the write was.
    pub span: Span,
}

/// Control flow out of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flow {
    /// Fell off the end.
    Next,
    /// `break` reached.
    Break,
    /// `continue` reached.
    Continue,
}

/// The enable side of an extracted value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum En {
    /// Assigned on every path.
    Always,
    /// Assigned on no path.
    Never,
    /// Assigned when the expression is 1.
    Cond(ExprId),
}

/// Lowering failed; diagnostics were already reported.
#[derive(Debug)]
pub(super) struct Failed;

/// The symbolic executor for one process.
pub(super) struct Exec<'a> {
    /// The module being rewritten.
    pub m: &'a mut Module,
    /// Where problems go.
    pub diags: &'a mut Diagnostics,
    /// The interpretation mode.
    pub mode: Mode,
    /// Loop unrolling cap.
    pub max_unroll: u32,
    /// Span of the process, for objects without a better one.
    pub span: Span,
    nodes: Vec<SNode>,
    node_ids: HashMap<SNode, SId>,
    /// Current value of every net assigned with `=`.
    pub blocking: BTreeMap<NetId, SId>,
    /// Next value of every net assigned with `<=`.
    pub nba: BTreeMap<NetId, SId>,
    /// Nets whose previous value was read on some path (only meaningful
    /// for blocking assignments in combinational processes).
    pub read_unassigned: BTreeSet<NetId>,
    /// Memory writes in statement order.
    pub mem_writes: Vec<MemWrite>,
    /// Read-port cells created for `MemRead` expressions, by data net.
    pub rd_ports: BTreeMap<NetId, CellId>,
    /// Cells created while executing (for rollback on failure).
    pub created_cells: Vec<CellId>,
    /// Simulation-only statements dropped, as `(span, what)`.
    pub dropped: Vec<(Span, &'static str)>,
    /// Counters.
    pub stats: PassStats,
    path: Vec<ExprId>,
    subst_memo: HashMap<ExprId, ExprId>,
    lowered: HashMap<(SId, NetId), ExprId>,
    extracted: HashMap<(SId, NetId), (En, Option<SId>)>,
    has_hold: HashMap<SId, bool>,
    /// Nets assigned by this process, with the span of the first
    /// assignment (for diagnostics).
    pub assigned_spans: BTreeMap<NetId, Span>,
}

/// Resolves nets through the executor's blocking values for constant
/// evaluation.
struct BlockingEnv<'b> {
    m: &'b Module,
    nodes: &'b [SNode],
    blocking: &'b BTreeMap<NetId, SId>,
}

impl Env for BlockingEnv<'_> {
    fn net(&mut self, net: NetId) -> Option<Const> {
        let sid = *self.blocking.get(&net)?;
        match self.nodes[sid] {
            SNode::Expr(e) => eval(self.m, e, self),
            _ => None,
        }
    }
}

impl<'a> Exec<'a> {
    /// Starts executing with empty state.
    pub(super) fn new(
        m: &'a mut Module,
        diags: &'a mut Diagnostics,
        mode: Mode,
        span: Span,
        max_unroll: u32,
    ) -> Self {
        let mut exec = Exec {
            m,
            diags,
            mode,
            max_unroll,
            span,
            nodes: Vec::new(),
            node_ids: HashMap::new(),
            blocking: BTreeMap::new(),
            nba: BTreeMap::new(),
            read_unassigned: BTreeSet::new(),
            mem_writes: Vec::new(),
            rd_ports: BTreeMap::new(),
            created_cells: Vec::new(),
            dropped: Vec::new(),
            stats: PassStats::default(),
            path: Vec::new(),
            subst_memo: HashMap::new(),
            lowered: HashMap::new(),
            extracted: HashMap::new(),
            has_hold: HashMap::new(),
            assigned_spans: BTreeMap::new(),
        };
        let hold = exec.node(SNode::Hold);
        debug_assert_eq!(hold, HOLD);
        exec
    }

    // --- node arena -------------------------------------------------------

    /// Interns a node.
    pub(super) fn node(&mut self, node: SNode) -> SId {
        if let Some(id) = self.node_ids.get(&node) {
            return *id;
        }
        let id = self.nodes.len();
        self.nodes.push(node.clone());
        self.node_ids.insert(node, id);
        id
    }

    /// The node with the given id.
    pub(super) fn get(&self, id: SId) -> &SNode {
        &self.nodes[id]
    }

    fn expr_node(&mut self, e: ExprId) -> SId {
        self.node(SNode::Expr(e))
    }

    /// True when a `Hold` leaf is reachable from `id`.
    fn contains_hold(&mut self, id: SId) -> bool {
        if let Some(v) = self.has_hold.get(&id) {
            return *v;
        }
        let v = match self.nodes[id].clone() {
            SNode::Hold => true,
            SNode::Expr(_) => false,
            SNode::Mux { then_, else_, .. } => {
                self.contains_hold(then_) || self.contains_hold(else_)
            }
            SNode::Case { arms, default, .. } => {
                let mut any = self.contains_hold(default);
                for arm in arms {
                    any |= self.contains_hold(arm);
                }
                any
            }
        };
        self.has_hold.insert(id, v);
        v
    }

    // --- diagnostics -------------------------------------------------------

    fn error(&mut self, span: Span, message: impl Into<String>) -> Failed {
        self.diags.push(
            Diagnostic::error(message)
                .with_code("S0010")
                .with_label(span, "in this process")
                .with_note("this construct is not synthesisable; the process is left as is"),
        );
        Failed
    }

    fn net_name(&self, net: NetId) -> String {
        self.m.nets[net].name.to_string()
    }

    // --- values ------------------------------------------------------------

    /// The single-bit conjunction of the current path conditions.
    fn path_cond(&mut self) -> ExprId {
        let span = self.span;
        let mut acc = mk_bit(self.m, true, span);
        for c in self.path.clone() {
            acc = mk_and(self.m, acc, c, span);
        }
        acc
    }

    /// Reads the current value of `net` as seen by the next statement.
    fn read(&mut self, net: NetId, span: Span) -> ExprId {
        match self.blocking.get(&net).copied() {
            Some(sid) => {
                if self.contains_hold(sid) {
                    self.read_unassigned.insert(net);
                }
                self.lower(sid, net, span)
            }
            None => {
                if self.mode == Mode::Comb {
                    self.read_unassigned.insert(net);
                }
                mk_net(self.m, net, span)
            }
        }
    }

    /// Rewrites `e` with the current blocking values substituted for the
    /// nets they assign, memory reads turned into read ports, and closed
    /// subtrees folded to constants. Returns `e` itself when nothing under
    /// it changes.
    pub(super) fn subst(&mut self, e: ExprId) -> ExprId {
        if let Some(r) = self.subst_memo.get(&e) {
            return *r;
        }
        let expr = self.m.expr(e).clone();
        let result = match &expr.kind {
            ExprKind::Const(_) | ExprKind::String(_) => e,
            ExprKind::Net(net) => {
                if self.blocking.contains_key(net) {
                    self.read(*net, expr.span)
                } else {
                    if self.mode == Mode::Comb {
                        self.read_unassigned.insert(*net);
                    }
                    e
                }
            }
            ExprKind::MemRead { mem, addr } => {
                let addr = self.subst(*addr);
                self.read_port(*mem, addr, expr.span)
            }
            _ => {
                let ops = operands(&expr.kind);
                let new_ops: Vec<ExprId> = ops.iter().map(|o| self.subst(*o)).collect();
                if new_ops == ops {
                    e
                } else {
                    let mut kind = expr.kind.clone();
                    let mut i = 0;
                    replace_operands(&mut kind, &mut |slot| {
                        *slot = new_ops[i];
                        i += 1;
                    });
                    if let ExprKind::Call { .. } = kind {
                        self.m
                            .add_expr(crate::ir::Expr::new(kind, expr.ty.clone(), expr.span))
                    } else {
                        let id = mk(self.m, kind, expr.span);
                        simplify(self.m, id)
                    }
                }
            }
        };
        self.subst_memo.insert(e, result);
        result
    }

    /// Evaluates `e` (before substitution) under the current blocking
    /// values, when it is constant.
    fn try_const(&self, e: ExprId) -> Option<Const> {
        let mut env = BlockingEnv {
            m: self.m,
            nodes: &self.nodes,
            blocking: &self.blocking,
        };
        eval(self.m, e, &mut env)
    }

    /// Creates an unclocked read port for `mem[addr]` and returns its
    /// data net as an expression.
    fn read_port(&mut self, mem: MemoryId, addr: ExprId, span: Span) -> ExprId {
        let elem = self.m.memories[mem].elem.clone();
        let base = format!("{}$rd", self.m.memories[mem].name);
        let data = add_wire(self.m, &base, elem, span);
        let cell = add_cell(
            self.m,
            &base,
            CellKind::MemRdPort {
                mem,
                clocked: false,
            },
            vec![("addr", addr)],
            vec![("data", data)],
            span,
        );
        self.created_cells.push(cell);
        self.rd_ports.insert(data, cell);
        self.stats.bump("memory read ports", 1);
        mk_net(self.m, data, span)
    }

    /// Lowers a symbolic value to an expression, `Hold` reading `net`.
    pub(super) fn lower(&mut self, sid: SId, net: NetId, span: Span) -> ExprId {
        if let Some(e) = self.lowered.get(&(sid, net)) {
            return *e;
        }
        let result = match self.nodes[sid].clone() {
            SNode::Hold => mk_net(self.m, net, span),
            SNode::Expr(e) => e,
            SNode::Mux { cond, then_, else_ } => {
                let t = self.lower(then_, net, span);
                let f = self.lower(else_, net, span);
                mk_mux(self.m, cond, t, f, span)
            }
            SNode::Case {
                matches,
                arms,
                default,
                parallel,
            } => {
                let d = self.lower(default, net, span);
                let vals: Vec<ExprId> = arms.iter().map(|a| self.lower(*a, net, span)).collect();
                if parallel && !arms.is_empty() {
                    self.pmux(net, &matches, &vals, d, span)
                } else {
                    let mut acc = d;
                    for (m, v) in matches.iter().zip(vals).rev() {
                        acc = mk_mux(self.m, *m, v, acc, span);
                    }
                    acc
                }
            }
        };
        self.lowered.insert((sid, net), result);
        result
    }

    /// Emits a `pmux` cell: arm 0 is select bit 0 and the low slice of `b`.
    fn pmux(
        &mut self,
        net: NetId,
        matches: &[ExprId],
        vals: &[ExprId],
        default: ExprId,
        span: Span,
    ) -> ExprId {
        let ty = self.m.nets[net].ty.clone();
        let base = format!("{}$pmux", self.net_name(net));
        let y = add_wire(self.m, &base, ty.with_signed(false), span);
        let s = mk_concat(self.m, matches.iter().rev().copied().collect(), span);
        let b = mk_concat(self.m, vals.iter().rev().copied().collect(), span);
        let cell = add_cell(
            self.m,
            &base,
            CellKind::Pmux,
            vec![("a", default), ("b", b), ("s", s)],
            vec![("y", y)],
            span,
        );
        self.created_cells.push(cell);
        self.stats.bump("pmux cells", 1);
        mk_net(self.m, y, span)
    }

    // --- assignments ---------------------------------------------------------

    fn map_mut(&mut self, kind: AssignKind) -> &mut BTreeMap<NetId, SId> {
        match kind {
            AssignKind::Blocking => &mut self.blocking,
            AssignKind::NonBlocking => &mut self.nba,
        }
    }

    /// The full current value of `net` in the map for `kind`, `Hold`
    /// reading the net.
    fn current(&mut self, net: NetId, kind: AssignKind, span: Span) -> ExprId {
        let sid = self.map_mut(kind).get(&net).copied().unwrap_or(HOLD);
        if kind == AssignKind::Blocking && self.contains_hold(sid) && self.mode == Mode::Comb {
            self.read_unassigned.insert(net);
        }
        self.lower(sid, net, span)
    }

    fn set(&mut self, net: NetId, kind: AssignKind, value: ExprId, span: Span) {
        let sid = self.expr_node(value);
        self.map_mut(kind).insert(net, sid);
        self.assigned_spans.entry(net).or_insert(span);
        self.subst_memo.clear();
    }

    /// Records an assignment of the substituted `value` to `target`.
    fn assign(
        &mut self,
        target: &Lvalue,
        value: ExprId,
        kind: AssignKind,
        span: Span,
    ) -> Result<(), Failed> {
        match target {
            Lvalue::Net(net) => {
                self.check_bits_net(*net, span)?;
                self.set(*net, kind, value, span);
            }
            Lvalue::Slice { net, hi, lo } => {
                let width = self.check_bits_net(*net, span)?;
                let cur = self.current(*net, kind, span);
                let mut parts = Vec::new();
                if *hi + 1 < width {
                    let s = mk_slice(self.m, cur, width - 1, hi + 1, span);
                    parts.push(simplify(self.m, s));
                }
                parts.push(value);
                if *lo > 0 {
                    let s = mk_slice(self.m, cur, lo - 1, 0, span);
                    parts.push(simplify(self.m, s));
                }
                let new = mk_concat(self.m, parts, span);
                let new = simplify(self.m, new);
                self.set(*net, kind, new, span);
            }
            Lvalue::Index { net, index } => {
                let width = self.check_bits_net(*net, span)?;
                let index = self.subst(*index);
                if let Some(i) = self.m.expr(index).as_const().and_then(Const::to_u64) {
                    if i >= u64::from(width) {
                        // Out-of-range constant index: the write is dropped.
                        return Ok(());
                    }
                    let i = u32::try_from(i).unwrap_or(u32::MAX);
                    let lv = Lvalue::Slice {
                        net: *net,
                        hi: i,
                        lo: i,
                    };
                    return self.assign(&lv, value, kind, span);
                }
                let cur = self.current(*net, kind, span);
                let one = mk_const(self.m, Const::from_u64(1, width), span);
                let mask = mk_binary(self.m, BinaryOp::Shl, one, index, span);
                let nmask = mk_unary(self.m, UnaryOp::Not, mask, span);
                let kept = mk_binary(self.m, BinaryOp::And, cur, nmask, span);
                let v = mk(
                    self.m,
                    ExprKind::Resize {
                        expr: value,
                        width,
                        signed: false,
                    },
                    span,
                );
                let shifted = mk_binary(self.m, BinaryOp::Shl, v, index, span);
                let new = mk_binary(self.m, BinaryOp::Or, kept, shifted, span);
                self.set(*net, kind, new, span);
            }
            Lvalue::Concat(parts) => {
                let total = self.lvalue_width(target, span)?;
                let mut hi = total;
                for part in parts {
                    let w = self.lvalue_width(part, span)?;
                    if w == 0 {
                        continue;
                    }
                    let piece = mk_slice(self.m, value, hi - 1, hi - w, span);
                    let piece = simplify(self.m, piece);
                    self.assign(part, piece, kind, span)?;
                    hi -= w;
                }
            }
            Lvalue::MemElem { mem, addr } => {
                let addr = self.subst(*addr);
                self.mem_write(*mem, addr, value, None, span);
            }
        }
        Ok(())
    }

    /// The width of a net that must be a bit vector.
    fn check_bits_net(&mut self, net: NetId, span: Span) -> Result<u32, Failed> {
        match self.m.nets[net].ty.width() {
            Some(w) => Ok(w),
            None => {
                let name = self.net_name(net);
                Err(self.error(
                    span,
                    format!(
                        "assignment to `{name}` of type `{}` is not synthesisable",
                        self.m.nets[net].ty
                    ),
                ))
            }
        }
    }

    fn lvalue_width(&mut self, lv: &Lvalue, span: Span) -> Result<u32, Failed> {
        Ok(match lv {
            Lvalue::Net(net) => self.check_bits_net(*net, span)?,
            Lvalue::Slice { hi, lo, .. } => hi - lo + 1,
            Lvalue::Index { .. } => 1,
            Lvalue::Concat(parts) => {
                let mut total = 0;
                for p in parts {
                    total += self.lvalue_width(p, span)?;
                }
                total
            }
            Lvalue::MemElem { mem, .. } => self.m.memories[*mem].elem.width().unwrap_or(0),
        })
    }

    fn mem_write(
        &mut self,
        mem: MemoryId,
        addr: ExprId,
        data: ExprId,
        enable: Option<ExprId>,
        span: Span,
    ) {
        let mut en = self.path_cond();
        if let Some(e) = enable {
            en = mk_and(self.m, en, e, span);
        }
        self.mem_writes.push(MemWrite {
            mem,
            addr,
            data,
            en,
            span,
        });
    }

    // --- statements ------------------------------------------------------------

    /// Executes a block.
    pub(super) fn exec_block(&mut self, block: &Block) -> Result<Flow, Failed> {
        for stmt in block {
            let flow = self.exec_stmt(stmt)?;
            if flow != Flow::Next {
                return Ok(flow);
            }
        }
        Ok(Flow::Next)
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<Flow, Failed> {
        let span = stmt.span;
        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                kind,
                delay,
            } => {
                if delay.is_some() {
                    self.stats.bump("delays ignored", 1);
                }
                let v = self.subst(*value);
                self.assign(target, v, *kind, span)?;
                Ok(Flow::Next)
            }
            StmtKind::If { cond, then_, else_ } => self.exec_if(*cond, then_, else_, span),
            StmtKind::Case {
                subject,
                kind,
                qualifier,
                arms,
                default,
            } => self.exec_case(*subject, *kind, *qualifier, arms, default.as_ref(), span),
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some((lv, e)) = init {
                    let v = self.subst(*e);
                    self.assign(lv, v, AssignKind::Blocking, span)?;
                }
                let mut count = 0u32;
                loop {
                    if let Some(c) = cond
                        && !self.loop_cond(*c, span)?
                    {
                        break;
                    }
                    self.unroll_tick(&mut count, span)?;
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Next | Flow::Continue => {}
                    }
                    if let Some((lv, e)) = step {
                        let v = self.subst(*e);
                        self.assign(lv, v, AssignKind::Blocking, span)?;
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::While { cond, body } => {
                let mut count = 0u32;
                while self.loop_cond(*cond, span)? {
                    self.unroll_tick(&mut count, span)?;
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::Repeat { count, body } => {
                let Some(n) = self.try_const(*count).and_then(|c| c.to_u64()) else {
                    return Err(self.error(span, "`repeat` count is not constant"));
                };
                let mut done = 0u32;
                for _ in 0..n {
                    self.unroll_tick(&mut done, span)?;
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::Forever { .. } => Err(self.error(span, "`forever` loop in a process")),
            StmtKind::Block { body, .. } => self.exec_block(body),
            StmtKind::Wait(_) => Err(self.error(span, "`wait` in a process")),
            StmtKind::SysCall { .. } => {
                self.dropped.push((span, "system call"));
                Ok(Flow::Next)
            }
            StmtKind::Assert { .. } => {
                self.dropped.push((span, "assertion"));
                Ok(Flow::Next)
            }
            StmtKind::Finish => {
                self.dropped.push((span, "`$finish`"));
                Ok(Flow::Next)
            }
            StmtKind::Stop => {
                self.dropped.push((span, "`$stop`"));
                Ok(Flow::Next)
            }
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                let addr = self.subst(*addr);
                let data = self.subst(*value);
                let en = enable.map(|e| self.subst(e));
                self.mem_write(*mem, addr, data, en, span);
                Ok(Flow::Next)
            }
            StmtKind::Break => Ok(Flow::Break),
            StmtKind::Continue => Ok(Flow::Continue),
        }
    }

    fn loop_cond(&mut self, cond: ExprId, span: Span) -> Result<bool, Failed> {
        match self.try_const(cond).map(|c| c.truth()) {
            Some(crate::logic::Bit::One) => Ok(true),
            Some(crate::logic::Bit::Zero) => Ok(false),
            _ => Err(self.error(span, "loop condition is not constant")),
        }
    }

    fn unroll_tick(&mut self, count: &mut u32, span: Span) -> Result<(), Failed> {
        *count += 1;
        if *count > self.max_unroll {
            return Err(self.error(
                span,
                format!(
                    "loop exceeds the unrolling limit of {} iterations",
                    self.max_unroll
                ),
            ));
        }
        Ok(())
    }

    fn exec_if(
        &mut self,
        cond: ExprId,
        then_: &Block,
        else_: &Block,
        span: Span,
    ) -> Result<Flow, Failed> {
        if let Some(c) = self.try_const(cond) {
            return if c.truth() == crate::logic::Bit::One {
                self.exec_block(then_)
            } else {
                self.exec_block(else_)
            };
        }
        let c = self.subst(cond);
        let pre_blocking = self.blocking.clone();
        let pre_nba = self.nba.clone();

        self.path.push(c);
        let flow_t = self.exec_block(then_)?;
        self.path.pop();
        let then_blocking = std::mem::replace(&mut self.blocking, pre_blocking);
        let then_nba = std::mem::replace(&mut self.nba, pre_nba);

        let nc = mk_not(self.m, c, span);
        self.path.push(nc);
        let flow_e = self.exec_block(else_)?;
        self.path.pop();
        let else_blocking = std::mem::take(&mut self.blocking);
        let else_nba = std::mem::take(&mut self.nba);

        self.blocking = self.merge_if(c, then_blocking, else_blocking);
        self.nba = self.merge_if(c, then_nba, else_nba);
        self.subst_memo.clear();
        if flow_t != flow_e {
            return Err(self.error(span, "`break` or `continue` under a non-constant condition"));
        }
        Ok(flow_t)
    }

    fn merge_if(
        &mut self,
        cond: ExprId,
        then_: BTreeMap<NetId, SId>,
        else_: BTreeMap<NetId, SId>,
    ) -> BTreeMap<NetId, SId> {
        let keys: BTreeSet<NetId> = then_.keys().chain(else_.keys()).copied().collect();
        let mut out = BTreeMap::new();
        for net in keys {
            let t = then_.get(&net).copied().unwrap_or(HOLD);
            let f = else_.get(&net).copied().unwrap_or(HOLD);
            let v = if t == f {
                t
            } else {
                self.node(SNode::Mux {
                    cond,
                    then_: t,
                    else_: f,
                })
            };
            out.insert(net, v);
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_case(
        &mut self,
        subject: ExprId,
        kind: CaseKind,
        qualifier: CaseQualifier,
        arms: &[crate::ir::CaseArm],
        default: Option<&Block>,
        span: Span,
    ) -> Result<Flow, Failed> {
        // Static selection when everything is constant.
        if let Some(s) = self.try_const(subject) {
            let mut all_const = true;
            for arm in arms {
                let mut hit = false;
                for v in &arm.values {
                    match self.try_const(*v) {
                        Some(item) if item.width() == s.width() => {
                            hit |= match kind {
                                CaseKind::Plain => s.case_eq(&item).truth().to_bool() == Some(true),
                                CaseKind::Z => s.casez_match(&item),
                                CaseKind::X => s.casex_match(&item),
                            };
                        }
                        _ => {
                            all_const = false;
                        }
                    }
                }
                if hit && all_const {
                    return self.exec_block(&arm.body);
                }
                if !all_const {
                    break;
                }
            }
            if all_const {
                return match default {
                    Some(d) => self.exec_block(d),
                    None => Ok(Flow::Next),
                };
            }
        }

        let s = self.subst(subject);
        let width = self.m.expr(s).ty.width().unwrap_or(0);
        let mut matches = Vec::with_capacity(arms.len());
        let mut consts: Vec<Option<Const>> = Vec::new();
        for arm in arms {
            let mut m = None;
            for v in &arm.values {
                let item = self.subst(*v);
                let c = self.m.expr(item).as_const().cloned();
                consts.push(c.clone());
                let e = self.match_expr(s, item, kind, span);
                m = Some(match m {
                    None => e,
                    Some(prev) => mk_or(self.m, prev, e, span),
                });
            }
            matches.push(m.unwrap_or_else(|| mk_bit(self.m, false, span)));
        }
        let wildcard = kind != CaseKind::Plain;
        let all_const = consts.iter().all(Option::is_some);
        let disjoint = all_const && {
            let cs: Vec<&Const> = consts.iter().flatten().collect();
            let mut ok = true;
            for i in 0..cs.len() {
                for j in (i + 1)..cs.len() {
                    if patterns_overlap(cs[i], cs[j], wildcard) {
                        ok = false;
                    }
                }
            }
            ok
        };
        if qualifier == CaseQualifier::Unique && all_const && !disjoint {
            self.diags.push(
                Diagnostic::warning("`unique case` has overlapping items")
                    .with_code("S0011")
                    .with_span(span),
            );
        }
        let parallel = disjoint || qualifier == CaseQualifier::Unique;
        let full = kind == CaseKind::Plain && all_const && width <= 16 && {
            let distinct: HashSet<&Const> = consts
                .iter()
                .flatten()
                .filter(|c| c.is_fully_known())
                .collect();
            u64::try_from(distinct.len()).unwrap_or(u64::MAX) >= (1u64 << width)
        };

        let pre_blocking = self.blocking.clone();
        let pre_nba = self.nba.clone();
        let mut arm_blocking = Vec::with_capacity(arms.len());
        let mut arm_nba = Vec::with_capacity(arms.len());
        let mut flows = Vec::with_capacity(arms.len() + 1);
        let mut prior: Option<ExprId> = None;
        for (arm, m) in arms.iter().zip(&matches) {
            let cond = match (parallel, prior) {
                (true, _) | (false, None) => *m,
                (false, Some(p)) => {
                    let np = mk_not(self.m, p, span);
                    mk_and(self.m, *m, np, span)
                }
            };
            prior = Some(match prior {
                None => *m,
                Some(p) => mk_or(self.m, p, *m, span),
            });
            self.blocking = pre_blocking.clone();
            self.nba = pre_nba.clone();
            self.path.push(cond);
            flows.push(self.exec_block(&arm.body)?);
            self.path.pop();
            arm_blocking.push(std::mem::take(&mut self.blocking));
            arm_nba.push(std::mem::take(&mut self.nba));
        }
        self.blocking = pre_blocking;
        self.nba = pre_nba;
        let (default_blocking, default_nba) = match default {
            Some(d) if !full => {
                let cond = match prior {
                    Some(p) => mk_not(self.m, p, span),
                    None => mk_bit(self.m, true, span),
                };
                self.path.push(cond);
                flows.push(self.exec_block(d)?);
                self.path.pop();
                (
                    std::mem::take(&mut self.blocking),
                    std::mem::take(&mut self.nba),
                )
            }
            _ => (
                std::mem::take(&mut self.blocking),
                std::mem::take(&mut self.nba),
            ),
        };
        // A full case never reaches the default: `merge_case` turns a hold
        // there into a don't-care rather than a latch or an enable.
        self.blocking = self.merge_case(&matches, parallel, arm_blocking, default_blocking, full);
        self.nba = self.merge_case(&matches, parallel, arm_nba, default_nba, full);
        self.subst_memo.clear();

        if flows.windows(2).any(|w| w[0] != w[1]) {
            return Err(self.error(span, "`break` or `continue` under a non-constant condition"));
        }
        Ok(flows.first().copied().unwrap_or(Flow::Next))
    }

    /// The single-bit match of `subject` against `item`.
    fn match_expr(&mut self, subject: ExprId, item: ExprId, kind: CaseKind, span: Span) -> ExprId {
        let wildcard =
            kind != CaseKind::Plain && self.m.expr(item).as_const().is_none_or(|c| c.has_unknown());
        let op = if wildcard {
            BinaryOp::WildEq
        } else {
            BinaryOp::Eq
        };
        mk_binary(self.m, op, subject, item, span)
    }

    fn merge_case(
        &mut self,
        matches: &[ExprId],
        parallel: bool,
        arms: Vec<BTreeMap<NetId, SId>>,
        default: BTreeMap<NetId, SId>,
        full: bool,
    ) -> BTreeMap<NetId, SId> {
        let keys: BTreeSet<NetId> = arms
            .iter()
            .chain(std::iter::once(&default))
            .flat_map(|m| m.keys().copied())
            .collect();
        let mut out = BTreeMap::new();
        for net in keys {
            let vals: Vec<SId> = arms
                .iter()
                .map(|m| m.get(&net).copied().unwrap_or(HOLD))
                .collect();
            let mut d = default.get(&net).copied().unwrap_or(HOLD);
            if full && d == HOLD {
                let span = self.span;
                let ty = self.m.nets[net].ty.clone();
                let x = mk_undef(self.m, &ty, span);
                d = self.expr_node(x);
            }
            let v = if vals.iter().all(|v| *v == d) {
                d
            } else {
                self.node(SNode::Case {
                    matches: matches.to_vec(),
                    arms: vals,
                    default: d,
                    parallel,
                })
            };
            out.insert(net, v);
        }
        out
    }

    // --- extraction ---------------------------------------------------------------

    /// Splits a symbolic value into the condition under which the net is
    /// assigned and the value it receives then. `None` data means the net
    /// is never assigned.
    pub(super) fn extract(&mut self, sid: SId, net: NetId) -> (En, Option<SId>) {
        if let Some(r) = self.extracted.get(&(sid, net)) {
            return *r;
        }
        let span = self.span;
        let result = match self.nodes[sid].clone() {
            SNode::Hold => (En::Never, None),
            SNode::Expr(_) => (En::Always, Some(sid)),
            SNode::Mux { cond, then_, else_ } => {
                let (et, dt) = self.extract(then_, net);
                let (ef, df) = self.extract(else_, net);
                match (et, ef) {
                    (En::Never, En::Never) => (En::Never, None),
                    (En::Never, ef) => {
                        let nc = mk_not(self.m, cond, span);
                        let e = self.en_expr(ef);
                        (En::Cond(mk_and(self.m, nc, e, span)), df)
                    }
                    (et, En::Never) => {
                        let e = self.en_expr(et);
                        (En::Cond(mk_and(self.m, cond, e, span)), dt)
                    }
                    (et, ef) => {
                        let en = if et == En::Always && ef == En::Always {
                            En::Always
                        } else {
                            let a = self.en_expr(et);
                            let b = self.en_expr(ef);
                            En::Cond(mk_mux(self.m, cond, a, b, span))
                        };
                        let d = self.node(SNode::Mux {
                            cond,
                            then_: dt.unwrap_or(HOLD),
                            else_: df.unwrap_or(HOLD),
                        });
                        (en, Some(d))
                    }
                }
            }
            SNode::Case {
                matches,
                arms,
                default,
                parallel,
            } => {
                let ex: Vec<(En, Option<SId>)> =
                    arms.iter().map(|a| self.extract(*a, net)).collect();
                let (ed, dd) = self.extract(default, net);
                if ex.iter().all(|(e, _)| *e == En::Never) && ed == En::Never {
                    (En::Never, None)
                } else {
                    let en = if ex.iter().all(|(e, _)| *e == En::Always) && ed == En::Always {
                        En::Always
                    } else {
                        let mut acc = self.en_expr(ed);
                        for (m, (e, _)) in matches.iter().zip(&ex).rev() {
                            let v = self.en_expr(*e);
                            acc = mk_mux(self.m, *m, v, acc, span);
                        }
                        En::Cond(acc)
                    };
                    let mut new_matches = Vec::new();
                    let mut new_arms = Vec::new();
                    for (m, (e, d)) in matches.iter().zip(&ex) {
                        if *e != En::Never {
                            new_matches.push(*m);
                            new_arms.push(d.unwrap_or(HOLD));
                        }
                    }
                    let d = match dd {
                        Some(d) => d,
                        None => {
                            let ty = self.m.nets[net].ty.clone();
                            let x = mk_undef(self.m, &ty, span);
                            self.expr_node(x)
                        }
                    };
                    let node = if new_arms.is_empty() {
                        d
                    } else {
                        self.node(SNode::Case {
                            matches: new_matches,
                            arms: new_arms,
                            default: d,
                            parallel,
                        })
                    };
                    (en, Some(node))
                }
            }
        };
        self.extracted.insert((sid, net), result);
        result
    }

    /// An enable as a single-bit expression.
    pub(super) fn en_expr(&mut self, en: En) -> ExprId {
        let span = self.span;
        match en {
            En::Always => mk_bit(self.m, true, span),
            En::Never => mk_bit(self.m, false, span),
            En::Cond(e) => e,
        }
    }

    /// Undoes the cells created so far (for a failed lowering).
    pub(super) fn rollback(&mut self) {
        let doomed: HashSet<CellId> = self.created_cells.iter().copied().collect();
        self.m.cells.retain(|id, _| !doomed.contains(&id));
        self.created_cells.clear();
        self.rd_ports.clear();
    }

    /// The type of a net.
    pub(super) fn net_type(&self, net: NetId) -> Type {
        self.m.nets[net].ty.clone()
    }
}

/// True when some subject value matches both patterns. With `wildcard`,
/// `x`/`z` bits of a pattern match anything; without it, a pattern with
/// unknown bits never matches hardware and overlaps only itself.
pub(super) fn patterns_overlap(a: &Const, b: &Const, wildcard: bool) -> bool {
    if a.width() != b.width() {
        return false;
    }
    if !wildcard {
        return a == b && a.is_fully_known();
    }
    (0..a.width()).all(|i| {
        let (x, y) = (a.bit(i), b.bit(i));
        !x.is_known() || !y.is_known() || x == y
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_rules() {
        let a = Const::parse_verilog("4'b10zz").unwrap();
        let b = Const::parse_verilog("4'b1011").unwrap();
        let c = Const::parse_verilog("4'b0011").unwrap();
        assert!(patterns_overlap(&a, &b, true));
        assert!(!patterns_overlap(&a, &c, true));
        assert!(!patterns_overlap(&a, &b, false));
        assert!(patterns_overlap(&b, &b, false));
        assert!(!patterns_overlap(&a, &a, false));
        assert!(!patterns_overlap(&b, &Const::from_u64(1, 3), true));
    }
}
