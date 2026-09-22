//! k-induction: unbounded proofs from bounded checks.
//!
//! Sheeran, Singh and Stålmarck ("Checking Safety Properties Using
//! Induction and a SAT-Solver", FMCAD 2000) observed that a property `P`
//! holds in every reachable state if
//!
//! 1. **base case**: `P` holds in the first `k` states of every trace from
//!    the initial state (a BMC run to depth `k - 1`), and
//! 2. **inductive step**: any `k + 1` consecutive states, starting
//!    *anywhere*, whose first `k` satisfy `P` also satisfy `P` in the last
//!    one: `P(s0) ∧ T(s0,s1) ∧ … ∧ P(sk-1) ∧ T(sk-1,sk) ⇒ P(sk)`.
//!
//! The step is checked by asking the solver for a trace with `P` in frames
//! `0..k` and `¬P` in frame `k`, with a *free* initial state. `Unsat` means
//! `P` is `k`-inductive and therefore an invariant; `Sat` gives a
//! "counter-example to induction" that may start from an unreachable
//! state, so `k` is increased and the search continues. Both unrollings
//! grow incrementally.
//!
//! With [`InductOptions::unique_states`] the step also requires the `k + 1`
//! states to be pairwise distinct (the "simple path" constraint). Any
//! unreachable-state counter-example to induction must then lie on a
//! loop-free path, of which a finite-state system has only finitely many,
//! so some `k` proves every true property: the method becomes complete,
//! at the price of `O(k²)` extra clauses over the state bits.
//!
//! `assume` properties are enforced in every frame of both cases.

use super::blast::{BlastOptions, Blaster};
use super::sat::SolveResult;
use super::trace::Trace;
use super::unroll::{InitMode, Transition, Unrolling};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Design, ModuleId};

/// Options of [`induct`].
#[derive(Clone, Debug)]
pub struct InductOptions {
    /// The largest `k` tried.
    pub max_k: u32,
    /// How frame 0 of the base case is constrained.
    pub init: InitMode,
    /// Add the simple-path constraint to the inductive step.
    pub unique_states: bool,
    /// Conflicts allowed per SAT call before giving up; `None` for no
    /// limit.
    pub conflict_limit: Option<u64>,
    /// Options of the bit-blaster.
    pub blast: BlastOptions,
}

impl Default for InductOptions {
    fn default() -> Self {
        InductOptions {
            max_k: 10,
            init: InitMode::Reset,
            unique_states: true,
            conflict_limit: None,
            blast: BlastOptions::default(),
        }
    }
}

/// The verdict of a k-induction run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InductOutcome {
    /// Every assert holds in every reachable state; it is `k`-inductive.
    Proved {
        /// The induction depth that succeeded.
        k: u32,
    },
    /// The base case found a real counter-example.
    Failed {
        /// The frame in which the assert is 0.
        frame: u32,
        /// The names of every assert that is 0 in that frame.
        properties: Vec<String>,
        /// The counter-example.
        trace: Trace,
    },
    /// Neither proved nor refuted up to `k`: the property may be true but
    /// not inductive at this depth, or a solve hit its limit.
    Unknown {
        /// The last depth tried.
        k: u32,
    },
}

/// What [`induct`] returns.
#[derive(Clone, Debug)]
pub struct InductReport {
    /// The verdict.
    pub outcome: InductOutcome,
    /// Warnings, and errors when the design could not be blasted.
    pub diags: Diagnostics,
}

/// Runs k-induction on `module` of `design`.
pub fn induct(design: &Design, module: ModuleId, options: &InductOptions) -> InductReport {
    let m = design.module(module);
    match Blaster::new(m, &options.blast) {
        Ok(blaster) => {
            let mut report = induct_system(&blaster, options);
            let mut diags = blaster.diagnostics().clone();
            diags.append(&mut report.diags);
            report.diags = diags;
            report
        }
        Err(diags) => InductReport {
            outcome: InductOutcome::Unknown { k: 0 },
            diags,
        },
    }
}

/// Runs k-induction on any [`Transition`] (a module or a miter).
pub fn induct_system(system: &dyn Transition, options: &InductOptions) -> InductReport {
    let mut diags = Diagnostics::new();
    let mut base = Unrolling::new(system, options.init);
    let mut step = Unrolling::new(system, InitMode::Free);
    base.set_conflict_limit(options.conflict_limit);
    step.set_conflict_limit(options.conflict_limit);
    let give_up = |diags: &mut Diagnostics, what: &str, k: u32| {
        diags.push(
            Diagnostic::warning(format!(
                "k-induction of `{}` gave up in the {what} at k = {k}: conflict limit reached",
                system.name()
            ))
            .with_code("F0019"),
        );
    };

    for k in 0..=options.max_k {
        let ku = usize::try_from(k).expect("k fits usize");
        // Base case: the asserts hold in frame k from the initial state.
        if k > 0 {
            base.extend();
        }
        let bad = base.bad(ku);
        match base.solve(&[bad]) {
            SolveResult::Sat => {
                let solver = base.solver();
                let properties = base.frames()[ku]
                    .asserts
                    .iter()
                    .filter(|p| solver.value(p.lit.var()) == Some(p.lit.is_neg()))
                    .map(|p| p.name.clone())
                    .collect();
                return InductReport {
                    outcome: InductOutcome::Failed {
                        frame: k,
                        properties,
                        trace: base.trace(ku + 1),
                    },
                    diags,
                };
            }
            SolveResult::Unsat => base.add_unit(!bad),
            SolveResult::Unknown => {
                give_up(&mut diags, "base case", k);
                return InductReport {
                    outcome: InductOutcome::Unknown { k },
                    diags,
                };
            }
        }
        // Inductive step: asserts hold in frames 0..k, fail in frame k.
        if k > 0 {
            let prev = step.bad(ku - 1);
            step.add_unit(!prev);
            step.extend();
            if options.unique_states {
                for j in 0..ku {
                    step.add_distinct(j, ku);
                }
            }
        }
        let bad = step.bad(ku);
        match step.solve(&[bad]) {
            SolveResult::Unsat => {
                return InductReport {
                    outcome: InductOutcome::Proved { k },
                    diags,
                };
            }
            SolveResult::Sat => {}
            SolveResult::Unknown => {
                give_up(&mut diags, "inductive step", k);
                return InductReport {
                    outcome: InductOutcome::Unknown { k },
                    diags,
                };
            }
        }
    }
    InductReport {
        outcome: InductOutcome::Unknown { k: options.max_k },
        diags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Name, Reset, Type};
    use crate::logic::Logic;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// A 4-bit counter that wraps at `wrap` (back to 0), asserting
    /// `q <= bound`.
    fn wrapping_counter(wrap: u64, bound: u64) -> (Design, ModuleId) {
        let mut b = ModuleBuilder::new("wrap", span());
        let clk = b.input("clk", Type::bit());
        let q = b.output("q", Type::bits(4));
        let (qv, one, clkv) = (b.net(q), b.const_u64(4, 1), b.net(clk));
        let inc = b.add(qv, one);
        let top = b.const_u64(4, wrap);
        let at_top = b.eq(qv, top);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Logic::zero(4),
                }),
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), inc),
                (Name::new("rst"), at_top),
            ],
            vec![(Name::new("q"), q)],
        );
        let lim = b.const_u64(4, bound);
        let ok = b.le(qv, lim);
        let p = b.add_net("bounded", Type::bit());
        b.net_attr(p, "formal_assert", 1);
        b.assign(p, ok);
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        (d, id)
    }

    #[test]
    fn proves_an_inductive_invariant() {
        // q <= 9 with wrap at 9 is 1-inductive: from q <= 9, the next
        // value is q + 1 <= 9 or 0.
        let (d, id) = wrapping_counter(9, 9);
        let report = induct(&d, id, &InductOptions::default());
        assert_eq!(report.outcome, InductOutcome::Proved { k: 1 });
        assert!(report.diags.is_empty());
    }

    #[test]
    fn needs_unique_states_for_a_weaker_bound() {
        // q <= 12 is true (q never exceeds 9) but not 1-inductive: from
        // the unreachable q = 12 the successor is 13. The only bad path
        // with the property holding before the last frame is
        // 10 -> 11 -> 12 -> 13, and 10 has no predecessor (9 wraps to 0),
        // so the step succeeds exactly at k = 4.
        let (d, id) = wrapping_counter(9, 12);
        let opts = InductOptions {
            max_k: 2,
            ..InductOptions::default()
        };
        let report = induct(&d, id, &opts);
        assert_eq!(report.outcome, InductOutcome::Unknown { k: 2 });
        let opts = InductOptions {
            max_k: 6,
            ..InductOptions::default()
        };
        let report = induct(&d, id, &opts);
        assert_eq!(report.outcome, InductOutcome::Proved { k: 4 });
    }

    #[test]
    fn base_case_failure_is_a_counter_example() {
        let (d, id) = wrapping_counter(9, 5);
        let report = induct(&d, id, &InductOptions::default());
        let InductOutcome::Failed {
            frame,
            properties,
            trace,
        } = report.outcome
        else {
            panic!("{:?}", report.outcome);
        };
        assert_eq!(frame, 6);
        assert_eq!(properties, ["bounded"]);
        assert_eq!(trace.frames[6].outputs[0].1.to_u64(), Some(6));
    }

    #[test]
    fn combinational_designs_prove_at_k_zero() {
        let mut b = ModuleBuilder::new("comb", span());
        let a = b.input("a", Type::bits(3));
        let av = b.net(a);
        let ra = b.reduce_or(av);
        let zero = b.const_u64(3, 0);
        let is_zero = b.eq(av, zero);
        let either = b.or(ra, is_zero);
        let p = b.add_net("tautology", Type::bit());
        b.net_attr(p, "formal_assert", 1);
        b.assign(p, either);
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        let report = induct(&d, id, &InductOptions::default());
        assert_eq!(report.outcome, InductOutcome::Proved { k: 0 });
        // Blaster errors propagate as Unknown with diagnostics.
        let mut b = ModuleBuilder::new("bad", span());
        b.inout("io", Type::bit());
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        let report = induct(&d, id, &InductOptions::default());
        assert_eq!(report.outcome, InductOutcome::Unknown { k: 0 });
        assert!(report.diags.has_errors());
    }
}
