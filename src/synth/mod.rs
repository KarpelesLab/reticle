//! This module is the technology-independent front half of phase 5 of
//! `ROADMAP.md`. It takes a validated [`Design`] whose modules hold
//! processes and continuous assignments, and rewrites every module into
//! cells (`dff`, `dlatch`, `pmux`, memory ports) plus continuous
//! assignments whose expression trees hold the combinational logic. The
//! result is still the same IR, so it can be simulated, checked and emitted
//! like any other design; the later half of the phase (arithmetic lowering,
//! the AIG optimiser and LUT / cell mapping) will consume it.
//!
//! # Pipeline
//!
//! [`run`] executes the default pipeline:
//!
//! 1. [`proc::ProcLower`]: symbolic execution of every synthesisable
//!    process into mux trees, flip-flop / latch / memory-port inference,
//!    `initial` blocks turned into init values.
//! 2. An optimisation loop iterated to a fixpoint (capped by
//!    [`SynthOptions::max_iterations`]): [`opt::ConstFold`], [`opt::Dce`],
//!    [`fsm::Fsm`] (when enabled), [`opt::Merge`], [`opt::ConstFold`],
//!    [`opt::WReduce`], [`opt::FfOpt`], [`opt::Dce`].
//! 3. [`cellify::Cellify`] (unless [`SynthOptions::cellify`] is off),
//!    followed by [`opt::Merge`] and [`opt::Dce`]: the expression trees
//!    that survived in cell inputs and continuous assignments become
//!    discrete cells, so the design can be written as a netlist. It runs
//!    after the loop because the optimiser is most effective on the
//!    expression form.
//!
//! Every pass implements [`Pass`] and reports [`PassStats`]; a
//! [`PassManager`] runs a custom sequence when the default is not wanted.
//! After each pass the design is validated in debug builds (or always,
//! with [`SynthOptions::validate`]) so an invariant broken by a pass shows
//! up at the pass, not three stages later.
//!
//! # Output conventions
//!
//! - Combinational logic is built as expression trees on `assign`
//!   statements and cell inputs: `if` becomes a `mux(...)` ternary, a
//!   priority `case` a chain of them, a parallel `case` a `pmux` cell.
//!   [`cellify::Cellify`] then turns each operator into a cell of the
//!   matching kind, leaving only netlist connections (constants, nets,
//!   slices, concatenations, resizes) on the ports. Nothing is broken
//!   down to gate level; that is technology mapping's job.
//! - `dff` cells: `rst` (synchronous or asynchronous) has priority over
//!   `en`, matching the `if (rst) ... else if (en) ...` shape they are
//!   inferred from.
//! - `pmux` cells: select bit `i` of `s` picks bits `[(i+1)·w-1 : i·w]` of
//!   `b`; the first arm in source order is bit 0 (the least significant
//!   slice); `a` is the default when no bit is set.
//! - Memory ports: a clocked read port samples the contents *before* the
//!   writes of the same clock edge (read-before-write), which is what a
//!   non-blocking `q <= mem[a]` next to `mem[b] <= d` means in the process
//!   form. Several write ports produced from one process are emitted in
//!   statement order; the IR has no port priority, so a simultaneous write
//!   of one address by two ports is undefined.
//! - `x` is a don't-care: `mux(c, v, 'x)` folds to `v`. The self-check in
//!   the test suite therefore treats `x` bits of the reference as
//!   matching anything.
//! - Simulation-only statements (`$display`, assertions, `$finish`) are
//!   dropped with a note; `initial` blocks that do more than assign
//!   constants, and processes with waits, are reported as unsynthesisable
//!   and left in place.
//!
//! [`report`] summarises the result (cell counts, inferred storage with
//! source spans) as text.
//!
//! # Diagnostics
//!
//! | Code    | Meaning                                                      |
//! |---------|--------------------------------------------------------------|
//! | `S0001` | The design is not valid; synthesis did not run               |
//! | `S0010` .. `S0020` | Process lowering and FSM extraction (see [`proc`], [`fsm`]) |
//! | `S0030` | A construct [`cellify`] cannot turn into cells               |
//! | `S0031` .. `S0033` | Reserved for post-synthesis verification          |

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Design, Module, ModuleId};

pub mod cellify;
pub mod eval;
pub mod fsm;
pub mod opt;
pub mod proc;
pub mod report;

// And-Inverter Graph: structural hashing, rewriting, balancing, FRAIGing.
// (A plain comment, not a doc comment: an outer doc on a module whose file
// carries its own `//!` docs makes rustdoc resolve that file's intra-doc
// links in this scope instead of the module's, breaking every one.)
pub mod aig;

// Gate libraries for standard-cell mapping.
pub mod cells;

// Technology mapping: k-LUT covering and standard-cell mapping.
pub mod techmap;

pub(crate) mod util;

#[cfg(test)]
mod selfcheck;

pub use report::{Report, SynthStats};

/// How finite state machines are re-encoded by [`fsm::Fsm`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FsmEncoding {
    /// Re-encode only registers carrying an `fsm_encoding` attribute
    /// (`"one-hot"`, `"binary"`, `"gray"`); leave the others alone.
    #[default]
    Auto,
    /// Never touch state registers.
    None,
    /// One flip-flop per state.
    OneHot,
    /// Dense binary codes numbered in state order.
    Binary,
    /// Gray codes (adjacent numbers differ in one bit).
    Gray,
}

impl FsmEncoding {
    /// The encoding named by an `fsm_encoding` attribute value.
    pub fn from_attr(value: &str) -> Option<FsmEncoding> {
        match value {
            "auto" => Some(FsmEncoding::Auto),
            "none" => Some(FsmEncoding::None),
            "one-hot" | "onehot" | "one_hot" => Some(FsmEncoding::OneHot),
            "binary" => Some(FsmEncoding::Binary),
            "gray" => Some(FsmEncoding::Gray),
            _ => None,
        }
    }
}

/// Knobs for [`run`].
#[derive(Clone, Debug)]
pub struct SynthOptions {
    /// Replace the expression trees left in cell inputs and continuous
    /// assignments with discrete cells ([`cellify::Cellify`]), so the
    /// result can be written as a netlist. On by default.
    pub cellify: bool,
    /// How state registers are re-encoded.
    pub fsm_encoding: FsmEncoding,
    /// Reserved for the flattening step of a later phase: when true,
    /// module boundaries are preserved. The current pipeline never
    /// flattens, so the flag has no effect yet.
    pub keep_hierarchy: bool,
    /// Upper bound on optimisation-loop iterations.
    pub max_iterations: u32,
    /// Upper bound on the number of iterations a loop may be unrolled to
    /// before it is reported as unsynthesisable.
    pub max_unroll: u32,
    /// Validate the design after every pass and panic on a violation.
    /// Defaults to true in debug builds and false in release builds.
    pub validate: bool,
}

impl Default for SynthOptions {
    fn default() -> Self {
        SynthOptions {
            cellify: true,
            fsm_encoding: FsmEncoding::Auto,
            keep_hierarchy: true,
            max_iterations: 8,
            max_unroll: 1 << 16,
            validate: cfg!(debug_assertions),
        }
    }
}

/// What a pass did, for reports and for the fixpoint loop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PassStats {
    /// True when the pass modified the module.
    pub changed: bool,
    /// Named counters in a fixed order (`"folded"`, `"removed cells"`, ...).
    pub counters: Vec<(&'static str, u64)>,
}

impl PassStats {
    /// Adds `n` to the counter `key`, creating it on first use, and marks
    /// the pass as having changed something when `n > 0`.
    pub fn bump(&mut self, key: &'static str, n: u64) {
        if n == 0 {
            return;
        }
        self.changed = true;
        match self.counters.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v += n,
            None => self.counters.push((key, n)),
        }
    }

    /// The value of counter `key` (0 when absent).
    pub fn get(&self, key: &str) -> u64 {
        self.counters
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(0, |(_, v)| *v)
    }

    /// Merges another set of statistics into this one.
    pub fn merge(&mut self, other: &PassStats) {
        self.changed |= other.changed;
        for (k, v) in &other.counters {
            self.bump(k, *v);
        }
    }
}

/// A transformation of one module.
pub trait Pass {
    /// The pass name used in reports (`proc_lower`, `const_fold`, ...).
    fn name(&self) -> &'static str;

    /// Rewrites `module`, reporting problems to `diags`.
    fn run(&self, module: &mut Module, diags: &mut Diagnostics) -> PassStats;
}

/// Runs a sequence of passes over every module of a design.
pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
    options: SynthOptions,
}

impl PassManager {
    /// An empty manager with the given options.
    pub fn new(options: SynthOptions) -> Self {
        PassManager {
            passes: Vec::new(),
            options,
        }
    }

    /// Appends a pass.
    pub fn add(&mut self, pass: impl Pass + 'static) -> &mut Self {
        self.passes.push(Box::new(pass));
        self
    }

    /// The options in effect.
    pub fn options(&self) -> &SynthOptions {
        &self.options
    }

    /// Runs every pass in order over every module, returning per-pass
    /// statistics summed over the modules.
    pub fn run(&self, design: &mut Design, diags: &mut Diagnostics) -> Vec<(String, PassStats)> {
        let mut results = Vec::with_capacity(self.passes.len());
        for pass in &self.passes {
            let stats = run_pass(pass.as_ref(), design, &self.options, diags);
            results.push((pass.name().to_owned(), stats));
        }
        results
    }
}

/// Runs one pass over every module and validates afterwards when asked.
fn run_pass(
    pass: &dyn Pass,
    design: &mut Design,
    options: &SynthOptions,
    diags: &mut Diagnostics,
) -> PassStats {
    let mut total = PassStats::default();
    let ids: Vec<ModuleId> = design.modules.ids().collect();
    for id in ids {
        let module = &mut design.modules[id];
        if module.blackbox {
            continue;
        }
        let stats = pass.run(module, diags);
        total.merge(&stats);
    }
    if options.validate {
        let problems = crate::ir::validate::validate(design);
        assert!(
            !problems.has_errors(),
            "pass `{}` left the design invalid:\n{}",
            pass.name(),
            problems
                .iter()
                .map(|d| format!("  {}", d.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    total
}

/// Runs the default synthesis pipeline on `design`.
///
/// The design is validated first; an invalid design is left untouched and
/// its problems are copied into `diags`. Otherwise every module is lowered
/// and optimised in place, and the returned [`SynthStats`] holds per-pass
/// statistics and a [`Report`] of the result.
pub fn run(design: &mut Design, options: &SynthOptions, diags: &mut Diagnostics) -> SynthStats {
    let mut stats = SynthStats::default();
    let mut problems = crate::ir::validate::validate(design);
    if problems.has_errors() {
        diags.push(
            Diagnostic::error("synthesis needs a valid design")
                .with_code("S0001")
                .with_note("fix the IR validation errors reported above"),
        );
        diags.append(&mut problems);
        return stats;
    }

    let lowered = run_pass(&proc::ProcLower::new(options), design, options, diags);
    stats.passes.push(("proc_lower".to_owned(), lowered));

    let mut loop_passes: Vec<Box<dyn Pass>> = vec![Box::new(opt::ConstFold), Box::new(opt::Dce)];
    if options.fsm_encoding != FsmEncoding::None {
        loop_passes.push(Box::new(fsm::Fsm::new(options.fsm_encoding)));
    }
    loop_passes.push(Box::new(opt::Merge));
    loop_passes.push(Box::new(opt::ConstFold));
    loop_passes.push(Box::new(opt::WReduce));
    loop_passes.push(Box::new(opt::FfOpt));
    loop_passes.push(Box::new(opt::Dce));

    for _ in 0..options.max_iterations {
        let mut changed = false;
        for pass in &loop_passes {
            let s = run_pass(pass.as_ref(), design, options, diags);
            changed |= s.changed;
            stats.passes.push((pass.name().to_owned(), s));
        }
        stats.iterations += 1;
        if !changed {
            break;
        }
    }

    if options.cellify {
        let s = run_pass(&cellify::Cellify, design, options, diags);
        stats.passes.push(("cellify".to_owned(), s));
        // Share the cells the rewrite duplicated, then collect the nets and
        // expression nodes it orphaned.
        for pass in [&opt::Merge as &dyn Pass, &opt::Dce] {
            let s = run_pass(pass, design, options, diags);
            stats.passes.push((pass.name().to_owned(), s));
        }
    }

    stats.report = Report::of_design(design);
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_counters() {
        let mut s = PassStats::default();
        s.bump("a", 0);
        assert!(!s.changed);
        s.bump("a", 2);
        s.bump("a", 3);
        s.bump("b", 1);
        assert!(s.changed);
        assert_eq!(s.get("a"), 5);
        assert_eq!(s.get("c"), 0);
        let mut t = PassStats::default();
        t.merge(&s);
        assert_eq!(t.counters, [("a", 5), ("b", 1)]);
    }

    #[test]
    fn encoding_names() {
        assert_eq!(FsmEncoding::from_attr("one-hot"), Some(FsmEncoding::OneHot));
        assert_eq!(FsmEncoding::from_attr("binary"), Some(FsmEncoding::Binary));
        assert_eq!(FsmEncoding::from_attr("gray"), Some(FsmEncoding::Gray));
        assert_eq!(FsmEncoding::from_attr("none"), Some(FsmEncoding::None));
        assert_eq!(FsmEncoding::from_attr("auto"), Some(FsmEncoding::Auto));
        assert_eq!(FsmEncoding::from_attr("?"), None);
        assert_eq!(SynthOptions::default().fsm_encoding, FsmEncoding::Auto);
    }

    #[test]
    fn rejects_invalid_design() {
        use crate::ir::Type;
        use crate::ir::builder::ModuleBuilder;
        use crate::source::{SourceMap, Span};
        let mut map = SourceMap::new();
        let file = map.add("t", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("m", span);
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(8));
        let an = b.net(a);
        b.assign(y, an);
        let mut d = Design::new();
        d.add_module(b.finish());
        let mut diags = Diagnostics::new();
        let stats = run(&mut d, &SynthOptions::default(), &mut diags);
        assert!(diags.has_errors());
        assert!(stats.passes.is_empty());
        assert_eq!(diags.iter().next().unwrap().code, Some("S0001"));
    }
}
