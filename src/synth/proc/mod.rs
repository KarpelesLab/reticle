//! Process lowering: from behavioural processes to cells and assigns.
//!
//! [`ProcLower`] rewrites every synthesisable process of a module:
//!
//! - **Combinational** (`always @*`, `always_comb`, and explicit
//!   sensitivity lists, which are treated the same with a warning when the
//!   list is incomplete): each assigned net becomes a continuous assign of
//!   its mux tree. A net not assigned on every path keeps its value on the
//!   other paths, which is a level-sensitive latch: a `dlatch` cell is
//!   emitted with warning `S0012`, unless the net is a process-local
//!   temporary (never read before being written, never read outside the
//!   process), in which case no storage is needed.
//! - **Sequential** (`always @(posedge clk ...)`): each assigned net
//!   becomes a `dff` cell. An `if (rst)` at the top of the body whose
//!   branch assigns a constant becomes the cell's reset: asynchronous when
//!   `rst` is one of the process's asynchronous control edges (with the
//!   matching polarity), synchronous otherwise. A net assigned only under
//!   some condition gets that condition as clock enable. Partial (slice or
//!   indexed) assignments are merged into the whole register with the
//!   untouched bits fed back from `q`. A register whose next value is
//!   exactly a memory read becomes a clocked `memrd` port instead of a
//!   flip-flop plus an unclocked port.
//! - **Memory writes** (`mem[a] <= d`, `memwrite`) become `memwr` ports,
//!   clocked in sequential processes, with the enable derived from the
//!   path condition. Memory reads in continuous assigns become unclocked
//!   `memrd` ports too, so no `MemRead` expression survives.
//! - **`initial`** blocks that only assign constants set the `init`
//!   attribute of the nets and the initial contents of memories.
//!
//! Limits: one clock per process; `Type::Array` nets, waits, `forever`,
//! non-constant loop bounds, and `break` / `continue` under a non-constant
//! condition are reported with `S0010` and the process is left in place.
//! Only one asynchronous control per register is expressible; a second
//! one is treated as synchronous (`S0013`). Simulation-only statements are
//! dropped with note `S0014`.

mod exec;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::walk::{stmt_exprs, stmt_nets, walk_block};
use crate::ir::{
    AttrValue, BinaryOp, CellKind, Const, Edge, ExprId, ExprKind, Lvalue, Memory, Module, NetId,
    NetKind, Polarity, Process, ProcessKind, Reset, UnaryOp, expr::operands,
};
use crate::source::Span;
use crate::synth::eval::eval_closed;
use crate::synth::util::{add_cell, add_wire, is_kept, is_port, mk_bit, mk_net};
use crate::synth::{Pass, PassStats, SynthOptions};

use exec::{En, Exec, Failed, Flow, HOLD, MemWrite, Mode, SId};

/// The process-lowering pass; see the module docs.
pub struct ProcLower {
    max_unroll: u32,
}

impl ProcLower {
    /// A pass configured from the synthesis options.
    pub fn new(options: &SynthOptions) -> Self {
        ProcLower {
            max_unroll: options.max_unroll,
        }
    }
}

impl Default for ProcLower {
    fn default() -> Self {
        ProcLower::new(&SynthOptions::default())
    }
}

impl Pass for ProcLower {
    fn name(&self) -> &'static str {
        "proc_lower"
    }

    fn run(&self, m: &mut Module, diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let processes = std::mem::take(&mut m.processes);
        if processes.is_empty()
            && !has_continuous_mem_read(m)
            && m.assigns.iter().all(|a| a.delay.is_none())
        {
            m.processes = processes;
            return stats;
        }
        let reads: Vec<BTreeSet<NetId>> = processes.values().map(|p| process_reads(m, p)).collect();
        let base_external = external_reads(m);
        let mut kept = Vec::new();
        for (i, process) in processes.values().enumerate() {
            if let ProcessKind::Sensitive(nets) = &process.kind {
                let missing: Vec<String> = reads[i]
                    .iter()
                    .filter(|n| !nets.contains(n))
                    .map(|n| format!("`{}`", m.nets[*n].name))
                    .collect();
                if !missing.is_empty() {
                    diags.push(
                        Diagnostic::warning(format!(
                            "incomplete sensitivity list: {} not listed; synthesised as combinational",
                            missing.join(", ")
                        ))
                        .with_code("S0017")
                        .with_span(process.span),
                    );
                }
            }
            let external = |net: NetId| {
                base_external[net.index()]
                    || reads
                        .iter()
                        .enumerate()
                        .any(|(j, r)| j != i && r.contains(&net))
            };
            let mut ctx = Ctx {
                m,
                diags,
                stats: &mut stats,
                max_unroll: self.max_unroll,
                external: &external,
            };
            let ok = match &process.kind {
                ProcessKind::Initial => ctx.lower_initial(process),
                ProcessKind::Comb => ctx.lower_comb(process),
                ProcessKind::Sensitive(_) => ctx.lower_comb(process),
                ProcessKind::Sequential { clocks, resets } => {
                    ctx.lower_seq(process, clocks, resets)
                }
                ProcessKind::Free => {
                    diags.push(
                        Diagnostic::error(
                            "process without a sensitivity list or clock is not synthesisable",
                        )
                        .with_code("S0010")
                        .with_span(process.span),
                    );
                    Err(Failed)
                }
            };
            match ok {
                Ok(()) => stats.bump("processes lowered", 1),
                Err(Failed) => kept.push(process.clone()),
            }
        }
        for p in kept {
            m.processes.push(p);
        }
        lower_continuous_mem_reads(m, &mut stats);
        let mut delays = 0u64;
        for a in &mut m.assigns {
            if a.delay.take().is_some() {
                delays += 1;
            }
        }
        stats.bump("delays ignored", delays);
        regs_to_wires(m, &mut stats);
        stats
    }
}

/// Per-process lowering context.
struct Ctx<'a> {
    m: &'a mut Module,
    diags: &'a mut Diagnostics,
    stats: &'a mut PassStats,
    max_unroll: u32,
    /// True for nets read outside the process being lowered.
    external: &'a dyn Fn(NetId) -> bool,
}

/// The plan for one register.
struct FfPlan {
    net: NetId,
    reset: Option<(Reset, ExprId)>,
    en: En,
    d: ExprId,
    span: Span,
    /// The process has asynchronous controls but this net is not assigned
    /// a constant under one of them. Whether that deserves a warning is
    /// only known once the net turns out to be a flip-flop, so the
    /// decision is carried here rather than reported on the spot.
    unreset: bool,
}

impl Ctx<'_> {
    fn exec(&mut self, process: &Process, mode: Mode) -> Result<Exec<'_>, Failed> {
        let mut exec = Exec::new(self.m, self.diags, mode, process.span, self.max_unroll);
        match exec.exec_block(&process.body) {
            Ok(Flow::Next) => {}
            Ok(Flow::Break | Flow::Continue) => {
                exec.rollback();
                exec.diags.push(
                    Diagnostic::error("`break` or `continue` outside a loop")
                        .with_code("S0010")
                        .with_span(process.span),
                );
                return Err(Failed);
            }
            Err(Failed) => {
                exec.rollback();
                return Err(Failed);
            }
        }
        if !exec.dropped.is_empty() {
            let what: BTreeSet<&str> = exec.dropped.iter().map(|(_, w)| *w).collect();
            let mut d = Diagnostic::note(format!(
                "{} simulation-only statement{} dropped ({})",
                exec.dropped.len(),
                if exec.dropped.len() == 1 { "" } else { "s" },
                what.into_iter().collect::<Vec<_>>().join(", ")
            ))
            .with_code("S0014");
            for (i, (span, _)) in exec.dropped.iter().enumerate() {
                d = if i == 0 {
                    d.with_span(*span)
                } else {
                    d.with_secondary(*span, "")
                };
            }
            exec.diags.push(d);
        }
        Ok(exec)
    }

    /// The final symbolic value of every assigned net, rejecting nets
    /// assigned with both `=` and `<=`.
    fn final_values(exec: &mut Exec<'_>, span: Span) -> Result<BTreeMap<NetId, SId>, Failed> {
        let mut out = BTreeMap::new();
        for (net, sid) in exec.blocking.clone() {
            out.insert(net, sid);
        }
        for (net, sid) in exec.nba.clone() {
            if out.contains_key(&net) {
                let name = exec.m.nets[net].name.clone();
                exec.rollback();
                exec.diags.push(
                    Diagnostic::error(format!(
                        "`{name}` is assigned with both blocking and non-blocking assignments"
                    ))
                    .with_code("S0015")
                    .with_span(span),
                );
                return Err(Failed);
            }
            out.insert(net, sid);
        }
        Ok(out)
    }

    /// Rejects a net that already has a whole-net driver.
    fn check_driver(exec: &mut Exec<'_>, net: NetId, span: Span) -> Result<(), Failed> {
        let driven = exec
            .m
            .assigns
            .iter()
            .any(|a| matches!(a.target, Lvalue::Net(n) if n == net))
            || exec.m.cells.iter().any(|(id, c)| {
                !exec.created_cells.contains(&id) && c.outputs.iter().any(|(_, n)| *n == net)
            });
        if driven {
            let name = exec.m.nets[net].name.clone();
            exec.rollback();
            exec.diags.push(
                Diagnostic::error(format!(
                    "`{name}` is driven by this process and by another driver"
                ))
                .with_code("S0016")
                .with_span(span),
            );
            return Err(Failed);
        }
        Ok(())
    }

    fn lower_initial(&mut self, process: &Process) -> Result<(), Failed> {
        let span = process.span;
        let mut exec = self.exec(process, Mode::Initial)?;
        let values = Self::final_values(&mut exec, span)?;
        let mut inits: Vec<(NetId, Const)> = Vec::new();
        for (net, sid) in values {
            let e = exec.lower(sid, net, span);
            match eval_closed(exec.m, e) {
                Some(c) => inits.push((net, c)),
                None => {
                    let name = exec.m.nets[net].name.clone();
                    exec.rollback();
                    exec.diags.push(
                        Diagnostic::error(format!(
                            "`initial` block assigns a non-constant value to `{name}`"
                        ))
                        .with_code("S0010")
                        .with_label(span, "in this process")
                        .with_note("only constant initial values are synthesisable; the process is left as is"),
                    );
                    return Err(Failed);
                }
            }
        }
        let mut mem_inits: Vec<(crate::ir::MemoryId, u64, Const)> = Vec::new();
        for w in exec.mem_writes.clone() {
            let en = eval_closed(exec.m, w.en).and_then(|c| c.to_u64());
            let addr = eval_closed(exec.m, w.addr).and_then(|c| c.to_u64());
            let data = eval_closed(exec.m, w.data);
            match (en, addr, data) {
                (Some(0), _, _) => {}
                (Some(1), Some(a), Some(d)) if a < exec.m.memories[w.mem].size => {
                    mem_inits.push((w.mem, a, d));
                }
                _ => {
                    let name = exec.m.memories[w.mem].name.clone();
                    exec.rollback();
                    exec.diags.push(
                        Diagnostic::error(format!(
                            "`initial` block writes a non-constant value or address to `{name}`"
                        ))
                        .with_code("S0010")
                        .with_label(w.span, "in this process")
                        .with_note("only constant initial values are synthesisable; the process is left as is"),
                    );
                    return Err(Failed);
                }
            }
        }
        for (net, c) in inits {
            self.m.nets[net].attrs.set("init", AttrValue::Const(c));
            self.stats.bump("init values", 1);
        }
        for (mem, addr, data) in mem_inits {
            set_mem_init(&mut self.m.memories[mem], addr, data);
            self.stats.bump("memory init values", 1);
        }
        Ok(())
    }

    fn lower_comb(&mut self, process: &Process) -> Result<(), Failed> {
        let span = process.span;
        let external = self.external;
        let mut exec = self.exec(process, Mode::Comb)?;
        let values = Self::final_values(&mut exec, span)?;
        let mut assigns = Vec::new();
        let mut latches = Vec::new();
        for (net, sid) in values {
            Self::check_driver(&mut exec, net, span)?;
            let (en, d) = exec.extract(sid, net);
            let d = d.unwrap_or(HOLD);
            let d = exec.lower(d, net, span);
            let assigned_at = exec.assigned_spans.get(&net).copied().unwrap_or(span);
            match en {
                En::Always => assigns.push((net, d, assigned_at)),
                En::Never => {}
                En::Cond(e) => {
                    let needed = external(net)
                        || is_port(exec.m, net)
                        || exec.read_unassigned.contains(&net)
                        || is_kept(&exec.m.nets[net].attrs);
                    if needed {
                        latches.push((net, e, d, assigned_at));
                    } else {
                        exec.stats.bump("temporaries dropped", 1);
                    }
                }
            }
        }
        let mem_writes = exec.mem_writes.clone();
        let mut stats = std::mem::take(&mut exec.stats);
        drop(exec);
        for (net, d, at) in assigns {
            self.m.assigns.push(crate::ir::Assign {
                target: Lvalue::Net(net),
                value: d,
                delay: None,
                attrs: crate::ir::Attrs::new(),
                span: at,
            });
            stats.bump("assigns", 1);
        }
        for (net, en, d, at) in latches {
            let name = self.m.nets[net].name.clone();
            self.diags.push(
                Diagnostic::warning(format!("latch inferred for `{name}`"))
                    .with_code("S0012")
                    .with_label(at, "not assigned on every path")
                    .with_note("add an `else` or a default assignment to make it combinational"),
            );
            let base = format!("{name}$latch");
            add_cell(
                self.m,
                &base,
                CellKind::Dlatch,
                vec![("en", en), ("d", d)],
                vec![("q", net)],
                at,
            );
            stats.bump("latches", 1);
        }
        for w in mem_writes {
            self.write_port(&w, None);
            stats.bump("memory write ports", 1);
        }
        self.stats.merge(&stats);
        Ok(())
    }

    fn write_port(&mut self, w: &MemWrite, clk: Option<ExprId>) {
        let base = format!("{}$wr", self.m.memories[w.mem].name);
        let mut inputs = vec![("addr", w.addr), ("data", w.data), ("en", w.en)];
        if let Some(clk) = clk {
            inputs.push(("clk", clk));
        }
        add_cell(
            self.m,
            &base,
            CellKind::MemWrPort {
                mem: w.mem,
                clocked: clk.is_some(),
            },
            inputs,
            Vec::new(),
            w.span,
        );
    }

    fn lower_seq(
        &mut self,
        process: &Process,
        clocks: &[Edge],
        resets: &[Edge],
    ) -> Result<(), Failed> {
        let span = process.span;
        let clock = match clocks {
            [clk] if clk.polarity != Polarity::Any => *clk,
            [_] => {
                self.diags.push(
                    Diagnostic::error("clock without an edge is not synthesisable")
                        .with_code("S0010")
                        .with_span(span),
                );
                return Err(Failed);
            }
            _ => {
                self.diags.push(
                    Diagnostic::error("a process with several clocks is not synthesisable")
                        .with_code("S0010")
                        .with_span(span)
                        .with_note("split it into one process per clock"),
                );
                return Err(Failed);
            }
        };
        let mut exec = self.exec(process, Mode::Seq)?;
        // Only a net the process assigns non-blockingly is meant to be a
        // register. A blocking assignment inside a clocked block is a
        // temporary, computed and consumed within the cycle, and an
        // inlined Verilog function leaves one per local and per argument:
        // warning that those have no asynchronous reset is noise about
        // something that is not a register.
        let intended_registers: std::collections::BTreeSet<NetId> =
            exec.nba.keys().copied().collect();
        let values = Self::final_values(&mut exec, span)?;
        let mut plans = Vec::new();
        for (net, sid) in values {
            Self::check_driver(&mut exec, net, span)?;
            let assigned_at = exec.assigned_spans.get(&net).copied().unwrap_or(span);
            let (reset, rest, unreset) = split_reset(&mut exec, sid, net, resets, assigned_at);
            let (en, d) = exec.extract(rest, net);
            let d = match d {
                Some(d) => exec.lower(d, net, assigned_at),
                None => mk_net(exec.m, net, assigned_at),
            };
            plans.push(FfPlan {
                net,
                reset,
                en,
                d,
                span: assigned_at,
                unreset: unreset && intended_registers.contains(&net),
            });
        }
        // Count how often each read-port data net is used, to decide which
        // reads can be registered directly in the port.
        let mut roots: Vec<ExprId> = Vec::new();
        for p in &plans {
            roots.push(p.d);
            if let En::Cond(e) = p.en {
                roots.push(e);
            }
            if let Some((_, r)) = &p.reset {
                roots.push(*r);
            }
        }
        for w in &exec.mem_writes {
            roots.extend([w.addr, w.data, w.en]);
        }
        for id in &exec.created_cells {
            roots.extend(exec.m.cells[*id].inputs.iter().map(|(_, e)| *e));
        }
        let uses = net_use_counts(exec.m, &roots);
        let mem_writes = exec.mem_writes.clone();
        let rd_ports = exec.rd_ports.clone();
        let mut stats = std::mem::take(&mut exec.stats);
        drop(exec);

        let clk = mk_net(self.m, clock.net, span);
        for plan in plans {
            let en_expr = match plan.en {
                En::Always => None,
                En::Never => None,
                En::Cond(e) => Some(e),
            };
            // Registered read: `q <= mem[a]` with nothing else reading the
            // port becomes a clocked read port driving `q`.
            if plan.reset.is_none()
                && let ExprKind::Net(data) = self.m.expr(plan.d).kind
                && let Some(cell) = rd_ports.get(&data)
                && uses.get(&data).copied().unwrap_or(0) == 1
            {
                let en = en_expr.unwrap_or_else(|| mk_bit(self.m, true, plan.span));
                let c = &mut self.m.cells[*cell];
                let CellKind::MemRdPort { mem, .. } = c.kind else {
                    unreachable!("read port cell")
                };
                c.kind = CellKind::MemRdPort { mem, clocked: true };
                c.inputs.push((crate::ir::Name::new("clk"), clk));
                c.inputs.push((crate::ir::Name::new("en"), en));
                c.outputs = vec![(crate::ir::Name::new("data"), plan.net)];
                c.span = plan.span;
                stats.bump("registered memory reads", 1);
                continue;
            }
            let has_enable = en_expr.is_some();
            let mut inputs = vec![("clk", clk), ("d", plan.d)];
            if let Some(e) = en_expr {
                inputs.push(("en", e));
            }
            let reset = plan.reset.map(|(r, rst)| {
                inputs.push(("rst", rst));
                r
            });
            // Now that the net is certainly a flip-flop, a missing
            // asynchronous reset is worth saying. Reporting it earlier
            // warned about process temporaries, such as the locals an
            // inlined Verilog function leaves behind, which never become
            // registers at all.
            if plan.unreset {
                let name = self.m.nets[plan.net].name.clone();
                self.diags.push(
                    Diagnostic::warning(format!(
                        "`{name}` is assigned in a process with asynchronous controls but not as a constant under one of them; no asynchronous reset inferred"
                    ))
                    .with_code("S0013")
                    .with_span(plan.span)
                    .with_note("the register only reacts to the clock; check the reset branch"),
                );
            }
            let name = format!("{}$ff", self.m.nets[plan.net].name);
            add_cell(
                self.m,
                &name,
                CellKind::Dff {
                    clk_pos: clock.polarity == Polarity::Pos,
                    has_enable,
                    reset,
                },
                inputs,
                vec![("q", plan.net)],
                plan.span,
            );
            stats.bump("flip-flops", 1);
        }
        for w in mem_writes {
            self.write_port(&w, Some(clk));
            stats.bump("memory write ports", 1);
        }
        self.stats.merge(&stats);
        Ok(())
    }
}

/// Peels a reset condition down to the net it tests, and says whether the
/// reset is active high.
///
/// Frontends spell the same test differently: Verilog's `if (!rst_n)`
/// arrives as `not(rst_n)`, while VHDL's `if rst = '1'` arrives as
/// `eq(rst, 1'd1)`. Both mean the same register, so both must be
/// recognised; missing one silently demotes an asynchronous reset to a
/// synchronous one, which is a change in hardware behaviour rather than a
/// missed optimisation.
///
/// Only `Eq` and `Ne` against a one-bit constant are peeled, not the
/// case-equality operators, whose result differs when the tested net is
/// `x` or `z`.
fn reset_polarity(exec: &Exec<'_>, cond: ExprId) -> (ExprId, bool) {
    let mut expr = cond;
    let mut active_high = true;
    // Bounded: a reset condition is a handful of nodes, and a cycle in the
    // expression arena would be a bug elsewhere.
    for _ in 0..8 {
        match exec.m.expr(expr).kind {
            ExprKind::Unary {
                op: UnaryOp::Not | UnaryOp::LogicNot,
                expr: inner,
            } if exec.m.expr(inner).ty.is_bit() => {
                active_high = !active_high;
                expr = inner;
            }
            ExprKind::Binary {
                op: op @ (BinaryOp::Eq | BinaryOp::Ne),
                lhs,
                rhs,
            } => {
                // One side must be a one-bit constant; the other is the net.
                let (net_side, constant) = match (
                    exec.m.expr(lhs).as_const().cloned(),
                    exec.m.expr(rhs).as_const().cloned(),
                ) {
                    (None, Some(c)) => (lhs, c),
                    (Some(c), None) => (rhs, c),
                    _ => break,
                };
                if constant.width() != 1 || !exec.m.expr(net_side).ty.is_bit() {
                    break;
                }
                let Some(bit) = constant.to_u64() else {
                    // A comparison against `x` or `z` never holds; leave it
                    // alone rather than guessing a polarity.
                    break;
                };
                let tests_one = (bit == 1) == (op == BinaryOp::Eq);
                if !tests_one {
                    active_high = !active_high;
                }
                expr = net_side;
            }
            _ => break,
        }
    }
    (expr, active_high)
}

/// Peels a reset off the top of a symbolic value: `Mux { cond, Const, rest }`
/// where `cond` is an asynchronous control edge (async reset) or any
/// condition when the process has none (sync reset). Returns the reset
/// with its `rst` expression, and the remaining tree.
fn split_reset(
    exec: &mut Exec<'_>,
    sid: SId,
    net: NetId,
    resets: &[Edge],
    span: Span,
) -> (Option<(Reset, ExprId)>, SId, bool) {
    let exec::SNode::Mux { cond, then_, else_ } = exec.get(sid).clone() else {
        return (None, sid, !resets.is_empty());
    };
    let exec::SNode::Expr(value) = exec.get(then_).clone() else {
        return (None, sid, !resets.is_empty());
    };
    let Some(value) = exec.m.expr(value).as_const().cloned() else {
        return (None, sid, !resets.is_empty());
    };
    let width = exec.net_type(net).width().unwrap_or(0);
    if value.width() != width {
        return (None, sid, false);
    }
    let (rst, active_high) = reset_polarity(exec, cond);
    let rst_net = exec.m.expr(rst).as_net();
    let asynchronous = match rst_net.and_then(|n| resets.iter().find(|e| e.net == n)) {
        Some(edge) => {
            let expected = if active_high {
                Polarity::Pos
            } else {
                Polarity::Neg
            };
            if edge.polarity != expected && edge.polarity != Polarity::Any {
                let name = exec.m.nets[edge.net].name.clone();
                exec.diags.push(
                    Diagnostic::warning(format!(
                        "asynchronous control `{name}` is tested with the opposite polarity of its edge; treated as synchronous"
                    ))
                    .with_code("S0013")
                    .with_span(span),
                );
                false
            } else {
                true
            }
        }
        None => false,
    };
    // A reset branch was found, so nothing is missing even when the tested
    // net is not one of the process's asynchronous controls: that is an
    // ordinary synchronous reset.
    let unreset = false;
    (
        Some((
            Reset {
                asynchronous,
                active_high,
                value,
            },
            rst,
        )),
        else_,
        unreset,
    )
}

/// Writes `data` at `addr` of a memory's initial contents.
fn set_mem_init(mem: &mut Memory, addr: u64, data: Const) {
    let init = mem.init.get_or_insert_with(Vec::new);
    let width = mem.elem.width().unwrap_or(0);
    let idx = usize::try_from(addr).unwrap_or(usize::MAX);
    while init.len() <= idx {
        init.push(Const::x(width));
    }
    init[idx] = data;
}

/// How many times each net is referenced from the expression DAG rooted at
/// `roots`, counting every edge into a `Net` node.
fn net_use_counts(m: &Module, roots: &[ExprId]) -> HashMap<NetId, u32> {
    let mut refs: HashMap<ExprId, u32> = HashMap::new();
    let mut seen = vec![false; m.exprs.len()];
    let mut stack: Vec<ExprId> = roots.to_vec();
    for r in roots {
        *refs.entry(*r).or_insert(0) += 1;
    }
    while let Some(id) = stack.pop() {
        if seen[id.index()] {
            continue;
        }
        seen[id.index()] = true;
        for op in operands(&m.expr(id).kind) {
            *refs.entry(op).or_insert(0) += 1;
            stack.push(op);
        }
    }
    let mut out: HashMap<NetId, u32> = HashMap::new();
    for (id, n) in refs {
        if let ExprKind::Net(net) = m.expr(id).kind {
            *out.entry(net).or_insert(0) += n;
        }
    }
    out
}

/// Every net read by the expressions of a process (including its
/// sensitivity and clock nets).
fn process_reads(m: &Module, p: &Process) -> BTreeSet<NetId> {
    let mut roots = Vec::new();
    walk_block(&p.body, &mut |s| stmt_exprs(s, &mut |e| roots.push(e)));
    let mut out = BTreeSet::new();
    let mut seen = vec![false; m.exprs.len()];
    let mut stack = roots;
    while let Some(id) = stack.pop() {
        if seen[id.index()] {
            continue;
        }
        seen[id.index()] = true;
        if let ExprKind::Net(n) = m.expr(id).kind {
            out.insert(n);
        }
        stack.extend(operands(&m.expr(id).kind));
    }
    out
}

/// Nets read by the module's continuous logic and instances, or exposed
/// by ports, with processes excluded (the caller has taken them out).
fn external_reads(m: &Module) -> Vec<bool> {
    let mut out = vec![false; m.nets.len()];
    for p in &m.ports {
        out[p.net.index()] = true;
    }
    m.for_each_expr(|_, e| {
        if let ExprKind::Net(n) = e.kind {
            out[n.index()] = true;
        }
    });
    out
}

fn has_continuous_mem_read(m: &Module) -> bool {
    let mut found = false;
    m.for_each_expr(|_, e| found |= matches!(e.kind, ExprKind::MemRead { .. }));
    found
}

/// Turns every `MemRead` reachable from continuous logic into an
/// unclocked read port.
fn lower_continuous_mem_reads(m: &mut Module, stats: &mut PassStats) {
    let mut roots = Vec::new();
    for a in &m.assigns {
        roots.push(a.value);
        crate::ir::walk::lvalue_exprs(&a.target, &mut |e| roots.push(e));
    }
    for (_, c) in m.cells.iter() {
        roots.extend(c.inputs.iter().map(|(_, e)| *e));
    }
    for (_, i) in m.instances.iter() {
        roots.extend(i.connections.iter().map(|(_, e)| *e));
    }
    let mut reads = Vec::new();
    let mut seen = vec![false; m.exprs.len()];
    let mut stack = roots;
    while let Some(id) = stack.pop() {
        if seen[id.index()] {
            continue;
        }
        seen[id.index()] = true;
        if matches!(m.expr(id).kind, ExprKind::MemRead { .. }) {
            reads.push(id);
        }
        stack.extend(operands(&m.expr(id).kind));
    }
    if reads.is_empty() {
        return;
    }
    reads.sort();
    let mut replacement: HashMap<ExprId, ExprId> = HashMap::new();
    for id in reads {
        let ExprKind::MemRead { mem, addr } = m.expr(id).kind else {
            continue;
        };
        let span = m.expr(id).span;
        let elem = m.memories[mem].elem.clone();
        let base = format!("{}$rd", m.memories[mem].name);
        let data = add_wire(m, &base, elem, span);
        add_cell(
            m,
            &base,
            CellKind::MemRdPort {
                mem,
                clocked: false,
            },
            vec![("addr", addr)],
            vec![("data", data)],
            span,
        );
        let net = mk_net(m, data, span);
        replacement.insert(id, net);
        stats.bump("memory read ports", 1);
    }
    m.map_exprs(|id| replacement.get(&id).copied().unwrap_or(id));
}

/// Registers no longer written by a process become wires.
fn regs_to_wires(m: &mut Module, stats: &mut PassStats) {
    let mut written = vec![false; m.nets.len()];
    for (_, p) in m.processes.iter() {
        walk_block(&p.body, &mut |s| {
            stmt_nets(s, &mut |n| written[n.index()] = true)
        });
    }
    let mut count = 0;
    for (id, net) in m.nets.iter_mut() {
        if net.kind != NetKind::Wire && !written[id.index()] {
            net.kind = NetKind::Wire;
            count += 1;
        }
    }
    stats.bump("regs to wires", count);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::validate::validate_module;
    use crate::ir::{AssignKind, CaseArm, CaseKind, CaseQualifier, Type};
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    fn lower(m: &mut Module) -> (PassStats, Diagnostics) {
        let mut diags = Diagnostics::new();
        let stats = ProcLower::default().run(m, &mut diags);
        let problems = validate_module(m);
        assert!(
            !problems.has_errors(),
            "{}",
            problems
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n")
        );
        (stats, diags)
    }

    fn text(m: &Module) -> String {
        m.to_text()
    }

    #[test]
    fn counter_with_sync_reset_and_enable() {
        let mut b = ModuleBuilder::new("counter", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let en = b.input("en", Type::bit());
        let q = b.output_reg("q", Type::bits(8));
        let (rstv, env, qv) = (b.net(rst), b.net(en), b.net(q));
        let zero = b.const_u64(8, 0);
        let one = b.const_u64(8, 1);
        let next = b.add(qv, one);
        let mut p = b.process(Some("count"), ProcessKind::posedge(clk));
        let mut reset = b.block();
        reset.nonblocking(q, zero);
        let mut count = b.block();
        count.nonblocking(q, next);
        let mut run = b.block();
        run.if_(env, count.finish(), Vec::new());
        p.if_(rstv, reset.finish(), run.finish());
        b.end_process(p);
        let mut m = b.finish();
        let (stats, diags) = lower(&mut m);
        assert!(diags.is_empty());
        assert_eq!(stats.get("flip-flops"), 1);
        assert!(m.processes.is_empty());
        let t = text(&m);
        assert!(
            t.contains("cell q$ff dff pos en srst pos 8'd0 (clk=%clk, d=add(%q, 8'd1), en=%en, rst=%rst) -> (q=%q)"),
            "{t}"
        );
        assert!(t.contains("net %q u8 wire"));
    }

    #[test]
    fn async_reset_active_low() {
        let mut b = ModuleBuilder::new("reg", span());
        let clk = b.input("clk", Type::bit());
        let rst_n = b.input("rst_n", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q = b.output_reg("q", Type::bits(4));
        let (rv, dv) = (b.net(rst_n), b.net(d));
        let nrst = b.lnot(rv);
        let ones = b.const_u64(4, 15);
        let mut p = b.process(
            None,
            ProcessKind::Sequential {
                clocks: vec![Edge::pos(clk)],
                resets: vec![Edge::neg(rst_n)],
            },
        );
        let mut reset = b.block();
        reset.nonblocking(q, ones);
        let mut run = b.block();
        run.nonblocking(q, dv);
        p.if_(nrst, reset.finish(), run.finish());
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert!(diags.is_empty());
        let t = text(&m);
        assert!(
            t.contains("cell q$ff dff pos arst neg 4'd15 (clk=%clk, d=%d, rst=%rst_n) -> (q=%q)"),
            "{t}"
        );
    }

    #[test]
    fn async_reset_polarity_mismatch_warns() {
        let mut b = ModuleBuilder::new("reg", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q = b.output_reg("q", Type::bits(4));
        let (rv, dv) = (b.net(rst), b.net(d));
        let zero = b.const_u64(4, 0);
        let mut p = b.process(
            None,
            ProcessKind::Sequential {
                clocks: vec![Edge::pos(clk)],
                resets: vec![Edge::neg(rst)],
            },
        );
        let mut reset = b.block();
        reset.nonblocking(q, zero);
        let mut run = b.block();
        run.nonblocking(q, dv);
        p.if_(rv, reset.finish(), run.finish());
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert_eq!(diags.warning_count(), 1);
        assert!(text(&m).contains("srst pos 4'd0"));
    }

    #[test]
    fn missing_reset_branch_warns() {
        let mut b = ModuleBuilder::new("reg", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q = b.output_reg("q", Type::bits(4));
        let r = b.output_reg("r", Type::bits(4));
        let (rv, dv) = (b.net(rst), b.net(d));
        let zero = b.const_u64(4, 0);
        let mut p = b.process(
            None,
            ProcessKind::Sequential {
                clocks: vec![Edge::pos(clk)],
                resets: vec![Edge::pos(rst)],
            },
        );
        let mut reset = b.block();
        reset.nonblocking(q, zero);
        let mut run = b.block();
        run.nonblocking(q, dv);
        run.nonblocking(r, dv);
        p.if_(rv, reset.finish(), run.finish());
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert_eq!(diags.warning_count(), 1);
        let t = text(&m);
        assert!(t.contains("cell q$ff dff pos arst pos 4'd0"), "{t}");
        assert!(
            t.contains("cell r$ff dff pos en (clk=%clk, d=%d, en=not(%rst))"),
            "{t}"
        );
    }

    /// A blocking assignment inside a clocked block is a temporary, not a
    /// register, so it must not be warned about for lacking an
    /// asynchronous reset. Inlining a Verilog function leaves one such
    /// temporary per local and per argument, which made the warning fire
    /// for names the user never declared.
    #[test]
    fn blocking_temporaries_are_not_unreset_registers() {
        let mut b = ModuleBuilder::new("tmp", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let d = b.input("d", Type::bits(8));
        let q = b.output_reg("q", Type::bits(8));
        // A process-local temporary, blocking-assigned then read.
        let t = b.add_reg("t", Type::bits(8));
        let (rstv, dv) = (b.net(rst), b.net(d));
        let one = b.const_u64(8, 1);
        let zero = b.const_u64(8, 0);

        let mut p = b.process(
            Some("seq"),
            ProcessKind::Sequential {
                clocks: vec![Edge::pos(clk)],
                resets: vec![Edge::pos(rst)],
            },
        );
        let mut reset = b.block();
        reset.nonblocking(q, zero);
        let mut run = b.block();
        run.blocking(t, dv);
        let tv = b.net(t);
        let sum = b.add(tv, one);
        run.nonblocking(q, sum);
        p.if_(rstv, reset.finish(), run.finish());
        b.end_process(p);

        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        let warnings: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == Some("S0013"))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            warnings.is_empty(),
            "warned about a blocking temporary: {warnings:?}"
        );
    }

    /// Frontends spell the reset test differently, and the shape must not
    /// decide whether the reset is asynchronous. VHDL's `rst = '1'` arrives
    /// as `eq(rst, 1'd1)`; demoting that to a synchronous reset changes the
    /// hardware, so it is a correctness bug rather than a missed
    /// optimisation.
    #[test]
    fn equality_reset_tests_are_asynchronous_too() {
        for tested_bit in [0u64, 1] {
            let mut b = ModuleBuilder::new("eqrst", span());
            let clk = b.input("clk", Type::bit());
            let rst = b.input("rst", Type::bit());
            let d = b.input("d", Type::bits(8));
            let q = b.output_reg("q", Type::bits(8));
            let (rstv, dv) = (b.net(rst), b.net(d));
            let tested = b.const_u64(1, tested_bit);
            let cond = b.eq(rstv, tested);
            let zero = b.const_u64(8, 0);
            let mut reset = b.block();
            reset.nonblocking(q, zero);
            let mut keep = b.block();
            keep.nonblocking(q, dv);
            let edge = if tested_bit == 1 {
                Edge::pos(rst)
            } else {
                Edge::neg(rst)
            };
            let mut p = b.process(
                Some("seq"),
                ProcessKind::Sequential {
                    clocks: vec![Edge::pos(clk)],
                    resets: vec![edge],
                },
            );
            p.if_(cond, reset.finish(), keep.finish());
            b.end_process(p);

            let mut m = b.finish();
            let (_, diags) = lower(&mut m);

            let cell = m
                .cells
                .iter()
                .map(|(_, c)| c)
                .find(|c| matches!(c.kind, CellKind::Dff { .. }))
                .expect("no flip-flop inferred");
            let CellKind::Dff { reset, .. } = &cell.kind else {
                unreachable!()
            };
            let reset = reset.as_ref().expect("no reset inferred");
            assert!(
                reset.asynchronous,
                "rst = '{tested_bit}' should stay asynchronous"
            );
            assert_eq!(
                reset.active_high,
                tested_bit == 1,
                "wrong polarity for rst = '{tested_bit}'"
            );
            assert!(
                !diags.iter().any(|d| d.message.contains("no asynchronous")),
                "warned despite inferring the reset"
            );
        }
    }

    #[test]
    fn comb_full_and_partial_case() {
        let mut b = ModuleBuilder::new("dec", span());
        let sel = b.input("sel", Type::bits(2));
        let a = b.input("a", Type::bits(4));
        let bb = b.input("b", Type::bits(4));
        let y = b.output_reg("y", Type::bits(4));
        let z = b.output_reg("z", Type::bits(4));
        let (sv, av, bv) = (b.net(sel), b.net(a), b.net(bb));
        let c0 = b.const_u64(2, 0);
        let c1 = b.const_u64(2, 1);
        let c2 = b.const_u64(2, 2);
        let c3 = b.const_u64(2, 3);
        let mut p = b.process(None, ProcessKind::Comb);
        let arm = |v: Vec<ExprId>, body: Vec<crate::ir::Stmt>| CaseArm { values: v, body };
        let mut b0 = b.block();
        b0.blocking(y, av);
        let mut b1 = b.block();
        b1.blocking(y, bv);
        let mut b2 = b.block();
        let x = b.xor(av, bv);
        b2.blocking(y, x);
        let mut b3 = b.block();
        let o = b.or(av, bv);
        b3.blocking(y, o);
        p.case(
            sv,
            CaseKind::Plain,
            CaseQualifier::None,
            vec![
                arm(vec![c0], b0.finish()),
                arm(vec![c1], b1.finish()),
                arm(vec![c2], b2.finish()),
                arm(vec![c3], b3.finish()),
            ],
            None,
        );
        let mut z0 = b.block();
        z0.blocking(z, av);
        let mut z1 = b.block();
        z1.blocking(z, bv);
        p.case(
            sv,
            CaseKind::Plain,
            CaseQualifier::None,
            vec![arm(vec![c0], z0.finish()), arm(vec![c1], z1.finish())],
            None,
        );
        b.end_process(p);
        let mut m = b.finish();
        let (stats, diags) = lower(&mut m);
        assert_eq!(diags.warning_count(), 1);
        assert!(diags.iter().any(|d| d.message == "latch inferred for `z`"));
        assert_eq!(stats.get("latches"), 1);
        assert_eq!(stats.get("pmux cells"), 2);
        let t = text(&m);
        assert!(t.contains("cell y$pmux pmux (a=4'bxxxx, b={or(%a, %b), xor(%a, %b), %b, %a}, s={eq(%sel, 2'd3), eq(%sel, 2'd2), eq(%sel, 2'd1), eq(%sel, 2'd0)}) -> (y=%y$pmux)"), "{t}");
        assert!(t.contains("assign %y = %y$pmux"), "{t}");
        assert!(t.contains("cell z$latch dlatch (en=mux(eq(%sel, 2'd0), 1'd1, mux(eq(%sel, 2'd1), 1'd1, 1'd0)), d=%z$pmux) -> (q=%z)"), "{t}");
    }

    #[test]
    fn nested_ifs_and_blocking_ssa() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bit());
        let c = b.input("c", Type::bit());
        let x = b.input("x", Type::bits(4));
        let y = b.output_reg("y", Type::bits(4));
        let t = b.add_reg("t", Type::bits(4));
        let (av, cv, xv, tv) = (b.net(a), b.net(c), b.net(x), b.net(t));
        let one = b.const_u64(4, 1);
        let two = b.const_u64(4, 2);
        let inc = b.add(tv, one);
        let mut p = b.process(None, ProcessKind::Comb);
        p.blocking(t, xv);
        let mut inner_t = b.block();
        inner_t.blocking(t, inc);
        let mut inner = b.block();
        inner.if_(cv, inner_t.finish(), vec![]);
        let mut outer_e = b.block();
        outer_e.blocking(t, two);
        p.if_(av, inner.finish(), outer_e.finish());
        p.blocking(y, tv);
        b.end_process(p);
        let mut m = b.finish();
        let (stats, diags) = lower(&mut m);
        assert!(diags.is_empty());
        assert_eq!(stats.get("assigns"), 2);
        let t = text(&m);
        assert!(
            t.contains("assign %y = mux(%a, mux(%c, add(%x, 4'd1), %x), 4'd2)"),
            "{t}"
        );
    }

    #[test]
    fn for_loop_unrolls_and_partial_writes_merge() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let x = b.input("x", Type::bits(4));
        let q = b.output_reg("q", Type::bits(8));
        let i = b.add_reg("i", Type::bits(8));
        let (xv, iv) = (b.net(x), b.net(i));
        let zero = b.const_u64(8, 0);
        let two = b.const_u64(8, 2);
        let one = b.const_u64(8, 1);
        let cond = b.lt(iv, two);
        let step = b.add(iv, one);
        let idx = b.index(xv, iv);
        let mut body = b.block();
        body.nonblocking(Lvalue::Index { net: q, index: iv }, idx);
        let mut p = b.process(None, ProcessKind::posedge(clk));
        p.for_(
            Some((Lvalue::Net(i), zero)),
            Some(cond),
            Some((Lvalue::Net(i), step)),
            body.finish(),
        );
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert!(diags.is_empty());
        let t = text(&m);
        assert!(t.contains("d={%q[7:2], %x[1:0]}"), "{t}");
        // The loop variable ends up as a flip-flop holding its final value.
        assert!(
            t.contains("cell i$ff dff pos (clk=%clk, d=8'd2) -> (q=%i)"),
            "{t}"
        );
    }

    #[test]
    fn memory_registered_read_and_write() {
        let mut b = ModuleBuilder::new("ram", span());
        let clk = b.input("clk", Type::bit());
        let we = b.input("we", Type::bit());
        let addr = b.input("addr", Type::bits(4));
        let wdata = b.input("wdata", Type::bits(8));
        let rdata = b.output_reg("rdata", Type::bits(8));
        let rom_out = b.output("rom_out", Type::bits(8));
        let mem = b.memory("mem", Type::bits(8), 16);
        let (wev, addrv, wdatav) = (b.net(we), b.net(addr), b.net(wdata));
        let rd = b.mem_read(mem, addrv);
        let rd2 = b.mem_read(mem, addrv);
        b.assign(rom_out, rd2);
        let mut p = b.process(None, ProcessKind::posedge(clk));
        p.mem_write(mem, addrv, wdatav, Some(wev));
        p.nonblocking(rdata, rd);
        b.end_process(p);
        let mut m = b.finish();
        let (stats, diags) = lower(&mut m);
        assert!(diags.is_empty());
        assert_eq!(stats.get("registered memory reads"), 1);
        assert_eq!(stats.get("memory write ports"), 1);
        let t = text(&m);
        assert!(
            t.contains(
                "cell mem$rd memrd @mem clocked (addr=%addr, clk=%clk, en=1'd1) -> (data=%rdata)"
            ),
            "{t}"
        );
        assert!(
            t.contains(
                "cell mem$wr memwr @mem clocked (addr=%addr, data=%wdata, en=%we, clk=%clk) -> ()"
            ),
            "{t}"
        );
        assert!(
            t.contains("cell mem$rd_2 memrd @mem (addr=%addr) -> (data=%mem$rd_2)"),
            "{t}"
        );
        assert!(t.contains("assign %rom_out = %mem$rd_2"), "{t}");
    }

    #[test]
    fn initial_blocks_become_init_values() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let q = b.output_reg("q", Type::bits(4));
        let mem = b.memory("mem", Type::bits(8), 4);
        let five = b.const_u64(4, 5);
        let a1 = b.const_u64(2, 1);
        let v = b.const_u64(8, 42);
        let mut init = b.process(None, ProcessKind::Initial);
        init.blocking(q, five);
        init.mem_write(mem, a1, v, None);
        let msg = b.string("hi");
        init.syscall("$display", vec![msg]);
        b.end_process(init);
        let qv = b.net(q);
        let one = b.const_u64(4, 1);
        let next = b.add(qv, one);
        let mut p = b.process(None, ProcessKind::posedge(clk));
        p.nonblocking(q, next);
        b.end_process(p);
        let mut m = b.finish();
        let (stats, diags) = lower(&mut m);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.iter().next().unwrap().code, Some("S0014"));
        assert_eq!(stats.get("init values"), 1);
        assert_eq!(stats.get("memory init values"), 1);
        let t = text(&m);
        assert!(t.contains("attr init = 4'd5\n  net %q u4 wire"), "{t}");
        assert!(t.contains("init 8'bxxxxxxxx 8'd42"), "{t}");
    }

    #[test]
    fn unsynthesisable_processes_are_kept() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let q = b.output_reg("q", Type::bits(4));
        let qv = b.net(q);
        let one = b.const_u64(4, 1);
        let next = b.add(qv, one);
        let mut init = b.process(None, ProcessKind::Initial);
        init.blocking(q, next);
        b.end_process(init);
        let mut free = b.process(None, ProcessKind::Free);
        free.wait_event(vec![Edge::pos(clk)]);
        free.nonblocking(q, next);
        b.end_process(free);
        let mut seq = b.process(None, ProcessKind::posedge(clk));
        seq.wait_delay(one);
        seq.nonblocking(q, next);
        b.end_process(seq);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert_eq!(diags.error_count(), 3);
        assert_eq!(m.processes.len(), 3);
        assert!(m.cells.is_empty());
        assert_eq!(m.nets[q].kind, NetKind::Reg);
    }

    #[test]
    fn mixed_assignment_kinds_and_multiple_clocks_fail() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let clk2 = b.input("clk2", Type::bit());
        let q = b.output_reg("q", Type::bits(4));
        let one = b.const_u64(4, 1);
        let mut p = b.process(None, ProcessKind::posedge(clk));
        p.blocking(q, one);
        p.nonblocking(q, one);
        b.end_process(p);
        let mut p2 = b.process(
            None,
            ProcessKind::Sequential {
                clocks: vec![Edge::pos(clk), Edge::pos(clk2)],
                resets: vec![],
            },
        );
        p2.nonblocking(q, one);
        b.end_process(p2);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert_eq!(diags.error_count(), 2);
        assert_eq!(m.processes.len(), 2);
    }

    #[test]
    fn sensitive_process_warns_when_incomplete() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bit());
        let c = b.input("c", Type::bit());
        let y = b.output_reg("y", Type::bit());
        let (av, cv) = (b.net(a), b.net(c));
        let v = b.and(av, cv);
        let mut p = b.process(None, ProcessKind::Sensitive(vec![a]));
        p.blocking(y, v);
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert_eq!(diags.warning_count(), 1);
        assert!(text(&m).contains("assign %y = and(%a, %c)"));
    }

    #[test]
    fn casez_priority_chain() {
        let mut b = ModuleBuilder::new("pri", span());
        let r = b.input("r", Type::bits(3));
        let y = b.output_reg("y", Type::bits(2));
        let rv = b.net(r);
        let p1 = b.constant(Const::parse_verilog("3'b1zz").unwrap());
        let p2 = b.constant(Const::parse_verilog("3'bz1z").unwrap());
        let p3 = b.constant(Const::parse_verilog("3'bzz1").unwrap());
        let (v2, v1, v0, vd) = (
            b.const_u64(2, 2),
            b.const_u64(2, 1),
            b.const_u64(2, 0),
            b.const_u64(2, 3),
        );
        let mut p = b.process(None, ProcessKind::Comb);
        let arm = |v: ExprId, val: ExprId, b: &ModuleBuilder| {
            let mut blk = b.block();
            blk.blocking(y, val);
            CaseArm {
                values: vec![v],
                body: blk.finish(),
            }
        };
        let arms = vec![arm(p1, v2, &b), arm(p2, v1, &b), arm(p3, v0, &b)];
        let mut d = b.block();
        d.blocking(y, vd);
        p.case(rv, CaseKind::Z, CaseQualifier::None, arms, Some(d.finish()));
        b.end_process(p);
        let mut m = b.finish();
        let (stats, diags) = lower(&mut m);
        assert!(diags.is_empty());
        assert_eq!(stats.get("pmux cells"), 0);
        let t = text(&m);
        assert!(t.contains("assign %y = mux(weq(%r, 3'b1zz), 2'd2, mux(weq(%r, 3'bz1z), 2'd1, mux(weq(%r, 3'bzz1), 2'd0, 2'd3)))"), "{t}");
    }

    #[test]
    fn concat_lvalue_and_variable_index() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let i = b.input("i", Type::bits(2));
        let hi = b.output_reg("hi", Type::bits(2));
        let lo = b.output_reg("lo", Type::bits(2));
        let bit = b.output_reg("bit", Type::bits(4));
        let (av, iv) = (b.net(a), b.net(i));
        let one = b.const_bit(true);
        let mut p = b.process(None, ProcessKind::Comb);
        p.blocking(Lvalue::Concat(vec![Lvalue::Net(hi), Lvalue::Net(lo)]), av);
        p.blocking(bit, av);
        p.blocking(
            Lvalue::Index {
                net: bit,
                index: iv,
            },
            one,
        );
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert!(diags.is_empty());
        let t = text(&m);
        assert!(t.contains("assign %hi = %a[3:2]"), "{t}");
        assert!(t.contains("assign %lo = %a[1:0]"), "{t}");
        assert!(
            t.contains("assign %bit = or(and(%a, not(shl(4'd1, %i))), shl(resize(1'd1, u4), %i))"),
            "{t}"
        );
    }

    #[test]
    fn constant_conditions_select_statically() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let y = b.output_reg("y", Type::bits(4));
        let av = b.net(a);
        let t = b.const_bit(true);
        let zero = b.const_u64(4, 0);
        let mut p = b.process(None, ProcessKind::Comb);
        let mut then = b.block();
        then.blocking(y, av);
        let mut els = b.block();
        els.blocking(y, zero);
        p.if_(t, then.finish(), els.finish());
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert!(diags.is_empty());
        assert!(text(&m).contains("assign %y = %a"));
    }

    #[test]
    fn assign_kind_helpers() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let q = b.output_reg("q", Type::bits(2));
        let qv = b.net(q);
        let mut p = b.process(None, ProcessKind::posedge(clk));
        p.assign(q, qv, AssignKind::NonBlocking);
        b.end_process(p);
        let mut m = b.finish();
        let (_, diags) = lower(&mut m);
        assert!(diags.is_empty());
        assert!(text(&m).contains("cell q$ff dff pos (clk=%clk, d=%q) -> (q=%q)"));
    }
}
