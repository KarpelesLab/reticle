//! Eligibility: whether a design can run in compiled fast mode, and if
//! not, exactly what is in the way.
//!
//! "It was slow and I do not know why" is the worst outcome a fast mode
//! can have, so [`check`] never answers with a bare `false`. It returns
//! one [`Ineligible`] per obstacle, each naming the object (a process, a
//! cell, a net, given by its hierarchical path) and the span it was
//! declared at, so a caller can render them with
//! [`Diagnostics`](crate::diag::Diagnostics) or print them directly.
//!
//! A design qualifies when it is *synchronous* and *two-state safe*:
//!
//! | Requirement | What breaks it |
//! |-------------|----------------|
//! | every process is combinational, clocked, or a time-zero initialiser | a `free` process, a `wait`, a delayed assignment |
//! | one clock | two clock nets, both edges of one net, a clock generated inside the design |
//! | no level-sensitive storage | a `dlatch` cell, a combinational process that does not assign on every path |
//! | one driver per bit | two drivers of the same bit, a `tristate` cell, an unresolved black box |
//! | no combinational loop | a cycle in the dependency graph of the combinational units |
//! | no race between processes | a clocked blocking write another clocked unit reads, or one net a process assigns both blockingly and non-blockingly |
//! | no `x` or `z` that matters | a constant with unknown bits, a state element still unknown after time zero |
//!
//! The last one is where [`CompileOptions::zero_init`] comes in: a design
//! whose registers have no reset and no initial value is perfectly
//! simulable in two states once someone says what "uninitialised" means,
//! and that someone is the caller.

use std::fmt;

use crate::diag::Diagnostics;
use crate::ir::{Design, Span};
use crate::sim::{FileProvider, SimOptions, Simulator};

/// Configuration for [`check`] and [`super::CompiledSim::new`].
pub struct CompileOptions {
    /// The module to instantiate as the root; defaults the same way
    /// [`SimOptions::top`] does.
    pub top: Option<String>,
    /// Treat a state element that is still `x` after time zero as zero,
    /// instead of refusing the design.
    ///
    /// A design with no reset and no initialiser is fine to simulate in
    /// two states once the caller says what uninitialised means; without
    /// this flag [`check`] refuses rather than guessing.
    pub zero_init: bool,
    /// Files for `$readmemh` / `$readmemb` in time-zero initialisers.
    pub files: Option<Box<dyn FileProvider>>,
    /// Seed for `$random` during the time-zero initialisation.
    pub seed: u64,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            top: None,
            zero_init: false,
            files: None,
            seed: 1,
        }
    }
}

impl fmt::Debug for CompileOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompileOptions")
            .field("top", &self.top)
            .field("zero_init", &self.zero_init)
            .field("files", &self.files.is_some())
            .field("seed", &self.seed)
            .finish()
    }
}

impl CompileOptions {
    /// The event-simulator options the compiled mode elaborates with.
    pub(crate) fn sim_options(self) -> SimOptions {
        SimOptions {
            top: self.top,
            seed: self.seed,
            files: self.files,
            ..SimOptions::default()
        }
    }
}

/// Why one object keeps a design out of compiled fast mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The design does not elaborate at all; the text is the first error.
    Elaboration(String),
    /// A `free` process (an `always` without a sensitivity list): it
    /// controls its own timing, which a cycle-based model has none of.
    FreeProcess,
    /// A `wait`, a `#` delay or an `after` clause.
    TimingControl,
    /// A statement the lowering has no straight-line form for.
    UnsupportedStatement(&'static str),
    /// An expression the lowering has no straight-line form for.
    UnsupportedExpression(&'static str),
    /// A system task other than `$display`, `$write`, `$finish`, `$stop`
    /// and their radix variants.
    UnsupportedTask(String),
    /// A cell kind with no cycle-based meaning.
    UnsupportedCell(String),
    /// A `tristate` cell or an `inout` needing wire resolution.
    Tristate,
    /// A net several drivers write the same bits of.
    MultiplyDriven,
    /// Level-sensitive storage: a `dlatch`, or a combinational unit that
    /// leaves a net unassigned on some path.
    Latch,
    /// A combinational unit reads a net it has not assigned yet.
    CombinationalLoop,
    /// Clocked storage on a second clock, or on both edges of one clock.
    SeveralClocks(String),
    /// The clock is produced inside the design rather than driven in.
    GeneratedClock,
    /// An asynchronous reset whose condition or value comes out of
    /// combinational logic.
    AsyncResetLogic,
    /// A sensitivity list that does not list everything the body reads, so
    /// the event simulator would keep a stale value the compiled engine
    /// recomputes.
    IncompleteSensitivity,
    /// A combinational process reads a net it assigns non-blockingly, so
    /// the result depends on how many delta cycles the event simulator
    /// takes.
    NonBlockingReadBack,
    /// A clocked process assigns a net with a blocking assignment and
    /// another clocked unit reads it, so the result depends on the order
    /// the two run in.
    BlockingRace,
    /// One process assigns the same net both blockingly and
    /// non-blockingly.
    MixedAssignment,
    /// A memory written outside a clock edge.
    MemoryOutsideEdge,
    /// A loop whose trip count is not fixed at compile time.
    DynamicLoopBound,
    /// A literal with `x` or `z` bits, which two states cannot carry.
    UnknownConstant(String),
    /// A state element still unknown after time zero; see
    /// [`CompileOptions::zero_init`].
    UninitialisedState,
    /// A net with no driver, no initial value, and a reader.
    UndrivenNet,
    /// A net only some of whose bits are driven.
    PartiallyDriven,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Elaboration(msg) => write!(f, "the design does not elaborate: {msg}"),
            Reason::FreeProcess => f.write_str(
                "a free-running process controls its own timing, which a cycle-based run has none of",
            ),
            Reason::TimingControl => {
                f.write_str("timing control (a wait or a delay) has no cycle-based meaning")
            }
            Reason::UnsupportedStatement(what) => {
                write!(f, "`{what}` has no straight-line form")
            }
            Reason::UnsupportedExpression(what) => {
                write!(f, "the expression `{what}` has no straight-line form")
            }
            Reason::UnsupportedTask(name) => {
                write!(f, "the system task `{name}` is not available in fast mode")
            }
            Reason::UnsupportedCell(kind) => {
                write!(f, "a `{kind}` cell has no cycle-based meaning")
            }
            Reason::Tristate => f.write_str("tri-state drive needs wire resolution, which two-state mode does not have"),
            Reason::MultiplyDriven => {
                f.write_str("several drivers write the same bits, which needs wire resolution")
            }
            Reason::Latch => f.write_str(
                "level-sensitive storage: the value is kept when no branch assigns it",
            ),
            Reason::CombinationalLoop => {
                f.write_str("a combinational loop: the value depends on itself")
            }
            Reason::SeveralClocks(other) => write!(
                f,
                "clocked on a second clock; the design is already clocked on {other}"
            ),
            Reason::GeneratedClock => {
                f.write_str("the clock is produced inside the design, so a cycle has no fixed meaning")
            }
            Reason::AsyncResetLogic => f.write_str(
                "an asynchronous reset is applied before the settle, so its condition and value must come from an input or a register, not from combinational logic",
            ),
            Reason::IncompleteSensitivity => f.write_str(
                "the sensitivity list does not cover everything the body reads",
            ),
            Reason::NonBlockingReadBack => f.write_str(
                "a combinational process reads a net it assigns non-blockingly",
            ),
            Reason::BlockingRace => f.write_str(
                "a clocked blocking assignment is read by another clocked unit, so the result depends on process order",
            ),
            Reason::MixedAssignment => f.write_str(
                "assigned both blockingly and non-blockingly by one process, which race with each other",
            ),
            Reason::MemoryOutsideEdge => {
                f.write_str("the memory is written outside a clock edge")
            }
            Reason::DynamicLoopBound => {
                f.write_str("the loop does not have a trip count fixed at compile time")
            }
            Reason::UnknownConstant(text) => {
                write!(f, "the literal `{text}` has x or z bits")
            }
            Reason::UninitialisedState => f.write_str(
                "still unknown after time zero; give it a reset or an initial value, or set `zero_init`",
            ),
            Reason::UndrivenNet => {
                f.write_str("has no driver and no initial value, but is read")
            }
            Reason::PartiallyDriven => f.write_str("only some of its bits are driven"),
        }
    }
}

/// One obstacle between a design and compiled fast mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ineligible {
    /// What is in the way, by hierarchical path where there is one
    /// (`top.u0.state`, `cell top.u0.ff0`, `process top.count`).
    pub object: String,
    /// Why.
    pub reason: Reason,
    /// Where it was declared, when the object carries a span.
    pub span: Option<Span>,
}

impl Ineligible {
    /// Builds one.
    pub(crate) fn new(object: impl Into<String>, reason: Reason, span: Option<Span>) -> Ineligible {
        Ineligible {
            object: object.into(),
            reason,
            span,
        }
    }
}

impl fmt::Display for Ineligible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.object, self.reason)
    }
}

/// Renders a list of obstacles as diagnostics, for a caller that already
/// has a [`Diagnostics`] to show.
pub fn diagnose(problems: &[Ineligible]) -> Diagnostics {
    let mut diags = Diagnostics::new();
    for p in problems {
        let mut d = crate::diag::Diagnostic::error(p.to_string());
        if let Some(span) = p.span {
            d = d.with_span(span);
        }
        diags.push(d);
    }
    diags
}

/// Analyses `design` and, when it qualifies, lowers it to a straight-line
/// [`Plan`](super::Plan).
///
/// # Errors
///
/// Returns every obstacle found, not just the first, so one run tells the
/// whole story. A design that does not elaborate at all comes back as a
/// single [`Reason::Elaboration`].
pub fn check(design: &Design, options: CompileOptions) -> Result<super::Plan<'_>, Vec<Ineligible>> {
    let zero_init = options.zero_init;
    let sim = match Simulator::new(design, options.sim_options()) {
        Ok(sim) => sim,
        Err(diags) => {
            let first = diags
                .iter()
                .find(|d| d.severity == crate::diag::Severity::Error)
                .map_or_else(|| "unknown error".to_owned(), |d| d.message.clone());
            return Err(vec![Ineligible::new(
                design.top.map_or_else(
                    || "design".to_owned(),
                    |t| design.modules[t].name.to_string(),
                ),
                Reason::Elaboration(first),
                None,
            )]);
        }
    };
    super::lower::build(sim, zero_init)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SourceMap, Span};

    fn design(text: &str) -> Design {
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        Design::parse_text(text, file).unwrap()
    }

    #[test]
    fn reasons_render() {
        let mut map = SourceMap::new();
        let file = map.add("t", "x").unwrap();
        let span = Span::new(file, 0, 1);
        let p = Ineligible::new("top.q", Reason::UninitialisedState, Some(span));
        assert!(p.to_string().starts_with("top.q: still unknown"));
        assert_eq!(diagnose(&[p]).error_count(), 1);
        for r in [
            Reason::Elaboration("no top".into()),
            Reason::FreeProcess,
            Reason::TimingControl,
            Reason::UnsupportedStatement("forever"),
            Reason::UnsupportedExpression("call"),
            Reason::UnsupportedTask("$monitor".into()),
            Reason::UnsupportedCell("dlatch".into()),
            Reason::Tristate,
            Reason::MultiplyDriven,
            Reason::Latch,
            Reason::CombinationalLoop,
            Reason::SeveralClocks("top.clk".into()),
            Reason::GeneratedClock,
            Reason::AsyncResetLogic,
            Reason::IncompleteSensitivity,
            Reason::NonBlockingReadBack,
            Reason::BlockingRace,
            Reason::MixedAssignment,
            Reason::MemoryOutsideEdge,
            Reason::DynamicLoopBound,
            Reason::UnknownConstant("4'bxx01".into()),
            Reason::UndrivenNet,
            Reason::PartiallyDriven,
        ] {
            assert!(!r.to_string().is_empty());
        }
    }

    #[test]
    fn options_debug_hides_the_provider() {
        let opts = CompileOptions {
            files: Some(Box::new(crate::sim::MemoryFiles::new())),
            ..CompileOptions::default()
        };
        assert!(format!("{opts:?}").contains("files: true"));
    }

    #[test]
    fn a_design_that_does_not_elaborate_says_so() {
        let d = design("module a\n  net %x u1 wire\nend\nmodule b\n  net %y u1 wire\nend\n");
        let err = check(&d, CompileOptions::default()).unwrap_err();
        assert!(matches!(err[0].reason, Reason::Elaboration(_)));
    }
}
