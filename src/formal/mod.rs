//! Formal verification: SAT solving, and the engines built on it.
//!
//! Phase 7 of `ROADMAP.md`. Everything here reduces a question about a
//! design to propositional satisfiability and asks the in-crate solver:
//!
//! - [`sat`]: a CDCL SAT solver (two watched literals, first-UIP learning
//!   with clause minimisation, VSIDS, phase saving, Luby or Glucose
//!   restarts, LBD-based learnt clause reduction, incremental solving with
//!   assumptions and conflict cores), plus DIMACS I/O.
//! - [`cnf`]: a Tseitin encoder that turns gate-level logic into clauses
//!   with structural hashing, so the same sub-circuit is encoded once.
//! - [`blast`]: the bit-blaster from the IR's cell form to CNF, one
//!   [`Lit`] per net bit per frame, with the 2-state reading of `x`
//!   documented there.
//! - [`unroll`]: the transition relation unrolled over frames, with the
//!   initial-state policy ([`InitMode`]) and an incremental solver.
//! - [`mod@bmc`]: bounded model checking of `assert` / `assume` / `cover`
//!   properties (Biere et al. 1999), with counter-example traces.
//! - [`mod@induct`]: k-induction for unbounded proofs (Sheeran, Singh,
//!   Stålmarck 2000), with the simple-path strengthening.
//! - [`equiv`]: combinational and sequential equivalence checking of two
//!   modules through a miter, the self-check of the synthesis flow.
//! - [`trace`]: counter-example traces rendered as text or VCD.
//! - [`reach`]: reachability-based lint (constant selects, dead enables,
//!   dead FSM states).
//!
//! [`verify`] is the one-call entry point: BMC, then k-induction, with a
//! [`VerifyReport`] that renders as text.
//!
//! # What the engines accept
//!
//! The engines work on synthesised designs: cells and continuous assigns,
//! no processes and no hierarchy (flatten first, or mark instances
//! `formal_free`). Properties are 1-bit nets carrying `formal_assert`,
//! `formal_assume` or `formal_cover` attributes, or an explicit list in
//! [`BlastOptions::properties`]. See [`blast`] for the full list of what is
//! modelled and how.
//!
//! # Diagnostics
//!
//! | Code    | Meaning                                                            |
//! |---------|--------------------------------------------------------------------|
//! | `F0001` | The module still has processes: run synthesis first                |
//! | `F0002` | A net, memory or expression is not a bit vector (or is a `Call`)   |
//! | `F0003` | An `inout` port                                                     |
//! | `F0004` | A `Dlatch` cell                                                     |
//! | `F0005` | A `Tristate` cell                                                   |
//! | `F0006` | A black box, instance or black-box module without `formal_free`    |
//! | `F0007` | An unclocked (level-sensitive) memory write port                   |
//! | `F0008` | A memory above [`BlastOptions::max_memory_bits`]                    |
//! | `F0009` | A constant with `z` bits outside a wildcard pattern                |
//! | `F0010` | Flip-flops on more than one clock (warning; checked as one domain) |
//! | `F0011` | An asynchronous reset (warning; checked as synchronous)            |
//! | `F0012` | A memory write in a continuous assignment                          |
//! | `F0013` | A combinational loop                                               |
//! | `F0014` | A property net that is not one bit wide, or does not exist         |
//! | `F0015` | An undriven net treated as a free input (warning)                  |
//! | `F0016` | The assumptions are unsatisfiable: the check is vacuous (warning)  |
//! | `F0017` | Port mismatch between the two modules of an equivalence check      |
//! | `F0018` | A state element without a reset value in a sequential equivalence check |
//! | `F0019` | A SAT call hit its conflict limit (warning)                        |
//! | `F0021` | Lint: a mux select or `?:` condition is constant within the bound  |
//! | `F0022` | Lint: a flip-flop enable is constant within the bound              |
//! | `F0023` | Lint: an FSM state value is unreachable within the bound           |

pub mod blast;
pub mod bmc;
pub mod cnf;
pub mod equiv;
pub mod induct;
pub mod reach;
pub mod sat;
pub mod trace;
pub mod unroll;

use std::fmt::Write;

pub use blast::{BlastOptions, BlastedFrame, Blaster, PropertyKind};
pub use bmc::{BmcOptions, BmcOutcome, BmcReport, CoverOutcome, bmc};
pub use cnf::CnfBuilder;
pub use equiv::{EquivOptions, EquivOutcome, EquivProof, EquivReport, check_equivalent};
pub use induct::{InductOptions, InductOutcome, InductReport, induct};
pub use reach::{ReachOptions, ReachReport, reach_lint};
pub use sat::{Lit, SolveResult, Solver, Var};
pub use trace::{Trace, TraceFrame};
pub use unroll::{InitMode, Transition, Unrolling};

use crate::diag::Diagnostics;
use crate::ir::{Design, ModuleId};

/// Options of [`verify`].
#[derive(Clone, Debug)]
pub struct VerifyOptions {
    /// Frames of bounded model checking.
    pub depth: u32,
    /// Largest induction depth tried after a safe bounded check; `0`
    /// skips induction.
    pub max_k: u32,
    /// How the initial state is constrained.
    pub init: InitMode,
    /// Conflicts allowed per SAT call; `None` for no limit.
    pub conflict_limit: Option<u64>,
    /// Options of the bit-blaster.
    pub blast: BlastOptions,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        VerifyOptions {
            depth: 20,
            max_k: 10,
            init: InitMode::Reset,
            conflict_limit: None,
            blast: BlastOptions::default(),
        }
    }
}

/// The verdict of [`verify`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every assert holds in every reachable state (`k`-inductive).
    Proved {
        /// The induction depth.
        k: u32,
    },
    /// Every assert holds for `depth + 1` frames; no unbounded proof.
    BoundedSafe {
        /// The checked depth.
        depth: u32,
    },
    /// An assert fails; the report carries the trace.
    Failed {
        /// The failing frame.
        frame: u32,
        /// The asserts that are 0 there.
        properties: Vec<String>,
    },
    /// The check could not run or gave up; see the diagnostics.
    Unknown,
}

/// What [`verify`] returns.
#[derive(Clone, Debug)]
pub struct VerifyReport {
    /// The module checked.
    pub module: String,
    /// The verdict on the asserts.
    pub verdict: Verdict,
    /// Number of `assert` properties; with zero, a passing verdict is
    /// vacuous and renders as such.
    pub assert_count: usize,
    /// The counter-example when the verdict is [`Verdict::Failed`].
    pub trace: Option<Trace>,
    /// The cover properties and their witnesses.
    pub covers: Vec<CoverOutcome>,
    /// Warnings and errors.
    pub diags: Diagnostics,
}

impl VerifyReport {
    /// True when the verdict is a proof or a bounded pass.
    pub fn passed(&self) -> bool {
        matches!(
            self.verdict,
            Verdict::Proved { .. } | Verdict::BoundedSafe { .. }
        )
    }

    /// Renders the report as text: the verdict, each cover, and the
    /// counter-example trace if any. Diagnostics are not included; render
    /// them with [`Diagnostics::render`].
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = write!(out, "module {}: ", self.module);
        match &self.verdict {
            Verdict::Proved { .. } | Verdict::BoundedSafe { .. } if self.assert_count == 0 => {
                let _ = writeln!(out, "NO ASSERTS (nothing to prove)");
            }
            Verdict::Proved { k } => {
                let _ = writeln!(out, "PROVED (every assert is {k}-inductive)");
            }
            Verdict::BoundedSafe { depth } => {
                let _ = writeln!(out, "SAFE to depth {depth} (no inductive proof found)");
            }
            Verdict::Failed { frame, properties } => {
                let list: Vec<String> = properties.iter().map(|p| format!("`{p}`")).collect();
                let _ = writeln!(
                    out,
                    "FAIL: assert {} violated at frame {frame}",
                    list.join(", ")
                );
            }
            Verdict::Unknown => {
                let _ = writeln!(out, "UNKNOWN");
            }
        }
        for c in &self.covers {
            match &c.reached {
                Some((frame, _)) => {
                    let _ = writeln!(out, "  cover `{}`: reached at frame {frame}", c.name);
                }
                None if c.unknown => {
                    let _ = writeln!(out, "  cover `{}`: undecided", c.name);
                }
                None => {
                    let _ = writeln!(out, "  cover `{}`: not reached", c.name);
                }
            }
        }
        if let Some(trace) = &self.trace {
            out.push_str("  trace:\n");
            for line in trace.render().lines() {
                let _ = writeln!(out, "    {line}");
            }
        }
        for c in &self.covers {
            if let Some((_, trace)) = &c.reached {
                let _ = writeln!(out, "  witness for `{}`:", c.name);
                for line in trace.render().lines() {
                    let _ = writeln!(out, "    {line}");
                }
            }
        }
        out
    }
}

/// Checks `module` of `design`: bounded model checking to
/// [`VerifyOptions::depth`], then k-induction up to [`VerifyOptions::max_k`]
/// when the bounded check passes.
pub fn verify(design: &Design, module: ModuleId, options: &VerifyOptions) -> VerifyReport {
    let name = design.module(module).name.to_string();
    let bmc_opts = BmcOptions {
        depth: options.depth,
        init: options.init,
        conflict_limit: options.conflict_limit,
        blast: options.blast.clone(),
    };
    let b = bmc(design, module, &bmc_opts);
    let assert_count = b.assert_count;
    let mut diags = b.diags;
    let (verdict, trace) = match b.outcome {
        BmcOutcome::Failed {
            frame,
            properties,
            trace,
        } => (Verdict::Failed { frame, properties }, Some(trace)),
        BmcOutcome::Unknown { .. } => (Verdict::Unknown, None),
        BmcOutcome::Safe { depth } => {
            if options.max_k == 0 || diags.has_errors() {
                (Verdict::BoundedSafe { depth }, None)
            } else {
                let ind_opts = InductOptions {
                    max_k: options.max_k,
                    init: options.init,
                    unique_states: true,
                    conflict_limit: options.conflict_limit,
                    blast: options.blast.clone(),
                };
                let mut i = induct(design, module, &ind_opts);
                // The blaster's warnings were already collected by bmc.
                let extra: Vec<_> = i
                    .diags
                    .iter()
                    .filter(|d| d.code == Some("F0019"))
                    .cloned()
                    .collect();
                diags.extend(extra);
                i.diags = Diagnostics::new();
                match i.outcome {
                    InductOutcome::Proved { k } => (Verdict::Proved { k }, None),
                    InductOutcome::Failed {
                        frame,
                        properties,
                        trace,
                    } => (Verdict::Failed { frame, properties }, Some(trace)),
                    InductOutcome::Unknown { .. } => (Verdict::BoundedSafe { depth }, None),
                }
            }
        }
    };
    VerifyReport {
        module: name,
        verdict,
        assert_count,
        trace,
        covers: b.covers,
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

    /// A 3-bit counter wrapping at `wrap`, asserting `q <= bound` and
    /// covering `q == 2`.
    fn counter(wrap: u64, bound: u64) -> (Design, ModuleId) {
        let mut b = ModuleBuilder::new("ctr", span());
        let clk = b.input("clk", Type::bit());
        let q = b.output("q", Type::bits(3));
        let (qv, one, clkv) = (b.net(q), b.const_u64(3, 1), b.net(clk));
        let inc = b.add(qv, one);
        let top = b.const_u64(3, wrap);
        let at_top = b.eq(qv, top);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Logic::zero(3),
                }),
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), inc),
                (Name::new("rst"), at_top),
            ],
            vec![(Name::new("q"), q)],
        );
        let lim = b.const_u64(3, bound);
        let ok = b.le(qv, lim);
        let p = b.add_net("bounded", Type::bit());
        b.net_attr(p, "formal_assert", 1);
        b.assign(p, ok);
        let two = b.const_u64(3, 2);
        let hit = b.eq(qv, two);
        let c = b.add_net("at_two", Type::bit());
        b.net_attr(c, "formal_cover", 1);
        b.assign(c, hit);
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        (d, id)
    }

    #[test]
    fn verify_proves_and_renders() {
        let (d, id) = counter(5, 5);
        let r = verify(&d, id, &VerifyOptions::default());
        assert!(r.passed());
        assert_eq!(r.verdict, Verdict::Proved { k: 1 });
        assert_eq!(
            r.render(),
            "module ctr: PROVED (every assert is 1-inductive)\n  cover `at_two`: reached at frame 2\n  witness for `at_two`:\n    frame | clk |   q |   q\n    ------+-----+-----+----\n        0 |   0 | 000 | 000\n        1 |   0 | 001 | 001\n        2 |   0 | 010 | 010\n"
        );
    }

    #[test]
    fn verify_fails_with_trace() {
        let (d, id) = counter(5, 3);
        let r = verify(&d, id, &VerifyOptions::default());
        assert!(!r.passed());
        assert_eq!(
            r.verdict,
            Verdict::Failed {
                frame: 4,
                properties: vec!["bounded".into()]
            }
        );
        let text = r.render();
        assert!(text.starts_with("module ctr: FAIL: assert `bounded` violated at frame 4\n"));
        assert!(text.contains("  trace:\n"));
        assert_eq!(r.trace.as_ref().map(Trace::len), Some(5));
    }

    #[test]
    fn verify_bounded_only_and_unknown() {
        // q <= 6 with wrap at 5: true, 2-inductive at most... proven with
        // max_k, bounded-safe with induction disabled.
        let (d, id) = counter(5, 6);
        let opts = VerifyOptions {
            max_k: 0,
            depth: 8,
            ..VerifyOptions::default()
        };
        let r = verify(&d, id, &opts);
        assert_eq!(r.verdict, Verdict::BoundedSafe { depth: 8 });
        assert!(r.render().starts_with("module ctr: SAFE to depth 8"));
        let r = verify(&d, id, &VerifyOptions::default());
        assert!(matches!(r.verdict, Verdict::Proved { .. }));

        let mut b = ModuleBuilder::new("bad", span());
        b.inout("io", Type::bit());
        let mut d = Design::new();
        let id = d.add_module(b.finish());
        let r = verify(&d, id, &VerifyOptions::default());
        assert_eq!(r.verdict, Verdict::Unknown);
        assert_eq!(r.render(), "module bad: UNKNOWN\n");
        assert!(r.diags.has_errors());
    }
}
