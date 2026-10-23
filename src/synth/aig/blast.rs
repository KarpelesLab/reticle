//! Bit-blasting the combinational part of a cell-form module into an AIG.
//!
//! [`from_module`] separates a module into what the AIG can absorb and
//! what stays behind. Absorbed are the continuous assignments whose
//! targets are whole nets, constant slices or concatenations of those, and
//! the combinational cells (`Not`, `And`, `Or`, `Xor`, `Mux`, `Pmux`, the
//! arithmetic, shift, comparison and reduction cells, `Lut`, `Buf`), when
//! every expression involved is a bit-vector expression without memory
//! reads or calls. Everything else — flip-flops, latches, memory ports,
//! tri-states, black boxes, instances, processes, and assignments the
//! blaster cannot express — is a *boundary*: the nets it drives become AIG
//! inputs, and the nets it reads become AIG outputs, so the logic between
//! is exactly the combinational cloud the optimiser and the mappers work
//! on. Module ports are boundaries too. Combinational loops are broken by
//! turning one net on each cycle into a boundary as well, except where
//! the cycle is none bit by bit, which is evaluated to a fixpoint instead.
//!
//! The returned [`Mapping`] records which `(net, bit)` each AIG input and
//! output stands for and which cells and assignments were absorbed, which
//! is what [`super::emit`] needs to write a mapped network back.
//!
//! Two-state semantics: `x` and `z` constant bits read as 0, undriven bits
//! of an absorbed net read as 0, and an out-of-range variable select reads
//! 0 (see [`super::arith`]). Signed operators (comparisons, division,
//! arithmetic shift, sign extension) follow the operand types the IR
//! carries.

use std::collections::HashMap;

use super::{Aig, Edge};
use crate::ir::cell::{Cell, CellId, CellKind};
use crate::ir::design::{Module, NetId, PortDir};
use crate::ir::expr::{BinaryOp, ExprId, ExprKind, UnaryOp};
use crate::ir::process::Lvalue;
use crate::ir::types::Type;
use crate::ir::walk::{stmt_exprs, walk_block};

/// One bit of a net.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BitRef {
    /// The net.
    pub net: NetId,
    /// The bit index, 0 for the LSB.
    pub bit: u32,
}

/// How an AIG relates to the module it was extracted from.
#[derive(Clone, Debug, Default)]
pub struct Mapping {
    /// For each AIG input (in order), the net bit it carries.
    pub inputs: Vec<BitRef>,
    /// For each AIG output (in order), the net bit it drives.
    pub outputs: Vec<BitRef>,
    /// Cells whose logic the AIG absorbed.
    pub absorbed_cells: Vec<CellId>,
    /// Indices into `Module::assigns` of absorbed assignments.
    pub absorbed_assigns: Vec<usize>,
}

/// A piece of an assignment target: `(net, lo, hi, value_lo)` says that
/// bits `lo ..= hi` of `net` take the value's bits from `value_lo` up.
type TargetPiece = (NetId, u32, u32, u32);

/// A source of some bits of an absorbed net.
#[derive(Clone, Copy, Debug)]
enum Driver {
    /// Bits `lo ..= hi` of the net come from bits `value_lo ..` of the
    /// value of assignment `index`.
    Assign {
        index: usize,
        lo: u32,
        hi: u32,
        value_lo: u32,
    },
    /// The whole net is the `y` output of a cell.
    Cell(CellId),
}

struct Blaster<'m> {
    m: &'m Module,
    aig: Aig,
    /// True for nets the AIG must treat as inputs.
    boundary: Vec<bool>,
    /// Absorbed drivers per net.
    drivers: Vec<Vec<Driver>>,
    net_bits: Vec<Option<Vec<Edge>>>,
    in_progress: Vec<bool>,
    /// Nets that close a cycle, evaluated by [`Blaster::fixpoint`]
    /// rather than made boundaries.
    self_loop: Vec<bool>,
    /// The expressions and nets cached since the innermost
    /// [`Blaster::fixpoint`] round began, which are all a round has to
    /// forget; `None` outside one.
    journal: Option<(Vec<ExprId>, Vec<NetId>)>,
    expr_bits: Vec<Option<Vec<Edge>>>,
    expr_ok: Vec<Option<bool>>,
    mapping: Mapping,
}

/// The widest net [`Blaster::fixpoint`] is tried on; a wider one closing
/// a cycle is a boundary, as every one was. A round costs an evaluation
/// of the cycle and there are up to the width plus one of them.
const FIXPOINT_WIDTH: u32 = 256;

/// Extracts the combinational logic of `module` into an AIG.
pub fn from_module(module: &Module) -> (Aig, Mapping) {
    let mut b = Blaster {
        m: module,
        aig: Aig::new(),
        boundary: vec![false; module.nets.len()],
        drivers: vec![Vec::new(); module.nets.len()],
        net_bits: vec![None; module.nets.len()],
        in_progress: vec![false; module.nets.len()],
        self_loop: vec![false; module.nets.len()],
        journal: None,
        expr_bits: vec![None; module.exprs.len()],
        expr_ok: vec![None; module.exprs.len()],
        mapping: Mapping::default(),
    };
    b.classify();
    let needed = b.needed_nets();
    for net in module.nets.ids() {
        let Some(read) = &needed[net.index()] else {
            continue;
        };
        if b.boundary[net.index()] {
            continue;
        }
        let bits = b.net(net);
        for (i, &e) in bits.iter().enumerate() {
            // Only the bits something outside the AIG reads are outputs:
            // `r[sel]` arrives as one bit of `r >> sel`, and mapping the
            // other bits of the shift cost a 32-way select 101 LUTs where
            // 11 do. `write_back` drives a bit with no output as zero,
            // which nothing reads.
            if !read.get(i).copied().unwrap_or(true) {
                continue;
            }
            b.aig.add_output(e);
            b.mapping.outputs.push(BitRef {
                net,
                bit: u32::try_from(i).expect("bit index"),
            });
        }
    }
    (b.aig, b.mapping)
}

/// The bit-vector width of a type, if it is one.
fn width_of(ty: &Type) -> Option<u32> {
    ty.width()
}

impl<'m> Blaster<'m> {
    // ---- classification ---------------------------------------------------

    fn classify(&mut self) {
        let m = self.m;
        // Nets that are not plain bit vectors, ports driven from outside,
        // and nets written by anything the AIG does not absorb.
        for (id, net) in m.nets.iter() {
            if width_of(&net.ty).is_none() {
                self.boundary[id.index()] = true;
            }
        }
        for port in &m.ports {
            if port.dir != PortDir::Out {
                self.boundary[port.net.index()] = true;
            }
        }
        for (_, process) in m.processes.iter() {
            walk_block(&process.body, &mut |stmt| {
                crate::ir::walk::stmt_nets(stmt, &mut |n| self.boundary[n.index()] = true);
            });
        }
        // Instance ports need no special case: without the rest of the
        // design their directions are unknown, but a net an instance
        // drives has no absorbed driver either, and every net without one
        // becomes a boundary below.
        // Cells: absorbed when combinational and expressible.
        let mut absorbed_cells = Vec::new();
        for (cid, cell) in m.cells.iter() {
            if self.cell_supported(cell) {
                absorbed_cells.push(cid);
            } else {
                for (_, out) in &cell.outputs {
                    self.boundary[out.index()] = true;
                }
            }
        }
        // Assignments: absorbed when the target decomposes and the value
        // is expressible; otherwise every target net is a boundary.
        let mut pieces: Vec<Option<Vec<TargetPiece>>> = Vec::new();
        for assign in &m.assigns {
            let ok = assign.delay.is_none()
                && self.expr_supported(assign.value)
                && m.exprs.get(assign.value).and_then(|e| width_of(&e.ty))
                    == lvalue_width(m, &assign.target);
            let decomposed = if ok {
                decompose_lvalue(m, &assign.target, 0)
            } else {
                None
            };
            if decomposed.is_none() {
                for n in assign.target.nets() {
                    self.boundary[n.index()] = true;
                }
            }
            pieces.push(decomposed);
        }
        // Cells absorbed so far drive their `y` net; record and note the
        // nets with an absorbed driver.
        let mut has_driver = vec![false; m.nets.len()];
        for &cid in &absorbed_cells {
            let y = m.cells[cid].output("y").expect("checked");
            has_driver[y.index()] = true;
        }
        for p in pieces.iter().flatten() {
            for &(net, _, _, _) in p {
                has_driver[net.index()] = true;
            }
        }
        for id in m.nets.ids() {
            if !has_driver[id.index()] {
                self.boundary[id.index()] = true;
            }
        }
        // Boundary status propagates through assignments: a concatenation
        // target with one boundary net keeps the whole assignment.
        loop {
            let mut changed = false;
            for p in pieces.iter_mut() {
                let Some(parts) = p else {
                    continue;
                };
                if parts.iter().any(|&(n, _, _, _)| self.boundary[n.index()]) {
                    for &(n, _, _, _) in parts.iter() {
                        if !self.boundary[n.index()] {
                            self.boundary[n.index()] = true;
                            changed = true;
                        }
                    }
                    *p = None;
                }
            }
            if !changed {
                break;
            }
        }
        // Cells whose output net is a boundary (also driven elsewhere)
        // are kept.
        absorbed_cells.retain(|&cid| {
            let y = m.cells[cid].output("y").expect("checked");
            !self.boundary[y.index()]
        });
        // Record drivers.
        for &cid in &absorbed_cells {
            let y = m.cells[cid].output("y").expect("checked");
            self.drivers[y.index()].push(Driver::Cell(cid));
        }
        for (index, p) in pieces.iter().enumerate() {
            if let Some(parts) = p {
                for &(net, lo, hi, value_lo) in parts {
                    self.drivers[net.index()].push(Driver::Assign {
                        index,
                        lo,
                        hi,
                        value_lo,
                    });
                }
            }
        }
        // Break combinational loops: a net on a cycle becomes a boundary,
        // and its drivers are dropped from the absorbed set.
        self.break_cycles();
        // A net that closes a cycle is kept when its bits settle, and made
        // a boundary like any other loop when they do not. This is decided
        // before what is absorbed is, because a net found to be a loop
        // after its driver had been dropped would be left undriven.
        let looped: Vec<NetId> = m
            .nets
            .ids()
            .filter(|n| self.self_loop[n.index()] && !self.boundary[n.index()])
            .collect();
        if !looped.is_empty() {
            for &net in &looped {
                if !self.closes_no_loop(net) || self.fixpoint(net).is_none() {
                    self.self_loop[net.index()] = false;
                    self.boundary[net.index()] = true;
                }
            }
            // The trial's work is thrown away but for the inputs it made,
            // which are boundary nets' and stay theirs.
            self.expr_bits.fill(None);
            for id in m.nets.ids() {
                if !self.boundary[id.index()] {
                    self.net_bits[id.index()] = None;
                }
            }
        }
        absorbed_cells.retain(|&cid| {
            let y = m.cells[cid].output("y").expect("checked");
            !self.boundary[y.index()]
        });
        let mut absorbed_assigns: Vec<usize> = Vec::new();
        for (index, p) in pieces.iter().enumerate() {
            if let Some(parts) = p
                && parts.iter().all(|&(n, _, _, _)| !self.boundary[n.index()])
            {
                absorbed_assigns.push(index);
            }
        }
        for id in m.nets.ids() {
            if self.boundary[id.index()] {
                self.drivers[id.index()].clear();
            }
        }
        self.mapping.absorbed_cells = absorbed_cells;
        self.mapping.absorbed_assigns = absorbed_assigns;
    }

    /// The nets an absorbed net's drivers read.
    fn dependencies(&self, net: NetId) -> Vec<NetId> {
        let mut deps = Vec::new();
        for d in &self.drivers[net.index()] {
            match *d {
                Driver::Assign { index, .. } => {
                    collect_nets(self.m, self.m.assigns[index].value, &mut deps);
                }
                Driver::Cell(cid) => {
                    for (_, e) in &self.m.cells[cid].inputs {
                        collect_nets(self.m, *e, &mut deps);
                    }
                }
            }
        }
        deps.sort_unstable();
        deps.dedup();
        deps
    }

    fn break_cycles(&mut self) {
        // Iterative DFS with colours; the target of a back edge is made a
        // boundary, which removes it from the graph.
        #[derive(Clone, Copy, PartialEq)]
        enum Colour {
            White,
            Grey,
            Black,
        }
        let n = self.m.nets.len();
        let mut colour = vec![Colour::White; n];
        for start in self.m.nets.ids() {
            if colour[start.index()] != Colour::White || self.boundary[start.index()] {
                continue;
            }
            let mut stack: Vec<(NetId, Vec<NetId>, usize)> =
                vec![(start, self.dependencies(start), 0)];
            colour[start.index()] = Colour::Grey;
            while let Some((node, deps, next)) = stack.last_mut() {
                if let Some(&dep) = deps.get(*next) {
                    *next += 1;
                    if self.boundary[dep.index()] {
                        continue;
                    }
                    match colour[dep.index()] {
                        Colour::White => {
                            colour[dep.index()] = Colour::Grey;
                            let deps = self.dependencies(dep);
                            stack.push((dep, deps, 0));
                        }
                        // The net that closes a cycle may close no loop
                        // bit by bit; `classify` tries it, up to a width.
                        Colour::Grey
                            if width_of(&self.m.nets[dep].ty)
                                .is_some_and(|w| w <= FIXPOINT_WIDTH) =>
                        {
                            self.self_loop[dep.index()] = true;
                        }
                        Colour::Grey => {
                            self.boundary[dep.index()] = true;
                        }
                        Colour::Black => {}
                    }
                } else {
                    colour[node.index()] = Colour::Black;
                    stack.pop();
                }
            }
        }
    }

    /// Nets whose value something outside the absorbed logic reads (or a
    /// port exposes), so they must be AIG outputs.
    /// Which bits of each net something outside the AIG reads: `None`
    /// for a net nothing reads, otherwise one flag per bit. A read through
    /// a constant slice of the net marks that slice; any other read marks
    /// every bit.
    fn needed_nets(&self) -> Vec<Option<Vec<bool>>> {
        let m = self.m;
        let mut needed: Vec<Option<Vec<bool>>> = vec![None; m.nets.len()];
        let width = |n: NetId| m.nets[n].ty.width().unwrap_or(1) as usize;
        let mark_bits = |needed: &mut Vec<Option<Vec<bool>>>, n: NetId, lo: u32, hi: u32| {
            let bits = needed[n.index()].get_or_insert_with(|| vec![false; width(n)]);
            for bit in lo..=hi {
                if let Some(b) = bits.get_mut(bit as usize) {
                    *b = true;
                }
            }
        };
        let all = |needed: &mut Vec<Option<Vec<bool>>>, n: NetId| {
            needed[n.index()] = Some(vec![true; width(n)]);
        };
        for port in &m.ports {
            all(&mut needed, port.net);
        }
        for (id, net) in m.nets.iter() {
            if net.attrs.is_kept() {
                all(&mut needed, id);
            }
        }
        let mut seen = vec![false; m.exprs.len()];
        let mut mark = |e: ExprId| {
            let mut stack = vec![e];
            while let Some(id) = stack.pop() {
                let Some(expr) = m.exprs.get(id) else {
                    continue;
                };
                if std::mem::replace(&mut seen[id.index()], true) {
                    continue;
                }
                let sliced = match &expr.kind {
                    ExprKind::Slice { base, hi, lo } => match m.exprs.get(*base).map(|b| &b.kind) {
                        Some(ExprKind::Net(n)) => Some((*n, *lo, *hi)),
                        _ => None,
                    },
                    _ => None,
                };
                match (&expr.kind, sliced) {
                    (_, Some((n, lo, hi))) => mark_bits(&mut needed, n, lo, hi),
                    (ExprKind::Net(n), None) => all(&mut needed, *n),
                    (kind, None) => stack.extend(crate::ir::expr::operands(kind)),
                }
            }
        };
        let absorbed: std::collections::HashSet<CellId> =
            self.mapping.absorbed_cells.iter().copied().collect();
        for (cid, cell) in m.cells.iter() {
            if !absorbed.contains(&cid) {
                for (_, e) in &cell.inputs {
                    mark(*e);
                }
            }
        }
        let absorbed_assigns: std::collections::HashSet<usize> =
            self.mapping.absorbed_assigns.iter().copied().collect();
        for (index, assign) in m.assigns.iter().enumerate() {
            if !absorbed_assigns.contains(&index) {
                mark(assign.value);
                crate::ir::walk::lvalue_exprs(&assign.target, &mut mark);
            } else {
                // Index expressions inside an absorbed target cannot occur
                // (such targets are not decomposable).
            }
        }
        for (_, inst) in m.instances.iter() {
            for (_, e) in &inst.connections {
                mark(*e);
            }
        }
        for (_, process) in m.processes.iter() {
            walk_block(&process.body, &mut |stmt| stmt_exprs(stmt, &mut mark));
        }
        needed
    }

    // ---- support checks --------------------------------------------------

    fn cell_supported(&mut self, cell: &Cell) -> bool {
        let simple = matches!(
            cell.kind,
            CellKind::Not
                | CellKind::And
                | CellKind::Or
                | CellKind::Xor
                | CellKind::Mux
                | CellKind::Pmux
                | CellKind::Add
                | CellKind::Sub
                | CellKind::Mul
                | CellKind::Div
                | CellKind::Mod
                | CellKind::Shl
                | CellKind::Shr
                | CellKind::Sshr
                | CellKind::Eq
                | CellKind::Ne
                | CellKind::Lt
                | CellKind::Le
                | CellKind::Gt
                | CellKind::Ge
                | CellKind::ReduceAnd
                | CellKind::ReduceOr
                | CellKind::ReduceXor
                | CellKind::Lut { .. }
                | CellKind::Buf
        );
        if !simple || cell.attrs.is_kept() {
            return false;
        }
        let Some(y) = cell.output("y") else {
            return false;
        };
        if cell.outputs.len() != 1 || self.m.nets.get(y).and_then(|n| width_of(&n.ty)).is_none() {
            return false;
        }
        let ports = cell.kind.input_ports();
        if cell.inputs.len() != ports.len() {
            return false;
        }
        for port in ports {
            match cell.input(port) {
                Some(e) if self.expr_supported(e) => {}
                _ => return false,
            }
        }
        if let CellKind::Lut { k, init } = &cell.kind {
            let a = cell
                .input("a")
                .and_then(|e| self.m.exprs.get(e))
                .and_then(|e| width_of(&e.ty));
            if a != Some(*k) || *k >= 32 || init.width() != 1u32 << k {
                return false;
            }
        }
        true
    }

    fn expr_supported(&mut self, id: ExprId) -> bool {
        if let Some(ok) = self.expr_ok.get(id.index()).copied().flatten() {
            return ok;
        }
        let ok = self.expr_supported_uncached(id);
        if let Some(slot) = self.expr_ok.get_mut(id.index()) {
            *slot = Some(ok);
        }
        ok
    }

    fn expr_supported_uncached(&mut self, id: ExprId) -> bool {
        let Some(expr) = self.m.exprs.get(id) else {
            return false;
        };
        if width_of(&expr.ty).is_none() {
            return false;
        }
        match &expr.kind {
            ExprKind::Const(_) => true,
            ExprKind::String(_) | ExprKind::MemRead { .. } | ExprKind::Call { .. } => false,
            ExprKind::Net(n) => self
                .m
                .nets
                .get(*n)
                .is_some_and(|n| width_of(&n.ty).is_some()),
            ExprKind::Slice { base, hi, lo } => {
                self.expr_supported(*base)
                    && hi >= lo
                    && self
                        .m
                        .exprs
                        .get(*base)
                        .and_then(|e| width_of(&e.ty))
                        .is_some_and(|w| *hi < w)
            }
            ExprKind::Index { base, index } => {
                self.expr_supported(*base) && self.expr_supported(*index)
            }
            ExprKind::IndexedSlice { base, offset, .. } => {
                self.expr_supported(*base) && self.expr_supported(*offset)
            }
            ExprKind::Concat(parts) => parts.clone().iter().all(|p| self.expr_supported(*p)),
            ExprKind::Replicate { expr, .. } | ExprKind::Unary { expr, .. } => {
                self.expr_supported(*expr)
            }
            ExprKind::Binary { lhs, rhs, op } => {
                let (lhs, rhs, op) = (*lhs, *rhs, *op);
                if !(self.expr_supported(lhs) && self.expr_supported(rhs)) {
                    return false;
                }
                let wl = self.m.exprs[lhs].ty.width();
                let wr = self.m.exprs[rhs].ty.width();
                op.is_shift() || wl == wr
            }
            ExprKind::Ternary { cond, then_, else_ } => {
                let (c, t, e) = (*cond, *then_, *else_);
                self.expr_supported(c)
                    && self.expr_supported(t)
                    && self.expr_supported(e)
                    && self.m.exprs[t].ty.width() == self.m.exprs[e].ty.width()
            }
            ExprKind::Resize { expr, .. } => self.expr_supported(*expr),
        }
    }

    // ---- evaluation --------------------------------------------------------

    fn net(&mut self, net: NetId) -> Vec<Edge> {
        if let Some(bits) = &self.net_bits[net.index()] {
            return bits.clone();
        }
        let width = width_of(&self.m.nets[net].ty).unwrap_or(0);
        if self.boundary[net.index()] || self.in_progress[net.index()] {
            // An input (or, defensively, a loop the cycle breaker missed).
            let bits: Vec<Edge> = (0..width)
                .map(|bit| {
                    self.mapping.inputs.push(BitRef { net, bit });
                    self.aig.add_input()
                })
                .collect();
            self.boundary[net.index()] = true;
            self.net_bits[net.index()] = Some(bits.clone());
            return bits;
        }
        if self.self_loop[net.index()]
            && let Some(bits) = self.fixpoint(net)
        {
            self.net_bits[net.index()] = Some(bits.clone());
            if let Some((_, nets)) = &mut self.journal {
                nets.push(net);
            }
            return bits;
        }
        self.in_progress[net.index()] = true;
        let bits = self.drive(net);
        self.in_progress[net.index()] = false;
        self.net_bits[net.index()] = Some(bits.clone());
        if let Some((_, nets)) = &mut self.journal {
            nets.push(net);
        }
        bits
    }

    /// A net that closes a cycle, evaluated to a fixpoint: its drivers,
    /// round the cycle, with every bit of it taken as zero, then again with
    /// what that gave, until two rounds agree, or `None` after one more
    /// round than it has bits.
    ///
    /// `b = g ^ (b >> 1)`, NewAE's conversion of a Gray code to binary, is
    /// a loop net by net (`b`, then the shift's net, then `b`) and none bit
    /// by bit: each bit reads only the one above it. Treated as a loop, the net became a boundary, its
    /// exclusive-or was left out of the AIG, and the flow stopped on a
    /// generic `$xor` it could not map. When no bit depends on itself,
    /// each round settles at least one more bit, so the width plus one
    /// rounds suffice; and because the AIG hashes its structure, settled
    /// bits are the same edges in two rounds, which is what is compared.
    /// A real loop never settles and stays a boundary.
    fn fixpoint(&mut self, net: NetId) -> Option<Vec<Edge>> {
        let width = width_of(&self.m.nets[net].ty).unwrap_or(0) as usize;
        let outer = self.journal.take();
        let mut guess = vec![Edge::FALSE; width];
        let mut settled = None;
        for _ in 0..=width + 1 {
            // What the last round cached may rest on its guess: forget it.
            // A boundary net's inputs are made once and never journaled.
            if let Some((exprs, nets)) = self.journal.take() {
                for e in exprs {
                    self.expr_bits[e.index()] = None;
                }
                for n in nets {
                    self.net_bits[n.index()] = None;
                }
            }
            self.journal = Some((Vec::new(), Vec::new()));
            self.net_bits[net.index()] = Some(guess.clone());
            let bits = self.drive(net);
            if bits == guess {
                settled = Some(bits);
                break;
            }
            guess = bits;
        }
        self.net_bits[net.index()] = None;
        let last = self.journal.take().unwrap_or_default();
        if settled.is_some() {
            // The last round's caches rest on the settled value, so they
            // stand; an enclosing round must forget them with its own.
            self.journal = outer.map(|(mut exprs, mut nets)| {
                exprs.extend(last.0);
                nets.extend(last.1);
                (exprs, nets)
            });
        } else {
            for e in last.0 {
                self.expr_bits[e.index()] = None;
            }
            for n in last.1 {
                self.net_bits[n.index()] = None;
            }
            self.journal = outer;
        }
        settled
    }

    /// Whether a net that closes a cycle closes no loop bit by bit: in a
    /// scratch AIG, with a fresh variable for each of its bits, its drivers
    /// fed their own result round the cycle one more time than it has bits
    /// no longer read any of those variables.
    ///
    /// Settling from a guess of zero is not enough on its own: `z = z & a`
    /// is a loop, a latch, and settles at zero, which would have made it a
    /// constant. A function that has stopped reading its guess is the same
    /// whatever the guess, so the zero guess [`Blaster::fixpoint`] starts
    /// from finds it exactly.
    fn closes_no_loop(&self, net: NetId) -> bool {
        let m = self.m;
        let mut scratch = Blaster {
            m,
            aig: Aig::new(),
            boundary: self.boundary.clone(),
            drivers: self.drivers.clone(),
            net_bits: vec![None; m.nets.len()],
            in_progress: vec![false; m.nets.len()],
            self_loop: self.self_loop.clone(),
            journal: None,
            expr_bits: vec![None; m.exprs.len()],
            expr_ok: self.expr_ok.clone(),
            mapping: Mapping::default(),
        };
        let width = width_of(&m.nets[net].ty).unwrap_or(0) as usize;
        let mut guess: Vec<Edge> = (0..width).map(|_| scratch.aig.add_input()).collect();
        let placeholders: std::collections::HashSet<u32> = guess.iter().map(|e| e.node()).collect();
        for _ in 0..=width {
            if let Some((exprs, nets)) = scratch.journal.take() {
                for e in exprs {
                    scratch.expr_bits[e.index()] = None;
                }
                for n in nets {
                    scratch.net_bits[n.index()] = None;
                }
            }
            scratch.journal = Some((Vec::new(), Vec::new()));
            scratch.net_bits[net.index()] = Some(guess.clone());
            guess = scratch.drive(net);
        }
        // Does any placeholder remain in the result's cone?
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut stack: Vec<u32> = guess.iter().map(|e| e.node()).collect();
        while let Some(node) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            if placeholders.contains(&node) {
                return false;
            }
            if scratch.aig.is_and(node) {
                let (a, b) = scratch.aig.fanins(node);
                stack.push(a.node());
                stack.push(b.node());
            }
        }
        true
    }

    /// A net's bits from its absorbed drivers.
    fn drive(&mut self, net: NetId) -> Vec<Edge> {
        let width = width_of(&self.m.nets[net].ty).unwrap_or(0);
        let mut bits = vec![Edge::FALSE; width as usize];
        let mut driven = vec![false; width as usize];
        let drivers = self.drivers[net.index()].clone();
        for d in drivers {
            match d {
                Driver::Cell(cid) => {
                    let value = self.cell(cid);
                    for (i, e) in value.into_iter().enumerate().take(width as usize) {
                        if !driven[i] {
                            bits[i] = e;
                            driven[i] = true;
                        }
                    }
                }
                Driver::Assign {
                    index,
                    lo,
                    hi,
                    value_lo,
                } => {
                    let value = self.expr(self.m.assigns[index].value);
                    for bit in lo..=hi {
                        let src = (bit - lo + value_lo) as usize;
                        let i = bit as usize;
                        if i < bits.len() && !driven[i] {
                            bits[i] = value.get(src).copied().unwrap_or(Edge::FALSE);
                            driven[i] = true;
                        }
                    }
                }
            }
        }
        bits
    }

    fn expr(&mut self, id: ExprId) -> Vec<Edge> {
        if let Some(bits) = &self.expr_bits[id.index()] {
            return bits.clone();
        }
        let bits = self.expr_uncached(id);
        self.expr_bits[id.index()] = Some(bits.clone());
        if let Some((exprs, _)) = &mut self.journal {
            exprs.push(id);
        }
        bits
    }

    fn operand_signed(&self, id: ExprId) -> bool {
        self.m.exprs[id].ty.is_signed()
    }

    fn expr_uncached(&mut self, id: ExprId) -> Vec<Edge> {
        let expr = &self.m.exprs[id];
        let width = width_of(&expr.ty).expect("checked bit vector");
        let kind = expr.kind.clone();
        let bits = match kind {
            ExprKind::Const(c) => self.aig.const_bits(&c),
            ExprKind::Net(n) => self.net(n),
            ExprKind::Slice { base, hi, lo } => {
                let b = self.expr(base);
                b[lo as usize..=hi as usize].to_vec()
            }
            ExprKind::Index { base, index } => {
                let b = self.expr(base);
                let i = self.expr(index);
                vec![self.aig.select_bits(&b, &i)]
            }
            ExprKind::IndexedSlice {
                base,
                offset,
                width: w,
                up,
            } => {
                let b = self.expr(base);
                let off = self.expr(offset);
                let shifted = if up {
                    self.aig.shr_bits(&b, &off)
                } else {
                    // base[offset -: w] = base[offset - w + 1 +: w]; a
                    // negative start is out of range.
                    let mut ext = off.clone();
                    ext.push(Edge::FALSE);
                    let sub = self
                        .aig
                        .const_u64(u64::from(w - 1), u32::try_from(ext.len()).expect("width"));
                    let start = self.aig.sub_bits(&ext, &sub);
                    let (start_bits, borrow) = start.split_at(off.len());
                    let s = self.aig.shr_bits(&b, start_bits);
                    let zero = vec![Edge::FALSE; b.len()];
                    self.aig.mux_bits(borrow[0], &zero, &s)
                };
                shifted[..w as usize].to_vec()
            }
            ExprKind::Concat(parts) => {
                let mut bits = Vec::with_capacity(width as usize);
                for part in parts.iter().rev() {
                    bits.extend(self.expr(*part));
                }
                bits
            }
            ExprKind::Replicate { count, expr } => {
                let e = self.expr(expr);
                let mut bits = Vec::with_capacity(width as usize);
                for _ in 0..count {
                    bits.extend_from_slice(&e);
                }
                bits
            }
            ExprKind::Unary { op, expr } => {
                let a = self.expr(expr);
                match op {
                    UnaryOp::Not => self.aig.not_bits(&a),
                    UnaryOp::Neg => self.aig.neg_bits(&a),
                    UnaryOp::ReduceAnd => vec![self.aig.reduce_and(&a)],
                    UnaryOp::ReduceOr => vec![self.aig.reduce_or(&a)],
                    UnaryOp::ReduceXor => vec![self.aig.reduce_xor(&a)],
                    UnaryOp::ReduceNand => vec![!self.aig.reduce_and(&a)],
                    UnaryOp::ReduceNor => vec![!self.aig.reduce_or(&a)],
                    UnaryOp::ReduceXnor => vec![!self.aig.reduce_xor(&a)],
                    UnaryOp::LogicNot => vec![!self.aig.reduce_or(&a)],
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.expr(lhs);
                let b = self.expr(rhs);
                let (sa, sb) = (self.operand_signed(lhs), self.operand_signed(rhs));
                self.binary(op, &a, &b, sa, sb, rhs)
            }
            ExprKind::Ternary { cond, then_, else_ } => {
                let c = self.expr(cond);
                let t = self.expr(then_);
                let e = self.expr(else_);
                let s = self.aig.reduce_or(&c);
                self.aig.mux_bits(s, &t, &e)
            }
            ExprKind::Resize {
                expr,
                width: w,
                signed,
            } => {
                let a = self.expr(expr);
                let sign_extend = signed && self.operand_signed(expr);
                self.aig.resize_bits(&a, w, sign_extend)
            }
            ExprKind::String(_) | ExprKind::MemRead { .. } | ExprKind::Call { .. } => {
                unreachable!("unsupported expression was filtered")
            }
        };
        let mut bits = bits;
        bits.resize(width as usize, Edge::FALSE);
        bits
    }

    /// Builds `op` over two already-blasted operands.
    ///
    /// `lhs_signed` and `rhs_signed` are the operand types' signedness: an
    /// operation is signed only when both are (IEEE 1364-2005 §5.5.1),
    /// while the arithmetic shift and the power operator look at one side
    /// only. `rhs` is the right operand's node, which the wildcard
    /// comparison needs in order to read its constant pattern.
    fn binary(
        &mut self,
        op: BinaryOp,
        a: &[Edge],
        b: &[Edge],
        lhs_signed: bool,
        rhs_signed: bool,
        rhs: ExprId,
    ) -> Vec<Edge> {
        let signed = lhs_signed && rhs_signed;
        let g = &mut self.aig;
        match op {
            BinaryOp::And => g.and_bits(a, b),
            BinaryOp::Or => g.or_bits(a, b),
            BinaryOp::Xor => g.xor_bits(a, b),
            BinaryOp::Xnor => {
                let x = g.xor_bits(a, b);
                g.not_bits(&x)
            }
            BinaryOp::LogicAnd => {
                let x = g.reduce_or(a);
                let y = g.reduce_or(b);
                vec![g.and(x, y)]
            }
            BinaryOp::LogicOr => {
                let x = g.reduce_or(a);
                let y = g.reduce_or(b);
                vec![g.or(x, y)]
            }
            BinaryOp::Add => g.add_bits(a, b),
            BinaryOp::Sub => g.sub_bits(a, b),
            BinaryOp::Mul => g.mul_bits(a, b),
            BinaryOp::Div => g.divmod_bits(a, b, signed).0,
            BinaryOp::Mod => g.divmod_bits(a, b, signed).1,
            BinaryOp::Pow => g.pow_bits(a, b, signed, rhs_signed),
            BinaryOp::Shl => g.shl_bits(a, b),
            BinaryOp::Shr => g.shr_bits(a, b),
            BinaryOp::Sshr => {
                if lhs_signed {
                    g.sshr_bits(a, b)
                } else {
                    g.shr_bits(a, b)
                }
            }
            BinaryOp::Eq | BinaryOp::CaseEq => vec![g.eq_bits(a, b)],
            BinaryOp::Ne | BinaryOp::CaseNe => vec![!g.eq_bits(a, b)],
            BinaryOp::WildEq => {
                // Bits of a constant pattern that are x/z match anything.
                let mask: Vec<bool> = match self.m.exprs[rhs].as_const() {
                    Some(c) => (0..c.width()).map(|i| c.bit(i).is_known()).collect(),
                    None => vec![true; b.len()],
                };
                let bits: Vec<Edge> = a
                    .iter()
                    .zip(b)
                    .zip(mask)
                    .map(|((&x, &y), keep)| if keep { g.xnor(x, y) } else { Edge::TRUE })
                    .collect();
                vec![g.and_n(&bits)]
            }
            BinaryOp::Lt => vec![g.lt_bits(a, b, signed)],
            BinaryOp::Gt => vec![g.lt_bits(b, a, signed)],
            BinaryOp::Le => vec![!g.lt_bits(b, a, signed)],
            BinaryOp::Ge => vec![!g.lt_bits(a, b, signed)],
        }
    }

    fn cell(&mut self, cid: CellId) -> Vec<Edge> {
        let cell = &self.m.cells[cid];
        let kind = cell.kind.clone();
        let input = |name: &str| cell.input(name).expect("checked port");
        let y_width = width_of(&self.m.nets[cell.output("y").expect("checked")].ty).unwrap_or(1);
        let bits = match kind {
            CellKind::Not => {
                let a = self.expr(input("a"));
                self.aig.not_bits(&a)
            }
            CellKind::Buf => self.expr(input("a")),
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
            | CellKind::Sshr
            | CellKind::Eq
            | CellKind::Ne
            | CellKind::Lt
            | CellKind::Le
            | CellKind::Gt
            | CellKind::Ge => {
                let (ia, ib) = (input("a"), input("b"));
                let a = self.expr(ia);
                let b = self.expr(ib);
                let (sa, sb) = (self.operand_signed(ia), self.operand_signed(ib));
                let op = match kind {
                    CellKind::And => BinaryOp::And,
                    CellKind::Or => BinaryOp::Or,
                    CellKind::Xor => BinaryOp::Xor,
                    CellKind::Add => BinaryOp::Add,
                    CellKind::Sub => BinaryOp::Sub,
                    CellKind::Mul => BinaryOp::Mul,
                    CellKind::Div => BinaryOp::Div,
                    CellKind::Mod => BinaryOp::Mod,
                    CellKind::Shl => BinaryOp::Shl,
                    CellKind::Shr => BinaryOp::Shr,
                    CellKind::Sshr => BinaryOp::Sshr,
                    CellKind::Eq => BinaryOp::Eq,
                    CellKind::Ne => BinaryOp::Ne,
                    CellKind::Lt => BinaryOp::Lt,
                    CellKind::Le => BinaryOp::Le,
                    CellKind::Gt => BinaryOp::Gt,
                    _ => BinaryOp::Ge,
                };
                self.binary(op, &a, &b, sa, sb, ib)
            }
            CellKind::ReduceAnd => {
                let a = self.expr(input("a"));
                vec![self.aig.reduce_and(&a)]
            }
            CellKind::ReduceOr => {
                let a = self.expr(input("a"));
                vec![self.aig.reduce_or(&a)]
            }
            CellKind::ReduceXor => {
                let a = self.expr(input("a"));
                vec![self.aig.reduce_xor(&a)]
            }
            CellKind::Mux => {
                let a = self.expr(input("a"));
                let b = self.expr(input("b"));
                let s = self.expr(input("s"));
                let s = self.aig.reduce_or(&s);
                self.aig.mux_bits(s, &b, &a)
            }
            CellKind::Pmux => {
                let a = self.expr(input("a"));
                let b = self.expr(input("b"));
                let s = self.expr(input("s"));
                let w = a.len();
                let any = self.aig.reduce_or(&s);
                let mut acc: Vec<Edge> = a.iter().map(|&x| self.aig.and(x, !any)).collect();
                for (i, &si) in s.iter().enumerate() {
                    let slice = &b[i * w..((i + 1) * w).min(b.len())];
                    for (j, &bj) in slice.iter().enumerate() {
                        let t = self.aig.and(bj, si);
                        acc[j] = self.aig.or(acc[j], t);
                    }
                }
                acc
            }
            CellKind::Lut { init, .. } => {
                let a = self.expr(input("a"));
                vec![self.aig.lut_bits(&a, &init)]
            }
            _ => unreachable!("unsupported cell was filtered"),
        };
        let mut bits = bits;
        bits.resize(y_width as usize, Edge::FALSE);
        bits
    }
}

/// Every net an expression reads.
fn collect_nets(m: &Module, root: ExprId, out: &mut Vec<NetId>) {
    let mut stack = vec![root];
    let mut seen: HashMap<ExprId, ()> = HashMap::new();
    while let Some(id) = stack.pop() {
        if seen.insert(id, ()).is_some() {
            continue;
        }
        let Some(expr) = m.exprs.get(id) else {
            continue;
        };
        if let ExprKind::Net(n) = expr.kind {
            out.push(n);
        }
        stack.extend(crate::ir::expr::operands(&expr.kind));
    }
}

/// The width of an lvalue when it is a plain bit-vector target.
fn lvalue_width(m: &Module, lv: &Lvalue) -> Option<u32> {
    match lv {
        Lvalue::Net(n) => m.nets.get(*n).and_then(|n| width_of(&n.ty)),
        Lvalue::Slice { hi, lo, .. } => (hi >= lo).then(|| hi - lo + 1),
        Lvalue::Concat(parts) => {
            let mut total = 0u32;
            for p in parts {
                total = total.checked_add(lvalue_width(m, p)?)?;
            }
            Some(total)
        }
        Lvalue::Index { .. } | Lvalue::MemElem { .. } => None,
    }
}

/// Splits an lvalue into `(net, lo, hi, value_lo)` pieces, or `None` when
/// it contains a variable index or a memory element.
fn decompose_lvalue(m: &Module, lv: &Lvalue, value_lo: u32) -> Option<Vec<TargetPiece>> {
    match lv {
        Lvalue::Net(n) => {
            let w = m.nets.get(*n).and_then(|n| width_of(&n.ty))?;
            if w == 0 {
                return Some(Vec::new());
            }
            Some(vec![(*n, 0, w - 1, value_lo)])
        }
        Lvalue::Slice { net, hi, lo } => {
            let w = m.nets.get(*net).and_then(|n| width_of(&n.ty))?;
            if hi < lo || *hi >= w {
                return None;
            }
            Some(vec![(*net, *lo, *hi, value_lo)])
        }
        Lvalue::Concat(parts) => {
            let mut out = Vec::new();
            let mut offset = value_lo;
            for p in parts.iter().rev() {
                let w = lvalue_width(m, p)?;
                out.extend(decompose_lvalue(m, p, offset)?);
                offset = offset.checked_add(w)?;
            }
            Some(out)
        }
        Lvalue::Index { .. } | Lvalue::MemElem { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Name;
    use crate::ir::builder::ModuleBuilder;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// A flip-flop reading one bit of a 16-bit shift is one AIG output,
    /// not sixteen. `r[sel]` arrives as bit 0 of `r >> sel`, and with every
    /// bit of the shift an output a 32-way select mapped to 101 LUTs where
    /// 18 do. Would catch the per-bit liveness going back to per-net; would
    /// not catch a bit marked dead that something reads, which the
    /// behavioural self-check and the mapped-netlist proofs are for.
    #[test]
    fn only_the_bits_something_reads_are_outputs() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let r = b.input("r", Type::bits(16));
        let sel = b.input("sel", Type::bits(4));
        let q = b.add_reg("q", Type::bit());
        let t = b.add_net("t", Type::bits(16));
        let (rn, seln) = (b.net(r), b.net(sel));
        let shifted = b.binary(BinaryOp::Shr, rn, seln);
        b.assign(t, shifted);
        let tn = b.net(t);
        let bit = b.slice(tn, 0, 0);
        let clkn = b.net(clk);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clkn), (Name::new("d"), bit)],
            vec![(Name::new("q"), q)],
        );
        let m = b.finish();
        let (_, mapping) = from_module(&m);
        let outputs: Vec<(NetId, u32)> = mapping.outputs.iter().map(|o| (o.net, o.bit)).collect();
        assert_eq!(outputs, [(t, 0)]);
    }

    #[test]
    fn extracts_between_registers_and_ports() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let q = b.add_reg("q", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let t = b.add_net("t", Type::bits(4));
        let (an, cn, qn, tn) = (b.net(a), b.net(c), b.net(q), b.net(t));
        let sum = b.add(an, cn);
        b.assign(t, sum);
        let x = b.xor(tn, qn);
        b.assign(y, x);
        let d = b.and(an, tn);
        let clkn = b.net(clk);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clkn), (Name::new("d"), d)],
            vec![(Name::new("q"), q)],
        );
        let m = b.finish();
        let (aig, mapping) = from_module(&m);
        // Inputs: a, c, q bits (clk is not read by absorbed logic).
        assert_eq!(mapping.inputs.len(), 12);
        // Outputs: y (port) and t (read by the flop's d expression).
        assert_eq!(mapping.outputs.len(), 8);
        assert_eq!(mapping.absorbed_assigns, [0, 1]);
        assert!(mapping.absorbed_cells.is_empty());
        assert_eq!(aig.outputs().len(), 8);
    }

    /// **A cycle net by net that is none bit by bit is absorbed whole.**
    /// `b = g ^ (b >> 1)`, how NewAE's FIFOs turn a Gray code back into
    /// binary: the shift is a net of its own, so `b` closes a cycle `b`,
    /// `t`, `b`, and was made a boundary, its exclusive-or left out of the
    /// AIG and unmappable. Now both assignments are absorbed, no input
    /// stands for `b`, and the AIG gives binary for every 5-bit Gray code.
    /// And a real loop beside it, `z = z & a`, which settles at zero from
    /// a guess of zero, is still a boundary rather than a constant.
    ///
    /// Would catch the cycle made a boundary again (an input for `b`), the
    /// fixpoint stopping a round early (a wrong high bit for some code), or
    /// convergence from a zero guess taken as proof (`z` driven by the AIG);
    /// would not catch a loop that does not read its guess in the scratch AIG
    /// and yet is one, which structural hashing cannot produce.
    #[test]
    fn a_cycle_that_is_none_bit_by_bit_is_absorbed() {
        let mut b = ModuleBuilder::new("m", span());
        let g = b.input("g", Type::bits(5));
        let out = b.output("b", Type::bits(5));
        let t = b.add_net("t", Type::bits(5));
        let bn = b.net(out);
        let one = b.const_u64(5, 1);
        let shifted = b.shr(bn, one);
        b.assign(t, shifted);
        let gn = b.net(g);
        let tn = b.net(t);
        let x = b.xor(gn, tn);
        b.assign(out, x);
        let a = b.input("a", Type::bit());
        let z = b.output("z", Type::bit());
        let zn = b.net(z);
        let an = b.net(a);
        let l = b.and(zn, an);
        b.assign(z, l);
        let m = b.finish();
        let (aig, mapping) = from_module(&m);
        assert!(
            mapping.inputs.iter().all(|r| r.net != out),
            "no input stands for `b`: {:?}",
            mapping.inputs
        );
        assert!(
            mapping.outputs.iter().all(|r| r.net != z),
            "the latch is still a boundary, not something the AIG drives"
        );
        let g_inputs: Vec<usize> = mapping
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.net == g)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(g_inputs.len(), 5);
        for code in 0u32..32 {
            let mut inputs = vec![false; mapping.inputs.len()];
            for (i, r) in mapping.inputs.iter().enumerate() {
                if r.net == g {
                    inputs[i] = code >> r.bit & 1 == 1;
                }
            }
            let values = aig.eval(&inputs);
            let mut got = 0u32;
            for (o, r) in mapping.outputs.iter().enumerate() {
                if r.net == out && values[o] {
                    got |= 1 << r.bit;
                }
            }
            // Binary from Gray: each bit the parity of the bits above.
            let mut want = 0u32;
            for bit in 0..5 {
                if (code >> bit).count_ones() % 2 == 1 {
                    want |= 1 << bit;
                }
            }
            assert_eq!(got, want, "Gray code {code:05b}");
        }
    }

    #[test]
    fn keeps_unsupported_assignments_and_breaks_loops() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(2));
        let y = b.output("y", Type::bits(2));
        let z = b.output("z", Type::bits(2));
        let mem = b.memory("mem", Type::bits(2), 4);
        let an = b.net(a);
        let rd = b.mem_read(mem, an);
        b.assign(y, rd);
        // A combinational loop: z = z & a.
        let zn = b.net(z);
        let l = b.and(zn, an);
        b.assign(z, l);
        let m = b.finish();
        let (aig, mapping) = from_module(&m);
        assert!(mapping.absorbed_assigns.is_empty());
        assert!(aig.outputs().is_empty());
        let mut b = ModuleBuilder::from_module(m, span());
        let w = b.output("w", Type::bits(2));
        let an = b.net(a);
        let wn = b.net(w);
        let inner = b.and(wn, an);
        let t = b.add_net("t", Type::bits(2));
        b.assign(t, inner);
        let tn = b.net(t);
        let outer = b.or(tn, an);
        b.assign(w, outer);
        let m = b.finish();
        let (aig, mapping) = from_module(&m);
        // The loop w -> t -> w is broken: one of the two nets is a boundary
        // and the other is still mapped.
        assert_eq!(mapping.absorbed_assigns.len(), 1);
        assert!(!aig.outputs().is_empty());
    }
}
