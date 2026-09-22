//! Unrolling a transition relation over time.
//!
//! Bounded model checking (Biere, Cimatti, Clarke, Zhu, "Symbolic Model
//! Checking without BDDs", TACAS 1999) asks whether a bad state is
//! reachable in `k` steps by writing `I(s0) ∧ T(s0, s1) ∧ … ∧ T(sk-1, sk)
//! ∧ ¬P(sk)` as one propositional formula. An [`Unrolling`] is that
//! formula under construction: a [`CnfBuilder`] holding `k + 1` blasted
//! frames of a [`Transition`], each frame's state literals being the
//! previous frame's next-state literals (so `T` costs no extra clauses),
//! plus a [`Solver`] kept in sync incrementally so that clauses learnt
//! while checking step `k` help with step `k + 1`.
//!
//! [`Transition`] abstracts over what is unrolled: a single module through
//! [`Blaster`], or the miter of two modules in [`super::equiv`].
//!
//! # Initial state
//!
//! [`InitMode`] decides frame 0. With [`InitMode::Reset`] every state
//! element that has a reset value or an `init` attribute (or a memory
//! initialiser) starts there and everything else is free; with
//! [`InitMode::Zero`] the rest starts at zero; with [`InitMode::Free`]
//! every state element is free, which is what the inductive step of
//! k-induction and a `--init-free` check want. Free `x` bits inside a
//! known initial value stay free in every mode.
//!
//! `assume` properties are asserted as unit clauses in every frame as it
//! is added, so every solve honours them.

use super::blast::{BlastedFrame, Blaster, StateSlot, const_lits};
use super::cnf::CnfBuilder;
use super::sat::{Lit, SolveResult, Solver, Stats};
use super::trace::Trace;
use crate::logic::Logic;

/// How the state at frame 0 is constrained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InitMode {
    /// Reset values and `init` attributes where the design has them; the
    /// remaining state is free.
    #[default]
    Reset,
    /// Reset values and `init` attributes where the design has them; the
    /// remaining state is zero.
    Zero,
    /// Every state element is free.
    Free,
}

/// Something that can be unrolled: a system with inputs, state and
/// properties, encoded one frame at a time.
pub trait Transition {
    /// A name for reports.
    fn name(&self) -> String;

    /// The state elements, in the order [`BlastedFrame::state`] uses.
    fn state_slots(&self) -> Vec<StateSlot>;

    /// Encodes one frame; `state` supplies the current-state literals
    /// (one vector per slot) or, when `None`, leaves them free.
    fn blast_frame(&self, cnf: &mut CnfBuilder, state: Option<&[Vec<Lit>]>) -> BlastedFrame;
}

impl Transition for Blaster<'_> {
    fn name(&self) -> String {
        self.module().name.to_string()
    }

    fn state_slots(&self) -> Vec<StateSlot> {
        Blaster::state_slots(self).to_vec()
    }

    fn blast_frame(&self, cnf: &mut CnfBuilder, state: Option<&[Vec<Lit>]>) -> BlastedFrame {
        Blaster::blast_frame(self, cnf, state, None)
    }
}

/// The literals of the initial state under `mode`.
fn initial_state(cnf: &mut CnfBuilder, slots: &[StateSlot], mode: InitMode) -> Vec<Vec<Lit>> {
    slots
        .iter()
        .map(|slot| {
            let width = usize::try_from(slot.width).expect("width fits usize");
            match (mode, &slot.init) {
                (InitMode::Free, _) | (InitMode::Reset, None) => cnf.new_vars(width),
                (InitMode::Zero, None) => const_lits(cnf, &Logic::zero(slot.width)),
                (_, Some(v)) => {
                    let lits = const_lits(cnf, &v.resize(slot.width));
                    debug_assert_eq!(lits.len(), width);
                    lits
                }
            }
        })
        .collect()
}

/// `k + 1` frames of a [`Transition`] with a solver kept in sync.
pub struct Unrolling<'t> {
    system: &'t dyn Transition,
    cnf: CnfBuilder,
    solver: Solver,
    loaded: usize,
    frames: Vec<BlastedFrame>,
}

impl<'t> Unrolling<'t> {
    /// Encodes frame 0 of `system` with its initial state per `mode`.
    pub fn new(system: &'t dyn Transition, mode: InitMode) -> Unrolling<'t> {
        let mut cnf = CnfBuilder::new();
        let slots = system.state_slots();
        let init = initial_state(&mut cnf, &slots, mode);
        let frame = system.blast_frame(&mut cnf, Some(&init));
        let mut u = Unrolling {
            system,
            cnf,
            solver: Solver::new(),
            loaded: 0,
            frames: Vec::new(),
        };
        u.push_frame(frame);
        u
    }

    fn push_frame(&mut self, frame: BlastedFrame) {
        for a in &frame.assumes {
            self.cnf.add_unit(a.lit);
        }
        self.frames.push(frame);
    }

    /// The system being unrolled.
    pub fn system(&self) -> &'t dyn Transition {
        self.system
    }

    /// The frames encoded so far; frame `k` is at index `k`.
    pub fn frames(&self) -> &[BlastedFrame] {
        &self.frames
    }

    /// Adds the next frame, tied to the last one's next state, and returns
    /// its index.
    pub fn extend(&mut self) -> usize {
        let state = self
            .frames
            .last()
            .expect("frame 0 exists")
            .next_state
            .clone();
        let frame = self.system.blast_frame(&mut self.cnf, Some(&state));
        self.push_frame(frame);
        self.frames.len() - 1
    }

    /// The clause builder, for adding constraints between frames.
    pub fn cnf(&mut self) -> &mut CnfBuilder {
        &mut self.cnf
    }

    /// Asserts a literal permanently.
    pub fn add_unit(&mut self, lit: Lit) {
        self.cnf.add_unit(lit);
    }

    /// Adds a clause permanently.
    pub fn add_clause(&mut self, lits: &[Lit]) {
        self.cnf.add_clause(lits);
    }

    /// Caps the conflicts per [`Unrolling::solve`]; `None` removes the cap.
    pub fn set_conflict_limit(&mut self, limit: Option<u64>) {
        self.solver.set_conflict_limit(limit);
    }

    /// Loads clauses added since the last solve into the solver.
    fn sync(&mut self) {
        while self.solver.num_vars() < usize::try_from(self.cnf.num_vars()).expect("fits") {
            self.solver.new_var();
        }
        let total = self.cnf.num_clauses();
        for clause in self.cnf.clauses().skip(self.loaded) {
            if !self.solver.add_clause(clause) {
                break;
            }
        }
        self.loaded = total;
    }

    /// Solves the unrolling so far under `assumptions`.
    pub fn solve(&mut self, assumptions: &[Lit]) -> SolveResult {
        self.sync();
        self.solver.solve_with_assumptions(assumptions)
    }

    /// The solver, for reading models after a satisfiable solve.
    pub fn solver(&self) -> &Solver {
        &self.solver
    }

    /// Solver statistics so far.
    pub fn stats(&self) -> Stats {
        self.solver.stats()
    }

    /// The values of the first `count` frames in the last model.
    pub fn trace(&self, count: usize) -> Trace {
        Trace::from_model(&self.frames[..count.min(self.frames.len())], &self.solver)
    }

    /// A literal true when some `assert` of frame `k` is violated.
    pub fn bad(&mut self, k: usize) -> Lit {
        let violated: Vec<Lit> = self.frames[k].asserts.iter().map(|p| !p.lit).collect();
        self.cnf.or_n(&violated)
    }

    /// Adds the constraint that the states of frames `i` and `j` differ,
    /// the "unique states" strengthening that makes k-induction complete
    /// for finite-state systems. A no-op when there is no state.
    pub fn add_distinct(&mut self, i: usize, j: usize) {
        let a: Vec<Lit> = self.frames[i]
            .state
            .iter()
            .flat_map(|s| s.lits.clone())
            .collect();
        let b: Vec<Lit> = self.frames[j]
            .state
            .iter()
            .flat_map(|s| s.lits.clone())
            .collect();
        if a.is_empty() {
            return;
        }
        let diff: Vec<Lit> = a
            .iter()
            .zip(&b)
            .map(|(&x, &y)| self.cnf.xor(x, y))
            .collect();
        self.cnf.add_clause(&diff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formal::blast::BlastOptions;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Module, Name, Reset, Type};
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// A 3-bit counter resetting to 2 with an `assume` that `en` is high.
    fn counter() -> Module {
        let mut b = ModuleBuilder::new("ctr", span());
        let clk = b.input("clk", Type::bit());
        let en = b.input("en", Type::bit());
        let q = b.output("q", Type::bits(3));
        let (qv, one, env, clkv) = (b.net(q), b.const_u64(3, 1), b.net(en), b.net(clk));
        let inc = b.add(qv, one);
        let d = b.mux(env, inc, qv);
        let f = b.const_bit(false);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Logic::from_u64(2, 3),
                }),
            },
            vec![
                (Name::new("clk"), clkv),
                (Name::new("d"), d),
                (Name::new("rst"), f),
            ],
            vec![(Name::new("q"), q)],
        );
        let always = b.add_net("always", Type::bit());
        b.net_attr(always, "formal_assume", 1);
        b.assign(always, env);
        b.finish()
    }

    fn q_value(u: &Unrolling, k: usize) -> u64 {
        u.trace(k + 1).frames[k].outputs[0].1.to_u64().unwrap()
    }

    #[test]
    fn frames_chain_and_init_modes_apply() {
        let m = counter();
        let blaster = Blaster::new(&m, &BlastOptions::default()).unwrap();
        let mut u = Unrolling::new(&blaster, InitMode::Reset);
        u.extend();
        u.extend();
        assert_eq!(u.frames().len(), 3);
        assert_eq!(u.system().name(), "ctr");
        assert_eq!(u.solve(&[]), SolveResult::Sat);
        // Reset to 2, then the assume forces counting: 2, 3, 4.
        assert_eq!((q_value(&u, 0), q_value(&u, 1), q_value(&u, 2)), (2, 3, 4));
        // Nothing else is possible.
        let q1 = u.frames()[1].outputs[0].lits.clone();
        assert_eq!(u.solve(&[q1[0]]), SolveResult::Sat);
        assert_eq!(u.solve(&[!q1[0]]), SolveResult::Unsat);
        assert!(u.stats().decisions < 1000);

        let mut z = Unrolling::new(&blaster, InitMode::Zero);
        assert_eq!(z.solve(&[]), SolveResult::Sat);
        assert_eq!(q_value(&z, 0), 2, "reset value wins over zero");

        let mut f = Unrolling::new(&blaster, InitMode::Free);
        let q0 = f.frames()[0].outputs[0].lits.clone();
        let seven: Vec<Lit> = q0.clone();
        assert_eq!(f.solve(&seven), SolveResult::Sat);
        assert_eq!(q_value(&f, 0), 7);
    }

    #[test]
    fn bad_and_distinct() {
        let mut b = ModuleBuilder::new("toggle", span());
        let clk = b.input("clk", Type::bit());
        let q = b.output("q", Type::bit());
        let (qv, clkv) = (b.net(q), b.net(clk));
        let nq = b.not(qv);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clkv), (Name::new("d"), nq)],
            vec![(Name::new("q"), q)],
        );
        let p = b.add_net("p", Type::bit());
        b.net_attr(p, "formal_assert", 1);
        b.assign(p, qv);
        let m = b.finish();
        let blaster = Blaster::new(&m, &BlastOptions::default()).unwrap();
        let mut u = Unrolling::new(&blaster, InitMode::Free);
        u.extend();
        u.extend();
        // The state toggles, so frames 0 and 2 are equal: demanding all
        // three distinct is impossible.
        u.add_distinct(0, 1);
        assert_eq!(u.solve(&[]), SolveResult::Sat);
        u.add_distinct(0, 2);
        assert_eq!(u.solve(&[]), SolveResult::Unsat);
        // A fresh unrolling: the assert fails in some frame of any trace.
        let mut u = Unrolling::new(&blaster, InitMode::Free);
        u.extend();
        let bad0 = u.bad(0);
        let bad1 = u.bad(1);
        u.add_unit(!bad0);
        assert_eq!(u.solve(&[bad1]), SolveResult::Sat);
        assert_eq!(u.solve(&[!bad1]), SolveResult::Unsat);
        u.set_conflict_limit(Some(0));
        u.add_clause(&[bad1, !bad1]);
        let _ = u.solver();
        assert!(u.cnf().num_vars() > 1);
    }
}
