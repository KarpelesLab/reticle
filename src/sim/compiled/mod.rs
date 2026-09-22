//! Compiled two-state fast mode: cycle-based simulation of synchronous
//! designs.
//!
//! The event-driven simulator in [`sim`](crate::sim) is the reference: it
//! is 4-state, it has delays, and it settles each time slot by running a
//! queue to quiescence. That generality costs a lot on the designs it is
//! used on most — synchronous logic driven by one clock, where the answer
//! to "what happens this cycle" does not need a queue at all. This module
//! is the other trade: it proves a design is cycle-based and two-state
//! safe, lowers it once to a flat list of operations over a register file
//! of machine words, and then runs that list.
//!
//! ```
//! use reticle::ir::Design;
//! use reticle::logic::Logic;
//! use reticle::sim::compiled::{CompileOptions, CompiledSim};
//! use reticle::source::SourceMap;
//!
//! let text = "\
//! module counter
//!   net %clk u1 wire
//!   net %rst u1 wire
//!   net %q u8 reg
//!   port clk in %clk
//!   port rst in %rst
//!   port q out %q
//!   process count seq posedge %clk
//!     if %rst
//!       %q <= 8'd0
//!     else
//!       %q <= add(%q, 8'd1)
//!     end
//!   end
//! end
//! ";
//! let mut map = SourceMap::new();
//! let file = map.add("counter.rtl", text).unwrap();
//! let design = Design::parse_text(text, file).unwrap();
//! let options = CompileOptions {
//!     zero_init: true,
//!     ..CompileOptions::default()
//! };
//! let mut sim = CompiledSim::new(&design, options).unwrap();
//! let rst = sim.net("counter.rst").unwrap();
//! let q = sim.net("counter.q").unwrap();
//! sim.set(rst, Logic::from_bool(true));
//! sim.step();
//! sim.set(rst, Logic::from_bool(false));
//! sim.run_cycles(7);
//! assert_eq!(sim.get(q).to_u64(), Some(7));
//! ```
//!
//! # The model
//!
//! One [`CompiledSim::step`] is one clock edge:
//!
//! 1. **Settle.** Run the combinational operations in topological order.
//!    One pass is enough, and that is the whole point: the order was
//!    computed at compile time, so there is no event queue, no delta
//!    cycle and no fixed point to converge to.
//! 2. **Edge.** Compute the next value of every register, every memory
//!    write and the surviving side effects. Nothing is visible yet.
//! 3. **Commit.** Make the next state current in one pass, and apply the
//!    queued memory writes. Committing everything at once is what makes a
//!    cycle-based simulator agree with the event simulator's non-blocking
//!    semantics without modelling them.
//!
//! The settle is skipped when no input has changed, so a run of `n`
//! cycles does `n` settles, not `2n`. Nothing allocates in the steady
//! state except a `$display` that actually produces text.
//!
//! # Next to the event simulator
//!
//! [`CompiledSim`] mirrors the parts of
//! [`Simulator`](crate::sim::Simulator) that have a cycle-based meaning,
//! with the same names and the same [`NetHandle`](crate::sim::NetHandle),
//! so a testbench can be pointed at either engine with almost no edits:
//!
//! | Event simulator | Compiled engine | Difference |
//! |-----------------|-----------------|------------|
//! | `net`, `memory`, `nets`, `memories`, `net_name`, `top_name`, `instance_paths` | same | none: both use the same elaboration |
//! | `get`, `set` | same, but `get` takes `&mut self` | it settles the combinational region first if an input changed |
//! | `get_mem`, `set_mem`, `mem_len` | same | none |
//! | `run_for(ticks)` | `run_cycles(n)` | a cycle, not a tick |
//! | `step()` | `step()` | one clock edge, not one time slot |
//! | `time()` | `time()` | cycles, not ticks |
//! | `output`, `take_output`, `messages`, `take_messages`, `status`, `finished` | same | none |
//!
//! # Eligibility
//!
//! [`check`] decides, and says exactly why when the answer is no; its
//! docs have the table of requirements. The short version: every process
//! is combinational, clocked or a time-zero initialiser; there is one
//! clock, driven from outside; nothing is level-sensitive; every bit has
//! one driver; and no `x` or `z` matters.
//!
//! # What fast mode does not do
//!
//! Deliberately absent, because they have no cycle-based meaning or no
//! two-state answer:
//!
//! - **Time.** No delays, no `wait`, no `#`, no `after`. [`CompiledSim::time`]
//!   counts cycles. `$time` and every other system function is rejected at
//!   [`check`] time rather than silently returning something wrong.
//! - **`x` and `z`.** A value is a plain bit vector. `check` rules out the
//!   static sources of unknown; three dynamic ones remain and are resolved
//!   as zero rather than `x`: a select outside its operand, division or
//!   remainder by zero, and a `Pmux` with several select bits set (which
//!   takes the lowest one).
//! - **Wire resolution.** No tri-state, no multiply driven net, no
//!   `inout` that needs resolving.
//! - **Concurrent assertions.** [`sim::assertion`](crate::sim::assertion)
//!   is sampled by the event scheduler and is not available here.
//!   *Immediate* assertions (the `assert` statement) are: they are
//!   evaluated once per cycle and reported through
//!   [`CompiledSim::messages`].
//! - **Coverage and waveforms.** No line or toggle coverage, and no VCD or
//!   FST. A per-cycle VCD is a natural extension — every net's slot is
//!   known and the values are already two-state — but it is not
//!   implemented, and a caller that needs a waveform should use the event
//!   simulator.
//! - **Side effects once per cycle.** A `$display` or an immediate
//!   assertion runs once per [`CompiledSim::step`], whichever kind of
//!   process it is in. The event simulator runs the ones in a
//!   combinational process once per delta cycle the process is triggered
//!   in, which may be several times in a cycle or none at all.
//!
//! What *is* here besides the run: `$display` and `$write` (formatted by
//! the same code the event simulator uses, so the text is identical),
//! `$finish`, `$stop`, memories with the event simulator's read-old write
//! semantics, asynchronous resets (applied before the settle, so a
//! register clears the moment its reset asserts, as it does in the event
//! simulator), and everything the time-zero initialisers did, because
//! compiled mode gets its initial state by running the event simulator's
//! time zero.
//!
//! # Layout
//!
//! | File        | Role                                                        |
//! |-------------|-------------------------------------------------------------|
//! | `check.rs`  | Eligibility: [`Ineligible`], [`Reason`], [`check`]           |
//! | `lower.rs`  | Symbolic execution into SSA, folding, CSE, topological order |
//! | `prog.rs`   | The program: slots, operations, memory layouts               |
//! | `words.rs`  | Two-state machine-word kernels                               |
//! | `engine.rs` | Execution: settle, edge, commit                              |
//! | `api.rs`    | [`Plan`] and [`CompiledSim`]                                 |

mod api;
mod check;
mod engine;
mod lower;
mod prog;
mod words;

pub use api::{CompiledSim, Plan};
pub use check::{CompileOptions, Ineligible, Reason, check, diagnose};
pub use prog::ProgramStats;

#[cfg(test)]
mod tests;
