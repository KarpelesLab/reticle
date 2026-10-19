//! Flip-flop optimisation.
//!
//! [`FfOpt`] rewrites `dff` cells one at a time until nothing applies:
//!
//! - A constant `en` of 1 is dropped; a constant `en` of 0 makes `d` the
//!   feedback `q`.
//! - A constant `rst` that is never active is dropped; one that is always
//!   active turns a synchronous reset into a constant `d`, and an
//!   asynchronous reset into a constant net.
//! - **Sync-reset extraction**: with no reset and no enable, `d =
//!   mux(c, K, rest)` (or `mux(c, rest, K)`) with constant `K` becomes a
//!   synchronous reset on `c`. It runs before enable extraction because
//!   `rst` has priority over `en` in the cell.
//! - **Enable extraction**: `d = mux(c, v, q)` becomes `en = c, d = v`
//!   (`mux(c, q, v)` becomes `en = not(c)`); an existing enable is
//!   ANDed in.
//!   The register's own value one multiplexer or more further down,
//!   `mux(c, v, mux(c2, v2, q))`, is an enable too: `en = c | c2`, `d =
//!   mux(c, v, v2)`; `hold_split` has the cases.
//! - **Sync-reset extraction by slice**: once nothing above applies, `d =
//!   mux(c, K, rest)` where only some bits of `K` are constant — `{0,
//!   inc}`, a counter that restarts at its increment — splits off those
//!   bits as a register with a synchronous reset, under any enable the
//!   register has; `split_by_reset` has why.
//! - **Enable extraction by slice**: a register whose `d` is a
//!   concatenation of `mux(c, v, q[slice])` parts, which a part-select
//!   write leaves, is split into one register per run of bits sharing a
//!   condition, each with its own enable; `split_by_slice` has why.
//! - A register whose `d` is its own `q` never changes: it is replaced by
//!   its initial value, its reset value when both agree, or `x`
//!   (don't-care) when it has neither.
//! - A register whose `d` is a constant `K` is replaced by `K` when its
//!   reset value (if any) and initial value (if any) are `K` too, or
//!   absent: before the first clock it holds `x` or `K` either way.
//! - A register whose `q` nothing reads is removed.
//! - Two registers with the same kind, inputs, attributes and initial
//!   value are merged; the second's `q` becomes an alias assign.
//!
//! The last three are the destructive ones, and a register that
//! [`crate::synth::keep`] calls kept — by its own attribute or by one on
//! the net its `q` drives, which is where a `(* keep *)` on a Verilog `reg`
//! lands — is exempt from all three. It is still canonicalised by the rules
//! above it, since those leave the same flip-flop on the same net.

use std::collections::{HashMap, HashSet};

use crate::diag::Diagnostics;
use crate::ir::{
    Assign, AttrValue, Attrs, BinaryOp, Cell, CellId, CellKind, Const, ExprId, ExprKind, Lvalue,
    Module, Name, Net, NetId, NetKind, Reset, Type, UnaryOp,
};
use crate::logic::Bit;
use crate::source::Span;
use crate::synth::keep::cell_is_kept;
use crate::synth::opt::{expr_net_refs, root_net_refs};
use crate::synth::util::{
    coerce, is_port, mk, mk_and, mk_concat, mk_const, mk_net, mk_not, mk_or, mk_slice,
};
use crate::synth::{Pass, PassStats};

/// The flip-flop optimisation pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct FfOpt;

impl Pass for FfOpt {
    fn name(&self) -> &'static str {
        "ff_opt"
    }

    fn run(&self, m: &mut Module, _diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let ids: Vec<CellId> = m
            .cells
            .iter()
            .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
            .map(|(id, _)| id)
            .collect();
        let pmuxes: HashMap<NetId, CellId> = m
            .cells
            .iter()
            .filter(|(_, c)| c.kind == CellKind::Pmux)
            .filter_map(|(id, c)| Some((c.output("y")?, id)))
            .collect();
        let mut names: Option<Names> = None;
        let mut constants: Vec<(CellId, Const)> = Vec::new();
        let mut work = ids;
        while let Some(id) = work.pop() {
            // A kept register is still canonicalised: a constant enable is
            // dropped and an enable or synchronous reset is lifted out of
            // its `d` mux, which leaves the same flip-flop driving the same
            // net. What it never does is stop being a flip-flop, so the one
            // outcome it refuses is becoming a constant — or several.
            let kept = cell_is_kept(m, &m.cells[id]);
            loop {
                match step(m, id, &mut stats) {
                    Step::Again => {}
                    Step::Done => {
                        if !kept {
                            let names = names.get_or_insert_with(|| Names::of(m));
                            let split = split_by_reset(m, id, names, &mut stats);
                            if split.is_empty() {
                                work.extend(split_by_slice(m, id, &pmuxes, names, &mut stats));
                            } else {
                                work.extend(split);
                            }
                        }
                        break;
                    }
                    Step::Constant(v) => {
                        if !kept {
                            constants.push((id, v));
                        }
                        break;
                    }
                }
            }
        }
        let mut doomed = Vec::new();
        for (id, value) in constants {
            let cell = &m.cells[id];
            let Some(q) = cell.output("q") else {
                continue;
            };
            let span = cell.span;
            let signed = m.nets[q].ty.is_signed();
            let e = mk_const(m, value.with_signed(signed), span);
            m.assigns.push(Assign {
                target: Lvalue::Net(q),
                value: e,
                delay: None,
                attrs: Attrs::new(),
                span,
            });
            doomed.push(id);
            stats.bump("constant registers", 1);
        }
        m.cells.retain(|id, _| !doomed.contains(&id));
        stats.bump("unused registers removed", remove_unused(m));
        stats.bump("registers merged", merge_duplicates(m));
        stats
    }
}

enum Step {
    Again,
    Done,
    Constant(Const),
}

fn const_of(m: &Module, id: ExprId) -> Option<&Const> {
    m.expr(id).as_const()
}

fn const_bit(m: &Module, id: ExprId) -> Option<bool> {
    let c = const_of(m, id)?;
    (c.width() == 1).then(|| c.bit(0).to_bool()).flatten()
}

fn init_of(m: &Module, q: NetId) -> Option<Const> {
    match m.nets[q].attrs.get("init")? {
        AttrValue::Const(c) => Some(c.clone()),
        _ => None,
    }
}

fn set_input(m: &mut Module, id: CellId, port: &str, value: Option<ExprId>) {
    let cell = &mut m.cells[id];
    cell.inputs.retain(|(n, _)| n.as_str() != port);
    if let Some(v) = value {
        cell.inputs.push((Name::new(port), v));
    }
    // Keep the canonical port order.
    const ORDER: [&str; 4] = ["clk", "d", "en", "rst"];
    cell.inputs.sort_by_key(|(n, _)| {
        ORDER
            .iter()
            .position(|p| *p == n.as_str())
            .unwrap_or(usize::MAX)
    });
}

fn step(m: &mut Module, id: CellId, stats: &mut PassStats) -> Step {
    let cell = m.cells[id].clone();
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset,
    } = cell.kind.clone()
    else {
        return Step::Done;
    };
    let (Some(q), Some(d)) = (cell.output("q"), cell.input("d")) else {
        return Step::Done;
    };
    let span = cell.span;
    let width = m.nets[q].ty.width().unwrap_or(0);
    let init = init_of(m, q);

    // Constant enable.
    if has_enable && let Some(en) = cell.input("en") {
        match const_bit(m, en) {
            Some(true) => {
                set_input(m, id, "en", None);
                m.cells[id].kind = CellKind::Dff {
                    clk_pos,
                    has_enable: false,
                    reset,
                };
                stats.bump("constant enables removed", 1);
                return Step::Again;
            }
            Some(false) => {
                let fb = mk_net(m, q, span);
                set_input(m, id, "d", Some(fb));
                set_input(m, id, "en", None);
                m.cells[id].kind = CellKind::Dff {
                    clk_pos,
                    has_enable: false,
                    reset,
                };
                stats.bump("constant enables removed", 1);
                return Step::Again;
            }
            None => {}
        }
    }

    // Constant reset.
    if let Some(r) = &reset
        && let Some(rst) = cell.input("rst")
        && let Some(level) = const_bit(m, rst)
    {
        let active = level == r.active_high;
        if !active {
            set_input(m, id, "rst", None);
            m.cells[id].kind = CellKind::Dff {
                clk_pos,
                has_enable,
                reset: None,
            };
            stats.bump("constant resets removed", 1);
            return Step::Again;
        }
        if r.asynchronous {
            return Step::Constant(r.value.clone());
        }
        let k = mk_const(m, r.value.clone(), span);
        set_input(m, id, "d", Some(k));
        set_input(m, id, "rst", None);
        set_input(m, id, "en", None);
        m.cells[id].kind = CellKind::Dff {
            clk_pos,
            has_enable: false,
            reset: None,
        };
        stats.bump("constant resets removed", 1);
        return Step::Again;
    }

    // Sync-reset extraction.
    if reset.is_none()
        && !has_enable
        && let ExprKind::Ternary { cond, then_, else_ } = m.expr(d).kind
    {
        let pick = if const_of(m, then_).is_some_and(|c| c.width() == width && c.is_fully_known()) {
            Some((then_, else_, true))
        } else if const_of(m, else_).is_some_and(|c| c.width() == width && c.is_fully_known()) {
            Some((else_, then_, false))
        } else {
            None
        };
        if let Some((k, rest, active_high)) = pick {
            let value = const_of(m, k).cloned().expect("constant branch");
            let (rst, active_high) = match m.expr(cond).kind {
                ExprKind::Unary {
                    op: UnaryOp::Not | UnaryOp::LogicNot,
                    expr,
                } if m.expr(expr).ty.is_bit() => (expr, !active_high),
                _ => (cond, active_high),
            };
            set_input(m, id, "d", Some(rest));
            set_input(m, id, "rst", Some(rst));
            m.cells[id].kind = CellKind::Dff {
                clk_pos,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high,
                    value,
                }),
            };
            stats.bump("sync resets extracted", 1);
            return Step::Again;
        }
    }

    // Enable extraction.
    if let ExprKind::Ternary { cond, then_, else_ } = m.expr(d).kind {
        let feedback = |e: ExprId| m.expr(e).kind == ExprKind::Net(q);
        let pick = if feedback(else_) {
            Some((cond, then_))
        } else if feedback(then_) {
            Some((mk_not(m, cond, span), else_))
        } else {
            None
        };
        if let Some((c, v)) = pick {
            let en = match (has_enable, cell.input("en")) {
                (true, Some(e)) => mk_and(m, e, c, span),
                _ => c,
            };
            set_input(m, id, "d", Some(v));
            set_input(m, id, "en", Some(en));
            m.cells[id].kind = CellKind::Dff {
                clk_pos,
                has_enable: true,
                reset,
            };
            stats.bump("enables extracted", 1);
            return Step::Again;
        }
        // The hold further down: `c ? v : (c2 ? v2 : q)`, which is what
        // `if (a) q <= x; else if (b) q <= y;` becomes once process
        // lowering has turned the chain round, enables on `c | c2` and
        // loads `c ? v : v2`.
        if let Feedback::Cond(c, v) = hold_split(m, d, q, HOLD_DEPTH) {
            let en = match (has_enable, cell.input("en")) {
                (true, Some(e)) => mk_and(m, e, c, span),
                _ => c,
            };
            set_input(m, id, "d", Some(v));
            set_input(m, id, "en", Some(en));
            m.cells[id].kind = CellKind::Dff {
                clk_pos,
                has_enable: true,
                reset,
            };
            stats.bump("enables extracted", 1);
            return Step::Again;
        }
    }

    // Feedback only: the register never changes.
    if m.expr(d).kind == ExprKind::Net(q) {
        let reset_value = reset.as_ref().map(|r| r.value.clone());
        return match (init, reset_value) {
            (Some(i), Some(r)) if i == r => Step::Constant(i),
            (Some(_), Some(_)) => Step::Done,
            (Some(i), None) => Step::Constant(i),
            (None, Some(r)) => Step::Constant(r),
            (None, None) => Step::Constant(Const::x(width)),
        };
    }

    // Constant data.
    if let Some(k) = const_of(m, d).cloned() {
        let reset_ok = reset.as_ref().is_none_or(|r| r.value == k);
        let init_ok = init.as_ref().is_none_or(|i| *i == k || !i.is_fully_known());
        if reset_ok && init_ok {
            return Step::Constant(k);
        }
    }
    Step::Done
}

/// The net and cell names a module already uses, collected once: asking
/// [`crate::synth::util::fresh_net_name`] per new register would rebuild
/// the set every time, and a register bank splits into thousands.
struct Names {
    nets: HashSet<String>,
    cells: HashSet<String>,
}

impl Names {
    fn of(m: &Module) -> Self {
        Self {
            nets: m
                .nets
                .values()
                .map(|n| n.name.as_str().to_owned())
                .collect(),
            cells: m
                .cells
                .values()
                .map(|c| c.name.as_str().to_owned())
                .collect(),
        }
    }

    fn fresh(taken: &mut HashSet<String>, base: &str) -> Name {
        let mut name = base.to_owned();
        let mut i = 2u32;
        while taken.contains(&name) {
            name = format!("{base}_{i}");
            i += 1;
        }
        taken.insert(name.clone());
        Name::new(name)
    }
}

/// What a run of a register's `d` does with the register's own bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// `cond ? value : q` (or, `inverted`, `cond ? q : value`).
    Load { cond: ExprId, inverted: bool },
    /// The register's own bits: these never change.
    Hold,
    /// Anything else, kept as it is.
    Other,
}

/// Contiguous bits of a register that share a [`Role`]; `parts` are the
/// values loaded (for [`Role::Load`]) or the `d` parts themselves, most
/// significant first.
struct Run {
    role: Role,
    lo: u32,
    width: u32,
    parts: Vec<ExprId>,
}

/// `d` seen through a one-arm `pmux` whose default is all `x`: the
/// default is a don't-care, so the cell is its arm, as `const_fold` takes
/// the other branch of a `mux` with an all-`x` one.
fn through_pmux(m: &Module, d: ExprId, pmuxes: &HashMap<NetId, CellId>) -> ExprId {
    let ExprKind::Net(n) = m.expr(d).kind else {
        return d;
    };
    let Some(&pm) = pmuxes.get(&n) else {
        return d;
    };
    let cell = &m.cells[pm];
    let (Some(a), Some(b), Some(s)) = (cell.input("a"), cell.input("b"), cell.input("s")) else {
        return d;
    };
    let undefined = const_of(m, a).is_some_and(|c| c.bits().iter().all(|b| *b == Bit::X));
    if undefined && m.expr(s).ty.width() == Some(1) {
        b
    } else {
        d
    }
}

fn flatten(m: &Module, e: ExprId, out: &mut Vec<ExprId>) {
    if let ExprKind::Concat(parts) = &m.expr(e).kind {
        for p in parts.clone() {
            flatten(m, p, out);
        }
    } else {
        out.push(e);
    }
}

/// True when `e` is bits `lo .. lo + width` of `q`, where they sit.
fn is_own_bits(m: &Module, e: ExprId, q: NetId, lo: u32, width: u32, total: u32) -> bool {
    match m.expr(e).kind {
        ExprKind::Net(n) => n == q && lo == 0 && width == total,
        ExprKind::Slice { base, hi, lo: l } => {
            m.expr(base).kind == ExprKind::Net(q) && l == lo && hi + 1 == lo + width
        }
        _ => false,
    }
}

/// **Enable extraction by slice.** A register whose `d` is a
/// concatenation of `cond ? value : q[slice]` parts — which is what a
/// part-select write, `r[i*8 +: 8] <= x`, lowers to — is split into one
/// register per run of bits that share a condition, each loading its
/// `value` under its own enable. Left whole, every bit's hold multiplexer
/// is a lookup table of its own; split, `d` is the value and the condition
/// is one enable for the run. A register written a byte at a time from a
/// bus is what every memory-mapped register bank is, and NewAE's Husky
/// gateware spent several thousand lookup tables on those multiplexers.
///
/// Returns the registers to look at again: the original, changed in
/// place, and any it was split into.
fn split_by_slice(
    m: &mut Module,
    id: CellId,
    pmuxes: &HashMap<NetId, CellId>,
    names: &mut Names,
    stats: &mut PassStats,
) -> Vec<CellId> {
    let cell = m.cells[id].clone();
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset,
    } = cell.kind.clone()
    else {
        return Vec::new();
    };
    let (Some(q), Some(d)) = (cell.output("q"), cell.input("d")) else {
        return Vec::new();
    };
    let Some(total) = m.nets[q].ty.width() else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    flatten(m, through_pmux(m, d, pmuxes), &mut parts);
    // Most significant first; `lo` counts down from the top.
    let mut runs: Vec<Run> = Vec::new();
    let mut top = total;
    for p in parts {
        let Some(width) = m.expr(p).ty.width().filter(|w| *w > 0 && *w <= top) else {
            return Vec::new();
        };
        let lo = top - width;
        top = lo;
        let own = |e| is_own_bits(m, e, q, lo, width, total);
        let (role, value) = match m.expr(p).kind {
            ExprKind::Ternary { cond, then_, else_ } if own(else_) => (
                Role::Load {
                    cond,
                    inverted: false,
                },
                then_,
            ),
            ExprKind::Ternary { cond, then_, else_ } if own(then_) => (
                Role::Load {
                    cond,
                    inverted: true,
                },
                else_,
            ),
            _ if own(p) => (Role::Hold, p),
            _ => (Role::Other, p),
        };
        match runs.last_mut() {
            Some(r) if r.role == role => {
                r.lo = lo;
                r.width += width;
                r.parts.push(value);
            }
            _ => runs.push(Run {
                role,
                lo,
                width,
                parts: vec![value],
            }),
        }
    }
    if top != 0 || !runs.iter().any(|r| matches!(r.role, Role::Load { .. })) {
        return Vec::new();
    }
    let span = cell.span;
    let en = cell.input("en").filter(|_| has_enable);
    let rst = cell.input("rst");
    let pieces: Vec<Piece> = runs
        .iter()
        .map(|run| {
            let (d, en) = match run.role {
                Role::Load { cond, inverted } => {
                    let c = if inverted {
                        mk_not(m, cond, span)
                    } else {
                        cond
                    };
                    let en = match en {
                        Some(e) => mk_and(m, e, c, span),
                        None => c,
                    };
                    (Some(mk_concat(m, run.parts.clone(), span)), Some(en))
                }
                Role::Hold => (None, en),
                Role::Other => (Some(mk_concat(m, run.parts.clone(), span)), en),
            };
            let hi = run.lo + run.width - 1;
            Piece {
                lo: run.lo,
                width: run.width,
                d,
                en,
                reset: reset
                    .as_ref()
                    .map(|r| (slice_reset(r, hi, run.lo, total), rst)),
            }
        })
        .collect();
    let again = rebuild(m, id, &cell, clk_pos, pieces, names);
    if again.len() == 1 {
        stats.bump("enables extracted", 1);
    } else {
        stats.bump("registers split by enable", 1);
        stats.bump("enables extracted", again.len() as u64);
    }
    again
}

/// `r` for bits `hi..=lo` of a register `total` wide.
fn slice_reset(r: &Reset, hi: u32, lo: u32, total: u32) -> Reset {
    Reset {
        value: if lo == 0 && hi + 1 == total {
            r.value.clone()
        } else {
            r.value.slice(hi, lo)
        },
        ..r.clone()
    }
}

/// One register of the several [`rebuild`] makes of one.
struct Piece {
    lo: u32,
    width: u32,
    /// `d`; `None` holds the piece's own value.
    d: Option<ExprId>,
    en: Option<ExprId>,
    /// The reset and the expression driving it.
    reset: Option<(Reset, Option<ExprId>)>,
}

/// Replaces register `id` with `pieces`, which cover its bits from the
/// most significant down. One piece covering them all changes the
/// register in place and keeps its output net; several get a net each,
/// with the `init` value split between them, and the old output becomes
/// their concatenation. Returns the registers made.
fn rebuild(
    m: &mut Module,
    id: CellId,
    cell: &Cell,
    clk_pos: bool,
    pieces: Vec<Piece>,
    names: &mut Names,
) -> Vec<CellId> {
    let (Some(q), Some(clk)) = (cell.output("q"), cell.input("clk")) else {
        return Vec::new();
    };
    let span = cell.span;
    let whole = pieces.len() == 1;
    let init = init_of(m, q);
    let qname = m.nets[q].name.as_str().to_owned();
    let mut outs: Vec<ExprId> = Vec::with_capacity(pieces.len());
    let mut again = Vec::with_capacity(pieces.len());
    for (i, piece) in pieces.into_iter().enumerate() {
        let hi = piece.lo + piece.width - 1;
        let out = if whole {
            q
        } else {
            let net = add_wire_named(
                m,
                Names::fresh(&mut names.nets, &format!("{qname}${}", piece.lo)),
                Type::bits(piece.width),
                span,
            );
            if let Some(c) = &init {
                m.nets[net]
                    .attrs
                    .set("init", AttrValue::Const(c.slice(hi, piece.lo)));
            }
            net
        };
        outs.push(mk_net(m, out, span));
        let d = piece.d.unwrap_or_else(|| mk_net(m, out, span));
        let mut inputs = vec![(Name::new("clk"), clk), (Name::new("d"), d)];
        if let Some(e) = piece.en {
            inputs.push((Name::new("en"), e));
        }
        let reset = piece.reset.map(|(r, rst)| {
            if let Some(rst) = rst {
                inputs.push((Name::new("rst"), rst));
            }
            r
        });
        let kind = CellKind::Dff {
            clk_pos,
            has_enable: piece.en.is_some(),
            reset,
        };
        let outputs = vec![(Name::new("q"), out)];
        if i == 0 {
            let c = &mut m.cells[id];
            c.kind = kind;
            c.inputs = inputs;
            c.outputs = outputs;
            again.push(id);
        } else {
            let name = Names::fresh(
                &mut names.cells,
                &format!("{}${}", cell.name.as_str(), piece.lo),
            );
            again.push(m.cells.push(Cell {
                name,
                kind,
                inputs,
                outputs,
                params: cell.params.clone(),
                attrs: cell.attrs.clone(),
                span,
            }));
        }
    }
    if !whole {
        m.nets[q].attrs.remove("init");
        let ty = m.nets[q].ty.clone();
        let joined = mk_concat(m, outs, span);
        let value = coerce(m, joined, &ty, span);
        m.assigns.push(Assign {
            target: Lvalue::Net(q),
            value,
            delay: None,
            attrs: Attrs::new(),
            span,
        });
    }
    again
}

/// How far [`known_bits`] looks into an expression.
const KNOWN_DEPTH: u32 = 6;

/// The value of each bit of `e` that does not depend on any net, least
/// significant first, looking `depth` operators down: through constants,
/// concatenations, slices, resizes, `not`, and a multiplexer whose arms
/// agree. `(c ? 0 : 1)` at eight bits is seven known zeros and an unknown,
/// which is what a counter that restarts at its increment has as `K`.
fn known_bits(m: &Module, e: ExprId, depth: u32) -> Vec<Option<bool>> {
    let expr = m.expr(e);
    let width = expr.ty.width().unwrap_or(0) as usize;
    let unknown = || vec![None; width];
    if depth == 0 {
        return unknown();
    }
    match &expr.kind {
        ExprKind::Const(c) => (0..c.width()).map(|i| c.bit(i).to_bool()).collect(),
        ExprKind::Concat(parts) => {
            let mut bits = Vec::with_capacity(width);
            for p in parts.iter().rev() {
                bits.extend(known_bits(m, *p, depth - 1));
            }
            if bits.len() == width { bits } else { unknown() }
        }
        ExprKind::Slice { base, hi, lo } => {
            let inner = known_bits(m, *base, depth - 1);
            match inner.get(*lo as usize..=*hi as usize) {
                Some(bits) if bits.len() == width => bits.to_vec(),
                _ => unknown(),
            }
        }
        ExprKind::Resize {
            expr: inner,
            width: w,
            signed,
        } => {
            let mut bits = known_bits(m, *inner, depth - 1);
            let fill = if *signed && m.expr(*inner).ty.is_signed() {
                bits.last().copied().flatten()
            } else {
                Some(false)
            };
            if bits.is_empty() {
                return unknown();
            }
            bits.resize(*w as usize, fill);
            bits
        }
        ExprKind::Unary {
            op: UnaryOp::Not,
            expr: inner,
        } => known_bits(m, *inner, depth - 1)
            .into_iter()
            .map(|b| b.map(|v| !v))
            .collect(),
        ExprKind::Ternary { then_, else_, .. } => {
            let (t, f) = (
                known_bits(m, *then_, depth - 1),
                known_bits(m, *else_, depth - 1),
            );
            if t.len() != width || f.len() != width {
                return unknown();
            }
            t.into_iter()
                .zip(f)
                .map(|(a, b)| if a == b { a } else { None })
                .collect()
        }
        _ => unknown(),
    }
}

/// True when `a` high makes `b` high, seen structurally: `b` is `a`, or
/// an `or` with a term `a` implies. An enable [`hold_split`] made is
/// `c | rest` for the arm on `c`, so a reset on `c` needs no `and` with it.
fn implies(m: &Module, a: ExprId, b: ExprId, depth: u32) -> bool {
    if a == b {
        return true;
    }
    match m.expr(b).kind {
        ExprKind::Binary {
            op: BinaryOp::Or | BinaryOp::LogicOr,
            lhs,
            rhs,
        } if depth > 0 => implies(m, a, lhs, depth - 1) || implies(m, a, rhs, depth - 1),
        _ => false,
    }
}

/// **Sync-reset extraction by slice, and under an enable.** The rule in
/// [`step`] takes `d = mux(c, K, rest)` as a reset only when all of `K` is
/// a constant and the register has no enable. A saturating counter that
/// restarts at `{0, inc}` has neither: `resetter ? {7'b0, inc} : count +
/// inc` under the enable `~&count` is eight multiplexers, one per bit, in
/// each of the Husky's 256 `esad` counters. Here the bits whose `K` is a
/// constant become a register of their own with that constant as a
/// synchronous reset, `d = rest`, and the rest keep the multiplexer.
///
/// An enable is kept, and the reset made `en & c`: the cell's reset has
/// priority over its enable, so a reset on `c` alone would fire on a
/// cycle the enable holds.
fn split_by_reset(
    m: &mut Module,
    id: CellId,
    names: &mut Names,
    stats: &mut PassStats,
) -> Vec<CellId> {
    let cell = m.cells[id].clone();
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset: None,
    } = cell.kind
    else {
        return Vec::new();
    };
    let (Some(q), Some(d)) = (cell.output("q"), cell.input("d")) else {
        return Vec::new();
    };
    let Some(total) = m.nets[q].ty.width() else {
        return Vec::new();
    };
    let ExprKind::Ternary { cond, then_, else_ } = m.expr(d).kind else {
        return Vec::new();
    };
    let known_in = |arm: ExprId| {
        let bits = known_bits(m, arm, KNOWN_DEPTH);
        (bits.len() == total as usize && bits.iter().any(Option::is_some)).then_some(bits)
    };
    let (bits, arm, rest, fires_high) = if let Some(b) = known_in(then_) {
        (b, then_, else_, true)
    } else if let Some(b) = known_in(else_) {
        (b, else_, then_, false)
    } else {
        return Vec::new();
    };
    let en = cell.input("en").filter(|_| has_enable);
    if en.is_none() && bits.iter().all(Option::is_some) {
        // The whole-register rule in `step`; it has already looked.
        return Vec::new();
    }
    let span = cell.span;
    // Runs of known and of other bits, most significant first: `(known,
    // lo, width)`.
    let mut runs: Vec<(bool, u32, u32)> = Vec::new();
    for bit in (0..total).rev() {
        let k = bits[bit as usize].is_some();
        match runs.last_mut() {
            Some(r) if r.0 == k => {
                r.1 = bit;
                r.2 += 1;
            }
            _ => runs.push((k, bit, 1)),
        }
    }
    let fire = if fires_high {
        cond
    } else {
        mk_not(m, cond, span)
    };
    let rst = match en {
        Some(e) if implies(m, fire, e, KNOWN_DEPTH) => fire,
        Some(e) => mk_and(m, e, fire, span),
        None => fire,
    };
    let whole = runs.len() == 1;
    let pieces: Vec<Piece> = runs
        .into_iter()
        .map(|(constant, lo, width)| {
            let hi = lo + width - 1;
            let cut = |m: &mut Module, e: ExprId| {
                if whole {
                    e
                } else {
                    mk_slice(m, e, hi, lo, span)
                }
            };
            let rest = cut(m, rest);
            if constant {
                let value = Const::from_bits(
                    &bits[lo as usize..=hi as usize]
                        .iter()
                        .map(|b| Bit::from(b.expect("a known run")))
                        .collect::<Vec<_>>(),
                );
                Piece {
                    lo,
                    width,
                    d: Some(rest),
                    en,
                    reset: Some((
                        Reset {
                            asynchronous: false,
                            active_high: true,
                            value,
                        },
                        Some(rst),
                    )),
                }
            } else {
                let k = cut(m, arm);
                let (t, e) = if fires_high { (k, rest) } else { (rest, k) };
                Piece {
                    lo,
                    width,
                    d: Some(mk(
                        m,
                        ExprKind::Ternary {
                            cond,
                            then_: t,
                            else_: e,
                        },
                        span,
                    )),
                    en,
                    reset: None,
                }
            }
        })
        .collect();
    let again = rebuild(m, id, &cell, clk_pos, pieces, names);
    stats.bump("sync resets extracted", 1);
    if again.len() > 1 {
        stats.bump("registers split by reset", 1);
    }
    again
}

fn add_wire_named(m: &mut Module, name: Name, ty: Type, span: Span) -> NetId {
    m.nets.push(Net {
        name,
        ty,
        kind: NetKind::Wire,
        attrs: Attrs::new(),
        span,
    })
}

/// How many multiplexers down [`hold_split`] looks for the register's
/// own value.
const HOLD_DEPTH: u32 = 4;

/// What a register's `d` does, seen as a tree of multiplexers.
enum Feedback {
    /// It is the register's own value: it holds.
    Hold,
    /// It never holds; the expression is unchanged.
    Load,
    /// It loads `value` when `en` is high and holds otherwise.
    Cond(ExprId, ExprId),
}

/// Splits `e`, the `d` of the register driving `q`, into an enable and
/// the value loaded under it, when the register's own value is one of the
/// leaves of a multiplexer tree at most `depth` deep. The hold leaves
/// become don't-cares of the value, since the enable is low wherever they
/// would have been chosen.
fn hold_split(m: &mut Module, e: ExprId, q: NetId, depth: u32) -> Feedback {
    if m.expr(e).kind == ExprKind::Net(q) {
        return Feedback::Hold;
    }
    let ExprKind::Ternary { cond, then_, else_ } = m.expr(e).kind else {
        return Feedback::Load;
    };
    if depth == 0 {
        return Feedback::Load;
    }
    let span = m.expr(e).span;
    let t = hold_split(m, then_, q, depth - 1);
    let f = hold_split(m, else_, q, depth - 1);
    let mux = |m: &mut Module, a, b| {
        mk(
            m,
            ExprKind::Ternary {
                cond,
                then_: a,
                else_: b,
            },
            span,
        )
    };
    match (t, f) {
        (Feedback::Hold, Feedback::Hold) => Feedback::Hold,
        (Feedback::Load, Feedback::Load) => Feedback::Load,
        (Feedback::Load, Feedback::Hold) => Feedback::Cond(cond, then_),
        (Feedback::Hold, Feedback::Load) => Feedback::Cond(mk_not(m, cond, span), else_),
        (Feedback::Cond(en, v), Feedback::Hold) => Feedback::Cond(mk_and(m, cond, en, span), v),
        (Feedback::Hold, Feedback::Cond(en, v)) => {
            let off = mk_not(m, cond, span);
            Feedback::Cond(mk_and(m, off, en, span), v)
        }
        (Feedback::Load, Feedback::Cond(en, v)) => {
            let value = mux(m, then_, v);
            Feedback::Cond(mk_or(m, cond, en, span), value)
        }
        (Feedback::Cond(en, v), Feedback::Load) => {
            let value = mux(m, v, else_);
            let off = mk_not(m, cond, span);
            Feedback::Cond(mk_or(m, off, en, span), value)
        }
        (Feedback::Cond(e1, v1), Feedback::Cond(e2, v2)) => {
            let en = mux(m, e1, e2);
            let value = mux(m, v1, v2);
            Feedback::Cond(en, value)
        }
    }
}

/// Removes registers whose output nothing reads.
fn remove_unused(m: &mut Module) -> u64 {
    let expr_refs = expr_net_refs(m);
    let root_refs = root_net_refs(m);
    let mut doomed = Vec::new();
    for (id, cell) in m.cells.iter() {
        if !matches!(cell.kind, CellKind::Dff { .. }) || cell_is_kept(m, cell) {
            continue;
        }
        let Some(q) = cell.output("q") else {
            continue;
        };
        if is_port(m, q) {
            continue;
        }
        // The only root reference is this cell's own output.
        if expr_refs[q.index()] == 0 && root_refs[q.index()] == 1 {
            doomed.push(id);
        }
    }
    m.cells.retain(|id, _| !doomed.contains(&id));
    u64::try_from(doomed.len()).unwrap_or(u64::MAX)
}

/// Merges registers with identical kind, inputs, attributes and init.
fn merge_duplicates(m: &mut Module) -> u64 {
    type Key = (
        String,
        Vec<(Name, ExprId)>,
        Vec<(Name, AttrValue)>,
        Option<Const>,
    );
    let mut table: HashMap<Key, CellId> = HashMap::new();
    let mut doomed = Vec::new();
    let mut aliases = Vec::new();
    for (id, cell) in m.cells.iter() {
        if !matches!(cell.kind, CellKind::Dff { .. }) || cell_is_kept(m, cell) {
            continue;
        }
        let Some(q) = cell.output("q") else {
            continue;
        };
        let attrs: Vec<(Name, AttrValue)> = cell
            .attrs
            .iter()
            .chain(cell.params.iter())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let key = (
            crate::synth::opt::merge::cell_kind_key(&cell.kind),
            cell.inputs.clone(),
            attrs,
            init_of(m, q),
        );
        match table.get(&key) {
            Some(rep) => {
                let keep = m.cells[*rep].output("q").expect("dff output");
                if m.nets[keep].ty == m.nets[q].ty {
                    aliases.push((q, keep, cell.span));
                    doomed.push(id);
                }
            }
            None => {
                table.insert(key, id);
            }
        }
    }
    for (dup, keep, span) in aliases {
        let value = mk_net(m, keep, span);
        m.assigns.push(Assign {
            target: Lvalue::Net(dup),
            value,
            delay: None,
            attrs: Attrs::new(),
            span,
        });
    }
    m.cells.retain(|id, _| !doomed.contains(&id));
    u64::try_from(doomed.len()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Type;
    use crate::ir::builder::ModuleBuilder;
    use crate::synth::opt::testutil::{run, span};

    fn dff(
        b: &mut ModuleBuilder,
        name: &str,
        clk: ExprId,
        d: ExprId,
        q: NetId,
        en: Option<ExprId>,
        reset: Option<(Reset, ExprId)>,
    ) -> CellId {
        let mut inputs = vec![(Name::new("clk"), clk), (Name::new("d"), d)];
        if let Some(e) = en {
            inputs.push((Name::new("en"), e));
        }
        let reset = reset.map(|(r, rst)| {
            inputs.push((Name::new("rst"), rst));
            r
        });
        b.cell(
            name,
            CellKind::Dff {
                clk_pos: true,
                has_enable: en.is_some(),
                reset,
            },
            inputs,
            vec![(Name::new("q"), q)],
        )
    }

    #[test]
    fn extracts_reset_and_enable() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let en = b.input("en", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q = b.output("q", Type::bits(4));
        let (clkn, rstn, enn, dn, qn) = (b.net(clk), b.net(rst), b.net(en), b.net(d), b.net(q));
        let zero = b.const_u64(4, 0);
        let inner = b.mux(enn, dn, qn);
        let nrst = b.lnot(rstn);
        let outer = b.mux(nrst, inner, zero);
        dff(&mut b, "ff", clkn, outer, q, None, None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("sync resets extracted"), 1);
        assert_eq!(stats.get("enables extracted"), 1);
        assert!(
            m.to_text().contains(
                "cell ff dff pos en srst pos 4'd0 (clk=%clk, d=%d, en=%en, rst=%rst) -> (q=%q)"
            ),
            "{}",
            m.to_text()
        );
    }

    #[test]
    fn constant_registers_become_assigns() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let q1 = b.output("q1", Type::bits(4));
        let q2 = b.output("q2", Type::bits(4));
        let q3 = b.output("q3", Type::bits(4));
        let q4 = b.output("q4", Type::bits(4));
        let (clkn, rstn) = (b.net(clk), b.net(rst));
        let five = b.const_u64(4, 5);
        let r = Reset {
            asynchronous: false,
            active_high: true,
            value: Const::from_u64(5, 4),
        };
        dff(&mut b, "f1", clkn, five, q1, None, Some((r.clone(), rstn)));
        let q2n = b.net(q2);
        dff(&mut b, "f2", clkn, q2n, q2, None, Some((r, rstn)));
        let q3n = b.net(q3);
        b.net_attr(q3, "init", Const::from_u64(9, 4));
        dff(&mut b, "f3", clkn, q3n, q3, None, None);
        let one = b.const_bit(true);
        let ra = Reset {
            asynchronous: true,
            active_high: true,
            value: Const::from_u64(3, 4),
        };
        let q4n = b.net(q4);
        dff(&mut b, "f4", clkn, q4n, q4, None, Some((ra, one)));
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("constant registers"), 4);
        let t = m.to_text();
        assert!(t.contains("assign %q1 = 4'd5"), "{t}");
        assert!(t.contains("assign %q2 = 4'd5"), "{t}");
        assert!(t.contains("assign %q3 = 4'd9"), "{t}");
        assert!(t.contains("assign %q4 = 4'd3"), "{t}");
        assert!(m.cells.is_empty());
    }

    #[test]
    fn constant_controls_unused_and_duplicates() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q1 = b.output("q1", Type::bits(4));
        let q2 = b.output("q2", Type::bits(4));
        let dead = b.add_net("dead", Type::bits(4));
        let (clkn, dn) = (b.net(clk), b.net(d));
        let one = b.const_bit(true);
        let zero = b.const_bit(false);
        let r = Reset {
            asynchronous: false,
            active_high: true,
            value: Const::from_u64(0, 4),
        };
        dff(
            &mut b,
            "f1",
            clkn,
            dn,
            q1,
            Some(one),
            Some((r.clone(), zero)),
        );
        dff(&mut b, "f2", clkn, dn, q2, None, None);
        dff(&mut b, "f3", clkn, dn, dead, Some(zero), None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("constant enables removed"), 2);
        assert_eq!(stats.get("constant resets removed"), 1);
        assert_eq!(stats.get("registers merged"), 1);
        assert_eq!(stats.get("constant registers"), 1);
        let t = m.to_text();
        assert!(
            t.contains("cell f1 dff pos (clk=%clk, d=%d) -> (q=%q1)"),
            "{t}"
        );
        assert!(t.contains("assign %q2 = %q1"), "{t}");
        assert!(t.contains("assign %dead = 4'bxxxx"), "{t}");
    }

    #[test]
    fn removes_unused_registers() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bits(4));
        let dead = b.add_net("dead", Type::bits(4));
        let (clkn, dn) = (b.net(clk), b.net(d));
        dff(&mut b, "f", clkn, dn, dead, None, None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("unused registers removed"), 1);
        assert!(m.cells.is_empty());
    }

    /// The defect this pass was caught by: a `keep` written on a Verilog
    /// `reg` lands on the **net**, and reading only the cell's attributes
    /// deleted the flip-flop anyway. None of the three destructive rules may
    /// touch a register kept either way round, whichever vendor spelling
    /// asked for it.
    #[test]
    fn a_kept_register_is_never_folded_removed_or_merged() {
        for spelling in ["keep", "DONT_TOUCH", "mark_debug", "preserve"] {
            for on_the_cell in [false, true] {
                let mut b = ModuleBuilder::new("m", span());
                let clk = b.input("clk", Type::bit());
                let probe = b.output("probe", Type::bit());
                let twin_a = b.output("twin_a", Type::bit());
                let twin_b = b.output("twin_b", Type::bit());
                let dead = b.add_net("dead", Type::bit());
                let clkn = b.net(clk);
                let zero = b.const_bit(false);
                let cells = [
                    dff(&mut b, "probe", clkn, zero, probe, None, None),
                    dff(&mut b, "twin_a", clkn, zero, twin_a, None, None),
                    dff(&mut b, "twin_b", clkn, zero, twin_b, None, None),
                    dff(&mut b, "dead", clkn, zero, dead, None, None),
                ];
                for (i, net) in [probe, twin_a, twin_b, dead].into_iter().enumerate() {
                    if on_the_cell {
                        b.module_mut().cells[cells[i]].attrs.set(spelling, 1);
                    } else {
                        b.net_attr(net, spelling, 1);
                    }
                }
                let mut m = b.finish();
                let stats = run(&FfOpt, &mut m);
                let what = format!("{spelling}, on the cell: {on_the_cell}");
                assert_eq!(stats.get("constant registers"), 0, "{what}");
                assert_eq!(stats.get("unused registers removed"), 0, "{what}");
                assert_eq!(stats.get("registers merged"), 0, "{what}");
                assert_eq!(m.cells.len(), 4, "{what}\n{}", m.to_text());
            }
        }
    }

    /// A `keep` that is switched off keeps nothing, and neither does one
    /// spelled as a textual falsehood.
    #[test]
    fn a_keep_that_is_off_folds_as_usual() {
        for value in [AttrValue::Int(0), AttrValue::String("false".into())] {
            let mut b = ModuleBuilder::new("m", span());
            let clk = b.input("clk", Type::bit());
            let q = b.output("q", Type::bit());
            let clkn = b.net(clk);
            let zero = b.const_bit(false);
            dff(&mut b, "f", clkn, zero, q, None, None);
            b.net_attr(q, "keep", value.clone());
            let mut m = b.finish();
            let stats = run(&FfOpt, &mut m);
            assert_eq!(stats.get("constant registers"), 1, "{value}");
            assert!(m.cells.is_empty(), "{value}");
        }
    }

    /// Keeping a register does not stop it reaching a fabric's enable pin:
    /// lifting the mux out leaves the same flip-flop on the same net.
    #[test]
    fn a_kept_register_still_gives_up_its_enable_and_reset() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let en = b.input("en", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q = b.output("q", Type::bits(4));
        let (clkn, rstn, enn, dn, qn) = (b.net(clk), b.net(rst), b.net(en), b.net(d), b.net(q));
        b.net_attr(q, "keep", 1);
        let zero = b.const_u64(4, 0);
        let inner = b.mux(enn, dn, qn);
        let nrst = b.lnot(rstn);
        let outer = b.mux(nrst, inner, zero);
        dff(&mut b, "ff", clkn, outer, q, None, None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("sync resets extracted"), 1);
        assert_eq!(stats.get("enables extracted"), 1);
        assert!(
            m.to_text().contains(
                "cell ff dff pos en srst pos 4'd0 (clk=%clk, d=%d, en=%en, rst=%rst) -> (q=%q)"
            ),
            "{}",
            m.to_text()
        );
    }

    /// A register written a byte at a time is split by byte, each byte
    /// loading under its own condition: the shape a part-select write
    /// leaves, seen through the one-arm `pmux` process lowering puts round
    /// it. The second byte holds on its condition rather than loading on
    /// it, and the reset value and `init` are split with the bits.
    ///
    /// Without the split this register stays whole with every bit's hold
    /// multiplexer in `d`; with the polarity of the inverted arm wrong its
    /// enable would read `and(%we, %c0)`; with the reset or `init` taken
    /// from the wrong end, the byte values below would swap.
    #[test]
    fn a_register_written_a_byte_at_a_time_takes_an_enable_per_byte() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let we = b.input("we", Type::bit());
        let c1 = b.input("c1", Type::bit());
        let c0 = b.input("c0", Type::bit());
        let hi = b.input("hi", Type::bits(8));
        let lo = b.input("lo", Type::bits(8));
        let q = b.output("q", Type::bits(16));
        let arm = b.add_net("arm", Type::bits(16));
        let (clkn, rstn, wen, c1n, c0n) = (b.net(clk), b.net(rst), b.net(we), b.net(c1), b.net(c0));
        let (hin, lon, qn) = (b.net(hi), b.net(lo), b.net(q));
        b.net_attr(q, "init", Const::from_u64(0x1234, 16));
        let q_hi = b.slice(qn, 15, 8);
        let q_lo = b.slice(qn, 7, 0);
        let load_hi = b.mux(c1n, hin, q_hi);
        let load_lo = b.mux(c0n, q_lo, lon);
        let parts = b.concat(vec![load_hi, load_lo]);
        let undefined = b.expr(ExprKind::Const(Const::x(16)));
        b.cell(
            "pm",
            CellKind::Pmux,
            vec![
                (Name::new("a"), undefined),
                (Name::new("b"), parts),
                (Name::new("s"), wen),
            ],
            vec![(Name::new("y"), arm)],
        );
        let armn = b.net(arm);
        let r = Reset {
            asynchronous: false,
            active_high: true,
            value: Const::from_u64(0xabcd, 16),
        };
        dff(&mut b, "ff", clkn, armn, q, Some(wen), Some((r, rstn)));
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("registers split by enable"), 1);
        let t = m.to_text();
        for line in [
            "cell ff dff pos en srst pos 8'd171 (clk=%clk, d=%hi, en=and(%we, %c1), rst=%rst) -> (q=%q$8)",
            "cell ff$0 dff pos en srst pos 8'd205 (clk=%clk, d=%lo, en=and(%we, not(%c0)), rst=%rst) -> (q=%q$0)",
            "assign %q = {%q$8, %q$0}",
            "attr init = 8'd18\n  net %q$8 u8 wire",
            "attr init = 8'd52\n  net %q$0 u8 wire",
        ] {
            assert!(t.contains(line), "missing `{line}`:\n{t}");
        }
    }

    /// A hold arm only counts as one when its bits are the register's own
    /// bits **in the same place**: `c ? x : q[7:0]` in the top byte is a
    /// rotation, and splitting it into an enable would make the top byte
    /// hold its own value instead of taking the bottom byte's.
    #[test]
    fn a_hold_arm_from_other_bits_is_not_an_enable() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let c = b.input("c", Type::bit());
        let x = b.input("x", Type::bits(8));
        let q = b.output("q", Type::bits(16));
        let (clkn, cn, xn, qn) = (b.net(clk), b.net(c), b.net(x), b.net(q));
        let q_lo = b.slice(qn, 7, 0);
        let q_lo2 = b.slice(qn, 7, 0);
        let top = b.mux(cn, xn, q_lo);
        let bottom = b.mux(cn, xn, q_lo2);
        let d = b.concat(vec![top, bottom]);
        dff(&mut b, "ff", clkn, d, q, None, None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("registers split by enable"), 1);
        let t = m.to_text();
        assert!(t.contains("d=%x, en=%c) -> (q=%q$0)"), "{t}");
        assert!(!t.contains("d=%x, en=%c) -> (q=%q$8)"), "{t}");
    }

    /// The bits of `d = c ? {0, x} : y` whose restart value is a constant
    /// become a register with a synchronous reset, under the enable the
    /// register already has. The reset takes priority over the enable in
    /// the cell, so it has to be `en & c` here, where `c` does not imply
    /// `en`: a reset on `c` alone would clear the register on a cycle the
    /// enable holds it.
    #[test]
    fn a_restart_under_an_enable_resets_only_when_enabled() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let en = b.input("en", Type::bit());
        let c = b.input("c", Type::bit());
        let x = b.input("x", Type::bit());
        let y = b.input("y", Type::bits(4));
        let q = b.output("q", Type::bits(4));
        let (clkn, enn, cn, xn, yn) = (b.net(clk), b.net(en), b.net(c), b.net(x), b.net(y));
        let zeros = b.const_u64(3, 0);
        let restart = b.concat(vec![zeros, xn]);
        let d = b.mux(cn, restart, yn);
        dff(&mut b, "ff", clkn, d, q, Some(enn), None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("registers split by reset"), 1);
        let t = m.to_text();
        assert!(
            t.contains("dff pos en srst pos 3'd0 (clk=%clk, d=%y[3:1], en=%en, rst=and(%en, %c))"),
            "{t}"
        );
        assert!(
            t.contains("d=mux(%c, {3'd0, %x}[0:0], %y[0:0]), en=%en) -> (q=%q$0)"),
            "{t}"
        );
    }

    /// A hold under the other arm: `c ? (c2 ? v : q) : w` loads unless
    /// `c` is high and `c2` low, so its enable is `not(c) | c2` and it
    /// loads `c ? v : w`. With the arms of the outer multiplexer swapped
    /// in the value, or the inner condition's polarity lost in the
    /// enable, this reads differently.
    #[test]
    fn a_hold_one_multiplexer_down_is_an_enable() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let c = b.input("c", Type::bit());
        let c2 = b.input("c2", Type::bit());
        let v = b.input("v", Type::bits(4));
        let w = b.input("w", Type::bits(4));
        let q = b.output("q", Type::bits(4));
        let (clkn, cn, c2n, vn, wn, qn) = (
            b.net(clk),
            b.net(c),
            b.net(c2),
            b.net(v),
            b.net(w),
            b.net(q),
        );
        let inner = b.mux(c2n, vn, qn);
        let d = b.mux(cn, inner, wn);
        dff(&mut b, "ff", clkn, d, q, None, None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("enables extracted"), 1);
        let t = m.to_text();
        assert!(
            t.contains(
                "cell ff dff pos en (clk=%clk, d=mux(%c, %v, %w), en=or(not(%c), %c2)) -> (q=%q)"
            ),
            "{t}"
        );
    }
}
