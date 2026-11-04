//! Which flip-flops around a multiply go into the `DSP48E1` that takes it.
//!
//! A block has two registers on each operand (`A1`/`A2`, `B1`/`B2`), one
//! after the multiplier (`M`) and one after the adder (`P`). A flip-flop that
//! moves into one is a flip-flop off the fabric and a path that stops being
//! long, so this is most of what a hard multiplier is worth: `mul18` of
//! `testdata/fpga/vivado/qor/` is one block and no register around it.
//!
//! # The rule, and where it is from
//!
//! **Measured**, from `testdata/fpga/vivado/dsp/inference.txt`, which is what
//! Vivado 2026.1 made of a design per question
//! (`tools/vivado-parity/gen_dsp_inference.py`):
//!
//! | design | stages written | Vivado's registers |
//! |---|---|---|
//! | `r_none` | none, operands from a shift register | `AREG` 1, `BREG` 2 (the shift register's bit 0 has a pad behind it) |
//! | `r_m1` | one after | `PREG`, not `MREG` |
//! | `r_m2` | two after | `MREG`, `PREG` |
//! | `r_m3` | three after | `MREG`, `PREG`, and the third into `A2`/`B2` |
//! | `r_a1m2` | one before, two after | `AREG` 2 (the shift register too), `MREG`, `PREG` |
//! | `r_a2` | two before | `AREG` 2, and the third back, the shift register, into `MREG` |
//! | `r_a3` | three before | `AREG` 2, `MREG`; the rest stays in the fabric |
//! | `c_fan` | every register also read elsewhere | as `r_a1m2`, with those kept in the fabric too |
//! | `c_en` | a common enable | `CEA2`, `CEB2`, `CEM`, `CEP` on it; `CEA1` and `CEB1`, whose stage has none, at one |
//! | `c_rst` | a common synchronous reset | `RSTA`, `RSTB`, `RSTM`, `RSTP`, and `A1` not used, since `RSTA` would reset the shift register's copy too |
//! | `c_rst1` | a reset to one | none of them |
//! | `c_init` | an initial value | none of them |
//!
//! So an operand's register goes in **even when something else reads it**:
//! the block holds a copy, and the original stays for its other reader, which
//! is why a shift register's bits can be `A1` and still be there.
//!
//! **What is not Vivado's**: `c_arst`, an asynchronous reset, where Vivado
//! does something this does not understand (`PREG` with its reset at zero and
//! the middle register gone); here such a flip-flop stays out, which is
//! correct and larger. `f_square`, a square, where Vivado writes `AREG` 1,
//! `MREG` and `PREG` for what this writes `AREG` 2 and `PREG`: the same
//! latency. Neither is a thing a test here pins as right, only as equivalent.
//!
//! Each move is safe because of what the block's registers are: they power up
//! holding zero, and a stage that moves past the multiplier does so only when
//! it powers up holding zero itself and has no enable or reset of its own,
//! since zero times anything is zero and the two forms then agree from the
//! first edge.

use std::collections::{HashMap, HashSet};

use super::primitives::{expr, slice_expr};
use crate::ir::{
    AttrValue, CellId, CellKind, ExprId, ExprKind, Lvalue, Module, NetId, Type, UnaryOp,
};
use crate::source::Span;

/// The most operand stages an operand could give up: two registers and the
/// multiplier's, reached by moving the last one forward.
const MOST_STAGES: usize = 3;

/// What controls a flip-flop.
#[derive(Clone, Debug)]
pub(super) struct Control {
    /// The clock.
    pub clk: ExprId,
    en: Option<ExprId>,
    /// The reset, always synchronous and to zero, and whether active high.
    rst: Option<(ExprId, bool)>,
}

impl Control {
    fn plain(&self) -> bool {
        self.en.is_none() && self.rst.is_none()
    }
}

/// One flip-flop an operand was traced back through.
#[derive(Clone, Debug)]
struct Stage {
    /// The operand as it is before this stage.
    before: ExprId,
    control: Control,
}

/// The flip-flops of a module and who reads what, gathered once per run of the
/// DSP step.
#[derive(Debug, Default)]
pub(super) struct Index {
    /// The flip-flop driving a net.
    dff_of_q: HashMap<NetId, CellId>,
    /// A net a whole-net continuous assignment drives, and from what.
    alias: HashMap<NetId, ExprId>,
    /// The flip-flops whose `d` is made of a net and nothing else.
    readers: HashMap<NetId, Vec<CellId>>,
    /// Flip-flops that became a block's output register: gone from the fabric,
    /// so nothing may be traced back through them.
    consumed: HashSet<CellId>,
}

/// The net an expression reads, through the operations that change nothing but
/// its width.
fn root_net(module: &Module, e: ExprId) -> Option<NetId> {
    match &module.exprs.get(e)?.kind {
        ExprKind::Net(net) => Some(*net),
        ExprKind::Slice { base, .. } => root_net(module, *base),
        ExprKind::Resize { expr, .. } => root_net(module, *expr),
        _ => None,
    }
}

impl Index {
    pub(super) fn new(module: &Module) -> Index {
        let mut out = Index::default();
        for (id, cell) in module.cells.iter() {
            if !matches!(cell.kind, CellKind::Dff { .. }) {
                continue;
            }
            if let Some(q) = cell.output("q") {
                out.dff_of_q.insert(q, id);
            }
            if let Some(net) = cell.input("d").and_then(|d| root_net(module, d)) {
                out.readers.entry(net).or_default().push(id);
            }
        }
        for assign in &module.assigns {
            if let Lvalue::Net(net) = assign.target
                && assign.delay.is_none()
            {
                out.alias.insert(net, assign.value);
            }
        }
        out
    }
}

/// The control of a flip-flop a block's register can stand in for: on the
/// rising edge, reset synchronously to zero if at all, holding zero at power
/// up, and not something the source asked to keep.
pub(super) fn control_of(module: &Module, id: CellId) -> Option<Control> {
    let cell = module.cells.get(id)?;
    let CellKind::Dff {
        clk_pos: true,
        reset,
        ..
    } = &cell.kind
    else {
        return None;
    };
    let q = cell.output("q")?;
    let attrs = &module.nets.get(q)?.attrs;
    if attrs.is_kept() {
        return None;
    }
    if let Some(init) = attrs.get("init") {
        let zero = match init {
            AttrValue::Const(value) => {
                (0..value.width()).all(|bit| value.bit(bit) == crate::logic::Bit::Zero)
            }
            AttrValue::Int(value) => *value == 0,
            AttrValue::String(_) => false,
        };
        if !zero {
            return None;
        }
    }
    let rst = match reset {
        None => None,
        Some(reset) => {
            let width = reset.value.width();
            let zero = (0..width).all(|bit| reset.value.bit(bit) == crate::logic::Bit::Zero);
            if reset.asynchronous || !zero {
                return None;
            }
            Some((cell.input("rst")?, reset.active_high))
        }
    };
    Some(Control {
        clk: cell.input("clk")?,
        en: cell.input("en"),
        rst,
    })
}

/// True when two expressions are the same signal, as far as a look at their
/// shape can tell: enough for one control net read in several places.
pub(super) fn same(module: &Module, x: ExprId, y: ExprId) -> bool {
    if x == y {
        return true;
    }
    let (Some(a), Some(b)) = (module.exprs.get(x), module.exprs.get(y)) else {
        return false;
    };
    match (&a.kind, &b.kind) {
        (ExprKind::Net(p), ExprKind::Net(q)) => p == q,
        (ExprKind::Const(p), ExprKind::Const(q)) => p == q,
        (
            ExprKind::Slice {
                base: p,
                hi: h0,
                lo: l0,
            },
            ExprKind::Slice {
                base: q,
                hi: h1,
                lo: l1,
            },
        ) => h0 == h1 && l0 == l1 && same(module, *p, *q),
        _ => false,
    }
}

fn same_opt(module: &Module, x: Option<ExprId>, y: Option<ExprId>) -> bool {
    match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => same(module, x, y),
        _ => false,
    }
}

fn same_rst(module: &Module, x: Option<(ExprId, bool)>, y: Option<(ExprId, bool)>) -> bool {
    match (x, y) {
        (None, None) => true,
        (Some((x, p)), Some((y, q))) => p == q && same(module, x, y),
        _ => false,
    }
}

/// `{parts}[hi:lo]` with only the parts it overlaps; `parts` is most significant
/// first, as a concatenation is.
fn slice_of_concat(
    module: &mut Module,
    parts: &[ExprId],
    hi: u32,
    lo: u32,
    span: Span,
) -> Option<ExprId> {
    let mut kept = Vec::new();
    let mut offset = 0u32;
    for part in parts.iter().rev() {
        let width = module.exprs.get(*part)?.ty.width()?;
        let (from, to) = (offset, offset + width - 1);
        if to >= lo && from <= hi {
            let top = hi.min(to) - from;
            let bottom = lo.max(from) - from;
            kept.push(slice_expr(module, *part, top, bottom, span));
        }
        offset += width;
    }
    kept.reverse();
    match kept.as_slice() {
        [] => None,
        [one] => Some(*one),
        _ => Some(expr(module, ExprKind::Concat(kept), span)),
    }
}

/// `e` with the bits it is read from one flip-flop earlier: the value `e` will
/// have on the next edge, if every net in it is such a flip-flop and they
/// share a control. Every flip-flop that has been passed gives its control to
/// `control`; `None` when one does not fit.
fn one_stage_back(
    module: &mut Module,
    index: &Index,
    e: ExprId,
    control: &mut Option<Control>,
    span: Span,
) -> Option<ExprId> {
    let kind = module.exprs.get(e)?.kind.clone();
    match kind {
        ExprKind::Net(net) => {
            if let Some(id) = index.dff_of_q.get(&net).copied() {
                if index.consumed.contains(&id) {
                    return None;
                }
                let found = control_of(module, id)?;
                match control {
                    None => *control = Some(found),
                    Some(known) => {
                        let fits = same(module, known.clk, found.clk)
                            && same_opt(module, known.en, found.en)
                            && same_rst(module, known.rst, found.rst);
                        if !fits {
                            return None;
                        }
                    }
                }
                return module.cells.get(id)?.input("d");
            }
            let alias = index.alias.get(&net).copied()?;
            one_stage_back(module, index, alias, control, span)
        }
        ExprKind::Slice { base, hi, lo } => {
            // A slice of a concatenation reads only the parts it overlaps: a
            // shift register's `{in[34:0], si}`, sliced above bit 0, has no
            // pad in it.
            if let ExprKind::Concat(parts) = module.exprs.get(base)?.kind.clone() {
                let narrowed = slice_of_concat(module, &parts, hi, lo, span)?;
                return one_stage_back(module, index, narrowed, control, span);
            }
            let earlier = one_stage_back(module, index, base, control, span)?;
            Some(slice_expr(module, earlier, hi, lo, span))
        }
        ExprKind::Concat(parts) => {
            let mut earlier = Vec::with_capacity(parts.len());
            for part in parts {
                earlier.push(one_stage_back(module, index, part, control, span)?);
            }
            Some(expr(module, ExprKind::Concat(earlier), span))
        }
        // A constant was this constant one edge ago too.
        ExprKind::Const(_) => Some(e),
        ExprKind::Resize {
            expr: inner,
            width,
            signed,
        } => {
            let earlier = one_stage_back(module, index, inner, control, span)?;
            // A resize takes its sign from its operand's type, which the
            // operand one stage back need not share.
            let inner_signed = module.exprs.get(inner)?.ty.is_signed();
            let inner_width = module.exprs.get(inner)?.ty.width()?;
            let earlier = if signed && inner_signed && width > inner_width {
                expr(
                    module,
                    ExprKind::Resize {
                        expr: earlier,
                        width: inner_width,
                        signed: true,
                    },
                    span,
                )
            } else {
                earlier
            };
            Some(expr(
                module,
                ExprKind::Resize {
                    expr: earlier,
                    width,
                    signed,
                },
                span,
            ))
        }
        _ => None,
    }
}

/// The stages behind one operand, nearest the multiplier first.
fn operand_stages(module: &mut Module, index: &Index, source: ExprId, span: Span) -> Vec<Stage> {
    let mut out: Vec<Stage> = Vec::new();
    let mut at = source;
    if matches!(
        module.exprs.get(at).map(|e| &e.kind),
        Some(ExprKind::Const(_))
    ) {
        return out;
    }
    while out.len() < MOST_STAGES {
        let mut control = None;
        let Some(before) = one_stage_back(module, index, at, &mut control, span) else {
            break;
        };
        let Some(control) = control else {
            // Nothing but constants: no flip-flop was passed.
            break;
        };
        out.push(Stage { before, control });
        at = before;
    }
    out
}

/// The stages an operand can give up under `clock`, which the first of them
/// sets if nothing has: `RSTA` resets both of an operand's registers, so all of
/// them have to reset alike.
fn side_stages(
    module: &mut Module,
    index: &Index,
    source: ExprId,
    clock: &mut Option<ExprId>,
    span: Span,
) -> Vec<Stage> {
    let stages = operand_stages(module, index, source, span);
    let mut kept: Vec<Stage> = Vec::new();
    for stage in stages {
        let reset_fits = kept
            .first()
            .is_none_or(|first| same_rst(module, first.control.rst, stage.control.rst));
        let clock_fits = clock.is_none_or(|clk| same(module, clk, stage.control.clk));
        if !reset_fits || !clock_fits {
            break;
        }
        *clock = Some(stage.control.clk);
        kept.push(stage);
    }
    kept
}

/// One flip-flop after the product.
#[derive(Clone, Debug)]
struct OutStage {
    cell: CellId,
    q: NetId,
    d: ExprId,
    control: Control,
}

/// The next flip-flop `net` feeds, when that is all it feeds.
fn output_stage(module: &Module, index: &Index, net: NetId, uses: &[usize]) -> Option<OutStage> {
    if uses.get(net.index()).copied().unwrap_or(0) != 1 {
        return None;
    }
    let readers = index.readers.get(&net)?;
    let [cell] = readers.as_slice() else {
        return None;
    };
    if index.consumed.contains(cell) {
        return None;
    }
    let control = control_of(module, *cell)?;
    let dff = module.cells.get(*cell)?;
    Some(OutStage {
        cell: *cell,
        q: dff.output("q")?,
        d: dff.input("d")?,
        control,
    })
}

/// `d` with the net `from` replaced by `value`, for the shapes
/// [`root_net`] looks through.
fn rewrite(
    module: &mut Module,
    d: ExprId,
    from: NetId,
    value: ExprId,
    span: Span,
) -> Option<ExprId> {
    let kind = module.exprs.get(d)?.kind.clone();
    match kind {
        ExprKind::Net(net) if net == from => {
            // A value carries the type the net does, since a resize downstream
            // reads its sign from it.
            let want = module.nets.get(net)?.ty.clone();
            let have = module.exprs.get(value)?.ty.clone();
            if want == have {
                return Some(value);
            }
            let (Type::Bits { width, signed }, Some(_)) = (want, have.width()) else {
                return None;
            };
            Some(expr(
                module,
                ExprKind::Resize {
                    expr: value,
                    width,
                    signed,
                },
                span,
            ))
        }
        ExprKind::Slice { base, hi, lo } => {
            let inner = rewrite(module, base, from, value, span)?;
            Some(slice_expr(module, inner, hi, lo, span))
        }
        ExprKind::Resize {
            expr: inner,
            width,
            signed,
        } => {
            let inner = rewrite(module, inner, from, value, span)?;
            Some(expr(
                module,
                ExprKind::Resize {
                    expr: inner,
                    width,
                    signed,
                },
                span,
            ))
        }
        _ => None,
    }
}

/// What taking a multiply's registers into its block comes to.
#[derive(Clone, Debug, Default)]
pub(super) struct Absorbed {
    /// The operand on the `a` port, before the block's registers.
    pub a: Option<ExprId>,
    /// The operand on the `b` port, before them.
    pub b: Option<ExprId>,
    /// `AREG` and `BREG`.
    pub areg: u32,
    /// As above.
    pub breg: u32,
    /// `MREG` and `PREG`.
    pub mreg: u32,
    /// As above.
    pub preg: u32,
    /// The clock, if any register is on.
    pub clk: Option<ExprId>,
    /// A pin and what goes on it: the enables and resets of the registers on.
    pub pins: Vec<(&'static str, ExprId)>,
    /// The flip-flops that became `M` and `P`, in order, whose outputs now come
    /// from the block.
    out: Vec<OutStage>,
}

/// The pins enabling and resetting one operand's two registers.
struct OperandPins {
    ce2: &'static str,
    ce1: &'static str,
    rst: &'static str,
}

const A_PINS: OperandPins = OperandPins {
    ce2: "CEA2",
    ce1: "CEA1",
    rst: "RSTA",
};
const B_PINS: OperandPins = OperandPins {
    ce2: "CEB2",
    ce1: "CEB1",
    rst: "RSTB",
};

/// The reset as a block's active-high pin wants it.
fn reset_value(module: &mut Module, rst: (ExprId, bool), span: Span) -> ExprId {
    if rst.1 {
        rst.0
    } else {
        expr(
            module,
            ExprKind::Unary {
                op: UnaryOp::Not,
                expr: rst.0,
            },
            span,
        )
    }
}

/// A constant one on a pin.
fn one(module: &mut Module, span: Span) -> ExprId {
    super::primitives::const_expr(module, crate::ir::Const::from_u64(1, 1), span)
}

/// Works out which flip-flops go in. `a` and `b` are the operands as they go on
/// the block, `y` the multiply's result.
#[allow(clippy::too_many_lines, reason = "the allocation is one table")]
#[allow(clippy::too_many_arguments, reason = "one multiply's whole context")]
pub(super) fn plan(
    module: &mut Module,
    index: &mut Index,
    a: ExprId,
    b: ExprId,
    y: NetId,
    uses: &[usize],
    clock_hint: Option<ExprId>,
    span: Span,
) -> Absorbed {
    let mut out: Vec<OutStage> = Vec::new();
    let mut at = y;
    while out.len() < MOST_STAGES {
        let Some(stage) = output_stage(module, index, at, uses) else {
            break;
        };
        at = stage.q;
        out.push(stage);
    }
    // One clock over everything that moves in. The stages after the product are
    // what the block is mostly for, so they choose, after what a sum it is part of
    // has chosen.
    let mut clock: Option<ExprId> = clock_hint;
    let mut kept: Vec<OutStage> = Vec::new();
    for stage in out {
        let fits = clock.is_none_or(|clk| same(module, clk, stage.control.clk));
        if !fits {
            break;
        }
        clock = Some(stage.control.clk);
        kept.push(stage);
    }
    let mut out = kept;
    // A third stage after the product can only be the first of them going back
    // to the operands, which it may only do with nothing controlling it. If it
    // has something, the block takes the first two and the third stays in the
    // fabric, since what is taken has to run unbroken from the product.
    if out.len() == MOST_STAGES && !out[0].control.plain() {
        out.truncate(2);
    }

    let mut a_stages = side_stages(module, index, a, &mut clock, span);
    let mut b_stages = side_stages(module, index, b, &mut clock, span);

    // The multiplier's own register: from a third stage after the product
    // going back, or a third on both operands coming forward.
    let back = out.len() == MOST_STAGES;
    let forward = out.is_empty()
        && a_stages.len() == MOST_STAGES
        && b_stages.len() == MOST_STAGES
        && a_stages[0].control.plain()
        && b_stages[0].control.plain();
    let mut pins: Vec<(&'static str, ExprId)> = Vec::new();
    let (mreg, preg);
    if forward {
        a_stages.remove(0);
        b_stages.remove(0);
        mreg = 1;
        preg = 0;
        let high = one(module, span);
        pins.push(("CEM", high));
    } else {
        preg = u32::from(!out.is_empty());
        mreg = u32::from(out.len() >= 2);
    }
    // After the product, nearest it first; the last is `P`, the one before
    // it `M`.
    let tail: Vec<OutStage> = if back { out[1..].to_vec() } else { out.clone() };
    if let Some(stage) = tail.last() {
        let high = stage.control.en.unwrap_or_else(|| one(module, span));
        pins.push(("CEP", high));
        if let Some(rst) = stage.control.rst {
            let value = reset_value(module, rst, span);
            pins.push(("RSTP", value));
        }
    }
    if tail.len() == 2 {
        let stage = &tail[0];
        let high = stage.control.en.unwrap_or_else(|| one(module, span));
        pins.push(("CEM", high));
        if let Some(rst) = stage.control.rst {
            let value = reset_value(module, rst, span);
            pins.push(("RSTM", value));
        }
    }

    // One operand's registers: a stage going back takes `A2`, whatever else
    // there is `A1` then, and otherwise the nearest stage is `A2`.
    let mut operand = |module: &mut Module,
                       source: ExprId,
                       mut stages: Vec<Stage>,
                       names: &OperandPins|
     -> (ExprId, u32) {
        let room = if back { 1 } else { 2 };
        stages.truncate(room);
        // `RSTA` is shared, so what goes behind a plain register has no reset.
        if back {
            stages.retain(|s| s.control.rst.is_none());
        }
        let count = stages.len() + usize::from(back);
        let mut enables: Vec<Option<ExprId>> = Vec::new();
        if back {
            enables.push(None);
        }
        enables.extend(stages.iter().map(|s| s.control.en));
        for (position, enable) in enables.iter().enumerate() {
            let value = enable.unwrap_or_else(|| one(module, span));
            pins.push((if position == 0 { names.ce2 } else { names.ce1 }, value));
        }
        if let Some(rst) = stages.first().and_then(|s| s.control.rst) {
            let value = reset_value(module, rst, span);
            pins.push((names.rst, value));
        }
        let source = stages.last().map_or(source, |s| s.before);
        (source, u32::try_from(count).unwrap_or(0))
    };
    let (a_source, areg) = operand(module, a, a_stages, &A_PINS);
    let (b_source, breg) = operand(module, b, b_stages, &B_PINS);

    for stage in &out {
        index.consumed.insert(stage.cell);
    }
    Absorbed {
        a: Some(a_source),
        b: Some(b_source),
        areg,
        breg,
        mreg: if back { 1 } else { mreg },
        preg,
        clk: clock,
        pins,
        out,
    }
}

/// What goes into every block of a split multiply: the operands' registers.
#[derive(Clone, Debug)]
pub(super) struct OperandPlan {
    /// The `a` operand before them.
    pub a: ExprId,
    /// The `b` operand before them.
    pub b: ExprId,
    areg: u32,
    breg: u32,
    clk: Option<ExprId>,
    pins: Vec<(&'static str, ExprId)>,
}

/// The enables and reset of one operand's registers, onto `pins`.
fn operand_pins(
    module: &mut Module,
    stages: &[Stage],
    names: &OperandPins,
    pins: &mut Vec<(&'static str, ExprId)>,
    span: Span,
) {
    for (position, stage) in stages.iter().enumerate() {
        let value = stage.control.en.unwrap_or_else(|| one(module, span));
        pins.push((if position == 0 { names.ce2 } else { names.ce1 }, value));
    }
    if let Some(rst) = stages.first().and_then(|s| s.control.rst) {
        let value = reset_value(module, rst, span);
        pins.push((names.rst, value));
    }
}

/// Which of the operands' flip-flops every block of a split multiply takes.
pub(super) fn operand_plan(
    module: &mut Module,
    index: &Index,
    a: ExprId,
    b: ExprId,
    span: Span,
) -> OperandPlan {
    let mut clock = None;
    let mut a_stages = side_stages(module, index, a, &mut clock, span);
    let mut b_stages = side_stages(module, index, b, &mut clock, span);
    a_stages.truncate(2);
    b_stages.truncate(2);
    let mut pins = Vec::new();
    operand_pins(module, &a_stages, &A_PINS, &mut pins, span);
    operand_pins(module, &b_stages, &B_PINS, &mut pins, span);
    OperandPlan {
        a: a_stages.last().map_or(a, |s| s.before),
        b: b_stages.last().map_or(b, |s| s.before),
        areg: u32::try_from(a_stages.len()).unwrap_or(0),
        breg: u32::try_from(b_stages.len()).unwrap_or(0),
        clk: clock,
        pins,
    }
}

/// The one flip-flop after a split multiply's sum that goes back to every
/// block's `M`: only a plain one may pass the adders, since the blocks' `M`s
/// power up holding zero and the sum of zeros is zero.
#[derive(Clone, Debug)]
pub(super) struct PlainOut {
    stage: OutStage,
}

/// That flip-flop, if the product has one and it has no enable, reset or
/// other clock.
pub(super) fn plain_output(
    module: &Module,
    index: &mut Index,
    clock: Option<ExprId>,
    y: NetId,
    uses: &[usize],
) -> Option<PlainOut> {
    let stage = output_stage(module, index, y, uses)?;
    if !stage.control.plain() || !clock.is_none_or(|clk| same(module, clk, stage.control.clk)) {
        return None;
    }
    index.consumed.insert(stage.cell);
    Some(PlainOut { stage })
}

impl PlainOut {
    /// The flip-flop, which goes.
    pub(super) fn cell(&self) -> CellId {
        self.stage.cell
    }

    /// What it drove and the value that now drives it.
    pub(super) fn connect(
        &self,
        module: &mut Module,
        y: NetId,
        value: ExprId,
        span: Span,
    ) -> Option<(NetId, ExprId)> {
        let at = rewrite(module, self.stage.d, y, value, span)?;
        Some((self.stage.q, at))
    }
}

impl OperandPlan {
    /// The clock the operands' registers are on, if they have any.
    pub(super) fn clock(&self) -> Option<ExprId> {
        self.clk
    }

    /// Sets the pins and parameters of one block of the split.
    pub(super) fn apply(
        &self,
        module: &mut Module,
        cell: CellId,
        after: Option<&PlainOut>,
        shape: &crate::fpga::device::DspShape,
    ) {
        let mreg = after.is_some();
        let high = after
            .is_some()
            .then(|| one(module, module.cells[cell].span));
        let cell = &mut module.cells[cell];
        let mut set = |port: &str, value: ExprId| {
            if let Some(input) = cell.inputs.iter_mut().find(|(p, _)| p.as_str() == port) {
                input.1 = value;
            }
        };
        let clk = self.clk.or_else(|| after.map(|a| a.stage.control.clk));
        if let Some(clk) = clk {
            set(shape.port("clk").unwrap_or("CLK"), clk);
        }
        for (pin, value) in &self.pins {
            set(pin, *value);
        }
        if let Some(high) = high {
            set("CEM", high);
        }
        for (name, value) in [
            ("AREG", self.areg),
            ("ACASCREG", self.areg),
            ("BREG", self.breg),
            ("BCASCREG", self.breg),
            ("MREG", u32::from(mreg)),
        ] {
            cell.params.set(name, i64::from(value));
        }
        cell.params.set("CREG", 1i64);
    }
}

/// The flip-flop after a sum that goes into its top block's `P`.
#[derive(Clone, Debug)]
pub(super) struct SumRegister {
    stage: OutStage,
}

/// That flip-flop, when the sum feeds only it and it is on the clock the sum's
/// other registers are.
pub(super) fn output_after(
    module: &Module,
    index: &mut Index,
    clock: Option<ExprId>,
    sum: NetId,
    uses: &[usize],
) -> Option<SumRegister> {
    let stage = output_stage(module, index, sum, uses)?;
    if !clock.is_none_or(|clk| same(module, clk, stage.control.clk)) {
        return None;
    }
    index.consumed.insert(stage.cell);
    Some(SumRegister { stage })
}

impl SumRegister {
    /// The clock it is on.
    pub(super) fn clock(&self) -> ExprId {
        self.stage.control.clk
    }

    /// The flip-flop, which goes.
    pub(super) fn cell(&self) -> CellId {
        self.stage.cell
    }

    /// The enable and reset `P` takes from it.
    pub(super) fn pins(&self, module: &mut Module, span: Span) -> (ExprId, Option<ExprId>) {
        let enable = self.stage.control.en.unwrap_or_else(|| one(module, span));
        let reset = self
            .stage
            .control
            .rst
            .map(|rst| reset_value(module, rst, span));
        (enable, reset)
    }

    /// What it drove and the value that now drives it.
    pub(super) fn connect(
        &self,
        module: &mut Module,
        sum: NetId,
        value: ExprId,
        span: Span,
    ) -> Option<(NetId, ExprId)> {
        let at = rewrite(module, self.stage.d, sum, value, span)?;
        Some((self.stage.q, at))
    }
}

/// The value `e` holds one flip-flop earlier, with that flip-flop's enable and
/// reset, for `C`'s register: `None` when it is not one the block could hold, or
/// on another clock.
pub(super) fn one_stage(
    module: &mut Module,
    index: &Index,
    e: ExprId,
    clock: Option<ExprId>,
    span: Span,
) -> Option<(ExprId, Option<ExprId>, Option<ExprId>)> {
    let mut control = None;
    let before = one_stage_back(module, index, e, &mut control, span)?;
    let control = control?;
    if !clock.is_none_or(|clk| same(module, clk, control.clk)) {
        return None;
    }
    let reset = control.rst.map(|rst| reset_value(module, rst, span));
    Some((before, control.en, reset))
}

impl Absorbed {
    /// A flip-flop after the product goes into `M` instead of `P`: the block's `P`
    /// is for the sum it is part of.
    pub(super) fn stage_on_m(&mut self) {
        if self.preg == 1 && self.mreg == 0 {
            self.preg = 0;
            self.mreg = 1;
            for (pin, _) in &mut self.pins {
                if *pin == "CEP" {
                    *pin = "CEM";
                }
                if *pin == "RSTP" {
                    *pin = "RSTM";
                }
            }
        }
    }

    /// True when any register went in.
    pub(super) fn any(&self) -> bool {
        self.areg + self.breg + self.mreg + self.preg > 0
    }

    /// The flip-flops that are now the block's, which go.
    pub(super) fn removed(&self) -> Vec<CellId> {
        self.out.iter().map(|stage| stage.cell).collect()
    }

    /// Drives what the last flip-flop drove from `value`, the multiply's result
    /// as `y` was typed, instead of driving `y`; `false` when there was no
    /// flip-flop after the product.
    ///
    /// # Panics
    ///
    /// Never for what [`plan`] gave: each `d` has a shape [`rewrite`] takes,
    /// which [`root_net`] is what found it by.
    pub(super) fn connect(
        &self,
        module: &mut Module,
        y: NetId,
        value: ExprId,
        span: Span,
    ) -> Option<(NetId, ExprId)> {
        let mut from = y;
        let mut at = value;
        let mut last = None;
        for stage in &self.out {
            at = rewrite(module, stage.d, from, at, span)?;
            from = stage.q;
            last = Some(stage.q);
        }
        last.map(|q| (q, at))
    }
}
