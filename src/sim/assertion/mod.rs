//! Concurrent assertions: a subset of SVA and PSL checked during a run.
//!
//! Immediate assertions (`assert (expr)`, VHDL `assert ... report`) are
//! ordinary statements and run in `process.rs` through
//! [`StmtKind::Assert`](crate::ir::StmtKind::Assert). *Concurrent*
//! assertions are what this module adds: a property over a clocking event,
//! checked on every tick of that clock, with an attempt started in every
//! cycle and all live attempts advanced in lockstep.
//!
//! # Using it
//!
//! ```
//! use reticle::ir::Design;
//! use reticle::sim::{SimOptions, Simulator};
//! use reticle::source::{SourceMap, Span};
//!
//! let text = "\
//! module m
//!   net %clk u1 reg
//!   net %full u1 reg
//!   net %push u1 reg
//!   process clkgen free
//!     wait for 8'd5
//!     %clk = not(%clk)
//!   end
//!   process stim initial
//!     %clk = 1'd0
//!     %full = 1'd1
//!     %push = 1'd0
//!     wait for 8'd40
//!     finish
//!   end
//! end
//! ";
//! let mut map = SourceMap::new();
//! let file = map.add("m.rtl", text).unwrap();
//! let design = Design::parse_text(text, file).unwrap();
//! let mut sim = Simulator::new(&design, SimOptions::default()).unwrap();
//! let id = sim
//!     .add_assertion_text(
//!         "no_push_when_full: assert property (@(posedge clk) full |-> !push);",
//!         Span::new(file, 0, 0),
//!     )
//!     .unwrap();
//! sim.run();
//! let result = &sim.assertion_results()[id.index()];
//! assert_eq!(result.failures, 0);
//! assert!(result.passes > 0);
//! ```
//!
//! A property may also be built through [`property::Directive`] directly,
//! or taken from a Verilog `assert property (...)` with
//! [`property::from_assertion`]: the front end keeps property text as
//! [`RawTokens`](crate::verilog::ast::RawTokens), and the parser from those
//! tokens lives here rather than in `src/verilog`, so adding SVA to the
//! simulator changed no front end.
//!
//! # Supported constructs
//!
//! Booleans over nets: `!`, `~`, `&&`, `||`, `->`, the comparisons `==`,
//! `!=`, `===`, `!==`, `<`, `<=`, `>`, `>=`, bit and part selects
//! (`q[3]`, `q[7:4]`), sized and unsized literals, and the sampled value
//! functions `$rose`, `$fell` and `$stable`.
//!
//! Sequences: `##n` and `##[m:n]` delays (including `##0` fusion,
//! `##[m:$]`, `##[*]` and `##[+]`), consecutive repetition `[*n]`,
//! `[*m:n]` and `[*m:$]`, goto repetition `[->n]` and `[->m:n]`, `and`,
//! `or`, `intersect`, `throughout`, `within`, and the PSL SERE braces
//! `{a; b}` (`##1`) and `{a : b}` (`##0`).
//!
//! Properties: `|->`, `|=>`, `not`, `and`, `or`, the PSL `always` and
//! `never`, an optional clocking event `@(posedge clk)` / `@(negedge clk)`
//! / `@(clk)`, and `disable iff (expr)`.
//!
//! Directives: `assert`, `assume` and `cover`, with an optional `label:`.
//!
//! # Not supported
//!
//! - Non-consecutive repetition `[=n]`, `first_match`, `s_eventually`,
//!   `eventually`, `until` and its variants, `nexttime`, `accept_on` /
//!   `reject_on`, and `restrict`.
//! - `$past`, `$sampled`, `$countones`, `$onehot` and the other sampled
//!   value or bit-vector system functions; only `$rose`, `$fell` and
//!   `$stable` are recognised.
//! - Sequence and property *declarations* (`sequence s; ... endsequence`),
//!   formal arguments, local variables and sequence method calls
//!   (`.triggered`, `.matched`): a directive is self-contained here.
//! - Multi-clocked properties: one directive has one clocking event, and
//!   `##1` inside it means one tick of that clock.
//! - Empty matches: `[*0]` and a range starting at zero in a repetition
//!   are rejected, so no construction has to handle an empty sequence.
//! - Action blocks (`else $error(...)`): a failure is reported as a
//!   [`Diagnostic`] and the run continues.
//! - `expect`, `assert final` and deferred assertions.
//!
//! # Semantics
//!
//! Values are *sampled*: the cycle at a clocking event reads the values
//! every net had at the start of the time slot the edge occurred in, which
//! is the preponed region of IEEE 1800 §4.4. A boolean that evaluates to
//! `x` or `z` is not a match, so an unknown makes a sequence fail rather
//! than succeed.
//!
//! One attempt starts at every clocking event, which is why `always p` is
//! the same as `p` here. An attempt ends when the property holds or fails;
//! attempts still pending when the run ends are counted as incomplete and
//! are not failures. An `assert` that holds without its antecedent ever
//! matching is counted as *vacuous* rather than as a pass.
//!
//! A failing `assert` is reported as an error [`Diagnostic`] and a failing
//! `assume` as a warning; both name the attempt's start time and cycle,
//! the cycle the failure happened in, and the sampled value of every net
//! the property reads. A `cover` reports nothing and counts its matches in
//! [`AssertionResult::passes`]; its non-matching attempts are counted in
//! [`AssertionResult::failures`] and are never reported. Failures never end
//! the run.
//!
//! # Layout
//!
//! | File           | Role                                             |
//! |----------------|--------------------------------------------------|
//! | `property.rs`  | The syntax tree and the parser                   |
//! | `automaton.rs` | Sequences as NFAs over boolean predicates        |
//! | `runtime.rs`   | Binding to signals, and stepping attempts        |
//! | `mod.rs`       | The public API and the scheduler hook            |

pub mod automaton;
pub mod property;
pub(crate) mod runtime;

use crate::diag::Diagnostic;
use crate::logic::Logic;
use crate::source::Span;

use self::property::{Directive, DirectiveKind};
use self::runtime::{Checker, Stats};
use super::Simulator;
use super::elab::SigId;
use super::sched::edge_matches;

/// A concurrent assertion added to a running simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssertionId(u32);

impl AssertionId {
    /// The position of this assertion in
    /// [`Simulator::assertion_results`].
    pub fn index(self) -> usize {
        // `u32` always fits `usize` on supported targets.
        self.0 as usize
    }
}

/// What one directive did over a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssertionResult {
    /// The directive's label, or a generated `assert#n`.
    pub name: String,
    /// Which directive.
    pub kind: DirectiveKind,
    /// Clocking events seen.
    pub cycles: u64,
    /// Attempts started.
    pub attempts: u64,
    /// Attempts that held; for a `cover`, the number of matches.
    pub passes: u64,
    /// Attempts that held without an antecedent ever matching.
    pub vacuous: u64,
    /// Attempts that failed; for a `cover`, the attempts that did not
    /// match, which is not a problem and is never reported.
    pub failures: u64,
    /// Attempts abandoned by `disable iff`.
    pub disabled: u64,
    /// Attempts still pending when the run ended.
    pub incomplete: u64,
    /// Where the directive came from.
    pub span: Span,
}

impl Simulator<'_> {
    /// Adds a concurrent assertion, resolving its net names against the
    /// top instance.
    ///
    /// The directive needs a clocking event ([`property::Directive::clock`],
    /// written `@(posedge clk)` in the text).
    ///
    /// # Errors
    ///
    /// Returns a diagnostic when the clock is missing, when a name is not
    /// a net of the design, or when the property is out of range of what
    /// the automaton builder accepts (see the [module docs](self)).
    pub fn add_assertion(&mut self, directive: &Directive) -> Result<AssertionId, Diagnostic> {
        let scope = self.top_name().to_owned();
        self.add_assertion_in(&scope, directive)
    }

    /// Adds a concurrent assertion whose unqualified net names resolve
    /// inside the instance at `scope` (a hierarchical path).
    ///
    /// # Errors
    ///
    /// As [`Simulator::add_assertion`].
    pub fn add_assertion_in(
        &mut self,
        scope: &str,
        directive: &Directive,
    ) -> Result<AssertionId, Diagnostic> {
        let Some(clock) = directive.clock.clone() else {
            return Err(Diagnostic::error(format!(
                "the {} property has no clocking event",
                directive.kind.as_str()
            ))
            .with_span(directive.span)
            .with_note("write `@(posedge clk)` before the property"));
        };
        let (compiled, clock_sig) = {
            let resolve = |name: &str| -> Option<(SigId, String)> {
                let handle = self
                    .net(name)
                    .or_else(|| self.net(&format!("{scope}.{name}")))?;
                Some((handle.0, self.net_name(handle).to_owned()))
            };
            let compiled = runtime::compile(directive, &resolve)?;
            let Some((sig, _)) = resolve(&clock.net.name) else {
                return Err(Diagnostic::error(format!(
                    "unknown clock net `{}` in a property",
                    clock.net.name
                ))
                .with_span(clock.net.span));
            };
            (compiled, sig)
        };
        let index = self.assertions.len();
        let name = directive
            .name
            .clone()
            .unwrap_or_else(|| format!("{}#{index}", directive.kind.as_str()));
        let previous = vec![Logic::x(1); compiled.nets.len()];
        self.assertions.push(Checker {
            name,
            kind: directive.kind,
            span: directive.span,
            clock: clock_sig,
            polarity: clock.polarity,
            compiled,
            attempts: Vec::new(),
            previous,
            cycle: 0,
            ticked: false,
            stats: Stats::default(),
        });
        self.rebuild_assertion_watch();
        Ok(AssertionId(
            u32::try_from(index).expect("fewer than 4 billion assertions"),
        ))
    }

    /// Parses `text` as a directive and adds it.
    ///
    /// The text may be a whole directive (`assert property (@(posedge clk)
    /// a |-> b);`) or just a property body; see
    /// [`property::parse_directive`].
    ///
    /// # Errors
    ///
    /// Returns the syntax error, or the binding error of
    /// [`Simulator::add_assertion`].
    pub fn add_assertion_text(
        &mut self,
        text: &str,
        span: Span,
    ) -> Result<AssertionId, Diagnostic> {
        let directive = property::parse_directive(text, span)?;
        self.add_assertion(&directive)
    }

    /// The number of concurrent assertions added.
    pub fn assertion_count(&self) -> usize {
        self.assertions.len()
    }

    /// What every directive did so far, in the order they were added.
    pub fn assertion_results(&self) -> Vec<AssertionResult> {
        self.assertions
            .iter()
            .map(|c| AssertionResult {
                name: c.name.clone(),
                kind: c.kind,
                cycles: c.cycle,
                attempts: c.stats.attempts,
                passes: c.stats.passes,
                vacuous: c.stats.vacuous,
                failures: c.stats.failures,
                disabled: c.stats.disabled,
                incomplete: c.incomplete(),
                span: c.span,
            })
            .collect()
    }

    /// Recomputes the signals to snapshot and the clocks to watch.
    fn rebuild_assertion_watch(&mut self) {
        let mut watch: Vec<SigId> = Vec::new();
        let mut clocks: Vec<SigId> = Vec::new();
        for c in &self.assertions {
            clocks.push(c.clock);
            watch.push(c.clock);
            watch.extend(c.compiled.nets.iter().map(|n| n.sig));
        }
        watch.sort_unstable();
        watch.dedup();
        clocks.sort_unstable();
        clocks.dedup();
        let sample: Vec<Logic> = watch
            .iter()
            .map(|s| self.signals[s.idx()].effective().clone())
            .collect();
        self.assert_sample = sample;
        self.assert_watch = watch;
        self.assert_clocks = clocks;
    }

    /// Snapshots the watched signals: the sampled (preponed) values the
    /// cycle of any clocking event in this time slot will read.
    pub(crate) fn sample_assertions(&mut self) {
        if self.assertions.is_empty() {
            return;
        }
        for (i, sig) in self.assert_watch.iter().enumerate() {
            self.assert_sample[i] = self.signals[sig.idx()].effective().clone();
        }
    }

    /// Notes a clocking event on `sig`; called from the propagation path.
    pub(crate) fn note_assertion_edge(&mut self, sig: SigId, old: &Logic, new: &Logic) {
        if self.assert_clocks.binary_search(&sig).is_err() {
            return;
        }
        for c in &mut self.assertions {
            if c.clock == sig && edge_matches(c.polarity, old, new) {
                c.ticked = true;
            }
        }
    }

    /// Runs the cycle of every assertion whose clock ticked in this time
    /// slot, at the end of the slot.
    pub(crate) fn run_assertions(&mut self) {
        if self.assertions.is_empty() {
            return;
        }
        for i in 0..self.assertions.len() {
            if !self.assertions[i].ticked {
                continue;
            }
            self.assertions[i].ticked = false;
            let now: Vec<Logic> = self.assertions[i]
                .compiled
                .nets
                .iter()
                .map(|n| self.sampled(n.sig))
                .collect();
            let time = self.now;
            let cycle = self.assertions[i].cycle;
            let report = self.assertions[i].tick(time, &now);
            if self.assertions[i].kind == DirectiveKind::Cover {
                // A cover that does not match is the normal case; only its
                // count is interesting.
                continue;
            }
            for (start_time, start_cycle) in report.failures {
                self.report_assertion(i, cycle, start_time, start_cycle, &now);
            }
        }
    }

    /// The sampled value of a watched signal.
    fn sampled(&self, sig: SigId) -> Logic {
        match self.assert_watch.binary_search(&sig) {
            Ok(i) => self.assert_sample[i].clone(),
            Err(_) => self.signals[sig.idx()].effective().clone(),
        }
    }

    /// Reports one failing attempt.
    fn report_assertion(
        &mut self,
        index: usize,
        cycle: u64,
        start_time: u64,
        start_cycle: u64,
        now: &[Logic],
    ) {
        let (message, values, span, kind) = {
            let c = &self.assertions[index];
            let what = match c.kind {
                DirectiveKind::Assert => "assertion",
                DirectiveKind::Assume => "assumption",
                DirectiveKind::Cover => "cover property",
            };
            let message = format!(
                "{what} `{}` failed at cycle {cycle} (time {}); the attempt started at cycle \
                 {start_cycle} (time {start_time})",
                c.name, self.now
            );
            let values = c
                .compiled
                .nets
                .iter()
                .zip(now)
                .map(|(net, value)| format!("{} = {value}", net.name))
                .collect::<Vec<_>>()
                .join(", ");
            (message, values, c.span, c.kind)
        };
        let diag = match kind {
            DirectiveKind::Assume => Diagnostic::warning(message),
            _ => Diagnostic::error(message),
        };
        let diag = diag.with_span(span);
        let diag = if values.is_empty() {
            diag
        } else {
            diag.with_note(format!("sampled values: {values}"))
        };
        self.messages.push(diag);
    }
}
