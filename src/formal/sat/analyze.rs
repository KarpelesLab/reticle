//! Conflict analysis: first-UIP learning, clause minimisation, LBD, and
//! the final analysis that extracts an assumption core.
//!
//! Starting from the conflicting clause, literals of the current decision
//! level are resolved away in reverse trail order using their reason
//! clauses until exactly one remains: the first unique implication point.
//! Literals from lower levels are collected into the learnt clause as they
//! are met. The result is asserting: after backjumping to the highest level
//! among its other literals it propagates the negation of the UIP.
//!
//! Minimisation then removes every literal whose reason clause consists
//! only of literals already in the learnt clause, transitively. The
//! recursion is done on an explicit stack, and an *abstract level* bitmask
//! (one bit per `level % 32`) rejects most candidates cheaply.

use super::clause::ClauseRef;
use super::{Lit, Solver, Var};

/// Counts distinct levels using a per-level stamp array, so it is linear
/// in the clause length with no clearing.
fn lbd_of(level: &[u32], stamp_of: &mut [u64], stamp: u64, lits: impl Iterator<Item = Lit>) -> u32 {
    let mut lbd = 0;
    for l in lits {
        let level = level[l.var().idx()] as usize;
        if stamp_of[level] != stamp {
            stamp_of[level] = stamp;
            lbd += 1;
        }
    }
    lbd
}

impl Solver {
    fn abstract_level(&self, v: Var) -> u32 {
        1 << (self.level[v.idx()] & 31)
    }

    /// Learns a clause from the conflict `confl`, writing it to `learnt`
    /// with the asserting literal first. Returns the level to backjump to
    /// and the clause's LBD.
    pub(super) fn analyze(&mut self, confl: ClauseRef, learnt: &mut Vec<Lit>) -> (u32, u32) {
        learnt.clear();
        learnt.push(Lit::from_raw(0)); // placeholder for the asserting literal
        let current = self.decision_level();
        let mut path_count = 0usize;
        let mut p: Option<Lit> = None;
        let mut index = self.trail.len();
        let mut confl = confl;

        loop {
            debug_assert!(!confl.is_none(), "conflict analysis reached a decision");
            if self.db.is_learnt(confl) {
                self.bump_clause(confl);
                // Glucose's dynamic LBD update: a clause whose literals now
                // span fewer levels than when it was learnt is more useful
                // than its recorded LBD says.
                let old = self.db.lbd(confl);
                if old > 2 {
                    let new = self.clause_lbd(confl);
                    if new + 1 < old {
                        self.db.set_lbd(confl, new);
                    }
                }
            }
            let start = usize::from(p.is_some());
            for i in start..self.db.len(confl) {
                let q = self.db.lit(confl, i);
                let v = q.var();
                if !self.seen[v.idx()] && self.level[v.idx()] > 0 {
                    self.order.bump(v);
                    self.seen[v.idx()] = true;
                    if self.level[v.idx()] >= current {
                        path_count += 1;
                    } else {
                        learnt.push(q);
                    }
                }
            }
            // Next literal of the current level on the trail, walking back.
            loop {
                index -= 1;
                if self.seen[self.trail[index].var().idx()] {
                    break;
                }
            }
            let q = self.trail[index];
            p = Some(q);
            confl = self.reason[q.var().idx()];
            self.seen[q.var().idx()] = false;
            path_count -= 1;
            if path_count == 0 {
                break;
            }
        }
        learnt[0] = !p.expect("at least one resolution step");

        // Minimise: drop literals implied by the rest of the clause.
        self.analyze_toclear.clear();
        self.analyze_toclear.extend_from_slice(learnt);
        self.stats.max_literals += learnt.len() as u64;
        let abstract_levels = learnt[1..]
            .iter()
            .fold(0u32, |acc, l| acc | self.abstract_level(l.var()));
        let mut j = 1;
        for i in 1..learnt.len() {
            let l = learnt[i];
            if self.reason[l.var().idx()].is_none() || !self.lit_redundant(l, abstract_levels) {
                learnt[j] = l;
                j += 1;
            }
        }
        learnt.truncate(j);
        self.stats.tot_literals += learnt.len() as u64;

        // Backjump level: the highest level among the other literals, moved
        // to position 1 so it is watched.
        let bt_level = if learnt.len() == 1 {
            0
        } else {
            let mut max_i = 1;
            for i in 2..learnt.len() {
                if self.level[learnt[i].var().idx()] > self.level[learnt[max_i].var().idx()] {
                    max_i = i;
                }
            }
            learnt.swap(1, max_i);
            self.level[learnt[1].var().idx()]
        };

        let lbd = self.compute_lbd(learnt);
        for l in &self.analyze_toclear {
            self.seen[l.var().idx()] = false;
        }
        (bt_level, lbd)
    }

    /// True if `p` can be dropped from the learnt clause because its reason
    /// clause (transitively) only involves literals already in it.
    /// `seen` marks the literals of the clause; on success the literals
    /// visited stay marked (they are all redundant too), on failure the
    /// marks added here are undone.
    fn lit_redundant(&mut self, p: Lit, abstract_levels: u32) -> bool {
        self.analyze_stack.clear();
        self.analyze_stack.push(p);
        let top = self.analyze_toclear.len();
        while let Some(q) = self.analyze_stack.pop() {
            let cr = self.reason[q.var().idx()];
            debug_assert!(!cr.is_none());
            for i in 1..self.db.len(cr) {
                let l = self.db.lit(cr, i);
                let v = l.var();
                if self.seen[v.idx()] || self.level[v.idx()] == 0 {
                    continue;
                }
                if !self.reason[v.idx()].is_none() && self.abstract_level(v) & abstract_levels != 0
                {
                    self.seen[v.idx()] = true;
                    self.analyze_stack.push(l);
                    self.analyze_toclear.push(l);
                } else {
                    for k in top..self.analyze_toclear.len() {
                        self.seen[self.analyze_toclear[k].var().idx()] = false;
                    }
                    self.analyze_toclear.truncate(top);
                    return false;
                }
            }
        }
        true
    }

    /// Number of distinct decision levels among the literals.
    fn compute_lbd(&mut self, lits: &[Lit]) -> u32 {
        self.lbd_counter += 1;
        lbd_of(
            &self.level,
            &mut self.lbd_stamp,
            self.lbd_counter,
            lits.iter().copied(),
        )
    }

    /// Number of distinct decision levels among a stored clause's literals.
    fn clause_lbd(&mut self, cr: ClauseRef) -> u32 {
        self.lbd_counter += 1;
        lbd_of(
            &self.level,
            &mut self.lbd_stamp,
            self.lbd_counter,
            self.db.lits(cr).iter().copied(),
        )
    }

    /// Computes the assumptions responsible for the assumption `p` being
    /// false, into `conflict_core` (MiniSat's `analyzeFinal`).
    ///
    /// Walks the trail backwards from the first assumption level, expanding
    /// reasons; a marked literal without a reason is an assumption
    /// (assumptions are the only decisions below `assumptions.len()`).
    pub(super) fn analyze_final(&mut self, p: Lit) {
        self.conflict_core.clear();
        self.conflict_core.push(p);
        if self.decision_level() == 0 {
            return;
        }
        self.seen[p.var().idx()] = true;
        for i in (self.trail_lim[0]..self.trail.len()).rev() {
            let q = self.trail[i];
            let v = q.var();
            if !self.seen[v.idx()] {
                continue;
            }
            let cr = self.reason[v.idx()];
            if cr.is_none() {
                debug_assert!(self.level[v.idx()] > 0);
                self.conflict_core.push(q);
            } else {
                for k in 1..self.db.len(cr) {
                    let l = self.db.lit(cr, k);
                    if self.level[l.var().idx()] > 0 {
                        self.seen[l.var().idx()] = true;
                    }
                }
            }
            self.seen[v.idx()] = false;
        }
        self.seen[p.var().idx()] = false;
    }
}
