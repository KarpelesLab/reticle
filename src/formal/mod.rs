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
//!
//! Still to come, in dependency order:
//!
//! - `blast`: the bit-blaster from [`crate::ir`] to CNF, one [`sat::Lit`]
//!   per bit per time step, with 4-state values reduced to 2-state under
//!   the chosen X semantics.
//! - `bmc`: bounded model checking of `assert` / `assume` / `cover`
//!   properties over the unrolled design, returning counter-example traces
//!   that the waveform writers can dump.
//! - `induct`: k-induction for unbounded proofs, strengthened with simple
//!   path constraints.
//! - `equiv`: combinational and sequential equivalence checking of two
//!   designs (pre- and post-synthesis), the self-check of the synthesis
//!   flow.
//! - `reach`: reachability-based lint (dead FSM states, unreachable
//!   branches).

pub mod cnf;
pub mod sat;

pub use cnf::CnfBuilder;
pub use sat::{Lit, SolveResult, Solver, Var};
