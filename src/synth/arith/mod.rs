//! Arithmetic lowering: adders, multipliers, comparators, shifters and
//! dividers, with a choice of architectures.
//!
//! Without this pass an `add` cell survives all the way to technology
//! mapping as one generic cell, and the AIG expands it into a ripple
//! carry. That is correct, and for a small FPGA it is usually what you
//! want — but it is *one* architecture, chosen implicitly, and a
//! designer who needs the adder on the critical path to be fast has no
//! way to ask. This module is the answer: every arithmetic cell has
//! several documented implementations, the choice is an option and a
//! per-cell attribute, and every implementation is proved equivalent to
//! the generic cell it replaces.
//!
//! # What is here
//!
//! | Module | Architectures |
//! |--------|---------------|
//! | [`adder`] | ripple carry, carry-select, carry-lookahead, Kogge-Stone, Brent-Kung |
//! | [`mul`] | shift-and-add array, radix-4 Booth, Wallace tree, Dadda tree; signed and unsigned |
//! | [`cmp`] | ripple and prefix comparators, for `==`, `<` and the signed variants |
//! | [`shift`] | barrel shifters at radix 2 and 4, and a funnel shifter that also rotates |
//! | [`div`] | the combinational restoring array, up to a width threshold |
//! | [`prefix`] | the parallel prefix networks the fast adder and the fast comparator share |
//! | [`bits`] | the gate emitter they all build on |
//! | [`dsp`] | recognition of multiplies a DSP block could implement |
//!
//! Each architecture is a function taking bit vectors and returning bit
//! vectors, built into a module through
//! [`crate::ir::builder::ModuleBuilder`], with the carry in and carry
//! out exposed so wider structures can be chained out of them. They are
//! useful on their own — a datapath generator can call [`adder::add`]
//! directly — and [`ArithLower`] is what applies them to a netlist.
//!
//! # The pass
//!
//! [`ArithLower`] replaces each arithmetic cell with the chosen
//! architecture, driven by [`ArithOptions`] and, per cell, by an
//! `arith` attribute:
//!
//! ```text
//! (* arith = "kogge_stone" *) assign sum = a + b;
//! ```
//!
//! The attribute takes any architecture name ([`AdderArch::name`] and
//! its siblings), or `"none"` to leave that one cell generic. A module
//! may carry the same attribute as its own default. An unrecognised
//! name is a warning (`S0041`) and the cell keeps the default.
//!
//! **Placement.** The pass belongs after the optimisation loop and
//! [`crate::synth::cellify`], and before technology mapping: the
//! optimiser is most effective on whole-word cells (it folds a constant
//! operand of an `add`, reduces its width, merges two identical ones),
//! and the mapper wants gates. Build it into a [`crate::synth::PassManager`]:
//!
//! ```no_run
//! # use reticle::ir::Design;
//! # use reticle::diag::Diagnostics;
//! use reticle::synth::arith::{ArithLower, ArithOptions};
//! use reticle::synth::{PassManager, SynthOptions, opt, run};
//!
//! # fn f(design: &mut Design, diags: &mut Diagnostics) {
//! let options = SynthOptions::default();
//! run(design, &options, diags);
//! let mut pm = PassManager::new(options);
//! pm.add(ArithLower::new(ArithOptions::default()));
//! pm.add(opt::Merge);
//! pm.add(opt::Dce);
//! pm.run(design, diags);
//! # }
//! ```
//!
//! **It is opt-in.** [`crate::synth::run`] does not run it. Lowering
//! arithmetic before the AIG hands the optimiser a netlist it can
//! rewrite but also one it can no longer recognise as an adder, and
//! whether that is a win depends on the design and the target; until
//! that is measured on real designs the default pipeline is left alone
//! and the pass is something a caller asks for. `docs/arithmetic.md`
//! has the measurements that exist so far.
//!
//! # Defaults
//!
//! The defaults are area-oriented, because a small FPGA is the common
//! case and because the architecture that wins on depth loses on cells
//! by a factor of two to four:
//!
//! - adders and subtractors: [`AdderArch::Ripple`];
//! - multipliers: [`MultiplierArch::Array`];
//! - comparators: [`CompareArch::Ripple`];
//! - shifters: [`ShifterArch::Barrel`] (radix 2);
//! - dividers: the restoring array up to 8 bits, generic above.
//!
//! Ask for [`AdderArch::KoggeStone`] or [`AdderArch::BrentKung`] when
//! the adder is on the critical path, [`MultiplierArch::Dadda`] when
//! the multiplier is, and [`CompareArch::Prefix`] for any comparison
//! wider than a few bits — a fast comparator costs `n - 1` prefix nodes
//! against a fast adder's `n log n`, so it is nearly free.
//!
//! # Width thresholds
//!
//! [`WidthThresholds`] caps what the pass will expand. Above the
//! multiplier cap the `mul` cell is left generic with a note (`S0042`);
//! above the divider cap, likewise with a warning (`S0043`), because a
//! combinational divider grows as the square of the width and the right
//! answer is a pipelined sequential divider, which a combinational pass
//! cannot produce. See [`div`].
//!
//! # Diagnostics
//!
//! | Code    | Meaning                                                     |
//! |---------|-------------------------------------------------------------|
//! | `S0041` | An `arith` attribute names no architecture for that cell     |
//! | `S0042` | A multiply is wider than the threshold and was left generic  |
//! | `S0043` | A division is wider than the threshold and was left generic  |

pub mod adder;
pub mod bits;
pub mod cmp;
pub mod div;
pub mod dsp;
pub mod mul;
pub mod prefix;
pub mod shift;

#[cfg(test)]
pub(crate) mod testkit;

use std::collections::HashSet;

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::builder::ModuleBuilder;
use crate::ir::{Attrs, CellId, CellKind, ExprId, Module, NetId};
use crate::source::Span;
use crate::synth::{Pass, PassStats};

pub use adder::{AdderArch, AdderOptions};
pub use bits::GateBuilder;
pub use cmp::CompareArch;
pub use dsp::{DspCandidate, DspKind, DspOperand};
pub use mul::{MultiplierArch, MultiplierOptions};
pub use shift::{ShiftKind, ShifterArch};

/// Every multiply in `module` that a DSP block could implement.
///
/// A thin alias for [`dsp::candidates`]; see that module for what is
/// recognised and what deliberately is not.
pub fn dsp_candidates(module: &Module) -> Vec<DspCandidate> {
    dsp::candidates(module)
}

/// Widths above (or below) which the pass declines to expand a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthThresholds {
    /// The narrowest operand the pass expands. Narrower cells are left
    /// generic for the AIG, which handles a handful of bits at least as
    /// well as any architecture here. One by default: expand
    /// everything.
    pub min_width: u32,
    /// The widest multiply expanded. A wider one is left generic with
    /// `S0042`, since the partial product matrix grows as the square of
    /// the width. 32 by default.
    pub multiplier: u32,
    /// The widest division expanded. A wider one is left generic with
    /// `S0043` and should be pipelined instead; see [`div`]. 8 by
    /// default.
    pub divider: u32,
}

impl Default for WidthThresholds {
    fn default() -> Self {
        WidthThresholds {
            min_width: 1,
            multiplier: 32,
            divider: 8,
        }
    }
}

/// Which architecture each kind of arithmetic cell is lowered to.
///
/// The defaults are area-oriented; see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ArithOptions {
    /// The adder architecture, used for `add` and `sub` cells and
    /// inside the multipliers and the divider.
    pub adder: AdderOptions,
    /// The multiplier architecture.
    pub multiplier: MultiplierOptions,
    /// The shifter architecture, used for `shl`, `shr` and `sshr`.
    pub shifter: ShifterArch,
    /// The comparator architecture, used for `eq`, `ne`, `lt`, `le`,
    /// `gt` and `ge`.
    pub comparator: CompareArch,
    /// Widths the pass declines to expand.
    pub width_thresholds: WidthThresholds,
}

impl ArithOptions {
    /// The area-oriented defaults.
    pub fn new() -> ArithOptions {
        ArithOptions::default()
    }

    /// The depth-oriented choice: Kogge-Stone adders, Dadda
    /// multipliers, prefix comparators, and the same thresholds.
    ///
    /// This is a convenience for asking "how fast could this be", not a
    /// recommendation: it costs two to four times the cells.
    pub fn fast() -> ArithOptions {
        ArithOptions {
            adder: AdderOptions::new(AdderArch::KoggeStone),
            multiplier: MultiplierOptions {
                arch: MultiplierArch::Dadda,
                adder: AdderOptions::new(AdderArch::KoggeStone),
            },
            shifter: ShifterArch::Barrel,
            comparator: CompareArch::Prefix,
            width_thresholds: WidthThresholds::default(),
        }
    }

    /// Applies an `arith` attribute value to the field that governs
    /// `kind`, returning false when the value names nothing.
    ///
    /// `"none"`, `"off"`, `"keep"` and `"generic"` are handled by the
    /// caller, not here.
    fn apply(&mut self, kind: &CellKind, value: &str) -> bool {
        match kind {
            CellKind::Add | CellKind::Sub => match AdderArch::from_name(value) {
                Some(a) => {
                    self.adder.arch = a;
                    true
                }
                None => false,
            },
            CellKind::Mul => match MultiplierArch::from_name(value) {
                Some(a) => {
                    self.multiplier.arch = a;
                    true
                }
                None => match AdderArch::from_name(value) {
                    // A multiplier ends in an adder, so naming an adder
                    // architecture on a `mul` is a sensible thing to
                    // want and is taken to mean that one.
                    Some(a) => {
                        self.multiplier.adder.arch = a;
                        true
                    }
                    None => false,
                },
            },
            CellKind::Div | CellKind::Mod => match AdderArch::from_name(value) {
                Some(a) => {
                    self.adder.arch = a;
                    true
                }
                None => false,
            },
            CellKind::Shl | CellKind::Shr | CellKind::Sshr => match ShifterArch::from_name(value) {
                Some(a) => {
                    self.shifter = a;
                    true
                }
                None => false,
            },
            CellKind::Eq
            | CellKind::Ne
            | CellKind::Lt
            | CellKind::Le
            | CellKind::Gt
            | CellKind::Ge => match CompareArch::from_name(value) {
                Some(a) => {
                    self.comparator = a;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
}

/// The attribute values that mean "leave this cell alone".
fn is_opt_out(value: &str) -> bool {
    matches!(value, "none" | "off" | "keep" | "generic")
}

/// The `arith` attribute in effect for a cell, and whether it was the
/// cell's own.
///
/// A module-wide attribute is a *default*, so one that names no
/// architecture for a given cell kind — `arith = "kogge_stone"` on a
/// module that also has a shifter in it — is simply not for that cell
/// and is passed over in silence. The same value written on the cell
/// itself is a request that cannot be honoured, and says so.
fn attribute<'a>(cell: &'a Attrs, module: &'a Attrs) -> Option<(&'a str, bool)> {
    if let Some(value) = cell.get("arith").and_then(crate::ir::AttrValue::as_str) {
        return Some((value, true));
    }
    module
        .get("arith")
        .and_then(crate::ir::AttrValue::as_str)
        .map(|value| (value, false))
}

/// Replaces arithmetic cells with a concrete architecture.
///
/// See the module docs for the options, the attribute and where the
/// pass belongs in a pipeline.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArithLower {
    /// The architectures to build.
    pub options: ArithOptions,
}

impl ArithLower {
    /// The pass with the given options.
    pub fn new(options: ArithOptions) -> ArithLower {
        ArithLower { options }
    }
}

/// One cell the pass decided to lower, with the options it resolved.
struct Job {
    cell: CellId,
    kind: CellKind,
    name: String,
    a: ExprId,
    b: ExprId,
    output: NetId,
    /// Width of the result, and of both operands except a shift amount.
    width: u32,
    /// Width of the right operand, which differs only for shifts.
    amount_width: u32,
    /// True when both operands are signed.
    signed: bool,
    /// True when the left operand alone is signed, which is what an
    /// arithmetic right shift looks at.
    left_signed: bool,
    span: Span,
    options: ArithOptions,
}

impl Pass for ArithLower {
    fn name(&self) -> &'static str {
        "arith_lower"
    }

    fn run(&self, module: &mut Module, diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let jobs = self.plan(module, diags);
        if jobs.is_empty() {
            return stats;
        }
        let span = module.span;
        let taken = std::mem::replace(module, Module::new("", span));
        let mut builder = ModuleBuilder::from_module(taken, span);
        let mut replaced: HashSet<CellId> = HashSet::new();
        for job in &jobs {
            builder.span = job.span;
            let cells = {
                let mut cx = GateBuilder::new(&mut builder, &job.name);
                let bits = build(&mut cx, job);
                let value = cx.join(&bits);
                let count = cx.cell_count();
                cx.builder().assign(job.output, value);
                count
            };
            stats.bump("lowered", 1);
            stats.bump("cells", cells);
            replaced.insert(job.cell);
        }
        *module = builder.finish();
        module.cells.retain(|id, _| !replaced.contains(&id));
        stats
    }
}

impl ArithLower {
    /// Decides which cells to lower and with which options.
    fn plan(&self, module: &Module, diags: &mut Diagnostics) -> Vec<Job> {
        let mut jobs = Vec::new();
        for (id, cell) in module.cells.iter() {
            if !is_arithmetic(&cell.kind) {
                continue;
            }
            let (Some(a), Some(b), Some(y)) = (cell.input("a"), cell.input("b"), cell.output("y"))
            else {
                continue;
            };
            let mut options = self.options;
            if let Some((value, from_cell)) = attribute(&cell.attrs, &module.attrs) {
                if is_opt_out(value) {
                    continue;
                }
                if !options.apply(&cell.kind, value) && from_cell {
                    diags.push(
                        Diagnostic::warning(format!(
                            "`{value}` is not an architecture for a `{}` cell",
                            cell.kind.keyword()
                        ))
                        .with_code("S0041")
                        .with_span(cell.span)
                        .with_note("the cell keeps the architecture the options chose"),
                    );
                }
            }
            let ta = &module.expr(a).ty;
            let tb = &module.expr(b).ty;
            let (Some(wa), Some(wb)) = (ta.width(), tb.width()) else {
                continue;
            };
            let width = if cell.kind.is_predicate() {
                wa
            } else {
                module.nets[y].ty.width().unwrap_or(wa)
            };
            if width == 0 || width < options.width_thresholds.min_width {
                continue;
            }
            match cell.kind {
                CellKind::Mul if width > options.width_thresholds.multiplier => {
                    diags.push(
                        Diagnostic::note(format!(
                            "`{}` is a {width}-bit multiply, wider than the threshold of {}; it stays a generic cell",
                            cell.name, options.width_thresholds.multiplier
                        ))
                        .with_code("S0042")
                        .with_span(cell.span),
                    );
                    continue;
                }
                CellKind::Div | CellKind::Mod if width > options.width_thresholds.divider => {
                    diags.push(
                        Diagnostic::warning(format!(
                            "`{}` is a {width}-bit division, wider than the threshold of {}; it stays a generic cell",
                            cell.name, options.width_thresholds.divider
                        ))
                        .with_code("S0043")
                        .with_span(cell.span)
                        .with_note(
                            "a combinational divider grows as the square of the width; \
                             write the division as a pipelined sequential block, or raise \
                             `width_thresholds.divider` if the area is really wanted",
                        ),
                    );
                    continue;
                }
                _ => {}
            }
            jobs.push(Job {
                cell: id,
                kind: cell.kind.clone(),
                name: cell.name.to_string(),
                a,
                b,
                output: y,
                width,
                amount_width: wb,
                signed: ta.is_signed() && tb.is_signed(),
                left_signed: ta.is_signed(),
                span: cell.span,
                options,
            });
        }
        jobs
    }
}

/// True for the cells this pass knows how to expand.
fn is_arithmetic(kind: &CellKind) -> bool {
    matches!(
        kind,
        CellKind::Add
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
    )
}

/// Builds the replacement network for one job, returning its bits LSB
/// first.
fn build(cx: &mut GateBuilder<'_>, job: &Job) -> Vec<ExprId> {
    let o = &job.options;
    let a = cx.split(job.a, job.width);
    match job.kind {
        CellKind::Shl | CellKind::Shr | CellKind::Sshr => {
            let amount = cx.split(job.b, job.amount_width);
            let kind = match job.kind {
                CellKind::Shl => ShiftKind::Left,
                CellKind::Sshr if job.left_signed => ShiftKind::RightArithmetic,
                _ => ShiftKind::RightLogical,
            };
            return shift::shift(cx, o.shifter, &a, &amount, kind);
        }
        _ => {}
    }
    let b = cx.split(job.b, job.width);
    match job.kind {
        CellKind::Add => {
            let zero = cx.zero();
            adder::add(cx, &o.adder, &a, &b, zero).bits
        }
        CellKind::Sub => {
            let zero = cx.zero();
            adder::subtract(cx, &o.adder, &a, &b, zero).bits
        }
        CellKind::Mul => {
            let width = usize::try_from(job.width).expect("a width fits in a usize");
            mul::multiply(cx, &o.multiplier, &a, &b, job.signed, width)
        }
        CellKind::Div => div::divide(cx, &o.adder, &a, &b, job.signed).quotient,
        CellKind::Mod => div::divide(cx, &o.adder, &a, &b, job.signed).remainder,
        CellKind::Eq => vec![cmp::equal(cx, o.comparator, &a, &b)],
        CellKind::Ne => {
            let eq = cmp::equal(cx, o.comparator, &a, &b);
            vec![cx.not(eq)]
        }
        CellKind::Lt => vec![cmp::less_than(cx, o.comparator, &a, &b, job.signed)],
        CellKind::Gt => vec![cmp::less_than(cx, o.comparator, &b, &a, job.signed)],
        CellKind::Le => {
            let gt = cmp::less_than(cx, o.comparator, &b, &a, job.signed);
            vec![cx.not(gt)]
        }
        CellKind::Ge => {
            let lt = cmp::less_than(cx, o.comparator, &a, &b, job.signed);
            vec![cx.not(lt)]
        }
        _ => unreachable!("`is_arithmetic` admitted a cell `build` does not handle"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Type;
    use crate::synth::arith::testkit::span;

    #[test]
    fn the_defaults_are_area_oriented() {
        let o = ArithOptions::new();
        assert_eq!(o.adder.arch, AdderArch::Ripple);
        assert_eq!(o.multiplier.arch, MultiplierArch::Array);
        assert_eq!(o.comparator, CompareArch::Ripple);
        assert_eq!(o.shifter, ShifterArch::Barrel);
        assert_eq!(o.width_thresholds.divider, 8);
        assert_eq!(o.width_thresholds.multiplier, 32);
        assert_eq!(o.width_thresholds.min_width, 1);
        let fast = ArithOptions::fast();
        assert_eq!(fast.adder.arch, AdderArch::KoggeStone);
        assert_eq!(fast.multiplier.arch, MultiplierArch::Dadda);
        assert_eq!(fast.comparator, CompareArch::Prefix);
    }

    /// An attribute value lands on the field that governs the cell, and
    /// a name from the wrong family is refused.
    #[test]
    fn attribute_values_apply_by_cell_kind() {
        let mut o = ArithOptions::default();
        assert!(o.apply(&CellKind::Add, "brent_kung"));
        assert_eq!(o.adder.arch, AdderArch::BrentKung);
        assert!(o.apply(&CellKind::Mul, "dadda"));
        assert_eq!(o.multiplier.arch, MultiplierArch::Dadda);
        // An adder name on a multiply picks its final adder.
        assert!(o.apply(&CellKind::Mul, "kogge_stone"));
        assert_eq!(o.multiplier.adder.arch, AdderArch::KoggeStone);
        assert!(o.apply(&CellKind::Sshr, "funnel"));
        assert_eq!(o.shifter, ShifterArch::Funnel);
        assert!(o.apply(&CellKind::Le, "prefix"));
        assert_eq!(o.comparator, CompareArch::Prefix);
        assert!(o.apply(&CellKind::Mod, "carry_select"));
        assert_eq!(o.adder.arch, AdderArch::CarrySelect);
        assert!(!o.apply(&CellKind::Add, "wallace"));
        assert!(!o.apply(&CellKind::Lt, "barrel"));
        assert!(!o.apply(&CellKind::And, "ripple"));
        for value in ["none", "off", "keep", "generic"] {
            assert!(is_opt_out(value));
        }
        assert!(!is_opt_out("ripple"));
    }

    /// The cell's own attribute wins over the module's, and the module's
    /// is marked as a default.
    #[test]
    fn a_cell_attribute_wins() {
        let mut cell = Attrs::new();
        let mut module = Attrs::new();
        assert_eq!(attribute(&cell, &module), None);
        module.set("arith", "brent_kung");
        assert_eq!(attribute(&cell, &module), Some(("brent_kung", false)));
        cell.set("arith", "kogge_stone");
        assert_eq!(attribute(&cell, &module), Some(("kogge_stone", true)));
        // A non-string value is not an architecture name.
        let mut numeric = Attrs::new();
        numeric.set("arith", 1);
        assert_eq!(attribute(&numeric, &Attrs::new()), None);
    }

    /// The pass leaves non-arithmetic cells, sequential cells and black
    /// boxes alone, and every net it adds is driven.
    #[test]
    fn only_arithmetic_is_touched() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let z = b.output("z", Type::bits(4));
        let (av, cv) = (b.net(a), b.net(c));
        b.cell2("and0", CellKind::And, av, cv, y);
        b.cell2("add0", CellKind::Add, av, cv, z);
        let mut m = b.finish();
        let mut diags = Diagnostics::new();
        let stats = ArithLower::default().run(&mut m, &mut diags);
        assert_eq!(stats.get("lowered"), 1);
        assert!(m.cell_by_name("and0").is_some());
        assert!(m.cell_by_name("add0").is_none());
        assert!(crate::ir::validate::validate_module(&m).is_empty());
        assert_eq!(ArithLower::default().name(), "arith_lower");
    }
}
