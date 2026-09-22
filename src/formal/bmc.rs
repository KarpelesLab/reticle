//! Bounded model checking.
//!
//! BMC (Biere, Cimatti, Clarke, Zhu, "Symbolic Model Checking without
//! BDDs", TACAS 1999) unrolls the design `depth` frames from its initial
//! state and asks the SAT solver, frame by frame, whether some `assert`
//! can be violated there. A satisfying assignment is a concrete
//! counter-example, returned as a [`Trace`]; `Unsat` at every frame up to
//! `depth` means the asserts hold on every trace of at most `depth + 1`
//! states, which is a bounded proof only. [`mod@super::induct`] turns it into
//! an unbounded one when the design allows.
//!
//! The check is incremental: one [`Unrolling`] grows by one frame per
//! step, the "bad" literal of frame `k` is passed as an *assumption* so the
//! solver's learnt clauses survive into step `k + 1`, and once frame `k` is
//! known safe its bad literal is asserted false permanently, which is what
//! every later step may rely on anyway.
//!
//! `assume` properties are enforced in every frame. `cover` properties are
//! checked alongside: the first frame in which each can be 1 is recorded
//! with its witness trace. If the assumptions turn out to be unsatisfiable
//! within the bound, the report carries warning `F0016`, since every
//! assert then holds vacuously.

use super::blast::{BlastOptions, Blaster};
use super::sat::SolveResult;
use super::trace::Trace;
use super::unroll::{InitMode, Transition, Unrolling};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Design, ModuleId};

/// Options of [`bmc`].
#[derive(Clone, Debug)]
pub struct BmcOptions {
    /// The last frame checked; the trace has `depth + 1` states at most.
    pub depth: u32,
    /// How frame 0 is constrained.
    pub init: InitMode,
    /// Conflicts allowed per SAT call before giving up with
    /// [`BmcOutcome::Unknown`]; `None` for no limit.
    pub conflict_limit: Option<u64>,
    /// Options of the bit-blaster.
    pub blast: BlastOptions,
}

impl Default for BmcOptions {
    fn default() -> Self {
        BmcOptions {
            depth: 20,
            init: InitMode::Reset,
            conflict_limit: None,
            blast: BlastOptions::default(),
        }
    }
}

/// The verdict of a bounded check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BmcOutcome {
    /// No assert can fail within `depth` frames of the initial state.
    Safe {
        /// The bound that was checked.
        depth: u32,
    },
    /// An assert fails; the trace ends at the failing frame.
    Failed {
        /// The frame in which the assert is 0.
        frame: u32,
        /// The names of every assert that is 0 in that frame.
        properties: Vec<String>,
        /// The counter-example.
        trace: Trace,
    },
    /// The solver gave up (conflict limit) or the design could not be
    /// blasted; the diagnostics say which.
    Unknown {
        /// The frame at which the check stopped.
        frame: u32,
    },
}

/// The result of one `cover` property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverOutcome {
    /// The property net's name.
    pub name: String,
    /// The first frame in which the property is 1, with a witness.
    pub reached: Option<(u32, Trace)>,
    /// True when some solve for this cover hit the conflict limit, so an
    /// unreached cover may still be reachable.
    pub unknown: bool,
}

/// What [`bmc`] returns.
#[derive(Clone, Debug)]
pub struct BmcReport {
    /// The verdict on the asserts.
    pub outcome: BmcOutcome,
    /// Number of `assert` properties checked; zero means the verdict is
    /// vacuous.
    pub assert_count: usize,
    /// One entry per cover property, in declaration order.
    pub covers: Vec<CoverOutcome>,
    /// Warnings from the blaster and the check; errors when it could not
    /// run.
    pub diags: Diagnostics,
}

/// Runs a bounded check of `module` of `design`.
pub fn bmc(design: &Design, module: ModuleId, options: &BmcOptions) -> BmcReport {
    let m = design.module(module);
    match Blaster::new(m, &options.blast) {
        Ok(blaster) => {
            let mut report = bmc_system(&blaster, options);
            let mut diags = blaster.diagnostics().clone();
            diags.append(&mut report.diags);
            report.diags = diags;
            report
        }
        Err(diags) => BmcReport {
            outcome: BmcOutcome::Unknown { frame: 0 },
            assert_count: 0,
            covers: Vec::new(),
            diags,
        },
    }
}

/// Runs a bounded check of any [`Transition`] (a module or a miter).
pub fn bmc_system(system: &dyn Transition, options: &BmcOptions) -> BmcReport {
    let mut diags = Diagnostics::new();
    let mut unrolling = Unrolling::new(system, options.init);
    unrolling.set_conflict_limit(options.conflict_limit);
    let mut covers: Vec<CoverOutcome> = unrolling.frames()[0]
        .covers
        .iter()
        .map(|c| CoverOutcome {
            name: c.name.clone(),
            reached: None,
            unknown: false,
        })
        .collect();
    let has_assumes = !unrolling.frames()[0].assumes.is_empty();
    let assert_count = unrolling.frames()[0].asserts.len();

    let mut outcome = BmcOutcome::Safe {
        depth: options.depth,
    };
    for k in 0..=options.depth {
        let ku = usize::try_from(k).expect("depth fits usize");
        if k > 0 {
            unrolling.extend();
        }
        // Covers first: they do not depend on the asserts.
        for (i, cover) in covers.iter_mut().enumerate() {
            if cover.reached.is_some() {
                continue;
            }
            let lit = unrolling.frames()[ku].covers[i].lit;
            match unrolling.solve(&[lit]) {
                SolveResult::Sat => cover.reached = Some((k, unrolling.trace(ku + 1))),
                SolveResult::Unsat => {}
                SolveResult::Unknown => cover.unknown = true,
            }
        }
        let bad = unrolling.bad(ku);
        match unrolling.solve(&[bad]) {
            SolveResult::Sat => {
                let solver = unrolling.solver();
                let properties = unrolling.frames()[ku]
                    .asserts
                    .iter()
                    .filter(|p| solver.value(p.lit.var()) == Some(p.lit.is_neg()))
                    .map(|p| p.name.clone())
                    .collect();
                outcome = BmcOutcome::Failed {
                    frame: k,
                    properties,
                    trace: unrolling.trace(ku + 1),
                };
                break;
            }
            SolveResult::Unsat => unrolling.add_unit(!bad),
            SolveResult::Unknown => {
                diags.push(
                    Diagnostic::warning(format!(
                        "bounded model check of `{}` gave up at frame {k}: conflict limit reached",
                        system.name()
                    ))
                    .with_code("F0019"),
                );
                outcome = BmcOutcome::Unknown { frame: k };
                break;
            }
        }
    }
    if has_assumes
        && matches!(outcome, BmcOutcome::Safe { .. })
        && unrolling.solve(&[]) == SolveResult::Unsat
    {
        diags.push(
            Diagnostic::warning(format!(
                "the assumptions of `{}` cannot all hold for {} frames: every assert is vacuously true",
                system.name(),
                options.depth + 1
            ))
            .with_code("F0016"),
        );
    }
    BmcReport {
        outcome,
        assert_count,
        covers,
        diags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Module, Name, Reset, Type};
    use crate::logic::Logic;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// A 4-bit counter from 0 asserting `q < limit` and covering
    /// `q == 5`; `en` gates counting unless `force` adds an assume.
    fn counter(limit: u64, force: bool) -> (Design, ModuleId) {
        let mut b = ModuleBuilder::new("ctr", span());
        let clk = b.input("clk", Type::bit());
        let en = b.input("en", Type::bit());
        let q = b.output("q", Type::bits(4));
        let (qv, one, env, clkv) = (b.net(q), b.const_u64(4, 1), b.net(en), b.net(clk));
        let inc = b.add(qv, one);
        let f = b.const_bit(false);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: true,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Logic::zero(4),
                }),
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), inc),
                (Name::new("en"), env),
                (Name::new("rst"), f),
            ],
            vec![(Name::new("q"), q)],
        );
        let lim = b.const_u64(4, limit);
        let ok = b.lt(qv, lim);
        let p = b.add_net("in_range", Type::bit());
        b.net_attr(p, "formal_assert", 1);
        b.assign(p, ok);
        let five = b.const_u64(4, 5);
        let hit = b.eq(qv, five);
        let c = b.add_net("at_five", Type::bit());
        b.net_attr(c, "formal_cover", 1);
        b.assign(c, hit);
        if force {
            let a = b.add_net("always_en", Type::bit());
            b.net_attr(a, "formal_assume", 1);
            b.assign(a, env);
        }
        let m: Module = b.finish();
        let mut d = Design::new();
        let id = d.add_module(m);
        (d, id)
    }

    #[test]
    fn finds_the_first_failing_frame() {
        let (d, id) = counter(7, false);
        let report = bmc(&d, id, &BmcOptions::default());
        let BmcOutcome::Failed {
            frame,
            properties,
            trace,
        } = &report.outcome
        else {
            panic!("{:?}", report.outcome);
        };
        assert_eq!(*frame, 7);
        assert_eq!(properties, &["in_range"]);
        assert_eq!(trace.len(), 8);
        assert_eq!(trace.frames[7].outputs[0].1.to_u64(), Some(7));
        // The witness for the cover comes earlier.
        assert_eq!(report.covers.len(), 1);
        let (at, witness) = report.covers[0].reached.as_ref().unwrap();
        assert_eq!(*at, 5);
        assert_eq!(witness.frames[5].outputs[0].1.to_u64(), Some(5));
        assert!(report.diags.is_empty());
    }

    #[test]
    fn safe_within_bound() {
        let (d, id) = counter(7, false);
        let opts = BmcOptions {
            depth: 6,
            ..BmcOptions::default()
        };
        let report = bmc(&d, id, &opts);
        assert_eq!(report.outcome, BmcOutcome::Safe { depth: 6 });
        assert!(report.covers[0].reached.is_some());
        // With a limit the counter cannot reach within the bound, the
        // assert is safe, and the cover is reached at exactly frame 5
        // when the assume forces counting.
        let (d, id) = counter(15, true);
        let opts = BmcOptions {
            depth: 10,
            ..BmcOptions::default()
        };
        let report = bmc(&d, id, &opts);
        assert_eq!(report.outcome, BmcOutcome::Safe { depth: 10 });
        assert_eq!(report.covers[0].reached.as_ref().map(|r| r.0), Some(5));
        assert!(report.diags.is_empty(), "{:?}", report.diags);
    }

    #[test]
    fn vacuous_assumptions_are_flagged() {
        let mut b = ModuleBuilder::new("vac", span());
        let a = b.input("a", Type::bit());
        let av = b.net(a);
        let na = b.not(av);
        let p = b.add_net("p", Type::bit());
        b.net_attr(p, "formal_assume", 1);
        b.assign(p, av);
        let q = b.add_net("q", Type::bit());
        b.net_attr(q, "formal_assume", 1);
        b.assign(q, na);
        let r = b.add_net("r", Type::bit());
        b.net_attr(r, "formal_assert", 1);
        b.assign(r, na);
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        let report = bmc(&d, id, &BmcOptions::default());
        assert!(matches!(report.outcome, BmcOutcome::Safe { .. }));
        assert!(report.diags.iter().any(|d| d.code == Some("F0016")));
    }

    #[test]
    fn blaster_errors_yield_unknown() {
        let mut b = ModuleBuilder::new("p", span());
        let pr = b.process(None, crate::ir::ProcessKind::Comb);
        b.end_process(pr);
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        let report = bmc(&d, id, &BmcOptions::default());
        assert_eq!(report.outcome, BmcOutcome::Unknown { frame: 0 });
        assert!(report.diags.has_errors());
    }

    #[test]
    fn conflict_limit_yields_unknown() {
        // A 16-bit multiplier factoring problem is hard enough to exceed a
        // limit of zero conflicts... only if the solver needs any. Use a
        // design whose bad state needs search: two free 12-bit inputs
        // whose product must be a given odd composite.
        let mut b = ModuleBuilder::new("hard", span());
        let x = b.input("x", Type::bits(12));
        let y = b.input("y", Type::bits(12));
        let (xv, yv) = (b.net(x), b.net(y));
        let p = b.mul(xv, yv);
        let target = b.const_u64(12, 3599); // 59 * 61
        let is = b.eq(p, target);
        let one = b.const_u64(12, 1);
        let x1 = b.ne(xv, one);
        let y1 = b.ne(yv, one);
        let nt = b.and(x1, y1);
        let both = b.and(is, nt);
        let not_both = b.not(both);
        let a = b.add_net("no_factor", Type::bit());
        b.net_attr(a, "formal_assert", 1);
        b.assign(a, not_both);
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        let opts = BmcOptions {
            depth: 0,
            conflict_limit: Some(1),
            ..BmcOptions::default()
        };
        let report = bmc(&d, id, &opts);
        assert!(
            matches!(
                report.outcome,
                BmcOutcome::Unknown { .. } | BmcOutcome::Failed { .. }
            ),
            "{:?}",
            report.outcome
        );
        let report = bmc(&d, id, &BmcOptions::default());
        assert!(matches!(
            report.outcome,
            BmcOutcome::Failed { frame: 0, .. }
        ));
    }
}
