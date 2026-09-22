//! Unit propagation with two watched literals and blockers.
//!
//! Every clause of length ≥ 2 keeps its first two literals *watched*: the
//! clause appears in the watch list of the negation of each. The invariant
//! maintained is that a watched literal is only false if the clause is
//! satisfied by its other watch, or the clause is unit/conflicting and the
//! solver has already noticed. When a literal `p` becomes true, only the
//! clauses watching `!p` are visited; each either finds a new non-false
//! literal to watch (and leaves `p`'s list), is satisfied (stays), becomes
//! unit (stays, enqueue) or is conflicting (stays, stop).
//!
//! Each watch entry carries a *blocker*, some other literal of the clause.
//! If the blocker is true the entry is kept without dereferencing the
//! clause, which is the common case and a large cache saving.
//!
//! Backtracking needs no work on the watch lists: false watched literals
//! simply become unassigned again.

use super::clause::ClauseRef;
use super::{FALSE, Solver, TRUE, Watcher, lit_value};

impl Solver {
    /// Propagates every enqueued literal. Returns a conflicting clause, if
    /// any; on conflict the queue is left drained.
    pub(super) fn propagate(&mut self) -> Option<ClauseRef> {
        let mut confl = None;
        while self.qhead < self.trail.len() {
            let p = self.trail[self.qhead];
            self.qhead += 1;
            self.stats.propagations += 1;
            let false_lit = !p;

            // Take the list out so the clause arena and the other lists can
            // be borrowed independently. No clause has duplicate literals,
            // so a new watch never lands back in this list.
            let mut ws = std::mem::take(&mut self.watches[p.index()]);
            let n = ws.len();
            let mut i = 0;
            let mut j = 0;
            'next: while i < n {
                let w = ws[i];
                i += 1;
                if lit_value(&self.assigns, w.blocker) == TRUE {
                    ws[j] = w;
                    j += 1;
                    continue;
                }

                let cr = w.cref;
                let c = self.db.lits_mut(cr);
                if c[0] == false_lit {
                    c.swap(0, 1);
                }
                debug_assert_eq!(c[1], false_lit);
                let first = c[0];
                let w2 = Watcher {
                    cref: cr,
                    blocker: first,
                };
                if first != w.blocker && lit_value(&self.assigns, first) == TRUE {
                    ws[j] = w2;
                    j += 1;
                    continue;
                }

                for k in 2..c.len() {
                    if lit_value(&self.assigns, c[k]) != FALSE {
                        c[1] = c[k];
                        c[k] = false_lit;
                        let new_watch = !c[1];
                        self.watches[new_watch.index()].push(w2);
                        continue 'next;
                    }
                }

                // Unit or conflicting: the watch stays.
                ws[j] = w2;
                j += 1;
                if lit_value(&self.assigns, first) == FALSE {
                    confl = Some(cr);
                    self.qhead = self.trail.len();
                    while i < n {
                        ws[j] = ws[i];
                        j += 1;
                        i += 1;
                    }
                } else {
                    self.enqueue(first, cr);
                }
            }
            ws.truncate(j);
            self.watches[p.index()] = ws;
        }
        confl
    }
}
