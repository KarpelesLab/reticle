//! A conflict-driven clause learning (CDCL) SAT solver.
//!
//! This is the decision procedure under every formal engine in Reticle: the
//! bit-blaster turns IR into CNF, and bounded model checking, k-induction
//! and equivalence checking are all sequences of incremental SAT calls. The
//! solver follows the MiniSat 2.2 / Glucose design, which is the common
//! ancestor of essentially every competitive solver today.
//!
//! # Encoding
//!
//! A [`Var`] is a dense index. A [`Lit`] is `2 * var + sign`, so `!lit`
//! flips the low bit and a literal indexes a per-literal table directly.
//! Assignments are stored per variable as `0` (true), `1` (false) or `2`
//! (unassigned), so the value of a literal is `assign ^ sign` (with anything
//! `>= 2` meaning unassigned), which is a branch-free lookup.
//!
//! Clauses live in one flat arena (`clause.rs`) and are referenced by
//! offset.
//!
//! # Search
//!
//! The main loop is the classic one:
//!
//! 1. **Propagate** (`propagate.rs`): two watched literals per clause
//!    (Moskewicz et al. 2001) with a *blocker* literal cached in the watch
//!    entry (Eén & Sörensson's MiniSat 2 refinement) so that a watch whose
//!    clause is already satisfied is skipped without touching the clause
//!    memory. A watched literal that becomes false is moved to another
//!    non-false literal when there is one; otherwise the clause is unit
//!    (enqueue the remaining literal) or conflicting.
//! 2. **Analyse** (`analyze.rs`): on a conflict, resolve backwards along the
//!    trail until only one literal of the current decision level remains,
//!    the *first unique implication point* (Marques-Silva & Sakallah 1999).
//!    The learnt clause is then *minimised* by dropping every literal whose
//!    reason clause is implied by the other literals, recursively
//!    (Sörensson & Biere 2009), using an explicit stack so recursion depth
//!    never depends on the problem. The clause's LBD (number of distinct
//!    decision levels, Audemard & Simon 2009) is recorded for the database
//!    reduction and the Glucose restart policy.
//! 3. **Backjump** to the second-highest level in the learnt clause, add the
//!    clause, and assert its first literal (which is now unit).
//! 4. **Decide** (`heuristics.rs`) by popping the most active unassigned
//!    variable (VSIDS) and assigning it its saved phase.
//!
//! Around that loop:
//!
//! - **Restarts** (`restart.rs`) follow the Luby sequence by default, or
//!   Glucose's LBD-average rule when [`Solver::set_restart_policy`] asks
//!   for it. Phase saving makes restarts cheap.
//! - **Learnt clause reduction** halves the learnt database periodically,
//!   dropping the clauses with the worst (highest) LBD and lowest activity
//!   first, and always keeping "glue" clauses (LBD ≤ 2), binary clauses and
//!   clauses currently acting as reasons.
//! - **Level-0 simplification** removes clauses satisfied by the top-level
//!   assignment and strips false literals from the rest whenever new
//!   top-level facts have been derived since the last pass.
//! - **Compaction** of the arena when more than a fifth of it is garbage;
//!   watch lists are rebuilt from the surviving clauses.
//!
//! # Incremental use and assumptions
//!
//! Clauses can be added between `solve` calls; learnt clauses and heuristic
//! state carry over. [`Solver::solve_with_assumptions`] asserts a set of
//! literals as the first decisions; if the instance is unsatisfiable under
//! them, [`Solver::conflict_core`] gives the subset responsible
//! (MiniSat's `analyzeFinal`). That is how a model checker asks "is the
//! property violated at step *k*" without cloning the solver.
//!
//! # References
//!
//! - J. Marques-Silva, K. Sakallah, "GRASP: A Search Algorithm for
//!   Propositional Satisfiability", IEEE Trans. Computers 48(5), 1999.
//! - M. Moskewicz, C. Madigan, Y. Zhao, L. Zhang, S. Malik, "Chaff:
//!   Engineering an Efficient SAT Solver", DAC 2001.
//! - N. Eén, N. Sörensson, "An Extensible SAT-solver", SAT 2003 (MiniSat).
//! - N. Sörensson, A. Biere, "Minimizing Learned Clauses", SAT 2009.
//! - G. Audemard, L. Simon, "Predicting Learnt Clauses Quality in Modern SAT
//!   Solvers", IJCAI 2009 (Glucose).
//! - K. Pipatsrisawat, A. Darwiche, "A Lightweight Component Caching Scheme
//!   for Satisfiability Solvers", SAT 2007 (phase saving).

mod analyze;
mod clause;
pub mod dimacs;
mod heuristics;
mod propagate;
mod restart;
#[cfg(test)]
mod tests;

use std::fmt;
use std::ops::Not;

use clause::{ClauseDb, ClauseRef};
use heuristics::VarOrder;
pub use restart::RestartPolicy;
use restart::RestartState;

/// A propositional variable, identified by a dense index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Var(u32);

impl Var {
    /// The variable with the given index.
    pub const fn new(index: u32) -> Var {
        Var(index)
    }

    /// The variable's index.
    pub const fn index(self) -> u32 {
        self.0
    }

    /// The index as a `usize`, for table lookups.
    pub(crate) const fn idx(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Var {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "x{}", self.0)
    }
}

/// A literal: a variable or its negation.
///
/// Encoded as `2 * var + sign` where `sign` is 1 for the negation. The
/// `Ord` order is by variable then polarity, which places `x` right before
/// `!x`; `add_clause` relies on that for its duplicate/tautology check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lit(u32);

impl Lit {
    /// The positive literal of `v`.
    pub const fn pos(v: Var) -> Lit {
        Lit(v.0 << 1)
    }

    /// The negative literal of `v`.
    pub const fn neg(v: Var) -> Lit {
        Lit((v.0 << 1) | 1)
    }

    /// `v` if `negative` is false, `!v` otherwise.
    pub const fn new(v: Var, negative: bool) -> Lit {
        Lit((v.0 << 1) | negative as u32)
    }

    /// The literal's variable.
    pub const fn var(self) -> Var {
        Var(self.0 >> 1)
    }

    /// True for a negated variable.
    pub const fn is_neg(self) -> bool {
        self.0 & 1 == 1
    }

    /// True for a plain (non-negated) variable.
    pub const fn is_pos(self) -> bool {
        !self.is_neg()
    }

    /// The dense index `2 * var + sign`, for per-literal tables.
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The literal from its DIMACS integer form (`3` is `x2`, `-3` is
    /// `!x2`); `None` for zero.
    pub fn from_dimacs(n: i32) -> Option<Lit> {
        let v = n.unsigned_abs().checked_sub(1)?;
        Some(Lit::new(Var(v), n < 0))
    }

    /// The literal's DIMACS integer form (1-based, negative when negated).
    ///
    /// # Panics
    ///
    /// If the variable index does not fit an `i32`, which DIMACS cannot
    /// express.
    pub fn to_dimacs(self) -> i32 {
        let n = i32::try_from(self.var().0 + 1).expect("variable index exceeds DIMACS range");
        if self.is_neg() { -n } else { n }
    }

    pub(crate) const fn from_raw(raw: u32) -> Lit {
        Lit(raw)
    }

    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

impl Not for Lit {
    type Output = Lit;

    fn not(self) -> Lit {
        Lit(self.0 ^ 1)
    }
}

impl fmt::Display for Lit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_neg() {
            f.write_str("!")?;
        }
        write!(f, "{}", self.var())
    }
}

/// The outcome of a `solve` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SolveResult {
    /// A satisfying assignment was found; see [`Solver::model`].
    Sat,
    /// No satisfying assignment exists (under the assumptions, if any; see
    /// [`Solver::conflict_core`]).
    Unsat,
    /// A conflict or propagation limit was reached first.
    Unknown,
}

/// Counters accumulated over the lifetime of a [`Solver`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Conflicts encountered.
    pub conflicts: u64,
    /// Decisions made (assumptions are not counted).
    pub decisions: u64,
    /// Literals propagated (trail entries processed).
    pub propagations: u64,
    /// Clauses learnt, including unit clauses.
    pub learnts: u64,
    /// Restarts performed.
    pub restarts: u64,
    /// Learnt-database reductions.
    pub reductions: u64,
    /// Learnt clauses deleted by reductions and simplification.
    pub removed_learnts: u64,
    /// Total literals in learnt clauses before minimisation.
    pub max_literals: u64,
    /// Total literals in learnt clauses after minimisation.
    pub tot_literals: u64,
}

// Assignment values. `UNDEF ^ 1 == 3` is also unassigned, so a literal's
// value is `assigns[var] ^ sign` and "unassigned" is `>= 2`.
const TRUE: u8 = 0;
const FALSE: u8 = 1;
const UNDEF: u8 = 2;

/// Value of `l` under `assigns`: `TRUE`, `FALSE`, or `>= 2` when unassigned.
#[inline]
fn lit_value(assigns: &[u8], l: Lit) -> u8 {
    assigns[l.var().idx()] ^ u8::from(l.is_neg())
}

/// A watch list entry: a clause and one of its other literals.
///
/// If the blocker is true the clause is satisfied and need not be visited.
#[derive(Clone, Copy)]
struct Watcher {
    cref: ClauseRef,
    blocker: Lit,
}

/// Outcome of one restart-bounded search run.
enum Search {
    Restart,
    Done(SolveResult),
}

const VAR_DECAY: f64 = 0.95;
const CLAUSE_DECAY: f32 = 0.999;
/// Conflicts before the first learnt-database reduction (Glucose's default).
const REDUCE_BASE: u64 = 2000;
/// Growth of the interval between reductions.
const REDUCE_INC: u64 = 300;

/// A CDCL SAT solver. See the [module documentation](self) for the design.
///
/// ```
/// use reticle::formal::sat::{Lit, SolveResult, Solver};
///
/// let mut s = Solver::new();
/// let a = Lit::pos(s.new_var());
/// let b = Lit::pos(s.new_var());
/// s.add_clause(&[a, b]);
/// s.add_clause(&[!a, b]);
/// assert_eq!(s.solve(), SolveResult::Sat);
/// assert_eq!(s.value(b.var()), Some(true));
/// assert_eq!(s.solve_with_assumptions(&[!b]), SolveResult::Unsat);
/// assert_eq!(s.conflict_core(), &[!b]);
/// ```
pub struct Solver {
    db: ClauseDb,
    /// Problem clauses currently stored (satisfied ones are dropped at
    /// level 0).
    clauses: Vec<ClauseRef>,
    learnts: Vec<ClauseRef>,
    /// `watches[l.index()]` lists the clauses watching `!l`, i.e. the ones
    /// to visit when `l` becomes true.
    watches: Vec<Vec<Watcher>>,

    assigns: Vec<u8>,
    level: Vec<u32>,
    reason: Vec<ClauseRef>,
    trail: Vec<Lit>,
    trail_lim: Vec<usize>,
    qhead: usize,

    order: VarOrder,
    cla_inc: f32,
    restart: RestartState,

    seen: Vec<bool>,
    analyze_stack: Vec<Lit>,
    analyze_toclear: Vec<Lit>,
    learnt_buf: Vec<Lit>,
    add_buf: Vec<Lit>,
    lbd_stamp: Vec<u64>,
    lbd_counter: u64,

    ok: bool,
    assumptions: Vec<Lit>,
    conflict_core: Vec<Lit>,
    model: Vec<Option<bool>>,
    simp_db_assigns: usize,
    next_reduce: u64,
    reduce_count: u64,
    conflict_limit: Option<u64>,
    propagation_limit: Option<u64>,
    conflict_budget: u64,
    propagation_budget: u64,
    stats: Stats,
}

impl Default for Solver {
    fn default() -> Solver {
        Solver::new()
    }
}

impl Solver {
    /// An empty solver with no variables and no clauses.
    pub fn new() -> Solver {
        Solver {
            db: ClauseDb::new(),
            clauses: Vec::new(),
            learnts: Vec::new(),
            watches: Vec::new(),
            assigns: Vec::new(),
            level: Vec::new(),
            reason: Vec::new(),
            trail: Vec::new(),
            trail_lim: Vec::new(),
            qhead: 0,
            order: VarOrder::new(VAR_DECAY),
            cla_inc: 1.0,
            restart: RestartState::new(RestartPolicy::Luby),
            seen: Vec::new(),
            analyze_stack: Vec::new(),
            analyze_toclear: Vec::new(),
            learnt_buf: Vec::new(),
            add_buf: Vec::new(),
            lbd_stamp: vec![0],
            lbd_counter: 0,
            ok: true,
            assumptions: Vec::new(),
            conflict_core: Vec::new(),
            model: Vec::new(),
            simp_db_assigns: 0,
            next_reduce: REDUCE_BASE,
            reduce_count: 0,
            conflict_limit: None,
            propagation_limit: None,
            conflict_budget: u64::MAX,
            propagation_budget: u64::MAX,
            stats: Stats::default(),
        }
    }

    /// Adds a fresh variable and returns it.
    pub fn new_var(&mut self) -> Var {
        let v = Var(u32::try_from(self.assigns.len()).expect("more than 2^32 variables"));
        assert!(v.0 < u32::MAX >> 1, "more than 2^31 variables");
        self.assigns.push(UNDEF);
        self.level.push(0);
        self.reason.push(ClauseRef::NONE);
        self.seen.push(false);
        self.watches.push(Vec::new());
        self.watches.push(Vec::new());
        self.lbd_stamp.push(0);
        self.order.add_var(v);
        v
    }

    /// Makes sure `v` exists, adding variables up to it.
    fn ensure_var(&mut self, v: Var) {
        while self.assigns.len() <= v.idx() {
            self.new_var();
        }
    }

    /// Number of variables.
    pub fn num_vars(&self) -> usize {
        self.assigns.len()
    }

    /// Number of problem clauses currently stored. Clauses satisfied at the
    /// top level are dropped, so this can go down over time.
    pub fn num_clauses(&self) -> usize {
        self.clauses.len()
    }

    /// Number of learnt clauses currently kept.
    pub fn num_learnts(&self) -> usize {
        self.learnts.len()
    }

    /// Lifetime counters.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The restart policy in use.
    pub fn restart_policy(&self) -> RestartPolicy {
        self.restart.policy()
    }

    /// Chooses the restart policy for subsequent `solve` calls.
    pub fn set_restart_policy(&mut self, policy: RestartPolicy) {
        self.restart.set_policy(policy);
    }

    /// Caps the conflicts per `solve` call; `None` removes the cap. A call
    /// that hits the cap returns [`SolveResult::Unknown`].
    pub fn set_conflict_limit(&mut self, limit: Option<u64>) {
        self.conflict_limit = limit;
    }

    /// Caps the propagations per `solve` call; `None` removes the cap. A
    /// call that hits the cap returns [`SolveResult::Unknown`].
    pub fn set_propagation_limit(&mut self, limit: Option<u64>) {
        self.propagation_limit = limit;
    }

    /// True while no top-level contradiction has been derived. Once false,
    /// every `solve` returns [`SolveResult::Unsat`].
    pub fn is_ok(&self) -> bool {
        self.ok
    }

    /// Adds a clause. Variables it mentions are created as needed.
    ///
    /// Duplicate literals are merged, a clause containing both `x` and `!x`
    /// is dropped as a tautology, and literals already false at the top
    /// level are removed. Returns `false` when the clause set has become
    /// unsatisfiable at the top level (an empty clause, or a unit clause
    /// that contradicts earlier ones); the solver then stays unsatisfiable.
    pub fn add_clause(&mut self, lits: &[Lit]) -> bool {
        debug_assert_eq!(self.decision_level(), 0);
        if !self.ok {
            return false;
        }
        for &l in lits {
            self.ensure_var(l.var());
        }
        let mut buf = std::mem::take(&mut self.add_buf);
        buf.clear();
        buf.extend_from_slice(lits);
        buf.sort_unstable();

        let mut satisfied = false;
        let mut j = 0;
        let mut prev: Option<Lit> = None;
        for i in 0..buf.len() {
            let l = buf[i];
            let value = lit_value(&self.assigns, l);
            if value == TRUE || prev == Some(!l) {
                satisfied = true;
                break;
            }
            if value != FALSE && prev != Some(l) {
                buf[j] = l;
                j += 1;
                prev = Some(l);
            }
        }
        buf.truncate(j);

        let result = if satisfied {
            true
        } else if buf.is_empty() {
            self.ok = false;
            false
        } else if buf.len() == 1 {
            self.enqueue(buf[0], ClauseRef::NONE);
            self.ok = self.propagate().is_none();
            self.ok
        } else {
            let cr = self.db.alloc(&buf, false, 0);
            self.clauses.push(cr);
            self.attach(cr);
            true
        };
        self.add_buf = buf;
        result
    }

    /// Solves the current clause set with no assumptions.
    pub fn solve(&mut self) -> SolveResult {
        self.solve_with_assumptions(&[])
    }

    /// Solves under `assumptions`: each literal is asserted before any
    /// decision, without becoming a permanent clause.
    ///
    /// On [`SolveResult::Unsat`], [`Solver::conflict_core`] lists the
    /// assumptions that took part in the refutation; if it is empty the
    /// clause set itself is unsatisfiable. Variables in the assumptions are
    /// created as needed.
    pub fn solve_with_assumptions(&mut self, assumptions: &[Lit]) -> SolveResult {
        debug_assert_eq!(self.decision_level(), 0);
        self.model.clear();
        self.conflict_core.clear();
        if !self.ok {
            return SolveResult::Unsat;
        }
        for &a in assumptions {
            self.ensure_var(a.var());
        }
        self.assumptions.clear();
        self.assumptions.extend_from_slice(assumptions);
        self.conflict_budget = self
            .conflict_limit
            .map_or(u64::MAX, |l| self.stats.conflicts.saturating_add(l));
        self.propagation_budget = self
            .propagation_limit
            .map_or(u64::MAX, |l| self.stats.propagations.saturating_add(l));

        let mut run = 0;
        let result = loop {
            self.restart.begin_run(run);
            match self.search() {
                Search::Done(r) => break r,
                Search::Restart => {
                    self.stats.restarts += 1;
                    run += 1;
                }
            }
        };

        match result {
            SolveResult::Sat => {
                self.model = self
                    .assigns
                    .iter()
                    .map(|&a| match a {
                        TRUE => Some(true),
                        FALSE => Some(false),
                        _ => None,
                    })
                    .collect();
            }
            SolveResult::Unsat => {
                if self.conflict_core.is_empty() {
                    self.ok = false;
                }
            }
            SolveResult::Unknown => {}
        }
        self.cancel_until(0);
        result
    }

    /// The value of `v` in the last model, or `None` if the last `solve`
    /// did not return [`SolveResult::Sat`] (or `v` was added since).
    pub fn value(&self, v: Var) -> Option<bool> {
        self.model.get(v.idx()).copied().flatten()
    }

    /// The last model, indexed by variable; empty unless the last `solve`
    /// returned [`SolveResult::Sat`].
    pub fn model(&self) -> &[Option<bool>] {
        &self.model
    }

    /// The assumptions responsible for the last [`SolveResult::Unsat`]
    /// answer of [`Solver::solve_with_assumptions`], as given (not
    /// negated). Empty if the clauses are unsatisfiable on their own.
    pub fn conflict_core(&self) -> &[Lit] {
        &self.conflict_core
    }

    // ----- search -------------------------------------------------------

    fn decision_level(&self) -> u32 {
        u32::try_from(self.trail_lim.len()).expect("decision level fits in u32")
    }

    fn new_decision_level(&mut self) {
        self.trail_lim.push(self.trail.len());
    }

    /// Assigns `p` true at the current level with `from` as its reason.
    fn enqueue(&mut self, p: Lit, from: ClauseRef) {
        let v = p.var();
        debug_assert!(lit_value(&self.assigns, p) >= UNDEF);
        self.assigns[v.idx()] = u8::from(p.is_neg());
        self.level[v.idx()] = self.decision_level();
        self.reason[v.idx()] = from;
        self.trail.push(p);
    }

    /// Undoes every assignment above `lvl`, saving phases.
    fn cancel_until(&mut self, lvl: u32) {
        if self.decision_level() <= lvl {
            return;
        }
        let start = self.trail_lim[lvl as usize];
        for i in (start..self.trail.len()).rev() {
            let p = self.trail[i];
            let v = p.var();
            self.assigns[v.idx()] = UNDEF;
            self.reason[v.idx()] = ClauseRef::NONE;
            self.order.save_phase(v, p.is_pos());
            self.order.insert(v);
        }
        self.qhead = start;
        self.trail.truncate(start);
        self.trail_lim.truncate(lvl as usize);
    }

    fn pick_branch_lit(&mut self) -> Option<Lit> {
        while let Some(v) = self.order.pop_max() {
            if self.assigns[v.idx()] == UNDEF {
                return Some(Lit::new(v, !self.order.phase(v)));
            }
        }
        None
    }

    /// One run between restarts.
    fn search(&mut self) -> Search {
        loop {
            if let Some(confl) = self.propagate() {
                self.stats.conflicts += 1;
                if self.decision_level() == 0 {
                    return Search::Done(SolveResult::Unsat);
                }
                let mut learnt = std::mem::take(&mut self.learnt_buf);
                let (bt_level, lbd) = self.analyze(confl, &mut learnt);
                self.cancel_until(bt_level);
                self.stats.learnts += 1;
                if learnt.len() == 1 {
                    self.enqueue(learnt[0], ClauseRef::NONE);
                } else {
                    let cr = self.db.alloc(&learnt, true, lbd);
                    self.learnts.push(cr);
                    self.attach(cr);
                    self.bump_clause(cr);
                    self.enqueue(learnt[0], cr);
                }
                self.learnt_buf = learnt;
                self.order.decay();
                self.cla_inc /= CLAUSE_DECAY;
                self.restart.on_conflict(lbd);
                continue;
            }

            if self.restart.due() {
                self.cancel_until(0);
                return Search::Restart;
            }
            if self.stats.conflicts >= self.conflict_budget
                || self.stats.propagations >= self.propagation_budget
            {
                self.cancel_until(0);
                return Search::Done(SolveResult::Unknown);
            }
            if self.decision_level() == 0 && !self.simplify() {
                return Search::Done(SolveResult::Unsat);
            }
            if self.stats.conflicts >= self.next_reduce {
                self.reduce_db();
            }

            // Assumptions first, then a heuristic decision.
            let mut next = None;
            while (self.decision_level() as usize) < self.assumptions.len() {
                let p = self.assumptions[self.decision_level() as usize];
                match lit_value(&self.assigns, p) {
                    TRUE => self.new_decision_level(),
                    FALSE => {
                        self.analyze_final(p);
                        return Search::Done(SolveResult::Unsat);
                    }
                    _ => {
                        next = Some(p);
                        break;
                    }
                }
            }
            let next = match next {
                Some(p) => p,
                None => {
                    self.stats.decisions += 1;
                    match self.pick_branch_lit() {
                        Some(p) => p,
                        None => return Search::Done(SolveResult::Sat),
                    }
                }
            };
            self.new_decision_level();
            self.enqueue(next, ClauseRef::NONE);
        }
    }

    // ----- clause database ---------------------------------------------

    fn attach(&mut self, cr: ClauseRef) {
        let c0 = self.db.lit(cr, 0);
        let c1 = self.db.lit(cr, 1);
        self.watches[(!c0).index()].push(Watcher {
            cref: cr,
            blocker: c1,
        });
        self.watches[(!c1).index()].push(Watcher {
            cref: cr,
            blocker: c0,
        });
    }

    fn bump_clause(&mut self, cr: ClauseRef) {
        let a = self.db.activity(cr) + self.cla_inc;
        self.db.set_activity(cr, a);
        if a > 1e20 {
            for &l in &self.learnts {
                let a = self.db.activity(l) * 1e-20;
                self.db.set_activity(l, a);
            }
            self.cla_inc *= 1e-20;
        }
    }

    /// True if `cr` is the reason for its first literal's assignment.
    fn locked(&self, cr: ClauseRef) -> bool {
        let c0 = self.db.lit(cr, 0);
        lit_value(&self.assigns, c0) == TRUE && self.reason[c0.var().idx()] == cr
    }

    /// Marks `cr` deleted. Watch lists are cleaned up in bulk afterwards by
    /// [`Solver::clean_watches`].
    fn remove_clause(&mut self, cr: ClauseRef) {
        if self.locked(cr) {
            let v = self.db.lit(cr, 0).var();
            self.reason[v.idx()] = ClauseRef::NONE;
        }
        self.db.mark_deleted(cr);
    }

    /// Drops watchers of deleted clauses.
    fn clean_watches(&mut self) {
        let db = &self.db;
        for ws in &mut self.watches {
            ws.retain(|w| !db.is_deleted(w.cref));
        }
    }

    /// Compacts the arena and rebuilds the watch lists and reasons.
    fn garbage_collect(&mut self) {
        let mut clauses = std::mem::take(&mut self.clauses);
        let mut learnts = std::mem::take(&mut self.learnts);
        let new_db = self.db.compact(&mut [&mut clauses, &mut learnts]);
        for &p in &self.trail {
            let v = p.var();
            let r = self.reason[v.idx()];
            if !r.is_none() {
                self.reason[v.idx()] = self.db.forwarded(r);
            }
        }
        self.db = new_db;
        self.clauses = clauses;
        self.learnts = learnts;
        for ws in &mut self.watches {
            ws.clear();
        }
        for i in 0..self.clauses.len() {
            self.attach(self.clauses[i]);
        }
        for i in 0..self.learnts.len() {
            self.attach(self.learnts[i]);
        }
    }

    fn maybe_garbage_collect(&mut self) {
        if self.db.wasted() * 5 > self.db.used() {
            self.garbage_collect();
        }
    }

    /// Halves the learnt clause database, keeping the most useful half.
    fn reduce_db(&mut self) {
        self.stats.reductions += 1;
        self.reduce_count += 1;
        self.next_reduce = self.stats.conflicts + REDUCE_BASE + REDUCE_INC * self.reduce_count;

        let db = &self.db;
        // Worst first: high LBD, then low activity.
        self.learnts.sort_by(|&a, &b| {
            db.lbd(b)
                .cmp(&db.lbd(a))
                .then_with(|| db.activity(a).total_cmp(&db.activity(b)))
        });
        let limit = self.learnts.len() / 2;
        let mut kept = Vec::with_capacity(self.learnts.len());
        let learnts = std::mem::take(&mut self.learnts);
        for (i, cr) in learnts.into_iter().enumerate() {
            if i < limit && self.db.lbd(cr) > 2 && self.db.len(cr) > 2 && !self.locked(cr) {
                self.remove_clause(cr);
                self.stats.removed_learnts += 1;
            } else {
                kept.push(cr);
            }
        }
        self.learnts = kept;
        self.clean_watches();
        self.maybe_garbage_collect();
    }

    /// Top-level simplification: drops satisfied clauses and false literals.
    /// Returns `false` if the clause set is unsatisfiable.
    fn simplify(&mut self) -> bool {
        debug_assert_eq!(self.decision_level(), 0);
        if !self.ok || self.propagate().is_some() {
            self.ok = false;
            return false;
        }
        if self.trail.len() == self.simp_db_assigns {
            return true;
        }
        let learnts = std::mem::take(&mut self.learnts);
        let (kept, removed) = self.simplify_list(learnts);
        self.learnts = kept;
        self.stats.removed_learnts += removed;
        let clauses = std::mem::take(&mut self.clauses);
        let (kept, _) = self.simplify_list(clauses);
        self.clauses = kept;
        self.clean_watches();
        self.maybe_garbage_collect();
        self.simp_db_assigns = self.trail.len();
        true
    }

    /// Removes satisfied clauses from `list` and strips false literals from
    /// the survivors. Returns the survivors and the number removed.
    fn simplify_list(&mut self, list: Vec<ClauseRef>) -> (Vec<ClauseRef>, u64) {
        let mut kept = Vec::with_capacity(list.len());
        let mut removed = 0;
        for cr in list {
            let lits = self.db.lits(cr);
            if lits.iter().any(|&l| lit_value(&self.assigns, l) == TRUE) {
                self.remove_clause(cr);
                removed += 1;
                continue;
            }
            // At level 0 with propagation complete, an unsatisfied clause
            // has both watched literals unassigned, so compacting the false
            // literals out of the tail keeps the watches valid.
            let n_false = lits
                .iter()
                .filter(|&&l| lit_value(&self.assigns, l) == FALSE)
                .count();
            if n_false > 0 {
                let assigns = &self.assigns;
                let c = self.db.lits_mut(cr);
                debug_assert!(lit_value(assigns, c[0]) >= UNDEF);
                debug_assert!(lit_value(assigns, c[1]) >= UNDEF);
                let mut j = 0;
                for i in 0..c.len() {
                    if lit_value(assigns, c[i]) != FALSE {
                        c[j] = c[i];
                        j += 1;
                    }
                }
                self.db.shrink(cr, j);
            }
            kept.push(cr);
        }
        (kept, removed)
    }
}
