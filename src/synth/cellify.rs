//! Cellify: expression trees to discrete cells.
//!
//! Process lowering and the optimisation loop leave combinational logic as
//! expression trees hanging off continuous assignments and cell inputs,
//! because that form is compact and is what the optimiser works on best.
//! The netlist formats cannot express it: JSON, BLIF and EDIF see a module
//! through [`crate::ir::emit::BitView`], which only understands *netlist
//! connections* — a constant, a net, or a slice, index, concatenation,
//! replication or resize of those. Anything else is rejected with
//! "`add` is not a netlist connection".
//!
//! [`Cellify`] closes that gap. After it has run, every continuous
//! assignment value, every cell input and every instance connection is a
//! netlist connection, and all the logic lives in cells.
//!
//! # What becomes a cell
//!
//! | Expression                        | Cells                                              |
//! |-----------------------------------|----------------------------------------------------|
//! | `not` `and` `or` `xor` `add` `sub` `mul` `div` `mod` `shl` `shr` `sshr` `eq` `ne` `lt` `le` `gt` `ge` `rand` `ror` `rxor` | the cell of the same name |
//! | `neg(a)`                          | `sub(0, a)`                                        |
//! | `xnor(a, b)`                      | `not(xor(a, b))`                                   |
//! | `rnand` `rnor` `rxnor` `lnot`     | `not` of the matching reduction (`lnot` is `not(ror(a))`) |
//! | `land` `lor`                      | `and` / `or` (both operands are one bit)           |
//! | `ceq` `cne`                       | `eq` / `ne`: the two coincide in the 2-state reading a netlist has, which is also what [`crate::formal::blast`] uses |
//! | `weq(a, <pattern>)`               | `eq` over the non-wildcard bits of the pattern     |
//! | `mux(c, t, f)`                    | `mux` (`s = 1` picks `b`, so `b = t` and `a = f`)  |
//! | `@mem[addr]`                      | an asynchronous `memrd` port                        |
//! | `a[i]`, `a[o +: w]`, `a[o -: w]` with a variable index | a shifter, see below          |
//!
//! Constants, nets, slices, constant indices, concatenations, replications
//! and resizes are *already* netlist connections, so they are kept as they
//! are and only their operands are cellified. In particular a `resize` is
//! not turned into a cell: the IR has no resize primitive, and `BitView`
//! renders one as the bit renaming it is. Widths and signedness are
//! preserved exactly: every cell drives a fresh net whose type is the type
//! the replaced node had, so the node's parent sees no change at all and
//! the IR's equal-width rules keep holding without any new `resize` node.
//!
//! # Variable selects
//!
//! A variable bit- or part-select becomes a **shifter**, not a mux tree:
//! `a[i]` is `shr(a, i)[0]` and `a[o +: w]` is `shr(a, o)[w-1:0]`, with a
//! `sub` in front of the shift amount for the `-:` form. One cell instead
//! of one per bit position keeps the netlist small, and the AIG optimiser
//! and the LUT mapper turn a shifter into the same mux tree a direct
//! expansion would have produced, only after structural hashing has shared
//! what it can.
//!
//! The one semantic difference: the IR reads an out-of-range select as
//! `x`, a shifter reads it as `0`. Synthesis treats `x` as a don't-care
//! (see the module docs of [`crate::synth`]), so resolving it to `0` is a
//! legal choice — but the formal engines model `x` as a free value, so a
//! design that really can select out of range may make
//! [`crate::synth::verify`] report a difference.
//!
//! # What cannot be cellified
//!
//! `pow`, a `weq` whose pattern is not a constant, and any [`Call`] left
//! in the design have no cell to become and are reported with `S0030`,
//! naming the operator or function. They are left in place, so the design
//! stays valid and the netlist emitters report the same construct again
//! with a span.
//!
//! [`Call`]: crate::ir::ExprKind::Call
//!
//! # Placement in the pipeline
//!
//! [`crate::synth::run`] runs this pass *after* the optimisation loop, so
//! the optimiser sees the expression form it is most effective on, and
//! follows it with [`crate::synth::opt::Merge`] and
//! [`crate::synth::opt::Dce`] so duplicate cells are shared and the nets
//! and expression nodes the rewrite orphaned are collected.

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{
    Attrs, BinaryOp, Bit, Cell, CellId, CellKind, Const, ExprId, ExprKind, Lvalue, Module, Name,
    Net, NetId, NetKind, Type, UnaryOp,
};
use crate::source::Span;
use crate::synth::util::{mk, mk_concat, mk_const, mk_net, mk_slice};
use crate::synth::{Pass, PassStats};

/// The cellify pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cellify;

impl Pass for Cellify {
    fn name(&self) -> &'static str {
        "cellify"
    }

    fn run(&self, module: &mut Module, diags: &mut Diagnostics) -> PassStats {
        let mut cx = Cellifier::new(module, diags);
        cx.run();
        cx.stats
    }
}

/// Where the cell that implements a node drives its result.
#[derive(Clone, Copy)]
enum Sink {
    /// A fresh net named after the surrounding object.
    Fresh,
    /// An existing net: the whole-net target of a continuous assignment,
    /// which the pass removes once the cell drives the net itself.
    Net(NetId),
}

/// The state of one module's rewrite.
struct Cellifier<'a> {
    module: &'a mut Module,
    diags: &'a mut Diagnostics,
    stats: PassStats,
    /// Every net and cell name in use, so a generated name collides with
    /// nothing.
    taken: HashSet<String>,
    /// The netlist connection each rewritten node was replaced by.
    memo: HashMap<ExprId, ExprId>,
    /// Whether a node is a netlist connection, memoised.
    structural: HashMap<ExprId, bool>,
    /// The cell [`Cellifier::emit_cell`] created last.
    last_cell: Option<CellId>,
    /// Number of cells the module had before the rewrite.
    original_cells: usize,
    /// Every cell created, with the index of the original cell it feeds
    /// ([`FRONT`] for one that feeds an assignment or an instance).
    created: Vec<(CellId, usize)>,
    /// Which original cell the cells created next belong to.
    anchor: usize,
}

/// [`Cellifier::anchor`] for a cell that belongs before every original
/// cell, because it feeds a continuous assignment or an instance.
const FRONT: usize = usize::MAX;

impl<'a> Cellifier<'a> {
    fn new(module: &'a mut Module, diags: &'a mut Diagnostics) -> Cellifier<'a> {
        let mut taken: HashSet<String> = HashSet::new();
        taken.extend(module.nets.values().map(|n| n.name.to_string()));
        taken.extend(module.cells.values().map(|c| c.name.to_string()));
        let original_cells = module.cells.len();
        Cellifier {
            module,
            diags,
            stats: PassStats::default(),
            taken,
            memo: HashMap::new(),
            structural: HashMap::new(),
            last_cell: None,
            original_cells,
            created: Vec::new(),
            anchor: FRONT,
        }
    }

    /// Rewrites every root that has to be a netlist connection: the value
    /// of each continuous assignment, each cell input and each instance
    /// connection. Assignments come first so that a cell can drive the
    /// assigned net directly instead of a buffer net.
    fn run(&mut self) {
        let mut drop_assign = Vec::with_capacity(self.module.assigns.len());
        for i in 0..self.module.assigns.len() {
            drop_assign.push(self.cellify_assign(i));
        }
        if drop_assign.iter().any(|d| *d) {
            let mut i = 0;
            self.module.assigns.retain(|_| {
                let keep = !drop_assign[i];
                i += 1;
                keep
            });
        }

        let cells: Vec<CellId> = self.module.cells.ids().collect();
        for (index, id) in cells.into_iter().enumerate() {
            self.anchor = index;
            let name = self.module.cells[id].name.to_string();
            let inputs = self.module.cells[id].inputs.clone();
            for (port, expr) in inputs {
                let new = self.lower(expr, &name);
                if new != expr {
                    let slot = self.module.cells[id]
                        .inputs
                        .iter_mut()
                        .find(|(p, _)| *p == port)
                        .expect("the port was just read from this cell");
                    slot.1 = new;
                }
            }
        }

        self.anchor = FRONT;
        let instances: Vec<crate::ir::InstanceId> = self.module.instances.ids().collect();
        for id in instances {
            let name = self.module.instances[id].name.to_string();
            let conns = self.module.instances[id].connections.clone();
            for (port, expr) in conns {
                let new = self.lower(expr, &name);
                if new != expr {
                    let slot = self.module.instances[id]
                        .connections
                        .iter_mut()
                        .find(|(p, _)| *p == port)
                        .expect("the port was just read from this instance");
                    slot.1 = new;
                }
            }
        }

        self.reorder_cells();
    }

    /// Moves every created cell in front of the object it feeds, so the
    /// arena stays in dependency order: a reader of the netlist (and the
    /// cycle-based interpreter the tests use) sees a cell's inputs
    /// computed before the cell itself.
    fn reorder_cells(&mut self) {
        if self.created.is_empty() {
            return;
        }
        let mut order: Vec<CellId> = self
            .created
            .iter()
            .filter(|(_, a)| *a == FRONT)
            .map(|(c, _)| *c)
            .collect();
        let originals: Vec<CellId> = self.module.cells.ids().take(self.original_cells).collect();
        for (index, id) in originals.into_iter().enumerate() {
            order.extend(
                self.created
                    .iter()
                    .filter(|(_, a)| *a == index)
                    .map(|(c, _)| *c),
            );
            order.push(id);
        }
        debug_assert_eq!(order.len(), self.module.cells.len());
        let cells: Vec<Cell> = order
            .into_iter()
            .map(|id| self.module.cells[id].clone())
            .collect();
        self.module.cells = crate::ir::Arena::new();
        for cell in cells {
            self.module.cells.push(cell);
        }
    }

    /// Rewrites assignment `i`, returning true when it became a cell
    /// driving the target net and should be dropped.
    fn cellify_assign(&mut self, i: usize) -> bool {
        let assign = &self.module.assigns[i];
        let (target, value) = (assign.target.clone(), assign.value);
        let direct = assign.delay.is_none()
            && matches!(target, Lvalue::Net(_))
            && !self.memo.contains_key(&value)
            && self.drives_a_cell(value)
            && match &target {
                Lvalue::Net(net) => self.module.nets[*net].ty == self.module.expr(value).ty,
                _ => false,
            };
        if direct {
            let Lvalue::Net(net) = target else {
                unreachable!("`direct` implies a whole-net target")
            };
            let name = self.module.nets[net].name.to_string();
            self.last_cell = None;
            let repl = self.lower_node(value, &name, Sink::Net(net));
            self.memo.insert(value, repl);
            // A construct with no cell form (a `pow`, a node that is not a
            // bit vector) leaves the target undriven, so keep the
            // assignment in that case.
            if self.module.expr(repl).as_net() == Some(net) {
                if let Some(cell) = self.last_cell {
                    let attrs = self.module.assigns[i].attrs.clone();
                    self.module.cells[cell].attrs.extend_from(&attrs);
                }
                self.stats.bump("assigns turned into cells", 1);
                return true;
            }
            self.module.assigns[i].value = repl;
            return false;
        }
        let name = lvalue_name(self.module, &target);
        let new = self.lower(value, &name);
        self.module.assigns[i].value = new;
        false
    }

    /// True when `id` is rewritten into a cell whose output can drive the
    /// target of a whole-net assignment directly.
    fn drives_a_cell(&self, id: ExprId) -> bool {
        match &self.module.expr(id).kind {
            ExprKind::Unary { .. } | ExprKind::Ternary { .. } | ExprKind::MemRead { .. } => true,
            ExprKind::Binary { op, rhs, .. } => match op {
                BinaryOp::Pow => false,
                BinaryOp::WildEq => match self.module.expr(*rhs).as_const() {
                    Some(c) => (0..c.width()).any(|i| c.bit(i).is_known()),
                    None => false,
                },
                _ => true,
            },
            _ => false,
        }
    }

    // --- the rewrite ------------------------------------------------------

    /// The netlist connection `id` is replaced by, cached.
    fn lower(&mut self, id: ExprId, ctx: &str) -> ExprId {
        if let Some(new) = self.memo.get(&id) {
            return *new;
        }
        if self.is_structural(id) {
            return id;
        }
        let new = self.lower_node(id, ctx, Sink::Fresh);
        self.memo.insert(id, new);
        new
    }

    /// Rewrites one node whose operands are not yet rewritten.
    fn lower_node(&mut self, id: ExprId, ctx: &str, sink: Sink) -> ExprId {
        let kind = self.module.expr(id).kind.clone();
        let ty = self.module.expr(id).ty.clone();
        let span = self.module.expr(id).span;
        if !ty.is_bits() {
            // Strings, arrays, integers and reals are not netlist
            // connections and have no cell form; the emitters report them.
            return id;
        }
        match kind {
            ExprKind::Const(_) | ExprKind::String(_) | ExprKind::Net(_) => id,
            ExprKind::Slice { base, hi, lo } => {
                if !self.module.expr(base).ty.is_bits() {
                    return id;
                }
                let b = self.lower(base, ctx);
                self.rebuild(id, ExprKind::Slice { base: b, hi, lo }, &ty, span)
            }
            ExprKind::Concat(parts) => {
                let new: Vec<ExprId> = parts.iter().map(|p| self.lower(*p, ctx)).collect();
                self.rebuild(id, ExprKind::Concat(new), &ty, span)
            }
            ExprKind::Replicate { count, expr } => {
                let e = self.lower(expr, ctx);
                self.rebuild(id, ExprKind::Replicate { count, expr: e }, &ty, span)
            }
            ExprKind::Resize {
                expr,
                width,
                signed,
            } => {
                if !self.module.expr(expr).ty.is_bits() {
                    return id;
                }
                let e = self.lower(expr, ctx);
                self.rebuild(
                    id,
                    ExprKind::Resize {
                        expr: e,
                        width,
                        signed,
                    },
                    &ty,
                    span,
                )
            }
            ExprKind::Index { base, index } => self.lower_index(id, base, index, ctx, span),
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => self.lower_indexed_slice(id, base, offset, width, up, ctx, span),
            ExprKind::Unary { op, expr } => self.lower_unary(op, expr, &ty, ctx, sink, span),
            ExprKind::Binary { op, lhs, rhs } => {
                self.lower_binary(id, op, lhs, rhs, &ty, ctx, sink, span)
            }
            ExprKind::Ternary { cond, then_, else_ } => {
                let s = self.lower(cond, ctx);
                let t = self.lower(then_, ctx);
                let f = self.lower(else_, ctx);
                self.emit_cell(
                    sink,
                    ctx,
                    CellKind::Mux,
                    vec![("a", f), ("b", t), ("s", s)],
                    "y",
                    ty,
                    span,
                )
            }
            ExprKind::MemRead { mem, addr } => {
                let a = self.lower(addr, ctx);
                let base = format!("{}$rd", self.module.memories[mem].name);
                self.emit_cell(
                    sink,
                    &base,
                    CellKind::MemRdPort {
                        mem,
                        clocked: false,
                    },
                    vec![("addr", a)],
                    "data",
                    ty,
                    span,
                )
            }
            ExprKind::Call { name, .. } => {
                self.reject(span, format!("a call to `{name}`"));
                id
            }
        }
    }

    /// Rebuilds a structural node over rewritten operands, reusing the
    /// original when nothing changed.
    fn rebuild(&mut self, id: ExprId, kind: ExprKind, ty: &Type, span: Span) -> ExprId {
        if crate::ir::expr::operands(&kind) == crate::ir::expr::operands(&self.module.expr(id).kind)
        {
            return id;
        }
        let new = mk(self.module, kind, span);
        debug_assert_eq!(
            &self.module.expr(new).ty,
            ty,
            "cellify changed a node's type"
        );
        new
    }

    fn lower_unary(
        &mut self,
        op: UnaryOp,
        expr: ExprId,
        ty: &Type,
        ctx: &str,
        sink: Sink,
        span: Span,
    ) -> ExprId {
        let a = self.lower(expr, ctx);
        let operand_ty = self.module.expr(a).ty.clone();
        let one = Type::bit();
        let reduce = |cx: &mut Self, kind: CellKind, sink: Sink| {
            cx.emit_cell(sink, ctx, kind, vec![("a", a)], "y", one.clone(), span)
        };
        match op {
            UnaryOp::Not => self.emit_cell(
                sink,
                ctx,
                CellKind::Not,
                vec![("a", a)],
                "y",
                ty.clone(),
                span,
            ),
            UnaryOp::Neg => {
                let width = operand_ty.width().unwrap_or(1);
                let zero = mk_const(
                    self.module,
                    Const::zero(width).with_signed(operand_ty.is_signed()),
                    span,
                );
                self.emit_cell(
                    sink,
                    ctx,
                    CellKind::Sub,
                    vec![("a", zero), ("b", a)],
                    "y",
                    ty.clone(),
                    span,
                )
            }
            UnaryOp::ReduceAnd => reduce(self, CellKind::ReduceAnd, sink),
            UnaryOp::ReduceOr => reduce(self, CellKind::ReduceOr, sink),
            UnaryOp::ReduceXor => reduce(self, CellKind::ReduceXor, sink),
            UnaryOp::ReduceNand => {
                let inner = reduce(self, CellKind::ReduceAnd, Sink::Fresh);
                self.invert(sink, ctx, inner, span)
            }
            UnaryOp::ReduceNor | UnaryOp::LogicNot => {
                let inner = reduce(self, CellKind::ReduceOr, Sink::Fresh);
                self.invert(sink, ctx, inner, span)
            }
            UnaryOp::ReduceXnor => {
                let inner = reduce(self, CellKind::ReduceXor, Sink::Fresh);
                self.invert(sink, ctx, inner, span)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_binary(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        lhs: ExprId,
        rhs: ExprId,
        ty: &Type,
        ctx: &str,
        sink: Sink,
        span: Span,
    ) -> ExprId {
        match op {
            BinaryOp::WildEq => self.lower_wildcard(id, lhs, rhs, ctx, sink, span),
            BinaryOp::Pow => {
                self.reject(span, format!("the `{}` operator", op.name()));
                id
            }
            BinaryOp::Xnor => {
                let a = self.lower(lhs, ctx);
                let b = self.lower(rhs, ctx);
                let inner = self.emit_cell(
                    Sink::Fresh,
                    ctx,
                    CellKind::Xor,
                    vec![("a", a), ("b", b)],
                    "y",
                    ty.clone(),
                    span,
                );
                self.invert_wide(sink, ctx, inner, ty.clone(), span)
            }
            _ => {
                let kind = binary_cell(op).expect("every other operator has a cell");
                let a = self.lower(lhs, ctx);
                let b = self.lower(rhs, ctx);
                self.emit_cell(
                    sink,
                    ctx,
                    kind,
                    vec![("a", a), ("b", b)],
                    "y",
                    ty.clone(),
                    span,
                )
            }
        }
    }

    /// `weq(a, pattern)`: an `eq` over the bits the pattern pins down.
    fn lower_wildcard(
        &mut self,
        id: ExprId,
        lhs: ExprId,
        rhs: ExprId,
        ctx: &str,
        sink: Sink,
        span: Span,
    ) -> ExprId {
        let Some(pattern) = self.module.expr(rhs).as_const().cloned() else {
            self.reject(span, "a `weq` whose pattern is not a constant".to_owned());
            return id;
        };
        let kept: Vec<u32> = (0..pattern.width())
            .filter(|i| pattern.bit(*i).is_known())
            .collect();
        if kept.is_empty() {
            // Every bit is a wildcard: the comparison always holds.
            return mk_const(self.module, Const::from_bool(true), span);
        }
        let a = self.lower(lhs, ctx);
        let (a, want) = if kept.len() == pattern.width() as usize {
            (a, pattern.clone())
        } else {
            let parts: Vec<ExprId> = kept
                .iter()
                .rev()
                .map(|i| mk_slice(self.module, a, *i, *i, span))
                .collect();
            let bits: Vec<Bit> = kept.iter().map(|i| pattern.bit(*i)).collect();
            (mk_concat(self.module, parts, span), Const::from_bits(&bits))
        };
        let b = mk_const(self.module, want, span);
        self.emit_cell(
            sink,
            ctx,
            CellKind::Eq,
            vec![("a", a), ("b", b)],
            "y",
            Type::bit(),
            span,
        )
    }

    /// `base[index]` with a variable index: `shr(base, index)[0]`.
    fn lower_index(
        &mut self,
        id: ExprId,
        base: ExprId,
        index: ExprId,
        ctx: &str,
        span: Span,
    ) -> ExprId {
        if !self.module.expr(base).ty.is_bits() {
            return id;
        }
        if self.module.expr(index).as_const().is_some() {
            let b = self.lower(base, ctx);
            let ty = self.module.expr(id).ty.clone();
            return self.rebuild(id, ExprKind::Index { base: b, index }, &ty, span);
        }
        let shifted = self.shift_down(base, index, ctx, span);
        mk_slice(self.module, shifted, 0, 0, span)
    }

    /// `base[offset +: width]` / `base[offset -: width]` with a variable
    /// offset: `shr(base, offset)[width-1:0]`, the `-:` form shifting by
    /// `offset - (width - 1)`.
    #[allow(clippy::too_many_arguments)]
    fn lower_indexed_slice(
        &mut self,
        id: ExprId,
        base: ExprId,
        offset: ExprId,
        width: u32,
        up: bool,
        ctx: &str,
        span: Span,
    ) -> ExprId {
        if !self.module.expr(base).ty.is_bits() {
            return id;
        }
        if self.module.expr(offset).as_const().is_some() {
            let b = self.lower(base, ctx);
            let ty = self.module.expr(id).ty.clone();
            return self.rebuild(
                id,
                ExprKind::IndexedSlice {
                    base: b,
                    offset,
                    width,
                    up,
                },
                &ty,
                span,
            );
        }
        let amount = if up || width <= 1 {
            offset
        } else {
            let off = self.lower(offset, ctx);
            let off_ty = self.module.expr(off).ty.clone();
            let bias = mk_const(
                self.module,
                Const::from_u64(u64::from(width - 1), off_ty.width().unwrap_or(1))
                    .with_signed(off_ty.is_signed()),
                span,
            );
            self.emit_cell(
                Sink::Fresh,
                ctx,
                CellKind::Sub,
                vec![("a", off), ("b", bias)],
                "y",
                off_ty,
                span,
            )
        };
        let shifted = self.shift_down(base, amount, ctx, span);
        mk_slice(self.module, shifted, width - 1, 0, span)
    }

    /// A `shr` cell shifting `base` right by `amount`, both cellified.
    fn shift_down(&mut self, base: ExprId, amount: ExprId, ctx: &str, span: Span) -> ExprId {
        let b = self.lower(base, ctx);
        let n = self.lower(amount, ctx);
        let ty = self.module.expr(b).ty.clone();
        self.emit_cell(
            Sink::Fresh,
            ctx,
            CellKind::Shr,
            vec![("a", b), ("b", n)],
            "y",
            ty,
            span,
        )
    }

    /// A one-bit `not` cell.
    fn invert(&mut self, sink: Sink, ctx: &str, a: ExprId, span: Span) -> ExprId {
        self.invert_wide(sink, ctx, a, Type::bit(), span)
    }

    /// A `not` cell of the given width.
    fn invert_wide(&mut self, sink: Sink, ctx: &str, a: ExprId, ty: Type, span: Span) -> ExprId {
        self.emit_cell(sink, ctx, CellKind::Not, vec![("a", a)], "y", ty, span)
    }

    // --- construction -----------------------------------------------------

    /// Adds a cell driving `sink`, and returns the net reference that
    /// replaces the expression node.
    #[allow(clippy::too_many_arguments)]
    fn emit_cell(
        &mut self,
        sink: Sink,
        ctx: &str,
        kind: CellKind,
        inputs: Vec<(&str, ExprId)>,
        out_port: &str,
        out_ty: Type,
        span: Span,
    ) -> ExprId {
        let base = format!("{ctx}${}", kind.keyword());
        let name = self.fresh(&base);
        let net = match sink {
            Sink::Net(net) => net,
            Sink::Fresh => self.module.nets.push(Net {
                name: name.clone(),
                ty: out_ty,
                kind: NetKind::Wire,
                attrs: Attrs::new(),
                span,
            }),
        };
        let cell = self.module.cells.push(Cell {
            name,
            kind,
            inputs: inputs.into_iter().map(|(p, e)| (Name::new(p), e)).collect(),
            outputs: vec![(Name::new(out_port), net)],
            params: Attrs::new(),
            attrs: Attrs::new(),
            span,
        });
        self.last_cell = Some(cell);
        self.created.push((cell, self.anchor));
        self.stats.bump("cells created", 1);
        mk_net(self.module, net, span)
    }

    /// A name used by no net and no cell, derived from `base`.
    fn fresh(&mut self, base: &str) -> Name {
        let mut candidate = base.to_owned();
        let mut i = 2u32;
        while self.taken.contains(&candidate) {
            candidate = format!("{base}_{i}");
            i += 1;
        }
        self.taken.insert(candidate.clone());
        Name::new(candidate)
    }

    /// Reports a construct with no cell form.
    fn reject(&mut self, span: Span, what: String) {
        self.diags.push(
            Diagnostic::error(format!("{what} cannot be turned into cells"))
                .with_code("S0030")
                .with_span(span)
                .with_note("the netlist formats cannot express it; rewrite it in the source"),
        );
    }

    // --- structural test --------------------------------------------------

    /// True when `id` is already a netlist connection, that is, exactly
    /// what [`crate::ir::emit::BitView`] evaluates to bits.
    fn is_structural(&mut self, id: ExprId) -> bool {
        if let Some(known) = self.structural.get(&id) {
            return *known;
        }
        let kind = self.module.expr(id).kind.clone();
        let answer = match &kind {
            ExprKind::Const(_) | ExprKind::Net(_) => true,
            ExprKind::Slice { base, .. } => {
                self.module.expr(*base).ty.is_bits() && self.is_structural(*base)
            }
            ExprKind::Index { base, index } => {
                self.module.expr(*base).ty.is_bits()
                    && self.module.expr(*index).as_const().is_some()
                    && self.is_structural(*base)
            }
            ExprKind::IndexedSlice { base, offset, .. } => {
                self.module.expr(*base).ty.is_bits()
                    && self.module.expr(*offset).as_const().is_some()
                    && self.is_structural(*base)
            }
            ExprKind::Concat(parts) => parts.iter().all(|p| self.is_structural(*p)),
            ExprKind::Replicate { expr, .. } => self.is_structural(*expr),
            ExprKind::Resize { expr, .. } => {
                self.module.expr(*expr).ty.is_bits() && self.is_structural(*expr)
            }
            ExprKind::String(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Ternary { .. }
            | ExprKind::MemRead { .. }
            | ExprKind::Call { .. } => false,
        };
        self.structural.insert(id, answer);
        answer
    }
}

/// The cell kind implementing a binary operator directly, if any.
fn binary_cell(op: BinaryOp) -> Option<CellKind> {
    Some(match op {
        BinaryOp::And | BinaryOp::LogicAnd => CellKind::And,
        BinaryOp::Or | BinaryOp::LogicOr => CellKind::Or,
        BinaryOp::Xor => CellKind::Xor,
        BinaryOp::Add => CellKind::Add,
        BinaryOp::Sub => CellKind::Sub,
        BinaryOp::Mul => CellKind::Mul,
        BinaryOp::Div => CellKind::Div,
        BinaryOp::Mod => CellKind::Mod,
        BinaryOp::Shl => CellKind::Shl,
        BinaryOp::Shr => CellKind::Shr,
        BinaryOp::Sshr => CellKind::Sshr,
        BinaryOp::Eq | BinaryOp::CaseEq => CellKind::Eq,
        BinaryOp::Ne | BinaryOp::CaseNe => CellKind::Ne,
        BinaryOp::Lt => CellKind::Lt,
        BinaryOp::Le => CellKind::Le,
        BinaryOp::Gt => CellKind::Gt,
        BinaryOp::Ge => CellKind::Ge,
        BinaryOp::Xnor | BinaryOp::Pow | BinaryOp::WildEq => return None,
    })
}

/// A name for the cells driving an assignment target, for readable output.
fn lvalue_name(m: &Module, lv: &Lvalue) -> String {
    match lv {
        Lvalue::Net(n) | Lvalue::Slice { net: n, .. } | Lvalue::Index { net: n, .. } => {
            m.nets[*n].name.to_string()
        }
        Lvalue::Concat(parts) => parts
            .first()
            .map_or_else(|| "assign".to_owned(), |p| lvalue_name(m, p)),
        Lvalue::MemElem { mem, .. } => m.memories[*mem].name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::emit::BitView;
    use crate::ir::validate::validate_module;
    use crate::ir::{AttrValue, MemoryId, PortDir};
    use crate::logic::Bit as LogicBit;
    use crate::source::SourceMap;
    use crate::synth::eval::{Env, eval, merge_unknown};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// xorshift64*, so the vectors are reproducible without a dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    /// Net values, plus the contents of every memory (taken from its
    /// `init`), so both forms read the same storage.
    struct State {
        nets: HashMap<NetId, Const>,
        mems: HashMap<MemoryId, Vec<Const>>,
    }

    impl Env for State {
        fn net(&mut self, net: NetId) -> Option<Const> {
            self.nets.get(&net).cloned()
        }

        fn mem(&mut self, mem: MemoryId, addr: &Const) -> Option<Const> {
            let cells = self.mems.get(&mem)?;
            let width = cells.first()?.width();
            Some(
                addr.to_u64()
                    .and_then(|a| cells.get(usize::try_from(a).ok()?))
                    .cloned()
                    .unwrap_or_else(|| Const::x(width)),
            )
        }
    }

    impl State {
        /// Random two-state values for every input port.
        fn random(m: &Module, rng: &mut Rng) -> State {
            let mut nets = HashMap::new();
            for port in &m.ports {
                if port.dir != PortDir::In {
                    continue;
                }
                let ty = &m.nets[port.net].ty;
                let width = ty.width().unwrap_or(1);
                let value = Const::from_u64(rng.next(), 64).resize(width);
                nets.insert(port.net, value.with_signed(ty.is_signed()));
            }
            let mut mems = HashMap::new();
            for (id, mem) in m.memories.iter() {
                if let Some(init) = &mem.init {
                    mems.insert(id, init.clone());
                }
            }
            State { nets, mems }
        }

        /// Evaluates every cell of `m` in arena order (which cellify
        /// leaves in dependency order) and records what it drives.
        fn run_cells(&mut self, m: &Module) {
            for (_, cell) in m.cells.iter() {
                let input = |port: &str, state: &mut State| {
                    eval(m, cell.input(port).expect("port"), state).expect("input evaluates")
                };
                let value = match &cell.kind {
                    CellKind::MemRdPort { mem, .. } => {
                        let addr = input("addr", self);
                        self.mem(*mem, &addr).expect("memory")
                    }
                    CellKind::Not => input("a", self).not(),
                    CellKind::ReduceAnd => input("a", self).reduce_and(),
                    CellKind::ReduceOr => input("a", self).reduce_or(),
                    CellKind::ReduceXor => input("a", self).reduce_xor(),
                    CellKind::Mux => {
                        let a = input("a", self);
                        let b = input("b", self);
                        match input("s", self).truth() {
                            LogicBit::One => b,
                            LogicBit::Zero => a,
                            _ => merge_unknown(&b, &a),
                        }
                    }
                    kind => {
                        let a = input("a", self);
                        let b = input("b", self);
                        match kind {
                            CellKind::And => a.and(&b),
                            CellKind::Or => a.or(&b),
                            CellKind::Xor => a.xor(&b),
                            CellKind::Add => a.add(&b),
                            CellKind::Sub => a.sub(&b),
                            CellKind::Mul => a.mul(&b),
                            CellKind::Div => a.div(&b),
                            CellKind::Mod => a.rem(&b),
                            CellKind::Shl => a.shl_by(&b),
                            CellKind::Shr => a.shr_by(&b),
                            CellKind::Sshr => a.sshr_by(&b),
                            CellKind::Eq => a.eq(&b),
                            CellKind::Ne => a.ne(&b),
                            CellKind::Lt => a.lt(&b),
                            CellKind::Le => a.le(&b),
                            CellKind::Gt => a.gt(&b),
                            CellKind::Ge => a.ge(&b),
                            other => panic!("cellify created a {other:?} cell"),
                        }
                    }
                };
                let (_, net) = cell.outputs[0];
                let ty = &m.nets[net].ty;
                assert_eq!(
                    value.width(),
                    ty.width().unwrap_or(0),
                    "cell `{}` drives `{}` with the wrong width",
                    cell.name,
                    m.nets[net].name
                );
                self.nets.insert(net, value.with_signed(ty.is_signed()));
            }
            for assign in &m.assigns {
                let value = eval(m, assign.value, self).expect("assign evaluates");
                if let Lvalue::Net(net) = &assign.target {
                    self.nets.insert(*net, value);
                }
            }
        }
    }

    /// Runs the pass on `m`, checking the module stays valid.
    fn cellify(m: &mut Module) -> Diagnostics {
        let mut diags = Diagnostics::new();
        Cellify.run(m, &mut diags);
        let problems = validate_module(m);
        assert!(
            !problems.has_errors(),
            "cellify left the module invalid:\n{}\n{}",
            problems
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            m.to_text()
        );
        diags
    }

    /// Compares the outputs of `before` and `after` over random vectors.
    /// Unknown bits of the reference are don't-care, as everywhere else
    /// in synthesis.
    fn compare(before: &Module, after: &Module, vectors: usize, seed: u64) {
        let mut rng = Rng(seed);
        for vector in 0..vectors {
            let mut a = State::random(before, &mut rng);
            let mut b = State {
                nets: a.nets.clone(),
                mems: a.mems.clone(),
            };
            let reference: Vec<(String, Option<Const>)> = before
                .assigns
                .iter()
                .map(|assign| {
                    let name = match &assign.target {
                        Lvalue::Net(n) => before.nets[*n].name.to_string(),
                        other => format!("{other:?}"),
                    };
                    (name, eval(before, assign.value, &mut a))
                })
                .collect();
            b.run_cells(after);
            for (name, want) in reference {
                let Some(want) = want else { continue };
                let net = after.net_by_name(&name).expect("output survives");
                let got = b.nets.get(&net).expect("output is driven").clone();
                assert_eq!(want.width(), got.width(), "`{name}` width");
                for i in 0..want.width() {
                    let (w, g) = (want.bit(i), got.bit(i));
                    assert!(
                        !w.is_known() || w == g,
                        "vector {vector}: `{name}` bit {i}: want {w:?}, got {g:?}\n{}",
                        after.to_text()
                    );
                }
            }
        }
    }

    /// One module holding one assignment per expression kind.
    fn kitchen_sink() -> Module {
        let mut b = ModuleBuilder::new("all", span());
        let a = b.input("a", Type::bits(8));
        let c = b.input("c", Type::bits(8));
        let sa = b.input("sa", Type::sbits(8));
        let sb = b.input("sb", Type::sbits(8));
        let s = b.input("s", Type::bit());
        let t = b.input("t", Type::bit());
        let idx = b.input("idx", Type::bits(3));
        let mem = b.memory("rom", Type::bits(8), 8);
        b.module_mut().memories[mem].init =
            Some((0..8).map(|i| Const::from_u64(i * 37 + 1, 8)).collect());

        let (av, cv, sav, sbv, sv, tv, iv) = (
            b.net(a),
            b.net(c),
            b.net(sa),
            b.net(sb),
            b.net(s),
            b.net(t),
            b.net(idx),
        );

        fn out(b: &mut ModuleBuilder, name: &str, value: ExprId) {
            let ty = b.module().expr(value).ty.clone();
            let net = b.add_net(name, ty);
            b.add_port(name, PortDir::Out, net);
            b.assign(net, value);
        }

        for (name, op) in [
            ("u_not", UnaryOp::Not),
            ("u_neg", UnaryOp::Neg),
            ("u_rand", UnaryOp::ReduceAnd),
            ("u_ror", UnaryOp::ReduceOr),
            ("u_rxor", UnaryOp::ReduceXor),
            ("u_rnand", UnaryOp::ReduceNand),
            ("u_rnor", UnaryOp::ReduceNor),
            ("u_rxnor", UnaryOp::ReduceXnor),
            ("u_lnot", UnaryOp::LogicNot),
        ] {
            let e = b.unary(op, av);
            out(&mut b, name, e);
        }
        // The same, on a signed operand, so signedness is exercised.
        let neg_signed = b.unary(UnaryOp::Neg, sav);
        out(&mut b, "u_neg_s", neg_signed);

        for (name, op) in [
            ("b_and", BinaryOp::And),
            ("b_or", BinaryOp::Or),
            ("b_xor", BinaryOp::Xor),
            ("b_xnor", BinaryOp::Xnor),
            ("b_add", BinaryOp::Add),
            ("b_sub", BinaryOp::Sub),
            ("b_mul", BinaryOp::Mul),
            ("b_div", BinaryOp::Div),
            ("b_mod", BinaryOp::Mod),
            ("b_shl", BinaryOp::Shl),
            ("b_shr", BinaryOp::Shr),
            ("b_sshr", BinaryOp::Sshr),
            ("b_eq", BinaryOp::Eq),
            ("b_ne", BinaryOp::Ne),
            ("b_ceq", BinaryOp::CaseEq),
            ("b_cne", BinaryOp::CaseNe),
            ("b_lt", BinaryOp::Lt),
            ("b_le", BinaryOp::Le),
            ("b_gt", BinaryOp::Gt),
            ("b_ge", BinaryOp::Ge),
        ] {
            let e = b.binary(op, av, cv);
            out(&mut b, name, e);
        }
        for (name, op) in [
            ("s_add", BinaryOp::Add),
            ("s_sub", BinaryOp::Sub),
            ("s_mul", BinaryOp::Mul),
            ("s_div", BinaryOp::Div),
            ("s_lt", BinaryOp::Lt),
            ("s_ge", BinaryOp::Ge),
            ("s_sshr", BinaryOp::Sshr),
        ] {
            let e = b.binary(op, sav, sbv);
            out(&mut b, name, e);
        }
        for (name, op) in [("l_and", BinaryOp::LogicAnd), ("l_or", BinaryOp::LogicOr)] {
            let e = b.binary(op, sv, tv);
            out(&mut b, name, e);
        }

        let tern = b.mux(sv, av, cv);
        out(&mut b, "ternary", tern);
        let tern_s = b.mux(tv, sav, sbv);
        out(&mut b, "ternary_s", tern_s);

        let read = b.mem_read(mem, iv);
        out(&mut b, "mem_read", read);

        // Structural wrappers over cellified operands.
        let sum = b.add(av, cv);
        let sl = b.slice(sum, 5, 2);
        out(&mut b, "slice_of_add", sl);
        let rep = b.replicate(2, sl);
        out(&mut b, "replicate", rep);
        let cat = b.concat(vec![sl, av]);
        out(&mut b, "concat", cat);
        let widened = b.zext(sum, 12);
        out(&mut b, "zext", widened);
        let signed_sum = b.add(sav, sbv);
        let sext = b.sext(signed_sum, 12);
        out(&mut b, "sext", sext);
        let trunc = b.resize(sum, 4, false);
        out(&mut b, "trunc", trunc);
        let k = b.const_u64(3, 5);
        let const_index = b.index(sum, k);
        out(&mut b, "const_index", const_index);
        let const_slice = b.indexed_slice(sum, k, 2, false);
        out(&mut b, "const_indexed_slice", const_slice);

        // Variable selects, with an index that cannot go out of range.
        let var_index = b.index(av, iv);
        out(&mut b, "var_index", var_index);
        let var_up = b.indexed_slice(av, iv, 1, true);
        out(&mut b, "var_up", var_up);

        // A nested tree whose operands are shared with other roots.
        let deep = b.xor(sum, tern);
        let deeper = b.mux(sv, deep, sum);
        out(&mut b, "deep", deeper);
        b.finish()
    }

    #[test]
    fn every_expression_kind_becomes_cells() {
        let before = kitchen_sink();
        let mut after = before.clone();
        let diags = cellify(&mut after);
        assert!(!diags.has_errors(), "{:?}", diags.iter().next());
        BitView::new(&after).expect("the result is a netlist");
        compare(&before, &after, 64, 0x5EED);
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let mut m = kitchen_sink();
        cellify(&mut m);
        let text = m.to_text();
        let diags = cellify(&mut m);
        assert!(!diags.has_errors());
        assert_eq!(m.to_text(), text, "cellify is not idempotent");
    }

    /// `weq` keeps only the bits its pattern pins down; an all-wildcard
    /// pattern folds to 1.
    #[test]
    fn wildcard_patterns_compare_the_known_bits() {
        let mut b = ModuleBuilder::new("wild", span());
        let a = b.input("a", Type::bits(4));
        let hit = b.output("hit", Type::bit());
        let any = b.output("any", Type::bit());
        let exact = b.output("exact", Type::bit());
        let av = b.net(a);
        let pattern = b.constant(Const::parse_verilog("4'bzz1z").unwrap());
        let all = b.constant(Const::parse_verilog("4'bzzzz").unwrap());
        let full = b.const_u64(4, 9);
        let e = b.binary(BinaryOp::WildEq, av, pattern);
        b.assign(hit, e);
        let e = b.binary(BinaryOp::WildEq, av, all);
        b.assign(any, e);
        let e = b.binary(BinaryOp::WildEq, av, full);
        b.assign(exact, e);
        let before = b.finish();
        let mut after = before.clone();
        let diags = cellify(&mut after);
        assert!(!diags.has_errors());
        BitView::new(&after).expect("the result is a netlist");
        let text = after.to_text();
        assert!(
            text.contains("eq (a=%a[1:1], b=1'd1) -> (y=%hit)"),
            "{text}"
        );
        assert!(text.contains("assign %any = 1'd1"), "{text}");
        assert!(text.contains("eq (a=%a, b=4'd9) -> (y=%exact)"), "{text}");
        compare(&before, &after, 32, 7);
    }

    /// A variable select becomes a shifter; out of range it reads zero,
    /// which is a legal resolution of the IR's `x`.
    #[test]
    fn variable_selects_become_shifters() {
        let mut b = ModuleBuilder::new("sel", span());
        let a = b.input("a", Type::bits(4));
        let i = b.input("i", Type::bits(4));
        let bit = b.output("bit", Type::bit());
        let up = b.output("up", Type::bits(2));
        let down = b.output("down", Type::bits(2));
        let (av, iv) = (b.net(a), b.net(i));
        let e = b.index(av, iv);
        b.assign(bit, e);
        let e = b.indexed_slice(av, iv, 2, true);
        b.assign(up, e);
        let e = b.indexed_slice(av, iv, 2, false);
        b.assign(down, e);
        let mut m = b.finish();
        cellify(&mut m);
        BitView::new(&m).expect("the result is a netlist");
        let text = m.to_text();
        assert_eq!(text.matches(" shr ").count(), 3, "{text}");
        assert_eq!(text.matches(" sub ").count(), 1, "{text}");

        let mut state = State {
            nets: HashMap::new(),
            mems: HashMap::new(),
        };
        let (a, i) = (m.net_by_name("a").unwrap(), m.net_by_name("i").unwrap());
        let (bit, up) = (m.net_by_name("bit").unwrap(), m.net_by_name("up").unwrap());
        for (index, want_bit, want_up) in [(0u64, 1u64, 0b01u64), (3, 1, 0b01), (7, 0, 0b00)] {
            state.nets.insert(a, Const::from_u64(0b1001, 4));
            state.nets.insert(i, Const::from_u64(index, 4));
            state.run_cells(&m);
            assert_eq!(state.nets[&bit].to_u64(), Some(want_bit), "a[{index}]");
            assert_eq!(state.nets[&up].to_u64(), Some(want_up), "a[{index} +: 2]");
        }
    }

    /// Whole-net assignments become cells that drive the net, keeping the
    /// assignment's attributes.
    #[test]
    fn assignments_become_the_driving_cell() {
        let mut b = ModuleBuilder::new("drive", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let wide = b.output("wide", Type::bits(8));
        let (av, cv) = (b.net(a), b.net(c));
        let sum = b.add(av, cv);
        b.assign(y, sum);
        b.module_mut().assigns[0].attrs.set("keep", 1);
        // A width change keeps the assignment: the cell cannot drive it.
        let ext = b.zext(sum, 8);
        b.assign(wide, ext);
        let mut m = b.finish();
        cellify(&mut m);
        assert_eq!(m.assigns.len(), 1);
        assert_eq!(m.cells.len(), 1);
        let cell = m.cells.values().next().unwrap();
        assert_eq!(cell.name, "y$add");
        assert_eq!(cell.output("y"), Some(m.net_by_name("y").unwrap()));
        assert_eq!(cell.attrs.get("keep"), Some(&AttrValue::Int(1)));
        // The shared sum is cellified once.
        assert_eq!(m.to_text().matches(" add ").count(), 1);
        BitView::new(&m).expect("the result is a netlist");
    }

    /// What has no cell form is reported, and the design stays valid.
    #[test]
    fn unsupported_constructs_are_reported() {
        let mut b = ModuleBuilder::new("nope", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let p = b.output("p", Type::bits(4));
        let f = b.output("f", Type::bits(4));
        let w = b.output("w", Type::bit());
        let (av, cv) = (b.net(a), b.net(c));
        let e = b.binary(BinaryOp::Pow, av, cv);
        b.assign(p, e);
        let e = b.call("$clog2", vec![av], Type::bits(4));
        b.assign(f, e);
        let e = b.binary(BinaryOp::WildEq, av, cv);
        b.assign(w, e);
        let mut m = b.finish();
        let diags = cellify(&mut m);
        let messages: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages.len(), 3, "{messages:?}");
        assert!(diags.iter().all(|d| d.code == Some("S0030")));
        assert!(messages[0].contains("`pow`"), "{messages:?}");
        assert!(messages[1].contains("`$clog2`"), "{messages:?}");
        assert!(messages[2].contains("`weq`"), "{messages:?}");
        assert!(BitView::new(&m).is_err());
    }

    /// A value that is not a bit vector has no cell form; the pass leaves
    /// it (and its assignment) alone rather than dropping the driver.
    #[test]
    fn non_vector_values_are_left_alone() {
        let mut b = ModuleBuilder::new("ints", span());
        let i = b.add_net("i", Type::Integer);
        let j = b.add_net("j", Type::Integer);
        let iv = b.net(i);
        let e = b.neg(iv);
        b.assign(j, e);
        let mut m = b.finish();
        let diags = cellify(&mut m);
        assert!(!diags.has_errors());
        assert_eq!(m.cells.len(), 0);
        assert_eq!(m.assigns.len(), 1);
        assert_eq!(m.assigns[0].value, e);
    }

    /// Cell inputs and instance connections are roots too, and the cells
    /// that feed them are placed in front of their reader.
    #[test]
    fn cell_and_instance_ports_are_cellified() {
        let mut b = ModuleBuilder::new("ports", span());
        let clk = b.input("clk", Type::bit());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let q = b.output("q", Type::bits(4));
        let (clkv, av, cv) = (b.net(clk), b.net(a), b.net(c));
        let sum = b.add(av, cv);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clkv), (Name::new("d"), sum)],
            vec![(Name::new("q"), q)],
        );
        let diff = b.sub(av, cv);
        b.instance(
            "u0",
            crate::ir::ModuleRef::Unresolved(Name::new("leaf")),
            vec![(Name::new("x"), diff)],
        );
        let mut m = b.finish();
        cellify(&mut m);
        let text = m.to_text();
        assert!(
            text.contains("cell ff$add add (a=%a, b=%c) -> (y=%ff$add)"),
            "{text}"
        );
        assert!(text.contains("d=%ff$add)"), "{text}");
        assert!(
            text.contains("cell u0$sub sub (a=%a, b=%c) -> (y=%u0$sub)"),
            "{text}"
        );
        assert!(text.contains("(x=%u0$sub)"), "{text}");
        let cells: Vec<&str> = m.cells.values().map(|c| c.name.as_str()).collect();
        assert_eq!(cells, ["u0$sub", "ff$add", "ff"]);
    }
}
