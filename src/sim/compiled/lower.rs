//! Lowering: from the flattened design to a straight-line program.
//!
//! The input is the *event simulator's own* elaboration: compiled mode
//! builds a [`Simulator`], which flattens the hierarchy, aliases ports,
//! collects drivers and fanout and runs time zero. Nothing about the
//! design's shape is re-derived here, so a hierarchical name, a
//! [`NetHandle`](crate::sim::NetHandle) and a memory mean exactly the same
//! thing in both engines — which is what makes the two comparable cycle
//! by cycle.
//!
//! # The shape of a cycle
//!
//! Every driver, combinational cell and combinational process is a *unit*
//! that reads some signals and writes some (ranges of) signals. The units
//! are sorted topologically once, at compile time; running them in that
//! order settles the whole combinational region in a single pass, with no
//! event queue and no fixed point to converge to. Clocked processes,
//! flip-flops, clocked memory ports and the surviving side effects form
//! the second list, which runs after the settle and only computes: the
//! next value of every register and the memory writes. The commit then
//! makes all of it current at once.
//!
//! # Symbolic execution
//!
//! A process is not interpreted, it is *executed symbolically*: each
//! statement updates a map from signal to the SSA value that holds it, an
//! `if` runs both arms on copies of that map and merges them with a `Mux`,
//! and a `case` is a chain of such merges in priority order. Loops are
//! unrolled, which is why their trip count has to be fixed at compile
//! time. Alongside the value, each written signal carries a *defined mask*
//! of the bits some path has assigned; merging intersects the masks, so a
//! bit that only one arm assigns ends up undefined, and a combinational
//! unit that leaves a bit undefined is exactly a latch — reported, not
//! silently mis-simulated.
//!
//! # Folding and sharing
//!
//! Because the order is already topological and the form is SSA, constant
//! folding and common subexpression elimination are one line each:
//! an operation whose operands are all constants is *executed* at compile
//! time against the register-file image (by the same [`State::exec`] the
//! engine uses, so the two can never disagree), and an operation whose
//! key has been seen before returns the value it produced.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ir::{
    AssignKind, BinaryOp, Block, CaseArm, CaseKind, Cell, CellKind, Edge, ExprId, ExprKind, Lvalue,
    MemoryId, Module, NetId, Polarity, Process, ProcessKind, Span, Stmt, StmtKind, UnaryOp,
    WaitKind,
};
use crate::logic::Logic;
use crate::sim::Simulator;
use crate::sim::elab::{
    DriverId, DriverSource, InstId, MemId, ProcId, SigId, Target, block_reads, expr_reads,
};
use crate::sim::value::{elem_count, elem_width, flat_width};

use super::check::{Ineligible, Reason};
use super::engine::State;
use super::prog::{MemLayout, Message, MsgArg, Op, Program, ProgramStats, Slot, known_words};
use super::words;

/// An index into the lowerer's value table.
type ValId = u32;

/// The unroll budget of one loop, after which the trip count is treated as
/// unbounded.
const MAX_UNROLL: u32 = 100_000;

/// Tags that name an operation kind in the [`Key`] used for common
/// subexpression elimination.
///
/// One namespace for every kind, structural and operator alike: two
/// different operations must never produce the same key, or one would
/// silently return the other's value -- a `rand` cell and a `ror` cell on
/// the same net both yield one unsigned bit from one operand, and nothing
/// but the tag tells them apart. Each operator gets its own tag, and the
/// same tag is used whether the operation came from an expression, a cell
/// or one of the lowering's own helpers, which is what lets those share a
/// value.
mod tag {
    use crate::ir::{BinaryOp, UnaryOp};

    /// Width change.
    pub(super) const RESIZE: u16 = 1;
    /// Two-way select.
    pub(super) const MUX: u16 = 2;
    /// Constant bit range.
    pub(super) const EXTRACT: u16 = 3;
    /// Constant bit range replacement.
    pub(super) const INSERT: u16 = 4;
    /// Computed element replacement.
    pub(super) const DYN_INSERT: u16 = 5;
    /// Computed element select.
    pub(super) const DYN_INDEX: u16 = 6;
    /// Computed bit range.
    pub(super) const DYN_EXTRACT: u16 = 7;
    /// Concatenation.
    pub(super) const CONCAT: u16 = 8;
    /// Replication.
    pub(super) const REPLICATE: u16 = 9;
    /// Asynchronous memory read.
    pub(super) const MEM_READ: u16 = 10;
    /// One-hot select.
    pub(super) const PMUX: u16 = 11;
    /// Lookup table.
    pub(super) const LUT: u16 = 12;
    /// Base of the unary operators.
    const UNARY: u16 = 100;
    /// Base of the binary operators.
    const BINARY: u16 = 200;

    /// The tag of one unary operator.
    pub(super) fn unary(op: UnaryOp) -> u16 {
        let i = UnaryOp::ALL.iter().position(|o| *o == op).unwrap_or(0);
        UNARY + u16::try_from(i).unwrap_or(0)
    }

    /// The tag of one binary operator.
    pub(super) fn binary(op: BinaryOp) -> u16 {
        let i = BinaryOp::ALL.iter().position(|o| *o == op).unwrap_or(0);
        BINARY + u16::try_from(i).unwrap_or(0)
    }
}

/// A cached operation, for common subexpression elimination.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    tag: u16,
    width: u32,
    signed: bool,
    srcs: Vec<ValId>,
    imm: Vec<i64>,
}

/// One combinational unit: a driver or a combinational process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnitKind {
    Driver(DriverId),
    Proc(ProcId),
}

/// A combinational unit with what it reads and writes.
struct Unit {
    kind: UnitKind,
    inst: InstId,
    reads: Vec<SigId>,
    /// Written signals; `None` is "somewhere in this signal, we cannot say
    /// where" (a computed index).
    writes: Vec<(SigId, Option<(u32, u32)>)>,
    label: String,
    span: Span,
}

/// Where one asynchronous reset comes from.
#[derive(Clone, Copy, Debug)]
enum AsyncSource {
    /// A clocked process with an `async` sensitivity entry.
    Process(usize),
    /// A `dff` cell with an asynchronous reset.
    Cell(usize),
}

/// One asynchronous reset: what makes it active and what it forces.
struct AsyncReset {
    id: usize,
    inst: InstId,
    source: AsyncSource,
    /// The signal the reset condition reads.
    cond: SigId,
    polarity: Polarity,
    /// The signals the reset forces; used to order the prologue.
    targets: Vec<SigId>,
    label: String,
    span: Span,
}

/// The process-local view of the design while one unit is executed.
struct Ctx<'d> {
    inst: InstId,
    m: &'d Module,
    /// True while lowering into the clock-edge list.
    seq: bool,
    /// True when a signal the unit does not assign keeps its current
    /// value: a register holds, a combinational net latches.
    hold: bool,
    /// True while lowering the asynchronous-reset prologue, where only an
    /// input or a register may be read.
    prologue: bool,
    /// Signals pinned to a value, used to force a reset condition true
    /// while the reset branch is lowered.
    force: BTreeMap<SigId, ValId>,
    /// The path condition, for the side effects nested under it.
    guard: Option<ValId>,
    cur: BTreeMap<SigId, ValId>,
    def: BTreeMap<SigId, Vec<u64>>,
    nba: BTreeMap<SigId, ValId>,
    nba_def: BTreeMap<SigId, Vec<u64>>,
    /// Signals some path assigned blockingly, and non-blockingly. The
    /// masks cannot answer this: a clocked unit seeds them full, because
    /// a register that no path assigns holds, and a conditional
    /// assignment intersects back to empty. These accumulate across both
    /// arms of a branch, which is what "some path" means.
    wrote: BTreeSet<SigId>,
    wrote_nba: BTreeSet<SigId>,
    writes: BTreeSet<SigId>,
    label: String,
    span: Span,
}

/// The lowering state.
pub(crate) struct Lowerer<'d> {
    sim: Simulator<'d>,
    zero_init: bool,
    errors: Vec<Ineligible>,
    vals: Vec<Slot>,
    is_const: Vec<bool>,
    fold: State,
    fold_prog: Program,
    messages: Vec<Message>,
    cse: HashMap<Key, (ValId, bool)>,
    consts: HashMap<(u32, bool, Vec<u64>), ValId>,
    cursor: usize,
    /// True while the asynchronous-reset prologue is being emitted, where
    /// values may not be shared with the rest of the program.
    in_prologue: bool,
    comb: Vec<Op>,
    seq: Vec<Op>,
    side: Vec<Op>,
    /// The value that holds each signal's settled value.
    env: Vec<Option<ValId>>,
    /// State index of each signal.
    state_of: Vec<Option<usize>>,
    state_sigs: Vec<SigId>,
    state_cur: Vec<Slot>,
    state_next: Vec<Slot>,
    /// Bits of each signal a driver has produced so far.
    covered: Vec<Option<Vec<u64>>>,
    widest: usize,
}

/// Every bit of a value of `width` bits.
fn full_mask(width: u32) -> Vec<u64> {
    let mut v = vec![u64::MAX; words::words_for(width)];
    words::mask_top(&mut v, width);
    v
}

/// No bit of a value of `width` bits.
fn empty_mask(width: u32) -> Vec<u64> {
    vec![0u64; words::words_for(width)]
}

/// Marks bits `[lo, lo + width)`.
fn mark(mask: &mut [u64], lo: u32, width: u32) {
    for i in lo..lo.saturating_add(width) {
        words::set_bit(mask, i, true);
    }
}

/// True when two ranges overlap; `None` means "the whole signal".
fn overlaps(a: Option<(u32, u32)>, b: Option<(u32, u32)>) -> bool {
    match (a, b) {
        (Some((al, aw)), Some((bl, bw))) => {
            let (ah, bh) = (u64::from(al) + u64::from(aw), u64::from(bl) + u64::from(bw));
            u64::from(al) < bh && u64::from(bl) < ah
        }
        _ => true,
    }
}

/// Builds a plan from an elaborated simulator.
pub(crate) fn build(
    sim: Simulator<'_>,
    zero_init: bool,
) -> Result<super::Plan<'_>, Vec<Ineligible>> {
    let signals = sim.signals.len();
    let low = Lowerer {
        sim,
        zero_init,
        errors: Vec::new(),
        vals: Vec::new(),
        is_const: Vec::new(),
        fold: State::new(&Program {
            comb: Vec::new(),
            seq: Vec::new(),
            init: Vec::new(),
            state_cur: 0,
            state_next: 0,
            state_len: 0,
            mems: Vec::new(),
            luts: Vec::new(),
            messages: Vec::new(),
            widest: 0,
        }),
        fold_prog: Program {
            comb: Vec::new(),
            seq: Vec::new(),
            init: Vec::new(),
            state_cur: 0,
            state_next: 0,
            state_len: 0,
            mems: Vec::new(),
            luts: Vec::new(),
            messages: Vec::new(),
            widest: 0,
        },
        messages: Vec::new(),
        cse: HashMap::new(),
        consts: HashMap::new(),
        cursor: 0,
        in_prologue: false,
        comb: Vec::new(),
        seq: Vec::new(),
        side: Vec::new(),
        env: vec![None; signals],
        state_of: vec![None; signals],
        state_sigs: Vec::new(),
        state_cur: Vec::new(),
        state_next: Vec::new(),
        covered: vec![None; signals],
        widest: 1,
    };
    low.run()
}

impl<'d> Lowerer<'d> {
    fn run(mut self) -> Result<super::Plan<'d>, Vec<Ineligible>> {
        let clock = self.scan();
        let units = self.collect_units();
        let order = self.sort_units(&units);
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        self.sim.run_until(0);
        let output = self.sim.take_output();
        self.allocate_state();
        self.allocate_memories();
        self.allocate_inputs(&units);
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        self.check_blocking_races();
        self.lower_async_resets();
        for i in order {
            self.lower_unit(&units[i]);
        }
        self.check_coverage();
        self.lower_sequential();
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        Ok(self.finish(clock, output))
    }

    // -- accessors -------------------------------------------------------

    fn module(&self, inst: InstId) -> &'d Module {
        self.sim.instances[inst.idx()].m
    }

    fn sig_of(&self, inst: InstId, net: NetId) -> SigId {
        self.sim.instances[inst.idx()].nets[net.index()]
    }

    fn mem_of(&self, inst: InstId, mem: MemoryId) -> MemId {
        self.sim.instances[inst.idx()].mems[mem.index()]
    }

    fn sig_name(&self, sig: SigId) -> String {
        self.sim.signals[sig.idx()].name.clone()
    }

    fn sig_width(&self, sig: SigId) -> u32 {
        self.sim.signals[sig.idx()].value.width()
    }

    fn sig_signed(&self, sig: SigId) -> bool {
        self.sim.signals[sig.idx()].ty.is_signed()
    }

    fn slot(&self, v: ValId) -> Slot {
        self.vals[v as usize]
    }

    fn err(&mut self, object: impl Into<String>, reason: Reason, span: Option<Span>) {
        let item = Ineligible::new(object, reason, span);
        if !self.errors.contains(&item) {
            self.errors.push(item);
        }
    }

    // -- value table -----------------------------------------------------

    fn alloc(&mut self, width: u32, signed: bool) -> ValId {
        let words = words::words_for(width);
        let off = u32::try_from(self.cursor).expect("register file exceeds 2^32 words");
        self.cursor += words;
        self.fold.regs.resize(self.cursor, 0);
        self.widest = self.widest.max(words);
        let id = u32::try_from(self.vals.len()).expect("value table exceeds 2^32 entries");
        self.vals.push(Slot {
            off,
            words: u32::try_from(words).expect("width fits"),
            width,
            signed,
        });
        self.is_const.push(false);
        id
    }

    /// A second name for a value with a different signedness; free,
    /// because signedness is metadata and not stored bits.
    fn alias(&mut self, v: ValId, signed: bool) -> ValId {
        let s = self.slot(v);
        if s.signed == signed {
            return v;
        }
        let id = u32::try_from(self.vals.len()).expect("value table exceeds 2^32 entries");
        self.vals.push(Slot { signed, ..s });
        self.is_const.push(self.is_const[v as usize]);
        id
    }

    fn constant(&mut self, value: &Logic) -> ValId {
        let width = value.width();
        let signed = value.is_signed();
        let words = known_words(value);
        if let Some(v) = self.consts.get(&(width, signed, words.clone())) {
            return *v;
        }
        let id = self.alloc(width, signed);
        let off = self.slot(id).off as usize;
        for (i, w) in words.iter().enumerate() {
            self.fold.regs[off + i] = *w;
        }
        self.is_const[id as usize] = true;
        self.consts.insert((width, signed, words), id);
        id
    }

    fn zero(&mut self, width: u32, signed: bool) -> ValId {
        let l = Logic::zero(width).with_signed(signed);
        self.constant(&l)
    }

    /// The constant a value holds, if it holds one.
    fn as_const(&self, v: ValId) -> Option<Logic> {
        if !self.is_const[v as usize] {
            return None;
        }
        let s = self.slot(v);
        Some(Logic::from_planes(
            s.width,
            s.signed,
            self.fold.regs[s.range()].to_vec(),
            vec![0u64; s.words as usize],
        ))
    }

    /// Emits one operation, folding it or reusing an equal one.
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        seq: bool,
        tag: u16,
        imm: &[i64],
        srcs: &[ValId],
        width: u32,
        signed: bool,
        foldable: bool,
        build: impl FnOnce(Slot, &[Slot]) -> Op,
    ) -> ValId {
        let key = Key {
            tag,
            width,
            signed,
            srcs: srcs.to_vec(),
            imm: imm.to_vec(),
        };
        // The asynchronous-reset prologue is the one part of the
        // program that is not in single assignment form: it writes a
        // register's own slot. Everything it computes therefore reads a
        // register from *before* that write, so sharing such a value with
        // the rest of the program — or with a later prologue entry —
        // would hand out a pre-reset value. A folded constant is exempt:
        // it depends on nothing that changes.
        if !self.in_prologue
            && let Some((v, in_seq)) = self.cse.get(&key)
            && (!*in_seq || seq)
        {
            return *v;
        }
        let dst = self.alloc(width, signed);
        let slots: Vec<Slot> = srcs.iter().map(|s| self.slot(*s)).collect();
        let op = build(self.slot(dst), &slots);
        let all_const = foldable && srcs.iter().all(|s| self.is_const[*s as usize]);
        if all_const {
            let Lowerer {
                fold, fold_prog, ..
            } = self;
            fold.exec(fold_prog, &op);
            self.is_const[dst as usize] = true;
            self.cse.insert(key, (dst, false));
        } else {
            if seq {
                self.seq.push(op);
            } else {
                self.comb.push(op);
            }
            if !self.in_prologue {
                self.cse.insert(key, (dst, seq));
            }
        }
        dst
    }

    fn resize(&mut self, seq: bool, v: ValId, width: u32, signed: bool) -> ValId {
        let s = self.slot(v);
        if s.width == width {
            return self.alias(v, signed);
        }
        self.emit(
            seq,
            tag::RESIZE,
            &[],
            &[v],
            width,
            signed,
            true,
            |dst, a| Op::Resize { dst, a: a[0] },
        )
    }

    // -- structural scan -------------------------------------------------

    /// Rejects everything that has no cycle-based meaning and finds the
    /// one clock.
    fn scan(&mut self) -> Option<(SigId, Polarity)> {
        let mut clock: Option<(SigId, Polarity)> = None;
        for p in 0..self.sim.procs.len() {
            let inst = self.sim.procs[p].inst;
            let process: &'d Process = self.sim.procs[p].process;
            let label = format!("process {}", self.sim.procs[p].name);
            match &process.kind {
                ProcessKind::Free => {
                    self.err(label, Reason::FreeProcess, Some(process.span));
                }
                ProcessKind::Sequential { clocks, .. } => {
                    self.check_timing(&label, &process.body);
                    match clocks.as_slice() {
                        [Edge { net, polarity }] if *polarity != Polarity::Any => {
                            let sig = self.sig_of(inst, *net);
                            self.note_clock(&mut clock, sig, *polarity, &label, process.span);
                        }
                        _ => self.err(
                            label,
                            Reason::SeveralClocks("one clock edge".into()),
                            Some(process.span),
                        ),
                    }
                }
                ProcessKind::Initial => self.check_timing(&label, &process.body),
                ProcessKind::Comb => {}
                ProcessKind::Sensitive(list) => {
                    let mut nets = BTreeSet::new();
                    let mut mems = BTreeSet::new();
                    block_reads(self.module(inst), &process.body, &mut nets, &mut mems);
                    let listed: BTreeSet<NetId> = list.iter().copied().collect();
                    if !nets.is_subset(&listed) {
                        self.err(label, Reason::IncompleteSensitivity, Some(process.span));
                    }
                }
            }
        }
        for c in 0..self.sim.cells.len() {
            let inst = self.sim.cells[c].inst;
            let cell: &'d Cell = self.sim.cells[c].cell;
            let label = format!("cell {}.{}", self.sim.instances[inst.idx()].path, cell.name);
            match &cell.kind {
                CellKind::Dff { clk_pos, .. } => {
                    let polarity = if *clk_pos {
                        Polarity::Pos
                    } else {
                        Polarity::Neg
                    };
                    self.cell_clock(inst, cell, polarity, &label, &mut clock);
                }
                CellKind::MemRdPort { clocked: true, .. }
                | CellKind::MemWrPort { clocked: true, .. } => {
                    self.cell_clock(inst, cell, Polarity::Pos, &label, &mut clock);
                }
                CellKind::MemWrPort { clocked: false, .. } => {
                    self.err(label, Reason::MemoryOutsideEdge, Some(cell.span));
                }
                CellKind::Dlatch => self.err(label, Reason::Latch, Some(cell.span)),
                CellKind::Tristate => self.err(label, Reason::Tristate, Some(cell.span)),
                CellKind::Blackbox(name) => self.err(
                    label,
                    Reason::UnsupportedCell(format!("blackbox {name}")),
                    Some(cell.span),
                ),
                _ => {}
            }
        }
        clock
    }

    fn cell_clock(
        &mut self,
        inst: InstId,
        cell: &'d Cell,
        polarity: Polarity,
        label: &str,
        clock: &mut Option<(SigId, Polarity)>,
    ) {
        let Some(e) = cell.input("clk") else {
            self.err(
                label.to_owned(),
                Reason::UnsupportedCell("clocked cell without a clock".into()),
                Some(cell.span),
            );
            return;
        };
        let m = self.module(inst);
        match m.exprs.get(e).map(|n| &n.kind) {
            Some(ExprKind::Net(n)) => {
                let sig = self.sig_of(inst, *n);
                self.note_clock(clock, sig, polarity, label, cell.span);
            }
            _ => self.err(label.to_owned(), Reason::GeneratedClock, Some(cell.span)),
        }
    }

    fn note_clock(
        &mut self,
        clock: &mut Option<(SigId, Polarity)>,
        sig: SigId,
        polarity: Polarity,
        label: &str,
        span: Span,
    ) {
        match clock {
            None => *clock = Some((sig, polarity)),
            Some((s, p)) if *s == sig && *p == polarity => {}
            Some((s, p)) => {
                let other = format!(
                    "{} ({})",
                    self.sig_name(*s),
                    if *p == Polarity::Pos {
                        "rising"
                    } else {
                        "falling"
                    }
                );
                self.err(label.to_owned(), Reason::SeveralClocks(other), Some(span));
            }
        }
    }

    /// Rejects `wait` and delayed assignments anywhere in a block.
    fn check_timing(&mut self, label: &str, body: &'d Block) {
        let mut bad: Option<Span> = None;
        crate::ir::walk::walk_block(body, &mut |stmt| {
            let hit = match &stmt.kind {
                StmtKind::Wait(_) => true,
                StmtKind::Assign { delay, .. } => delay.is_some(),
                StmtKind::Forever { .. } => true,
                _ => false,
            };
            if hit && bad.is_none() {
                bad = Some(stmt.span);
            }
        });
        if let Some(span) = bad {
            self.err(label.to_owned(), Reason::TimingControl, Some(span));
        }
    }

    // -- units -----------------------------------------------------------

    fn collect_units(&mut self) -> Vec<Unit> {
        let mut units = Vec::new();
        for d in 0..self.sim.drivers.len() {
            let did = DriverId::at(d);
            let inst = self.sim.drivers[d].inst;
            let target = self.sim.drivers[d].target.clone();
            let source = self.sim.drivers[d].source;
            let delay = self.sim.drivers[d].delay;
            if let DriverSource::Cell(c) = source
                && !self.sim.cells[c.idx()].cell.kind.is_combinational()
                && !matches!(
                    self.sim.cells[c.idx()].cell.kind,
                    CellKind::MemRdPort { clocked: false, .. }
                )
            {
                continue;
            }
            let mut writes = Vec::new();
            self.target_writes(&target, &mut writes);
            let label = match writes.first() {
                Some((sig, _)) => format!("driver of {}", self.sig_name(*sig)),
                None => "driver".to_owned(),
            };
            let span = self.driver_span(inst, source, &target);
            if delay != 0 {
                self.err(label.clone(), Reason::TimingControl, span);
            }
            let mut nets = BTreeSet::new();
            let mut mems = BTreeSet::new();
            let m = self.module(inst);
            let mut exprs = Vec::new();
            target.exprs(&mut exprs);
            for e in exprs {
                expr_reads(m, e, &mut nets, &mut mems);
            }
            match source {
                DriverSource::Expr(e) => expr_reads(m, e, &mut nets, &mut mems),
                DriverSource::Sig(_) => {}
                DriverSource::Cell(c) => {
                    for (_, e) in &self.sim.cells[c.idx()].cell.inputs {
                        expr_reads(m, *e, &mut nets, &mut mems);
                    }
                }
            }
            let mut reads: Vec<SigId> = nets.iter().map(|n| self.sig_of(inst, *n)).collect();
            if let DriverSource::Sig(s) = source {
                reads.push(s);
            }
            units.push(Unit {
                kind: UnitKind::Driver(did),
                inst,
                reads,
                writes,
                label,
                span: span.unwrap_or(self.sim.instances[inst.idx()].m.span),
            });
        }
        for p in 0..self.sim.procs.len() {
            let process: &'d Process = self.sim.procs[p].process;
            if !matches!(process.kind, ProcessKind::Comb | ProcessKind::Sensitive(_)) {
                continue;
            }
            let inst = self.sim.procs[p].inst;
            let label = format!("process {}", self.sim.procs[p].name);
            let mut nets = BTreeSet::new();
            let mut mems = BTreeSet::new();
            block_reads(self.module(inst), &process.body, &mut nets, &mut mems);
            let reads = nets.iter().map(|n| self.sig_of(inst, *n)).collect();
            let writes = self.block_writes(inst, &process.body);
            units.push(Unit {
                kind: UnitKind::Proc(ProcId::at(p)),
                inst,
                reads,
                writes,
                label,
                span: process.span,
            });
        }
        units
    }

    fn driver_span(&self, inst: InstId, source: DriverSource, target: &Target) -> Option<Span> {
        let m = self.module(inst);
        match source {
            DriverSource::Expr(e) => m.exprs.get(e).map(|n| n.span),
            DriverSource::Cell(c) => Some(self.sim.cells[c.idx()].cell.span),
            DriverSource::Sig(_) => {
                let mut sigs = Vec::new();
                target.sigs(&mut sigs);
                sigs.first().map(|s| self.sim.signals[s.idx()].span)
            }
        }
    }

    fn target_writes(&self, target: &Target, out: &mut Vec<(SigId, Option<(u32, u32)>)>) {
        match target {
            Target::Sig { sig, lo, width } => out.push((*sig, Some((*lo, *width)))),
            Target::Index { sig, .. } => out.push((*sig, None)),
            Target::Mem { .. } => {}
            Target::Concat(parts) => parts.iter().for_each(|p| self.target_writes(p, out)),
        }
    }

    fn block_writes(&self, inst: InstId, body: &Block) -> Vec<(SigId, Option<(u32, u32)>)> {
        let mut out = Vec::new();
        crate::ir::walk::walk_block(body, &mut |stmt| match &stmt.kind {
            StmtKind::Assign { target, .. } => {
                let t = self.sim.target_from_lvalue(inst, target);
                self.target_writes(&t, &mut out);
            }
            StmtKind::For { init, step, .. } => {
                for (lv, _) in init.iter().chain(step.iter()) {
                    let t = self.sim.target_from_lvalue(inst, lv);
                    self.target_writes(&t, &mut out);
                }
            }
            _ => {}
        });
        out
    }

    /// Topologically orders the combinational units, reporting every cycle
    /// and every multiply-driven bit.
    fn sort_units(&mut self, units: &[Unit]) -> Vec<usize> {
        let mut writers: BTreeMap<SigId, Vec<usize>> = BTreeMap::new();
        for (i, u) in units.iter().enumerate() {
            for (sig, _) in &u.writes {
                let list = writers.entry(*sig).or_default();
                if !list.contains(&i) {
                    list.push(i);
                }
            }
        }
        for (sig, list) in &writers {
            for (a, ua) in list.iter().enumerate() {
                for ub in &list[a + 1..] {
                    let conflict =
                        units[*ua]
                            .writes
                            .iter()
                            .filter(|(s, _)| s == sig)
                            .any(|(_, ra)| {
                                units[*ub]
                                    .writes
                                    .iter()
                                    .filter(|(s, _)| s == sig)
                                    .any(|(_, rb)| overlaps(*ra, *rb))
                            });
                    if conflict {
                        let name = self.sig_name(*sig);
                        let span = self.sim.signals[sig.idx()].span;
                        self.err(name, Reason::MultiplyDriven, Some(span));
                    }
                }
            }
        }
        // A process may read a net it writes, as long as it assigned it
        // first — that is a local temporary, and `read_sig` catches the
        // genuine case. A *driver* has no statements, so reading what it
        // drives is always a loop.
        for u in units {
            if matches!(u.kind, UnitKind::Driver(_))
                && u.reads.iter().any(|r| u.writes.iter().any(|(w, _)| w == r))
            {
                self.err(u.label.clone(), Reason::CombinationalLoop, Some(u.span));
            }
        }
        let n = units.len();
        let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut indeg = vec![0usize; n];
        for (i, u) in units.iter().enumerate() {
            let mut seen = BTreeSet::new();
            for sig in &u.reads {
                for w in writers.get(sig).map(Vec::as_slice).unwrap_or(&[]) {
                    if *w != i && seen.insert((*w, i)) {
                        succ[*w].push(i);
                        indeg[i] += 1;
                    }
                }
            }
        }
        let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<usize>> = (0..n)
            .filter(|i| indeg[*i] == 0)
            .map(std::cmp::Reverse)
            .collect();
        let mut order = Vec::with_capacity(n);
        while let Some(std::cmp::Reverse(i)) = ready.pop() {
            order.push(i);
            for j in succ[i].clone() {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    ready.push(std::cmp::Reverse(j));
                }
            }
        }
        if order.len() != n {
            for (i, u) in units.iter().enumerate() {
                if indeg[i] > 0 {
                    self.err(u.label.clone(), Reason::CombinationalLoop, Some(u.span));
                }
            }
        }
        order
    }

    // -- allocation ------------------------------------------------------

    /// Finds the state elements and gives them adjacent current and next
    /// blocks, so the commit is one `copy_within`.
    fn allocate_state(&mut self) {
        let mut sigs: BTreeSet<SigId> = BTreeSet::new();
        for p in 0..self.sim.procs.len() {
            let process: &'d Process = self.sim.procs[p].process;
            if !matches!(process.kind, ProcessKind::Sequential { .. }) {
                continue;
            }
            let inst = self.sim.procs[p].inst;
            for (sig, _) in self.block_writes(inst, &process.body) {
                sigs.insert(sig);
            }
        }
        for c in 0..self.sim.cells.len() {
            let inst = self.sim.cells[c].inst;
            let cell: &'d Cell = self.sim.cells[c].cell;
            let port = match &cell.kind {
                CellKind::Dff { .. } => "q",
                CellKind::MemRdPort { clocked: true, .. } => "data",
                _ => continue,
            };
            if let Some(net) = cell.output(port) {
                sigs.insert(self.sig_of(inst, net));
            }
        }
        let sigs: Vec<SigId> = sigs.into_iter().collect();
        let mut len = 0usize;
        for sig in &sigs {
            len += words::words_for(self.sig_width(*sig));
        }
        // Current block at 0, next block right after it.
        for (i, sig) in sigs.iter().enumerate() {
            let width = self.sig_width(*sig);
            let signed = self.sig_signed(*sig);
            let v = self.alloc(width, signed);
            let cur = self.slot(v);
            let next = Slot {
                off: cur.off + u32::try_from(len).expect("state block fits"),
                ..cur
            };
            self.env[sig.idx()] = Some(v);
            self.state_of[sig.idx()] = Some(i);
            self.state_sigs.push(*sig);
            self.state_cur.push(cur);
            self.state_next.push(next);
            let value = self.sim.signals[sig.idx()].value.clone();
            if value.has_unknown() && !self.zero_init {
                let name = self.sig_name(*sig);
                let span = self.sim.signals[sig.idx()].span;
                self.err(name, Reason::UninitialisedState, Some(span));
            }
            let words = known_words(&value.resize(width));
            let off = cur.off as usize;
            for (k, w) in words.iter().enumerate() {
                self.fold.regs[off + k] = *w;
            }
        }
        // Reserve the next-state block.
        self.cursor += len;
        self.fold.regs.resize(self.cursor, 0);
    }

    fn allocate_memories(&mut self) {
        let mut signed = vec![false; self.sim.memories.len()];
        for inst in 0..self.sim.instances.len() {
            let m = self.sim.instances[inst].m;
            for (mid, mem) in m.memories.iter() {
                let global = self.sim.instances[inst].mems[mid.index()];
                signed[global.idx()] = mem.elem.is_signed();
            }
        }
        for (i, mem) in self.sim.memories.iter().enumerate() {
            let elem_width = mem.elem_width;
            let elem_words = words::words_for(elem_width);
            let mut init = vec![0u64; elem_words * mem.data.len()];
            let mut unknown = false;
            for (e, value) in mem.data.iter().enumerate() {
                unknown |= value.has_unknown();
                for (k, w) in known_words(&value.resize(elem_width)).iter().enumerate() {
                    init[e * elem_words + k] = *w;
                }
            }
            if unknown && !self.zero_init {
                self.errors.push(Ineligible::new(
                    mem.name.clone(),
                    Reason::UninitialisedState,
                    None,
                ));
            }
            self.fold_prog.mems.push(MemLayout {
                elem_words,
                elem_width,
                signed: signed[i],
                len: mem.data.len(),
                init,
            });
        }
        self.fold.mems = self.fold_prog.mems.iter().map(|m| m.init.clone()).collect();
    }

    /// Gives every signal nothing drives a settable slot, seeded with its
    /// value at time zero.
    fn allocate_inputs(&mut self, units: &[Unit]) {
        let mut driven = vec![false; self.sim.signals.len()];
        let mut read = vec![false; self.sim.signals.len()];
        for u in units {
            for (sig, _) in &u.writes {
                driven[sig.idx()] = true;
            }
            for sig in &u.reads {
                read[sig.idx()] = true;
            }
        }
        for p in 0..self.sim.procs.len() {
            let process: &'d Process = self.sim.procs[p].process;
            if !matches!(process.kind, ProcessKind::Sequential { .. }) {
                continue;
            }
            let inst = self.sim.procs[p].inst;
            let mut nets = BTreeSet::new();
            let mut mems = BTreeSet::new();
            block_reads(self.module(inst), &process.body, &mut nets, &mut mems);
            for n in nets {
                read[self.sig_of(inst, n).idx()] = true;
            }
        }
        for c in 0..self.sim.cells.len() {
            let inst = self.sim.cells[c].inst;
            let cell: &'d Cell = self.sim.cells[c].cell;
            let m = self.module(inst);
            for (_, e) in &cell.inputs {
                let mut nets = BTreeSet::new();
                let mut mems = BTreeSet::new();
                expr_reads(m, *e, &mut nets, &mut mems);
                for n in nets {
                    read[self.sig_of(inst, n).idx()] = true;
                }
            }
        }
        let top_ports: BTreeSet<SigId> = self.sim.instances[0]
            .m
            .ports
            .iter()
            .map(|p| self.sim.instances[0].nets[p.net.index()])
            .collect();
        for i in 0..self.sim.signals.len() {
            let sig = SigId::at(i);
            if self.state_of[i].is_some() {
                if driven[i] {
                    let name = self.sig_name(sig);
                    let span = self.sim.signals[i].span;
                    self.err(name, Reason::MultiplyDriven, Some(span));
                }
                continue;
            }
            if driven[i] {
                self.covered[i] = Some(empty_mask(self.sig_width(sig)));
                let width = self.sig_width(sig);
                let signed = self.sig_signed(sig);
                let base = self.zero(width, signed);
                self.env[i] = Some(base);
                continue;
            }
            let value = self.sim.signals[i].value.clone();
            if read[i] && value.has_unknown() && !top_ports.contains(&sig) && !self.zero_init {
                let name = self.sig_name(sig);
                let span = self.sim.signals[i].span;
                self.err(name, Reason::UndrivenNet, Some(span));
                continue;
            }
            let width = self.sig_width(sig);
            let signed = self.sig_signed(sig);
            let v = self.alloc(width, signed);
            let off = self.slot(v).off as usize;
            for (k, w) in known_words(&value.resize(width)).iter().enumerate() {
                self.fold.regs[off + k] = *w;
            }
            self.env[i] = Some(v);
        }
    }

    /// Rejects a net one clocked process writes blockingly and another
    /// clocked unit reads.
    ///
    /// The event simulator makes a blocking write visible the instant it
    /// happens, so whether the reader sees the old or the new value
    /// depends on which process the scheduler runs first. Compiled mode
    /// has no such order — every clocked unit reads the committed state —
    /// so a design that depends on one is refused rather than given the
    /// answer compiled mode happens to produce.
    fn check_blocking_races(&mut self) {
        let mut writers: BTreeMap<SigId, (usize, Span)> = BTreeMap::new();
        for p in 0..self.sim.procs.len() {
            let process: &'d Process = self.sim.procs[p].process;
            if !matches!(process.kind, ProcessKind::Sequential { .. }) {
                continue;
            }
            let inst = self.sim.procs[p].inst;
            for sig in self.blocking_writes(inst, &process.body) {
                writers.entry(sig).or_insert((p, process.span));
            }
        }
        if writers.is_empty() {
            return;
        }
        let mut reads: Vec<(usize, BTreeSet<SigId>)> = Vec::new();
        for p in 0..self.sim.procs.len() {
            let process: &'d Process = self.sim.procs[p].process;
            if !matches!(process.kind, ProcessKind::Sequential { .. }) {
                continue;
            }
            let inst = self.sim.procs[p].inst;
            let mut nets = BTreeSet::new();
            let mut mems = BTreeSet::new();
            block_reads(self.module(inst), &process.body, &mut nets, &mut mems);
            reads.push((p, nets.iter().map(|n| self.sig_of(inst, *n)).collect()));
        }
        // A clocked cell reads at the edge too.
        let mut cell_reads: BTreeSet<SigId> = BTreeSet::new();
        for c in 0..self.sim.cells.len() {
            let inst = self.sim.cells[c].inst;
            let cell: &'d Cell = self.sim.cells[c].cell;
            if cell.kind.is_combinational() {
                continue;
            }
            let m = self.module(inst);
            for (_, e) in &cell.inputs {
                let mut nets = BTreeSet::new();
                let mut mems = BTreeSet::new();
                expr_reads(m, *e, &mut nets, &mut mems);
                for n in nets {
                    cell_reads.insert(self.sig_of(inst, n));
                }
            }
        }
        let clashes: Vec<(SigId, Span)> = writers
            .iter()
            .filter(|(sig, (p, _))| {
                cell_reads.contains(sig) || reads.iter().any(|(q, set)| q != p && set.contains(sig))
            })
            .map(|(sig, (_, span))| (*sig, *span))
            .collect();
        for (sig, span) in clashes {
            let net = self.sig_name(sig);
            self.err(net, Reason::BlockingRace, Some(span));
        }
    }

    /// The signals a block assigns with a *blocking* assignment.
    fn blocking_writes(&self, inst: InstId, body: &Block) -> BTreeSet<SigId> {
        let mut out = BTreeSet::new();
        let mut targets = Vec::new();
        crate::ir::walk::walk_block(body, &mut |stmt| match &stmt.kind {
            StmtKind::Assign {
                target,
                kind: AssignKind::Blocking,
                ..
            } => {
                let t = self.sim.target_from_lvalue(inst, target);
                self.target_writes(&t, &mut targets);
            }
            StmtKind::For { init, step, .. } => {
                for (lv, _) in init.iter().chain(step.iter()) {
                    let t = self.sim.target_from_lvalue(inst, lv);
                    self.target_writes(&t, &mut targets);
                }
            }
            _ => {}
        });
        out.extend(targets.into_iter().map(|(sig, _)| sig));
        out
    }

    fn check_coverage(&mut self) {
        for i in 0..self.covered.len() {
            let Some(mask) = self.covered[i].clone() else {
                continue;
            };
            let width = self.sig_width(SigId::at(i));
            if !words::all_ones(&mask, width) {
                let name = self.sig_name(SigId::at(i));
                let span = self.sim.signals[i].span;
                self.err(name, Reason::PartiallyDriven, Some(span));
            }
        }
    }

    // -- unit lowering ---------------------------------------------------

    fn new_ctx(&self, inst: InstId, seq: bool, label: String, span: Span) -> Ctx<'d> {
        Ctx {
            inst,
            m: self.module(inst),
            seq,
            hold: seq,
            prologue: false,
            force: BTreeMap::new(),
            guard: None,
            cur: BTreeMap::new(),
            def: BTreeMap::new(),
            nba: BTreeMap::new(),
            nba_def: BTreeMap::new(),
            wrote: BTreeSet::new(),
            wrote_nba: BTreeSet::new(),
            writes: BTreeSet::new(),
            label,
            span,
        }
    }

    /// Seeds the process-local view: a combinational unit starts with
    /// nothing defined, a clocked one with the register's current value.
    fn seed(&mut self, ctx: &mut Ctx<'d>, writes: &[(SigId, Option<(u32, u32)>)]) {
        for (sig, _) in writes {
            if !ctx.writes.insert(*sig) {
                continue;
            }
            let width = self.sig_width(*sig);
            let base = self.env[sig.idx()].expect("every signal has a value");
            ctx.cur.insert(*sig, base);
            ctx.nba.insert(*sig, base);
            // A clocked unit holds what no path assigns, so its
            // blocking view starts fully defined; a combinational one
            // latches, so its starts empty and the coverage check is what
            // reports the latch.
            let mask = if ctx.hold {
                full_mask(width)
            } else {
                empty_mask(width)
            };
            ctx.def.insert(*sig, mask);
            ctx.nba_def.insert(*sig, empty_mask(width));
        }
    }

    fn lower_unit(&mut self, unit: &Unit) {
        match unit.kind {
            UnitKind::Driver(d) => self.lower_driver(unit, d),
            UnitKind::Proc(p) => self.lower_comb_process(unit, p),
        }
    }

    fn lower_driver(&mut self, unit: &Unit, d: DriverId) {
        let source = self.sim.drivers[d.idx()].source;
        let target = self.sim.drivers[d.idx()].target.clone();
        let mut ctx = self.new_ctx(unit.inst, false, unit.label.clone(), unit.span);
        let value = match source {
            DriverSource::Expr(e) => self.expr(&mut ctx, e),
            DriverSource::Sig(s) => match self.env[s.idx()] {
                Some(v) => v,
                None => {
                    self.err(
                        unit.label.clone(),
                        Reason::CombinationalLoop,
                        Some(unit.span),
                    );
                    return;
                }
            },
            DriverSource::Cell(c) => match self.cell_value(&mut ctx, c) {
                Some(v) => v,
                None => return,
            },
        };
        self.drive_target(&mut ctx, &target, value);
    }

    /// Writes a driver's value into its target, tracking which bits of
    /// each signal are now driven.
    fn drive_target(&mut self, ctx: &mut Ctx<'d>, target: &Target, value: ValId) {
        match target {
            Target::Sig { sig, lo, width } => {
                let full = self.sig_width(*sig);
                let signed = self.sig_signed(*sig);
                let part = self.resize(ctx.seq, value, *width, signed);
                let new = if *lo == 0 && *width == full {
                    self.resize(ctx.seq, part, full, signed)
                } else {
                    let base = self.env[sig.idx()].expect("driven signal has a base");
                    self.insert(ctx.seq, base, part, i64::from(*lo), full, signed)
                };
                self.env[sig.idx()] = Some(new);
                if let Some(mask) = &mut self.covered[sig.idx()] {
                    mark(mask, *lo, *width);
                }
            }
            Target::Index {
                sig,
                index,
                elem,
                count,
            } => {
                let full = self.sig_width(*sig);
                let signed = self.sig_signed(*sig);
                let idx = self.expr(ctx, *index);
                let part = self.resize(ctx.seq, value, *elem, false);
                let base = self.env[sig.idx()].expect("driven signal has a base");
                let new = self.dyn_insert(ctx.seq, base, part, idx, *elem, *count, full, signed);
                self.env[sig.idx()] = Some(new);
                // Which element it drove is not known until it runs, so
                // no bit of the net counts as driven and the coverage
                // check reports the rest as undriven.
            }
            Target::Mem { .. } => {
                self.err(ctx.label.clone(), Reason::MemoryOutsideEdge, Some(ctx.span));
            }
            Target::Concat(parts) => {
                let total: u32 = parts.iter().map(|p| self.target_width(p)).sum();
                let v = self.resize(ctx.seq, value, total, false);
                let mut hi = total;
                for p in parts {
                    let w = self.target_width(p);
                    hi -= w;
                    let piece = self.extract(ctx.seq, v, i64::from(hi), w);
                    self.drive_target(ctx, p, piece);
                }
            }
        }
    }

    fn target_width(&self, target: &Target) -> u32 {
        match target {
            Target::Sig { width, .. } => *width,
            Target::Index { elem, .. } => *elem,
            Target::Mem { mem, .. } => self.sim.memories[mem.idx()].elem_width,
            Target::Concat(parts) => parts.iter().map(|p| self.target_width(p)).sum(),
        }
    }

    fn lower_comb_process(&mut self, unit: &Unit, p: ProcId) {
        let process: &'d Process = self.sim.procs[p.idx()].process;
        let mut ctx = self.new_ctx(unit.inst, false, unit.label.clone(), unit.span);
        self.seed(&mut ctx, &unit.writes);
        // A combinational process may not read back what it assigns
        // non-blockingly: the event simulator would take another delta.
        self.block(&mut ctx, &process.body);
        // What the process may write, which is what it has to write on
        // every path: a bit it leaves alone on one path is latched, but a
        // bit it never touches belongs to another driver.
        let mut expected: BTreeMap<SigId, Vec<u64>> = BTreeMap::new();
        for (sig, range) in &unit.writes {
            let width = self.sig_width(*sig);
            let mask = expected.entry(*sig).or_insert_with(|| empty_mask(width));
            match range {
                Some((lo, w)) => mark(mask, *lo, *w),
                None => mark(mask, 0, width),
            }
        }
        let sigs: Vec<SigId> = ctx.writes.iter().copied().collect();
        for sig in sigs {
            let width = self.sig_width(sig);
            let want = expected
                .get(&sig)
                .cloned()
                .unwrap_or_else(|| full_mask(width));
            let nba_def = ctx.nba_def.get(&sig).cloned().unwrap_or_default();
            let def = ctx.def.get(&sig).cloned().unwrap_or_default();
            let nba_any = ctx.wrote_nba.contains(&sig);
            if nba_any && ctx.wrote.contains(&sig) {
                let name = self.sig_name(sig);
                self.err(name, Reason::MixedAssignment, Some(unit.span));
                continue;
            }
            let (value, mask) = if nba_any {
                (ctx.nba[&sig], nba_def)
            } else {
                (ctx.cur[&sig], def)
            };
            let complete = want
                .iter()
                .zip(mask.iter().chain(std::iter::repeat(&0)))
                .all(|(w, d)| w & !d == 0);
            if !complete {
                let name = self.sig_name(sig);
                let span = self.sim.signals[sig.idx()].span;
                self.err(name, Reason::Latch, Some(span));
                continue;
            }
            self.env[sig.idx()] = Some(value);
            if let Some(covered) = &mut self.covered[sig.idx()] {
                for (c, w) in covered.iter_mut().zip(&want) {
                    *c |= *w;
                }
            }
        }
    }

    // -- asynchronous resets ---------------------------------------------

    /// Emits the asynchronous-reset prologue: the operations that run at
    /// the head of the combinational region and force a register to its
    /// reset value while its reset is active.
    ///
    /// An asynchronous reset is the one thing in a synchronous design that
    /// happens *between* edges, and a cycle-based engine that waited for
    /// the edge would disagree with the event simulator about everything
    /// the reset register feeds during that cycle — including what an
    /// unreset memory captures at the very next edge. Applying it before
    /// the settle costs one `Mux` per register and makes the two engines
    /// agree. The price is that the reset must be readable before the
    /// settle, which is why it has to come from an input or a register.
    fn lower_async_resets(&mut self) {
        let mut entries = self.async_entries();
        // A reset may itself come from a register with an asynchronous
        // reset — a reset synchroniser is exactly that — so an override
        // has to be emitted after the override of whatever its condition
        // reads. Repeatedly take an entry nothing else still has to
        // produce; what is left over is a cycle among the resets.
        let mut order = Vec::with_capacity(entries.len());
        while !entries.is_empty() {
            let ready = entries.iter().position(|e| {
                !entries
                    .iter()
                    .any(|o| o.id != e.id && o.targets.contains(&e.cond))
            });
            match ready {
                Some(i) => order.push(entries.remove(i)),
                None => {
                    for e in &entries {
                        self.err(e.label.clone(), Reason::AsyncResetLogic, Some(e.span));
                    }
                    return;
                }
            }
        }
        self.in_prologue = true;
        for entry in order {
            self.emit_async_reset(&entry);
        }
        self.in_prologue = false;
    }

    /// Every asynchronous reset in the design, with the signal its
    /// condition reads and the signals it forces.
    fn async_entries(&mut self) -> Vec<AsyncReset> {
        let mut out: Vec<AsyncReset> = Vec::new();
        for p in 0..self.sim.procs.len() {
            let process: &'d Process = self.sim.procs[p].process;
            let ProcessKind::Sequential { resets, .. } = &process.kind else {
                continue;
            };
            let inst = self.sim.procs[p].inst;
            let label = format!("process {}", self.sim.procs[p].name);
            let targets: Vec<SigId> = self
                .block_writes(inst, &process.body)
                .into_iter()
                .map(|(sig, _)| sig)
                .collect();
            for edge in resets {
                if edge.polarity == Polarity::Any {
                    self.err(label.clone(), Reason::AsyncResetLogic, Some(process.span));
                    continue;
                }
                out.push(AsyncReset {
                    id: out.len(),
                    inst,
                    source: AsyncSource::Process(p),
                    cond: self.sig_of(inst, edge.net),
                    polarity: edge.polarity,
                    targets: targets.clone(),
                    label: label.clone(),
                    span: process.span,
                });
            }
        }
        for c in 0..self.sim.cells.len() {
            let inst = self.sim.cells[c].inst;
            let cell: &'d Cell = self.sim.cells[c].cell;
            let CellKind::Dff { reset: Some(r), .. } = &cell.kind else {
                continue;
            };
            if !r.asynchronous {
                continue;
            }
            let label = format!("cell {}.{}", self.sim.instances[inst.idx()].path, cell.name);
            let Some(q) = cell.output("q") else { continue };
            let m = self.module(inst);
            let rst = cell.input("rst").and_then(|e| m.exprs.get(e));
            let Some(ExprKind::Net(n)) = rst.map(|node| &node.kind) else {
                self.err(label, Reason::AsyncResetLogic, Some(cell.span));
                continue;
            };
            out.push(AsyncReset {
                id: out.len(),
                inst,
                source: AsyncSource::Cell(c),
                cond: self.sig_of(inst, *n),
                polarity: if r.active_high {
                    Polarity::Pos
                } else {
                    Polarity::Neg
                },
                targets: vec![self.sig_of(inst, q)],
                label,
                span: cell.span,
            });
        }
        out
    }

    /// Emits one reset's override: the condition, the value it forces,
    /// and the `Mux` that applies it to the register's current state.
    fn emit_async_reset(&mut self, entry: &AsyncReset) {
        let Some(active) =
            self.reset_condition(entry.cond, entry.polarity, &entry.label, entry.span)
        else {
            return;
        };
        match entry.source {
            AsyncSource::Cell(c) => {
                let cell: &'d Cell = self.sim.cells[c].cell;
                let CellKind::Dff { reset: Some(r), .. } = &cell.kind else {
                    return;
                };
                let Some(&target) = entry.targets.first() else {
                    return;
                };
                let width = self.sig_width(target);
                let signed = self.sig_signed(target);
                let value = self.constant(&r.value.resize(width).with_signed(signed));
                self.force_on_reset(target, active, value);
            }
            AsyncSource::Process(p) => {
                let process: &'d Process = self.sim.procs[p].process;
                // Run the body with the reset forced active: the branch
                // folds, so only the reset arm is lowered, and what it
                // leaves in the local view is the reset value.
                let width = self.sig_width(entry.cond);
                let level =
                    self.constant(&Logic::from_bool(entry.polarity == Polarity::Pos).resize(width));
                let writes = self.block_writes(entry.inst, &process.body);
                let mut ctx = self.new_ctx(entry.inst, false, entry.label.clone(), entry.span);
                ctx.hold = true;
                ctx.prologue = true;
                ctx.force.insert(entry.cond, level);
                self.seed(&mut ctx, &writes);
                self.block(&mut ctx, &process.body);
                let targets: Vec<SigId> = ctx.writes.iter().copied().collect();
                for target in targets {
                    let seeded = self.env[target.idx()];
                    let nba = ctx.nba.get(&target).copied();
                    let cur = ctx.cur.get(&target).copied();
                    let value = match (nba, cur) {
                        (Some(v), _) if Some(v) != seeded => v,
                        (_, Some(v)) if Some(v) != seeded => v,
                        _ => continue,
                    };
                    self.force_on_reset(target, active, value);
                }
            }
        }
    }

    /// The one-bit "this reset is active" value, or `None` when the reset
    /// comes from logic the prologue cannot read.
    fn reset_condition(
        &mut self,
        sig: SigId,
        polarity: Polarity,
        label: &str,
        span: Span,
    ) -> Option<ValId> {
        if polarity == Polarity::Any || self.covered[sig.idx()].is_some() {
            self.err(label.to_owned(), Reason::AsyncResetLogic, Some(span));
            return None;
        }
        let value = self.env[sig.idx()]?;
        let level = self.truth(false, value);
        Some(if polarity == Polarity::Pos {
            level
        } else {
            self.emit(
                false,
                tag::unary(UnaryOp::LogicNot),
                &[],
                &[level],
                1,
                false,
                true,
                |dst, a| Op::Unary {
                    op: UnaryOp::LogicNot,
                    dst,
                    a: a[0],
                },
            )
        })
    }

    /// Emits `cur = active ? value : cur` at the head of the combinational
    /// region. The destination is a state slot, not a fresh one, so this
    /// is the one place the program is not in single assignment form —
    /// and it is safe because the op reads the slot before it writes it.
    fn force_on_reset(&mut self, target: SigId, active: ValId, value: ValId) {
        let Some(idx) = self.state_of[target.idx()] else {
            return;
        };
        let cur = self.state_cur[idx];
        let value = self.resize(false, value, cur.width, cur.signed);
        let slot = self.slot(value);
        if slot == cur {
            return;
        }
        self.comb.push(Op::Mux {
            dst: cur,
            s: self.slot(active),
            a: cur,
            b: slot,
        });
    }

    /// The one-bit truth value of an already-lowered value.
    fn truth(&mut self, seq: bool, v: ValId) -> ValId {
        if self.slot(v).width == 1 {
            return self.alias(v, false);
        }
        self.emit(
            seq,
            tag::unary(UnaryOp::ReduceOr),
            &[],
            &[v],
            1,
            false,
            true,
            |dst, a| Op::Unary {
                op: UnaryOp::ReduceOr,
                dst,
                a: a[0],
            },
        )
    }

    // -- the clock edge --------------------------------------------------

    fn lower_sequential(&mut self) {
        let mut next: Vec<Option<ValId>> = vec![None; self.state_sigs.len()];
        for p in 0..self.sim.procs.len() {
            let process: &'d Process = self.sim.procs[p].process;
            if !matches!(process.kind, ProcessKind::Sequential { .. }) {
                continue;
            }
            let inst = self.sim.procs[p].inst;
            let label = format!("process {}", self.sim.procs[p].name);
            let writes = self.block_writes(inst, &process.body);
            let mut ctx = self.new_ctx(inst, true, label.clone(), process.span);
            self.seed(&mut ctx, &writes);
            self.block(&mut ctx, &process.body);
            let sigs: Vec<SigId> = ctx.writes.iter().copied().collect();
            for sig in sigs {
                // A non-blocking write wins over a blocking one, since it
                // lands last; both to the same net in one process is a
                // race with itself, and the base a partial non-blocking
                // write would build on is not one this form can name.
                if ctx.wrote_nba.contains(&sig) && ctx.wrote.contains(&sig) {
                    let name = self.sig_name(sig);
                    self.err(name, Reason::MixedAssignment, Some(process.span));
                    continue;
                }
                let value = if ctx.wrote_nba.contains(&sig) {
                    ctx.nba[&sig]
                } else {
                    ctx.cur[&sig]
                };
                let Some(idx) = self.state_of[sig.idx()] else {
                    continue;
                };
                if next[idx].is_some() {
                    let name = self.sig_name(sig);
                    self.err(name, Reason::MultiplyDriven, Some(process.span));
                }
                next[idx] = Some(value);
            }
        }
        for c in 0..self.sim.cells.len() {
            self.lower_seq_cell(c, &mut next);
        }
        for (i, value) in next.into_iter().enumerate() {
            let cur = self.state_cur[i];
            let dst = self.state_next[i];
            let src = value.map_or(cur, |v| self.slot(v));
            self.seq.push(Op::Resize { dst, a: src });
        }
    }

    fn lower_seq_cell(&mut self, c: usize, next: &mut [Option<ValId>]) {
        let inst = self.sim.cells[c].inst;
        let cell: &'d Cell = self.sim.cells[c].cell;
        let label = format!("cell {}.{}", self.sim.instances[inst.idx()].path, cell.name);
        let mut ctx = self.new_ctx(inst, true, label.clone(), cell.span);
        let input = |low: &mut Self, ctx: &mut Ctx<'d>, port: &str| -> Option<ValId> {
            cell.input(port).map(|e| low.expr(ctx, e))
        };
        match &cell.kind {
            CellKind::Dff {
                has_enable, reset, ..
            } => {
                let Some(q) = cell.output("q") else {
                    return;
                };
                let sig = self.sig_of(inst, q);
                let width = self.sig_width(sig);
                let signed = self.sig_signed(sig);
                let cur = self.env[sig.idx()].expect("a flip-flop output is state");
                let d = input(self, &mut ctx, "d").unwrap_or(cur);
                let mut value = self.resize(true, d, width, signed);
                if *has_enable && let Some(en) = input(self, &mut ctx, "en") {
                    value = self.mux(true, en, cur, value, width, signed);
                }
                if let Some(rst) = reset {
                    let rv = self.constant(&rst.value.resize(width).with_signed(signed));
                    if let Some(r) = input(self, &mut ctx, "rst") {
                        let active = if rst.active_high {
                            r
                        } else {
                            self.emit(
                                true,
                                tag::unary(UnaryOp::LogicNot),
                                &[],
                                &[r],
                                1,
                                false,
                                true,
                                |dst, a| Op::Unary {
                                    op: UnaryOp::LogicNot,
                                    dst,
                                    a: a[0],
                                },
                            )
                        };
                        value = self.mux(true, active, value, rv, width, signed);
                    }
                }
                let Some(idx) = self.state_of[sig.idx()] else {
                    return;
                };
                if next[idx].is_some() {
                    self.err(self.sig_name(sig), Reason::MultiplyDriven, Some(cell.span));
                }
                next[idx] = Some(value);
            }
            CellKind::MemRdPort { mem, clocked: true } => {
                let Some(out) = cell.output("data") else {
                    return;
                };
                let sig = self.sig_of(inst, out);
                let width = self.sig_width(sig);
                let signed = self.sig_signed(sig);
                let cur = self.env[sig.idx()].expect("a registered read is state");
                let Some(addr) = input(self, &mut ctx, "addr") else {
                    return;
                };
                let mid = self.mem_of(inst, *mem);
                let read = self.mem_read(true, mid, addr, width, signed);
                let value = match input(self, &mut ctx, "en") {
                    Some(en) => self.mux(true, en, cur, read, width, signed),
                    None => read,
                };
                let Some(idx) = self.state_of[sig.idx()] else {
                    return;
                };
                next[idx] = Some(value);
            }
            CellKind::MemWrPort { mem, clocked: true } => {
                let (Some(addr), Some(data), Some(en)) = (
                    input(self, &mut ctx, "addr"),
                    input(self, &mut ctx, "data"),
                    input(self, &mut ctx, "en"),
                ) else {
                    return;
                };
                let mid = self.mem_of(inst, *mem);
                let layout = &self.fold_prog.mems[mid.idx()];
                let (ew, signed) = (layout.elem_width, layout.signed);
                let data = self.resize(true, data, ew, signed);
                self.seq.push(Op::MemWrite {
                    mem: u32::try_from(mid.idx()).expect("memory index fits"),
                    addr: self.slot(addr),
                    data: self.slot(data),
                    en: Some(self.slot(en)),
                });
            }
            _ => {}
        }
    }

    // -- statements ------------------------------------------------------

    fn block(&mut self, ctx: &mut Ctx<'d>, body: &'d Block) {
        for stmt in body {
            self.stmt(ctx, stmt);
        }
    }

    fn stmt(&mut self, ctx: &mut Ctx<'d>, stmt: &'d Stmt) {
        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                kind,
                delay,
            } => {
                if delay.is_some() {
                    self.err(ctx.label.clone(), Reason::TimingControl, Some(stmt.span));
                    return;
                }
                let v = self.expr(ctx, *value);
                let nba = *kind == AssignKind::NonBlocking;
                self.assign(ctx, target, v, nba);
            }
            StmtKind::If { cond, then_, else_ } => {
                let c = self.condition(ctx, *cond);
                self.branch(ctx, c, &mut |s, ctx| s.block(ctx, then_), &mut |s, ctx| {
                    s.block(ctx, else_)
                });
            }
            StmtKind::Case {
                subject,
                kind,
                arms,
                default,
                ..
            } => {
                let sv = self.expr(ctx, *subject);
                self.case_from(ctx, sv, *kind, arms, default.as_ref(), 0, stmt.span);
            }
            StmtKind::Block { body, .. } => self.block(ctx, body),
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some((lv, e)) = init {
                    let v = self.expr(ctx, *e);
                    self.assign(ctx, lv, v, false);
                }
                let mut n = 0u32;
                loop {
                    match cond {
                        Some(c) => match self.static_truth(ctx, *c) {
                            Some(true) => {}
                            Some(false) => break,
                            None => {
                                self.err(
                                    ctx.label.clone(),
                                    Reason::DynamicLoopBound,
                                    Some(stmt.span),
                                );
                                return;
                            }
                        },
                        None => {
                            self.err(ctx.label.clone(), Reason::DynamicLoopBound, Some(stmt.span));
                            return;
                        }
                    }
                    self.block(ctx, body);
                    if let Some((lv, e)) = step {
                        let v = self.expr(ctx, *e);
                        self.assign(ctx, lv, v, false);
                    }
                    n += 1;
                    if n >= MAX_UNROLL {
                        self.err(ctx.label.clone(), Reason::DynamicLoopBound, Some(stmt.span));
                        return;
                    }
                }
            }
            StmtKind::While { cond, body } => {
                let mut n = 0u32;
                loop {
                    match self.static_truth(ctx, *cond) {
                        Some(true) => {}
                        Some(false) => break,
                        None => {
                            self.err(ctx.label.clone(), Reason::DynamicLoopBound, Some(stmt.span));
                            return;
                        }
                    }
                    self.block(ctx, body);
                    n += 1;
                    if n >= MAX_UNROLL {
                        self.err(ctx.label.clone(), Reason::DynamicLoopBound, Some(stmt.span));
                        return;
                    }
                }
            }
            StmtKind::Repeat { count, body } => {
                let v = self.expr(ctx, *count);
                let Some(c) = self.as_const(v).and_then(|l| l.to_u64()) else {
                    self.err(ctx.label.clone(), Reason::DynamicLoopBound, Some(stmt.span));
                    return;
                };
                if c > u64::from(MAX_UNROLL) {
                    self.err(ctx.label.clone(), Reason::DynamicLoopBound, Some(stmt.span));
                    return;
                }
                for _ in 0..c {
                    self.block(ctx, body);
                }
            }
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                if !ctx.seq {
                    self.err(
                        ctx.label.clone(),
                        Reason::MemoryOutsideEdge,
                        Some(stmt.span),
                    );
                    return;
                }
                let a = self.expr(ctx, *addr);
                let v = self.expr(ctx, *value);
                let mid = self.mem_of(ctx.inst, *mem);
                let layout = &self.fold_prog.mems[mid.idx()];
                let (ew, signed) = (layout.elem_width, layout.signed);
                let v = self.resize(ctx.seq, v, ew, signed);
                let en = match enable {
                    Some(e) => {
                        let c = self.condition(ctx, *e);
                        Some(self.and_guard(ctx.seq, ctx.guard, c))
                    }
                    None => ctx.guard,
                };
                self.seq.push(Op::MemWrite {
                    mem: u32::try_from(mid.idx()).expect("memory index fits"),
                    addr: self.slot(a),
                    data: self.slot(v),
                    en: en.map(|g| self.slot(g)),
                });
            }
            // The asynchronous-reset prologue re-runs the body with the
            // reset forced, so its side effects would fire a second time.
            StmtKind::SysCall { .. }
            | StmtKind::Assert { .. }
            | StmtKind::Finish
            | StmtKind::Stop
                if ctx.prologue => {}
            StmtKind::SysCall { name, args } => self.syscall(ctx, name.as_str(), args, stmt.span),
            StmtKind::Assert {
                cond,
                severity,
                message,
            } => {
                let c = self.condition(ctx, *cond);
                let msg = self.message(ctx, message, 'd', false);
                let guard = ctx.guard.map(|g| self.slot(g));
                self.side.push(Op::Assert {
                    guard,
                    cond: self.slot(c),
                    msg,
                    severity: *severity,
                    span: stmt.span,
                });
            }
            StmtKind::Finish => {
                let guard = ctx.guard.map(|g| self.slot(g));
                self.side.push(Op::Finish { guard });
            }
            StmtKind::Stop => {
                let guard = ctx.guard.map(|g| self.slot(g));
                self.side.push(Op::Stop { guard });
            }
            StmtKind::Wait(WaitKind::Delay(_) | WaitKind::Event(_) | WaitKind::Until(_)) => {
                self.err(ctx.label.clone(), Reason::TimingControl, Some(stmt.span));
            }
            StmtKind::Forever { .. } => {
                self.err(
                    ctx.label.clone(),
                    Reason::UnsupportedStatement("forever"),
                    Some(stmt.span),
                );
            }
            StmtKind::Break => self.err(
                ctx.label.clone(),
                Reason::UnsupportedStatement("break"),
                Some(stmt.span),
            ),
            StmtKind::Continue => self.err(
                ctx.label.clone(),
                Reason::UnsupportedStatement("continue"),
                Some(stmt.span),
            ),
        }
    }

    fn syscall(&mut self, ctx: &mut Ctx<'d>, name: &str, args: &'d [ExprId], span: Span) {
        let (radix, newline) = match name {
            "$display" => ('d', true),
            "$displayb" => ('b', true),
            "$displayo" => ('o', true),
            "$displayh" => ('h', true),
            "$write" => ('d', false),
            "$writeb" => ('b', false),
            "$writeo" => ('o', false),
            "$writeh" => ('h', false),
            "$finish" => {
                let guard = ctx.guard.map(|g| self.slot(g));
                self.side.push(Op::Finish { guard });
                return;
            }
            "$stop" => {
                let guard = ctx.guard.map(|g| self.slot(g));
                self.side.push(Op::Stop { guard });
                return;
            }
            other => {
                self.err(
                    ctx.label.clone(),
                    Reason::UnsupportedTask(other.to_owned()),
                    Some(span),
                );
                return;
            }
        };
        let msg = self.message(ctx, args, radix, newline);
        let guard = ctx.guard.map(|g| self.slot(g));
        self.side.push(Op::Display { guard, msg });
    }

    fn message(
        &mut self,
        ctx: &mut Ctx<'d>,
        args: &'d [ExprId],
        radix: char,
        newline: bool,
    ) -> u32 {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            match ctx.m.exprs.get(*a).map(|n| &n.kind) {
                Some(ExprKind::String(s)) => out.push(MsgArg::Str(s.clone())),
                _ => {
                    let v = self.expr(ctx, *a);
                    out.push(MsgArg::Val(self.slot(v)));
                }
            }
        }
        let id = u32::try_from(self.messages.len()).expect("message table fits");
        self.messages.push(Message {
            inst: ctx.inst,
            args: out,
            radix,
            newline,
        });
        id
    }

    /// A `case`, lowered as a chain of guarded merges in priority order.
    #[allow(clippy::too_many_arguments)]
    fn case_from(
        &mut self,
        ctx: &mut Ctx<'d>,
        subject: ValId,
        kind: CaseKind,
        arms: &'d [CaseArm],
        default: Option<&'d Block>,
        i: usize,
        span: Span,
    ) {
        let Some(arm) = arms.get(i) else {
            if let Some(d) = default {
                self.block(ctx, d);
            }
            return;
        };
        let mut cond: Option<ValId> = None;
        for value in &arm.values {
            let m = self.case_match(ctx, subject, kind, *value, span);
            cond = Some(match cond {
                None => m,
                Some(prev) => self.emit(
                    ctx.seq,
                    tag::binary(BinaryOp::Or),
                    &[],
                    &[prev, m],
                    1,
                    false,
                    true,
                    |dst, a| Op::Binary {
                        op: BinaryOp::Or,
                        dst,
                        a: a[0],
                        b: a[1],
                    },
                ),
            });
        }
        let Some(cond) = cond else {
            self.case_from(ctx, subject, kind, arms, default, i + 1, span);
            return;
        };
        self.branch(
            ctx,
            cond,
            &mut |s, ctx| s.block(ctx, &arm.body),
            &mut |s, ctx| s.case_from(ctx, subject, kind, arms, default, i + 1, span),
        );
    }

    /// One `case` item as a one-bit condition. A literal with `x` or `z`
    /// bits is a wildcard under `casez` / `casex` and becomes a mask, which
    /// is why such a literal is accepted here and nowhere else.
    fn case_match(
        &mut self,
        ctx: &mut Ctx<'d>,
        subject: ValId,
        kind: CaseKind,
        item: ExprId,
        span: Span,
    ) -> ValId {
        // The event simulator harmonises the subject and the item to the
        // wider of the two and resizes both, so a wide item is not
        // truncated and a narrow one's zero extension is a run of care
        // bits, not of wildcards.
        let item_width = ctx
            .m
            .exprs
            .get(item)
            .and_then(|n| n.ty.width())
            .unwrap_or(0);
        let width = self.slot(subject).width.max(item_width);
        let subject = self.resize(ctx.seq, subject, width, self.slot(subject).signed);
        let literal = ctx
            .m
            .exprs
            .get(item)
            .and_then(|n| n.as_const())
            .filter(|c| c.has_unknown())
            .map(|c| c.resize(width));
        if let Some(c) = literal {
            let care: Vec<u64> = match kind {
                CaseKind::Plain => {
                    self.err(
                        ctx.label.clone(),
                        Reason::UnknownConstant(c.to_verilog_literal()),
                        Some(span),
                    );
                    return self.zero(1, false);
                }
                // `casez` treats only `z` as a wildcard, `casex` both.
                CaseKind::Z => c
                    .unknown_words()
                    .iter()
                    .zip(c.value_words())
                    .map(|(u, v)| !(u & v))
                    .collect(),
                CaseKind::X => c.unknown_words().iter().map(|u| !u).collect(),
            };
            return self.masked_eq(ctx, subject, &c, care);
        }
        let v = self.expr(ctx, item);
        let v = self.resize(ctx.seq, v, width, self.slot(v).signed);
        self.binop(ctx, BinaryOp::Eq, subject, v)
    }

    /// `(subject & care) == (pattern & care)`, the straight-line form of a
    /// wildcard comparison against a literal.
    fn masked_eq(
        &mut self,
        ctx: &mut Ctx<'d>,
        subject: ValId,
        pattern: &Logic,
        care: Vec<u64>,
    ) -> ValId {
        let mut mask = care;
        words::mask_top(&mut mask, pattern.width());
        let zero = vec![0u64; mask.len()];
        let mask_l = Logic::from_planes(pattern.width(), false, mask.clone(), zero.clone());
        let value: Vec<u64> = known_words(pattern)
            .iter()
            .zip(&mask)
            .map(|(v, m)| v & m)
            .collect();
        let value_l = Logic::from_planes(pattern.width(), false, value, zero);
        let mv = self.constant(&mask_l);
        let vv = self.constant(&value_l);
        let masked = self.binop(ctx, BinaryOp::And, subject, mv);
        self.binop(ctx, BinaryOp::Eq, masked, vv)
    }

    /// Runs both arms of a branch on copies of the local view and merges
    /// them.
    fn branch(
        &mut self,
        ctx: &mut Ctx<'d>,
        cond: ValId,
        then_f: &mut dyn FnMut(&mut Self, &mut Ctx<'d>),
        else_f: &mut dyn FnMut(&mut Self, &mut Ctx<'d>),
    ) {
        if let Some(c) = self.as_const(cond).and_then(|l| l.to_u64()) {
            if c != 0 {
                then_f(self, ctx);
            } else {
                else_f(self, ctx);
            }
            return;
        }
        let base_cur = ctx.cur.clone();
        let base_def = ctx.def.clone();
        let base_nba = ctx.nba.clone();
        let base_nba_def = ctx.nba_def.clone();
        let base_guard = ctx.guard;

        ctx.guard = Some(self.and_guard(ctx.seq, base_guard, cond));
        then_f(self, ctx);
        let then_cur = std::mem::replace(&mut ctx.cur, base_cur.clone());
        let then_def = std::mem::replace(&mut ctx.def, base_def.clone());
        let then_nba = std::mem::replace(&mut ctx.nba, base_nba.clone());
        let then_nba_def = std::mem::replace(&mut ctx.nba_def, base_nba_def.clone());

        let ncond = self.emit(
            ctx.seq,
            tag::unary(UnaryOp::LogicNot),
            &[],
            &[cond],
            1,
            false,
            true,
            |dst, a| Op::Unary {
                op: UnaryOp::LogicNot,
                dst,
                a: a[0],
            },
        );
        ctx.guard = Some(self.and_guard(ctx.seq, base_guard, ncond));
        else_f(self, ctx);
        let else_cur = std::mem::take(&mut ctx.cur);
        let else_def = std::mem::take(&mut ctx.def);
        let else_nba = std::mem::take(&mut ctx.nba);
        let else_nba_def = std::mem::take(&mut ctx.nba_def);
        ctx.guard = base_guard;

        ctx.cur = self.merge(ctx.seq, cond, &base_cur, &then_cur, &else_cur);
        ctx.nba = self.merge(ctx.seq, cond, &base_nba, &then_nba, &else_nba);
        ctx.def = merge_masks(&then_def, &else_def);
        ctx.nba_def = merge_masks(&then_nba_def, &else_nba_def);
    }

    /// Merges the two arms' views of a signal map.
    ///
    /// An arm that did not touch a signal keeps what it had on entry, so
    /// the fallback matters: the map the arms started from, and failing
    /// that the signal's value before the unit ran. Taking the one arm
    /// that did write it would apply the write on both paths.
    fn merge(
        &mut self,
        seq: bool,
        cond: ValId,
        base: &BTreeMap<SigId, ValId>,
        then_map: &BTreeMap<SigId, ValId>,
        else_map: &BTreeMap<SigId, ValId>,
    ) -> BTreeMap<SigId, ValId> {
        let mut out = BTreeMap::new();
        let keys: BTreeSet<SigId> = then_map.keys().chain(else_map.keys()).copied().collect();
        for sig in keys {
            let entry = base.get(&sig).copied().or(self.env[sig.idx()]);
            let t = then_map.get(&sig).copied().or(entry);
            let e = else_map.get(&sig).copied().or(entry);
            let value = match (t, e) {
                (Some(t), Some(e)) if t == e => t,
                (Some(t), Some(e)) => {
                    let width = self.slot(t).width.max(self.slot(e).width);
                    let signed = self.slot(t).signed && self.slot(e).signed;
                    self.mux(seq, cond, e, t, width, signed)
                }
                (Some(v), None) | (None, Some(v)) => v,
                (None, None) => continue,
            };
            out.insert(sig, value);
        }
        out
    }

    fn and_guard(&mut self, seq: bool, guard: Option<ValId>, cond: ValId) -> ValId {
        match guard {
            None => cond,
            Some(g) => self.emit(
                seq,
                tag::binary(BinaryOp::And),
                &[],
                &[g, cond],
                1,
                false,
                true,
                |dst, a| Op::Binary {
                    op: BinaryOp::And,
                    dst,
                    a: a[0],
                    b: a[1],
                },
            ),
        }
    }

    /// The one-bit truth value of a condition expression.
    fn condition(&mut self, ctx: &mut Ctx<'d>, e: ExprId) -> ValId {
        let v = self.expr(ctx, e);
        self.truth(ctx.seq, v)
    }

    /// The truth value of a condition when it is fixed at compile time.
    fn static_truth(&mut self, ctx: &mut Ctx<'d>, e: ExprId) -> Option<bool> {
        let v = self.condition(ctx, e);
        self.as_const(v).and_then(|l| l.to_u64()).map(|c| c != 0)
    }

    // -- assignment targets ----------------------------------------------

    fn assign(&mut self, ctx: &mut Ctx<'d>, target: &'d Lvalue, value: ValId, nba: bool) {
        match target {
            Lvalue::Net(n) => {
                let sig = self.sig_of(ctx.inst, *n);
                let width = self.sig_width(sig);
                let signed = self.sig_signed(sig);
                let v = self.resize(ctx.seq, value, width, signed);
                self.store(ctx, sig, v, Some((0, width)), nba);
            }
            Lvalue::Slice { net, hi, lo } => {
                let sig = self.sig_of(ctx.inst, *net);
                let ew = elem_width(&ctx.m.nets[*net].ty);
                let bit_lo = lo.saturating_mul(ew);
                let w = hi.saturating_sub(*lo).saturating_add(1).saturating_mul(ew);
                let full = self.sig_width(sig);
                let signed = self.sig_signed(sig);
                let part = self.resize(ctx.seq, value, w, false);
                let base = self.base_of(ctx, sig, nba);
                let new = self.insert(ctx.seq, base, part, i64::from(bit_lo), full, signed);
                self.store(ctx, sig, new, Some((bit_lo, w)), nba);
            }
            Lvalue::Index { net, index } => {
                let sig = self.sig_of(ctx.inst, *net);
                let ty = ctx.m.nets[*net].ty.clone();
                let (ew, count) = (elem_width(&ty), elem_count(&ty));
                let full = self.sig_width(sig);
                let signed = self.sig_signed(sig);
                let idx = self.expr(ctx, *index);
                let part = self.resize(ctx.seq, value, ew, false);
                let base = self.base_of(ctx, sig, nba);
                let new = self.dyn_insert(ctx.seq, base, part, idx, ew, count, full, signed);
                let range = self
                    .as_const(idx)
                    .and_then(|l| l.to_u64())
                    .filter(|i| *i < count)
                    .and_then(|i| u32::try_from(i * u64::from(ew)).ok())
                    .map(|lo| (lo, ew));
                self.store(ctx, sig, new, range, nba);
            }
            Lvalue::Concat(parts) => {
                let total: u32 = parts.iter().map(|p| self.lvalue_width(ctx, p)).sum();
                let v = self.resize(ctx.seq, value, total, false);
                let mut hi = total;
                for p in parts {
                    let w = self.lvalue_width(ctx, p);
                    hi -= w;
                    let piece = self.extract(ctx.seq, v, i64::from(hi), w);
                    self.assign(ctx, p, piece, nba);
                }
            }
            Lvalue::MemElem { mem, addr } => {
                if !ctx.seq {
                    self.err(ctx.label.clone(), Reason::MemoryOutsideEdge, Some(ctx.span));
                    return;
                }
                let a = self.expr(ctx, *addr);
                let mid = self.mem_of(ctx.inst, *mem);
                let layout = &self.fold_prog.mems[mid.idx()];
                let (ew, signed) = (layout.elem_width, layout.signed);
                let v = self.resize(ctx.seq, value, ew, signed);
                self.seq.push(Op::MemWrite {
                    mem: u32::try_from(mid.idx()).expect("memory index fits"),
                    addr: self.slot(a),
                    data: self.slot(v),
                    en: ctx.guard.map(|g| self.slot(g)),
                });
            }
        }
    }

    fn lvalue_width(&self, ctx: &Ctx<'d>, lv: &Lvalue) -> u32 {
        match lv {
            Lvalue::Net(n) => flat_width(&ctx.m.nets[*n].ty),
            Lvalue::Slice { net, hi, lo } => {
                let ew = elem_width(&ctx.m.nets[*net].ty);
                hi.saturating_sub(*lo).saturating_add(1).saturating_mul(ew)
            }
            Lvalue::Index { net, .. } => elem_width(&ctx.m.nets[*net].ty),
            Lvalue::Concat(parts) => parts.iter().map(|p| self.lvalue_width(ctx, p)).sum(),
            Lvalue::MemElem { mem, .. } => {
                let mid = self.mem_of(ctx.inst, *mem);
                self.sim.memories[mid.idx()].elem_width
            }
        }
    }

    /// The value a partial write builds on.
    fn base_of(&mut self, ctx: &mut Ctx<'d>, sig: SigId, nba: bool) -> ValId {
        let map = if nba { &ctx.nba } else { &ctx.cur };
        if let Some(v) = map.get(&sig) {
            return *v;
        }
        self.env[sig.idx()].unwrap_or_else(|| {
            let width = self.sig_width(sig);
            let signed = self.sig_signed(sig);
            self.zero(width, signed)
        })
    }

    /// Records an assignment in the local view.
    ///
    /// `range` is the bits the assignment named; `None` is a computed
    /// index, which writes one element but not one this form can name.
    fn store(
        &mut self,
        ctx: &mut Ctx<'d>,
        sig: SigId,
        value: ValId,
        range: Option<(u32, u32)>,
        nba: bool,
    ) {
        if !ctx.writes.contains(&sig) {
            // A write the pre-scan did not see (a target reached through a
            // path the walker does not model); treat it as a full write.
            ctx.writes.insert(sig);
        }
        let width = self.sig_width(sig);
        let (map, defs) = if nba {
            (&mut ctx.nba, &mut ctx.nba_def)
        } else {
            (&mut ctx.cur, &mut ctx.def)
        };
        map.insert(sig, value);
        let mask = defs.entry(sig).or_insert_with(|| empty_mask(width));
        // A computed index defines one element, but not one we can name,
        // so it defines nothing as far as completeness goes: a unit whose
        // only write to a net is `net[i] = ...` really does latch every
        // element the index misses.
        if let Some((lo, w)) = range {
            mark(mask, lo, w);
        }
        if nba {
            ctx.wrote_nba.insert(sig);
        } else {
            ctx.wrote.insert(sig);
        }
    }

    /// A combinational read of a signal, honouring the local view.
    fn read_sig(&mut self, ctx: &mut Ctx<'d>, sig: SigId) -> ValId {
        if let Some(v) = ctx.force.get(&sig) {
            return *v;
        }
        if ctx.prologue && self.covered[sig.idx()].is_some() {
            // The prologue runs before the combinational region, so a net
            // a driver produces has no value yet.
            self.err(ctx.label.clone(), Reason::AsyncResetLogic, Some(ctx.span));
        }
        if ctx.writes.contains(&sig) {
            let width = self.sig_width(sig);
            let defined = ctx.def.get(&sig).is_some_and(|m| words::all_ones(m, width));
            if !defined && !ctx.hold {
                self.err(ctx.label.clone(), Reason::CombinationalLoop, Some(ctx.span));
            }
            if let Some(v) = ctx.cur.get(&sig) {
                return *v;
            }
        }
        match self.env[sig.idx()] {
            Some(v) => v,
            None => {
                self.err(ctx.label.clone(), Reason::CombinationalLoop, Some(ctx.span));
                let width = self.sig_width(sig);
                let signed = self.sig_signed(sig);
                self.zero(width, signed)
            }
        }
    }

    // -- expressions -----------------------------------------------------

    fn expr(&mut self, ctx: &mut Ctx<'d>, e: ExprId) -> ValId {
        let Some(node) = ctx.m.exprs.get(e) else {
            return self.zero(1, false);
        };
        let span = node.span;
        match &node.kind {
            ExprKind::Const(c) => {
                if c.has_unknown() {
                    self.err(
                        ctx.label.clone(),
                        Reason::UnknownConstant(c.to_verilog_literal()),
                        Some(span),
                    );
                }
                self.constant(c)
            }
            ExprKind::String(_) => {
                self.err(
                    ctx.label.clone(),
                    Reason::UnsupportedExpression("a string outside a message"),
                    Some(span),
                );
                self.zero(1, false)
            }
            ExprKind::Net(n) => {
                let sig = self.sig_of(ctx.inst, *n);
                let v = self.read_sig(ctx, sig);
                let signed = ctx.m.nets[*n].ty.is_signed();
                self.alias(v, signed)
            }
            ExprKind::Slice { base, hi, lo } => {
                let ew = ctx.m.exprs.get(*base).map_or(1, |b| elem_width(&b.ty));
                let b = self.expr(ctx, *base);
                let width = hi.saturating_sub(*lo).saturating_add(1).saturating_mul(ew);
                let at = i64::from(*lo) * i64::from(ew);
                self.extract(ctx.seq, b, at, width)
            }
            ExprKind::Index { base, index } => {
                let (ew, count) = ctx
                    .m
                    .exprs
                    .get(*base)
                    .map_or((1, 0), |b| (elem_width(&b.ty), elem_count(&b.ty)));
                let b = self.expr(ctx, *base);
                let i = self.expr(ctx, *index);
                self.emit(
                    ctx.seq,
                    tag::DYN_INDEX,
                    &[i64::from(ew), i64::try_from(count).unwrap_or(i64::MAX)],
                    &[b, i],
                    ew,
                    false,
                    true,
                    |dst, a| Op::DynIndex {
                        dst,
                        a: a[0],
                        index: a[1],
                        elem: ew,
                        count,
                    },
                )
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => {
                let b = self.expr(ctx, *base);
                let o = self.expr(ctx, *offset);
                let up = *up;
                self.emit(
                    ctx.seq,
                    tag::DYN_EXTRACT,
                    &[i64::from(up)],
                    &[b, o],
                    *width,
                    false,
                    true,
                    |dst, a| Op::DynExtract {
                        dst,
                        a: a[0],
                        off: a[1],
                        up,
                    },
                )
            }
            ExprKind::Concat(parts) => {
                let vals: Vec<ValId> = parts.iter().map(|p| self.expr(ctx, *p)).collect();
                let width: u32 = vals
                    .iter()
                    .map(|v| self.slot(*v).width)
                    .fold(0u32, u32::saturating_add);
                self.emit(
                    ctx.seq,
                    tag::CONCAT,
                    &[],
                    &vals,
                    width,
                    false,
                    true,
                    |dst, a| Op::Concat {
                        dst,
                        parts: a.to_vec().into_boxed_slice(),
                    },
                )
            }
            ExprKind::Replicate { count, expr } => {
                let v = self.expr(ctx, *expr);
                let count = *count;
                let width = self.slot(v).width.saturating_mul(count);
                self.emit(
                    ctx.seq,
                    tag::REPLICATE,
                    &[i64::from(count)],
                    &[v],
                    width,
                    false,
                    true,
                    |dst, a| Op::Replicate {
                        dst,
                        a: a[0],
                        count,
                    },
                )
            }
            ExprKind::Unary { op, expr } => {
                let v = self.expr(ctx, *expr);
                let op = *op;
                let s = self.slot(v);
                let (width, signed) = if op.is_reduction() {
                    (1, false)
                } else {
                    (s.width, s.signed)
                };
                self.emit(
                    ctx.seq,
                    tag::unary(op),
                    &[],
                    &[v],
                    width,
                    signed,
                    true,
                    |dst, a| Op::Unary { op, dst, a: a[0] },
                )
            }
            ExprKind::Binary { op, lhs, rhs } => {
                // `==?` against a literal with `x` or `z` bits is a
                // wildcard comparison, so the unknown bits are a compile
                // time mask rather than a value the run has to carry.
                // This is what a synthesised `casez` looks like.
                let wildcard = (*op == BinaryOp::WildEq)
                    .then(|| ctx.m.exprs.get(*rhs).and_then(|n| n.as_const()))
                    .flatten()
                    .filter(|c| c.has_unknown())
                    .cloned();
                if let Some(c) = wildcard {
                    let a = self.expr(ctx, *lhs);
                    let care: Vec<u64> = c.unknown_words().iter().map(|u| !u).collect();
                    return self.masked_eq(ctx, a, &c, care);
                }
                let a = self.expr(ctx, *lhs);
                let b = self.expr(ctx, *rhs);
                self.binop(ctx, *op, a, b)
            }
            ExprKind::Ternary { cond, then_, else_ } => {
                let c = self.condition(ctx, *cond);
                let t = self.expr(ctx, *then_);
                let f = self.expr(ctx, *else_);
                self.select(ctx, c, f, t, span)
            }
            ExprKind::Resize {
                expr,
                width,
                signed,
            } => {
                let v = self.expr(ctx, *expr);
                let s = self.slot(v);
                let resized = self.resize(ctx.seq, v, *width, s.signed);
                self.alias(resized, *signed)
            }
            ExprKind::MemRead { mem, addr } => {
                let a = self.expr(ctx, *addr);
                let mid = self.mem_of(ctx.inst, *mem);
                let width = flat_width(&node.ty);
                let signed = node.ty.is_signed();
                self.mem_read(ctx.seq, mid, a, width, signed)
            }
            ExprKind::Call { .. } => {
                self.err(
                    ctx.label.clone(),
                    Reason::UnsupportedExpression("a function call"),
                    Some(span),
                );
                let width = flat_width(&node.ty);
                self.zero(width.max(1), false)
            }
        }
    }

    /// A binary operator, sized exactly as the event simulator's shared
    /// kernel sizes it.
    fn binop(&mut self, ctx: &mut Ctx<'d>, op: BinaryOp, a: ValId, b: ValId) -> ValId {
        let tag = tag::binary(op);
        let (sa, sb) = (self.slot(a), self.slot(b));
        let (a, b, width, signed) = if op.is_shift() {
            (a, b, sa.width, sa.signed)
        } else if op == BinaryOp::Pow {
            // The exponent is self-determined and keeps its own width,
            // but it does decide the result's signedness.
            (a, b, sa.width, sa.signed && sb.signed)
        } else if op.is_logical() {
            (a, b, 1, false)
        } else {
            let w = sa.width.max(sb.width);
            let a = self.resize(ctx.seq, a, w, sa.signed);
            let b = self.resize(ctx.seq, b, w, sb.signed);
            if op.is_predicate() {
                (a, b, 1, false)
            } else {
                (a, b, w, sa.signed && sb.signed)
            }
        };
        self.emit(ctx.seq, tag, &[], &[a, b], width, signed, true, |dst, s| {
            Op::Binary {
                op,
                dst,
                a: s[0],
                b: s[1],
            }
        })
    }

    /// A two-way select whose result type does not depend on which arm
    /// runs.
    ///
    /// Expression evaluation returns the taken branch *unchanged*, so the
    /// result carries that branch's own width and signedness. A
    /// straight-line form has to pick one at compile time, and the only
    /// case where that is the same answer is arms that already agree —
    /// which is what a well-formed IR has, since `infer_type` requires
    /// equal widths. Anything else is refused rather than guessed at.
    fn select(
        &mut self,
        ctx: &mut Ctx<'d>,
        cond: ValId,
        zero: ValId,
        one: ValId,
        span: Span,
    ) -> ValId {
        let (a, b) = (self.slot(zero), self.slot(one));
        if a.width != b.width || a.signed != b.signed {
            self.err(
                ctx.label.clone(),
                Reason::UnsupportedExpression("a select whose arms differ in width or signedness"),
                Some(span),
            );
        }
        let width = a.width.max(b.width);
        let signed = a.signed && b.signed;
        self.mux(ctx.seq, cond, zero, one, width, signed)
    }

    fn mux(
        &mut self,
        seq: bool,
        cond: ValId,
        zero: ValId,
        one: ValId,
        width: u32,
        signed: bool,
    ) -> ValId {
        let a = self.resize(seq, zero, width, signed);
        let b = self.resize(seq, one, width, signed);
        if a == b {
            return a;
        }
        self.emit(
            seq,
            tag::MUX,
            &[],
            &[cond, a, b],
            width,
            signed,
            true,
            |dst, s| Op::Mux {
                dst,
                s: s[0],
                a: s[1],
                b: s[2],
            },
        )
    }

    fn extract(&mut self, seq: bool, v: ValId, lo: i64, width: u32) -> ValId {
        self.emit(
            seq,
            tag::EXTRACT,
            &[lo],
            &[v],
            width,
            false,
            true,
            |dst, a| Op::Extract { dst, a: a[0], lo },
        )
    }

    fn insert(
        &mut self,
        seq: bool,
        base: ValId,
        part: ValId,
        lo: i64,
        width: u32,
        signed: bool,
    ) -> ValId {
        self.emit(
            seq,
            tag::INSERT,
            &[lo],
            &[base, part],
            width,
            signed,
            true,
            |dst, a| Op::Insert {
                dst,
                base: a[0],
                part: a[1],
                lo,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dyn_insert(
        &mut self,
        seq: bool,
        base: ValId,
        part: ValId,
        index: ValId,
        elem: u32,
        count: u64,
        width: u32,
        signed: bool,
    ) -> ValId {
        self.emit(
            seq,
            tag::DYN_INSERT,
            &[i64::from(elem), i64::try_from(count).unwrap_or(i64::MAX)],
            &[base, part, index],
            width,
            signed,
            true,
            |dst, a| Op::DynInsert {
                dst,
                base: a[0],
                part: a[1],
                index: a[2],
                elem,
                count,
            },
        )
    }

    fn mem_read(&mut self, seq: bool, mem: MemId, addr: ValId, width: u32, signed: bool) -> ValId {
        let idx = u32::try_from(mem.idx()).expect("memory index fits");
        self.emit(
            seq,
            tag::MEM_READ,
            &[i64::from(idx)],
            &[addr],
            width,
            signed,
            false,
            |dst, a| Op::MemRead {
                dst,
                mem: idx,
                addr: a[0],
            },
        )
    }

    // -- combinational cells ---------------------------------------------

    fn cell_value(&mut self, ctx: &mut Ctx<'d>, c: crate::sim::elab::CellRef) -> Option<ValId> {
        let cell: &'d Cell = self.sim.cells[c.idx()].cell;
        let kind = cell.kind.clone();
        let input = |low: &mut Self, ctx: &mut Ctx<'d>, port: &str| -> Option<ValId> {
            cell.input(port).map(|e| low.expr(ctx, e))
        };
        let bin = |low: &mut Self, ctx: &mut Ctx<'d>, op: BinaryOp| -> Option<ValId> {
            let a = cell.input("a").map(|e| low.expr(ctx, e))?;
            let b = cell.input("b").map(|e| low.expr(ctx, e))?;
            Some(low.binop(ctx, op, a, b))
        };
        let un = |low: &mut Self, ctx: &mut Ctx<'d>, op: UnaryOp| -> Option<ValId> {
            let a = cell.input("a").map(|e| low.expr(ctx, e))?;
            let s = low.slot(a);
            let (w, sg) = if op.is_reduction() {
                (1, false)
            } else {
                (s.width, s.signed)
            };
            Some(
                low.emit(ctx.seq, tag::unary(op), &[], &[a], w, sg, true, |dst, x| {
                    Op::Unary { op, dst, a: x[0] }
                }),
            )
        };
        match kind {
            CellKind::Buf => input(self, ctx, "a"),
            CellKind::Not => un(self, ctx, UnaryOp::Not),
            CellKind::ReduceAnd => un(self, ctx, UnaryOp::ReduceAnd),
            CellKind::ReduceOr => un(self, ctx, UnaryOp::ReduceOr),
            CellKind::ReduceXor => un(self, ctx, UnaryOp::ReduceXor),
            CellKind::And => bin(self, ctx, BinaryOp::And),
            CellKind::Or => bin(self, ctx, BinaryOp::Or),
            CellKind::Xor => bin(self, ctx, BinaryOp::Xor),
            CellKind::Add => bin(self, ctx, BinaryOp::Add),
            CellKind::Sub => bin(self, ctx, BinaryOp::Sub),
            CellKind::Mul => bin(self, ctx, BinaryOp::Mul),
            CellKind::Div => bin(self, ctx, BinaryOp::Div),
            CellKind::Mod => bin(self, ctx, BinaryOp::Mod),
            CellKind::Shl => bin(self, ctx, BinaryOp::Shl),
            CellKind::Shr => bin(self, ctx, BinaryOp::Shr),
            CellKind::Sshr => bin(self, ctx, BinaryOp::Sshr),
            CellKind::Eq => bin(self, ctx, BinaryOp::Eq),
            CellKind::Ne => bin(self, ctx, BinaryOp::Ne),
            CellKind::Lt => bin(self, ctx, BinaryOp::Lt),
            CellKind::Le => bin(self, ctx, BinaryOp::Le),
            CellKind::Gt => bin(self, ctx, BinaryOp::Gt),
            CellKind::Ge => bin(self, ctx, BinaryOp::Ge),
            CellKind::Mux => {
                let a = input(self, ctx, "a")?;
                let b = input(self, ctx, "b")?;
                let s = input(self, ctx, "s")?;
                Some(self.select(ctx, s, a, b, cell.span))
            }
            CellKind::Pmux => {
                let a = input(self, ctx, "a")?;
                let b = input(self, ctx, "b")?;
                let s = input(self, ctx, "s")?;
                let width = self.slot(a).width;
                // With nothing selected the result is the default, with
                // its signedness; with something selected it is a slice
                // of `b`, which is unsigned. One answer only when the
                // default is unsigned too.
                let signed = self.slot(a).signed;
                if signed {
                    self.err(
                        ctx.label.clone(),
                        Reason::UnsupportedExpression("a signed one-hot multiplexer"),
                        Some(cell.span),
                    );
                }
                Some(self.emit(
                    ctx.seq,
                    tag::PMUX,
                    &[],
                    &[s, a, b],
                    width,
                    signed,
                    true,
                    |dst, x| Op::Pmux {
                        dst,
                        s: x[0],
                        a: x[1],
                        b: x[2],
                    },
                ))
            }
            CellKind::Lut { init, .. } => {
                let a = input(self, ctx, "a")?;
                if init.has_unknown() {
                    self.err(
                        ctx.label.clone(),
                        Reason::UnknownConstant(init.to_verilog_literal()),
                        Some(cell.span),
                    );
                }
                let table = known_words(&init);
                let idx = u32::try_from(self.fold_prog.luts.len()).expect("lut table fits");
                self.fold_prog.luts.push(table);
                Some(self.emit(
                    ctx.seq,
                    tag::LUT,
                    &[i64::from(idx)],
                    &[a],
                    1,
                    false,
                    true,
                    |dst, x| Op::Lut {
                        dst,
                        a: x[0],
                        init: idx,
                    },
                ))
            }
            CellKind::MemRdPort {
                mem,
                clocked: false,
            } => {
                let addr = input(self, ctx, "addr")?;
                let mid = self.mem_of(ctx.inst, mem);
                let layout = &self.fold_prog.mems[mid.idx()];
                let (w, sg) = (layout.elem_width, layout.signed);
                Some(self.mem_read(ctx.seq, mid, addr, w, sg))
            }
            _ => None,
        }
    }

    // -- assembly --------------------------------------------------------

    fn finish(mut self, clock: Option<(SigId, Polarity)>, output: String) -> super::Plan<'d> {
        self.seq.append(&mut self.side);
        let state_len: usize = self
            .state_cur
            .iter()
            .map(|s| s.words as usize)
            .sum::<usize>();
        let state_cur = self.state_cur.first().map_or(0, |s| s.off as usize);
        let state_next = state_cur + state_len;
        let sig_slot: Vec<Option<Slot>> = (0..self.sim.signals.len())
            .map(|i| self.env[i].map(|v| self.vals[v as usize]))
            .collect();
        let settable: Vec<bool> = (0..self.sim.signals.len())
            .map(|i| self.covered[i].is_none())
            .collect();
        let stats = ProgramStats {
            comb_ops: self.comb.len(),
            seq_ops: self.seq.len(),
            registers: self.state_sigs.len(),
            state_bits: self
                .state_sigs
                .iter()
                .map(|s| self.sig_width(*s))
                .fold(0u32, u32::saturating_add),
            words: self.cursor,
            memories: self.fold_prog.mems.len(),
        };
        let prog = Program {
            comb: self.comb,
            seq: self.seq,
            init: self.fold.regs,
            state_cur,
            state_next,
            state_len,
            mems: self.fold_prog.mems,
            luts: self.fold_prog.luts,
            messages: self.messages,
            widest: self.widest,
        };
        super::Plan {
            sim: self.sim,
            prog,
            sig_slot,
            settable,
            state_sigs: self.state_sigs,
            clock,
            stats,
            output,
        }
    }
}

/// Intersects two defined-bit maps: a bit only one arm assigns is not
/// defined after the merge.
fn merge_masks(
    a: &BTreeMap<SigId, Vec<u64>>,
    b: &BTreeMap<SigId, Vec<u64>>,
) -> BTreeMap<SigId, Vec<u64>> {
    let mut out = BTreeMap::new();
    let keys: BTreeSet<SigId> = a.keys().chain(b.keys()).copied().collect();
    for sig in keys {
        let mask = match (a.get(&sig), b.get(&sig)) {
            (Some(x), Some(y)) => x.iter().zip(y).map(|(p, q)| p & q).collect(),
            _ => Vec::new(),
        };
        out.insert(sig, mask);
    }
    out
}
