//! Post-synthesis equivalence checking.
//!
//! Phase 5 of `ROADMAP.md` asks for an equivalence check of the synthesised
//! design against the pre-synthesis IR, through phase 7's engine.
//! [`check_synthesis`] is that check.
//!
//! # What is actually checked, and what is not
//!
//! The engine ([`crate::formal::equiv::check_equivalent`]) works on the
//! *cell* form: cells, continuous assignments and their expression trees.
//! It cannot read the process form, which is exactly what the design looks
//! like before synthesis. A check of "the input IR versus the output IR"
//! is therefore not something the engine can be handed directly.
//!
//! So this module does the next best thing, and says so plainly: it
//! lowers a **copy of the original** with a minimal, trusted pipeline —
//! [`crate::synth::proc::ProcLower`] alone, no optimisation, no FSM
//! re-encoding, no cellify — and proves *that* equivalent to the fully
//! optimised and cellified result. In other words:
//!
//! > **This checks that the optimisation and mapping passes preserved
//! > behaviour. It does not check that process lowering itself is
//! > correct.**
//!
//! Process lowering is the one pass the check takes on trust, because it
//! is the pass that produces the only form the checker can read. It is
//! covered instead by the golden tests in `testdata/synth/` and by the
//! simulation self-check in `crate::synth`'s test suite. Everything after
//! it — constant folding, dead-code elimination, width reduction,
//! flip-flop rewriting, common-subexpression merging, FSM re-encoding and
//! [`crate::synth::cellify`] — is what the proof covers, and those are the
//! passes that rewrite logic and are worth proving.
//!
//! # How the check runs
//!
//! Both versions go into one [`Design`] (the reference under
//! `<name>$golden`), because [`check_equivalent`] takes two modules of a
//! single design. Ports are matched by name, so the interface must be
//! unchanged — which synthesis guarantees.
//!
//! [`EquivOptions::match_state_by_name`] is on by default. Without it, a
//! sequential miter is rarely inductive: "the outputs agree" says nothing
//! about the registers, so k-induction starts from unreachable state pairs
//! and gives up. With it, every same-named, same-width flip-flop of the
//! two versions is also required to stay equal, and the conjunction is
//! usually 1-inductive. The pass names registers after the nets they
//! drive, and neither optimisation nor cellify renames a surviving
//! register, so the names line up.
//!
//! # When the answer is not a proof
//!
//! [`SynthVerifyReport::inconclusive`] is true when the checker could not
//! settle the question. The usual reasons:
//!
//! - The module still has an unsynthesisable process, a latch, a black
//!   box, an `inout` port or an instance, none of which the bit-blaster
//!   models (`F0001`, `F0003`, `F0004`, `F0006`).
//! - A register has no reset value, so there is no state to start the
//!   induction from (`F0018`).
//! - FSM re-encoding changed the state encoding, so the registers no
//!   longer match by name and value and the miter is not inductive.
//!
//! An inconclusive result is a warning, not a failure; a genuine
//! difference is [`EquivOutcome::Different`] with a counter-example trace.
//!
//! # Don't-cares are the one honest false alarm
//!
//! Synthesis treats `x` as a don't-care and resolves it to whatever is
//! cheapest: `mux(c, v, 'x)` folds to `v`, and the shifter
//! [`crate::synth::cellify`] builds for a variable select reads an
//! out-of-range index as `0` rather than `x`. The bit-blaster, by
//! contrast, models every `x` bit as a *free* value
//! ([`crate::formal::blast::const_lits`]), because that is the right
//! reading for a property check. The reference still holds the `x` the
//! source wrote, the result holds the value synthesis chose, and the
//! miter duly reports [`EquivOutcome::Different`].
//!
//! That is not a bug in either side: strict equivalence and don't-care
//! exploitation genuinely disagree. A design that leaves an output `x` on
//! some input therefore fails this check, and the counter-example says
//! which output and on which input, so it can be judged by eye. Designs
//! that assign every output on every path — which is what synthesisable
//! RTL should do — are unaffected.

use std::fmt::Write as _;

use crate::diag::{Diagnostic, Diagnostics};
use crate::formal::equiv::{EquivOptions, EquivOutcome, check_equivalent};
use crate::ir::{Design, ModuleId, Name};
use crate::synth::proc::ProcLower;
use crate::synth::{FsmEncoding, Pass, SynthOptions};

/// Appended to the reference module's name inside the combined design.
const REFERENCE_SUFFIX: &str = "$golden";

/// Knobs for [`check_synthesis`].
#[derive(Clone, Debug)]
pub struct VerifyOptions {
    /// Options handed to [`check_equivalent`]. The default has
    /// [`EquivOptions::match_state_by_name`] on, which is what makes the
    /// induction strong enough for a registered design.
    pub equiv: EquivOptions,
    /// Loop-unrolling bound used while lowering the reference copy;
    /// mirrors [`SynthOptions::max_unroll`].
    pub max_unroll: u32,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        VerifyOptions {
            equiv: EquivOptions::default(),
            max_unroll: SynthOptions::default().max_unroll,
        }
    }
}

/// What [`check_synthesis`] found for one module.
#[derive(Clone, Debug)]
pub struct SynthVerifyReport {
    /// The module checked.
    pub module: String,
    /// The verdict of the equivalence engine.
    pub outcome: EquivOutcome,
    /// Warnings from the check, and errors when it could not run.
    pub diags: Diagnostics,
}

impl SynthVerifyReport {
    /// True when the synthesised module was proved equivalent to the
    /// reference.
    pub fn passed(&self) -> bool {
        matches!(self.outcome, EquivOutcome::Equivalent(_))
    }

    /// True when the check neither proved equivalence nor found a
    /// difference; see the module docs for the usual causes.
    pub fn inconclusive(&self) -> bool {
        matches!(self.outcome, EquivOutcome::Unknown { .. })
    }

    /// Renders the verdict, with the counter-example trace if any.
    /// Diagnostics are not included; render them with
    /// [`Diagnostics::render`].
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = write!(out, "synthesis of {}: ", self.module);
        match &self.outcome {
            EquivOutcome::Equivalent(proof) => {
                let _ = writeln!(out, "EQUIVALENT ({proof:?})");
            }
            EquivOutcome::Different {
                frame,
                properties,
                trace,
            } => {
                let list: Vec<String> = properties.iter().map(|p| format!("`{p}`")).collect();
                let _ = writeln!(out, "DIFFERENT at frame {frame} ({})", list.join(", "));
                out.push_str("  trace:\n");
                for line in trace.render().lines() {
                    let _ = writeln!(out, "    {line}");
                }
            }
            EquivOutcome::Unknown { depth: Some(d) } => {
                let _ = writeln!(
                    out,
                    "INCONCLUSIVE (no difference to depth {d}, no inductive proof)"
                );
            }
            EquivOutcome::Unknown { depth: None } => {
                let _ = writeln!(out, "INCONCLUSIVE");
            }
        }
        out
    }
}

/// Checks module `module` of `synthesised` against the same module of
/// `original`, lowered with process lowering only.
///
/// `module` is a [`ModuleId`] of `synthesised`; the counterpart in
/// `original` is found by name, since synthesis never renames a module.
/// See the module docs for what the result does and does not establish.
pub fn check_synthesis(
    original: &Design,
    synthesised: &Design,
    module: ModuleId,
    options: &VerifyOptions,
) -> SynthVerifyReport {
    let name = synthesised.module(module).name.to_string();
    check(original, synthesised, module, &name, options)
}

/// [`check_synthesis`] with the module named rather than identified.
///
/// Returns `None` when `synthesised` has no module of that name.
pub fn check_synthesis_by_name(
    original: &Design,
    synthesised: &Design,
    name: &str,
    options: &VerifyOptions,
) -> Option<SynthVerifyReport> {
    let id = synthesised.module_by_name(name)?;
    Some(check(original, synthesised, id, name, options))
}

fn check(
    original: &Design,
    synthesised: &Design,
    module: ModuleId,
    name: &str,
    options: &VerifyOptions,
) -> SynthVerifyReport {
    let mut diags = Diagnostics::new();
    let span = synthesised.module(module).span;
    let unknown = |diags: Diagnostics| SynthVerifyReport {
        module: name.to_owned(),
        outcome: EquivOutcome::Unknown { depth: None },
        diags,
    };

    let Some(source) = original.module_by_name(name) else {
        diags.push(
            Diagnostic::error(format!(
                "`{name}` has no counterpart in the pre-synthesis design"
            ))
            .with_code("S0032")
            .with_span(span),
        );
        return unknown(diags);
    };

    // The reference: the original module, process-lowered and nothing
    // else, under a name of its own inside a copy of the synthesised
    // design (which is where `module` is valid).
    let mut reference = original.module(source).clone();
    reference.name = Name::new(format!("{name}{REFERENCE_SUFFIX}"));
    let lower_options = SynthOptions {
        cellify: false,
        fsm_encoding: FsmEncoding::None,
        max_unroll: options.max_unroll,
        validate: false,
        verify_equivalence: false,
        ..SynthOptions::default()
    };
    let mut lowering = Diagnostics::new();
    ProcLower::new(&lower_options).run(&mut reference, &mut lowering);
    // Lowering the reference re-reports whatever the real run already
    // reported; only its errors are worth repeating, as they explain an
    // inconclusive verdict.
    for d in lowering.iter().filter(|d| d.is_error()) {
        diags.push(d.clone());
    }

    let mut combined = synthesised.clone();
    let reference = combined.add_module(reference);

    let mut report = check_equivalent(&combined, reference, module, &options.equiv);
    diags.append(&mut report.diags);
    SynthVerifyReport {
        module: name.to_owned(),
        outcome: report.outcome,
        diags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formal::InitMode;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{Const, ExprKind, ProcessKind, Type};
    use crate::source::{SourceMap, Span};
    use crate::synth::run;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// Quick options: a shallow bound is enough for these tiny designs.
    fn options() -> VerifyOptions {
        VerifyOptions {
            equiv: EquivOptions {
                depth: 4,
                max_k: 2,
                init: InitMode::Reset,
                ..EquivOptions::default()
            },
            ..VerifyOptions::default()
        }
    }

    /// `q <= rst ? 0 : q + step`, a 4-bit counter with a synchronous reset.
    fn counter(step: u64) -> Design {
        let mut b = ModuleBuilder::new("ctr", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let q = b.output_reg("q", Type::bits(4));
        let (rstv, qv) = (b.net(rst), b.net(q));
        let zero = b.const_u64(4, 0);
        let one = b.const_u64(4, step);
        let next = b.add(qv, one);
        let mut p = b.process(Some("count"), ProcessKind::posedge(clk));
        let mut reset = b.block();
        reset.nonblocking(q, zero);
        let mut run_ = b.block();
        run_.nonblocking(q, next);
        p.if_(rstv, reset.finish(), run_.finish());
        b.end_process(p);
        let mut design = Design::new();
        let id = design.add_module(b.finish());
        design.top = Some(id);
        design
    }

    /// A correct run of the pipeline verifies.
    #[test]
    fn correct_synthesis_verifies() {
        let original = counter(1);
        let mut synthesised = original.clone();
        let mut diags = Diagnostics::new();
        run(&mut synthesised, &SynthOptions::default(), &mut diags);
        assert!(!diags.has_errors(), "{:?}", diags.iter().next());
        let id = synthesised.module_by_name("ctr").unwrap();
        let report = check_synthesis(&original, &synthesised, id, &options());
        assert!(report.passed(), "{}", report.render());
        assert!(report.render().starts_with("synthesis of ctr: EQUIVALENT"));
        assert!(
            check_synthesis_by_name(&original, &synthesised, "ctr", &options())
                .unwrap()
                .passed()
        );
        assert!(check_synthesis_by_name(&original, &synthesised, "nope", &options()).is_none());
    }

    /// The same, driven through the pipeline's own switch.
    #[test]
    fn pipeline_reports_the_verdict() {
        let mut design = counter(1);
        let mut diags = Diagnostics::new();
        let options = SynthOptions {
            verify_equivalence: true,
            ..SynthOptions::default()
        };
        let stats = run(&mut design, &options, &mut diags);
        assert_eq!(stats.verification.len(), 1);
        assert!(stats.verification[0].passed());
        assert!(!diags.has_errors());
        assert!(stats.render(None).contains("synthesis of ctr: EQUIVALENT"));
    }

    /// A deliberately broken pass: a wrong constant in the synthesised
    /// copy must be caught, otherwise the check proves nothing.
    #[test]
    fn injected_bug_is_caught() {
        let original = counter(1);
        let mut synthesised = original.clone();
        let mut diags = Diagnostics::new();
        run(&mut synthesised, &SynthOptions::default(), &mut diags);
        let id = synthesised.module_by_name("ctr").unwrap();

        // Pretend a pass mis-folded the increment: 1 becomes 2.
        let module = synthesised.module_mut(id);
        let mut patched = 0;
        let ids: Vec<_> = module.exprs.ids().collect();
        for e in ids {
            if module.exprs[e].kind == ExprKind::Const(Const::from_u64(1, 4)) {
                module.exprs[e].kind = ExprKind::Const(Const::from_u64(2, 4));
                patched += 1;
            }
        }
        assert_eq!(patched, 1, "the increment constant should be unique");

        let report = check_synthesis(&original, &synthesised, id, &options());
        assert!(!report.passed(), "{}", report.render());
        assert!(!report.inconclusive(), "{}", report.render());
        let text = report.render();
        assert!(text.starts_with("synthesis of ctr: DIFFERENT"), "{text}");
        assert!(text.contains("  trace:\n"), "{text}");
    }

    /// The check cannot run on a module the blaster refuses; that is a
    /// warning, not a false pass.
    #[test]
    fn unblastable_module_is_inconclusive() {
        let mut b = ModuleBuilder::new("io", span());
        b.inout("pad", Type::bit());
        let mut original = Design::new();
        original.add_module(b.finish());
        let synthesised = original.clone();
        let id = synthesised.module_by_name("io").unwrap();
        let report = check_synthesis(&original, &synthesised, id, &options());
        assert!(!report.passed());
        assert!(report.inconclusive());
        assert!(report.diags.has_errors());
        assert_eq!(report.render(), "synthesis of io: INCONCLUSIVE\n");
    }
}
