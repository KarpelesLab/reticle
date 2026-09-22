//! Sequence automata: a nondeterministic automaton over boolean predicates.
//!
//! A sequence describes a finite word over clock cycles: every letter of the
//! word is one cycle, and a letter is *accepted* when a boolean predicate
//! over the sampled net values holds in that cycle. Compiling a sequence to
//! an [`Nfa`] and stepping the live state set once per clocking event is how
//! a simulator evaluates SVA: one attempt is one [`Run`], many attempts step
//! over the same automaton in lockstep, and the automaton itself is built
//! once.
//!
//! # Transitions
//!
//! An edge is either an *epsilon* move (taken without consuming a cycle) or
//! a *consuming* move guarded by a conjunction of [`Lit`]s, each a predicate
//! or its negation. An empty conjunction is `true`, which is the letter that
//! `##n` delays are made of. Predicates are referenced by [`PredId`]; the
//! caller evaluates them all once per cycle and passes the resulting
//! `&[bool]` to [`Nfa::step`], so a predicate shared by several sequences is
//! evaluated once.
//!
//! Keeping negation in the guard rather than in the predicate table is what
//! makes goto repetition (`b[->n]`, "wait for `b` while `!b`") a small
//! machine instead of a second predicate table.
//!
//! # Constructions
//!
//! | Source form                     | Constructor                    |
//! |---------------------------------|--------------------------------|
//! | `b`                             | [`Nfa::predicate`]             |
//! | `s1 ##n s2`, `s1 ##[m:n] s2`    | [`Nfa::concat`]                |
//! | `##n s`, `##[m:n] s`            | [`Nfa::lead`]                  |
//! | `s[*n]`, `s[*m:n]`, `s[*m:$]`   | [`Nfa::repeat`]                |
//! | `b[->n]`, `b[->m:n]`            | [`Nfa::goto`]                  |
//! | `s1 or s2`                      | [`Nfa::or`]                    |
//! | `s1 and s2`                     | [`Nfa::and`]                   |
//! | `s1 intersect s2`               | [`Nfa::intersect`]             |
//! | `b throughout s`                | [`Nfa::throughout`]            |
//! | `s1 within s2`                  | [`Nfa::within`]                |
//!
//! `##0` (fusion, the last cycle of the left sequence being the first cycle
//! of the right one) is handled by merging the guards of the edges that
//! enter the left automaton's accepting state with the guards of the edges
//! that leave the right one's start, which is why [`Nfa::concat`] accepts a
//! minimum of zero.
//!
//! `and` and `intersect` are one product construction: a product state is a
//! pair of component states in which either side may already be done. `and`
//! lets the side that matched first wait for the other (the match ends at
//! the later end point); `intersect` requires both to end together. `within`
//! is `intersect` against the left sequence padded with `true[*]` on both
//! sides.
//!
//! Every sequence built here consumes at least one cycle: empty matches are
//! rejected by the parser (`[*0]`), so no construction has to cope with a
//! start state that already accepts.

use std::collections::{BTreeMap, BTreeSet};

/// Index of a boolean predicate in the checker's predicate table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PredId(pub u32);

impl PredId {
    /// The table index this id addresses.
    pub fn idx(self) -> usize {
        // `u32` always fits `usize` on supported targets.
        self.0 as usize
    }
}

/// One literal of a transition guard: a predicate or its negation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Lit {
    /// The predicate tested.
    pub pred: PredId,
    /// True when the transition needs the predicate to be false.
    pub negated: bool,
}

impl Lit {
    /// The literal `pred`.
    pub fn positive(pred: PredId) -> Lit {
        Lit {
            pred,
            negated: false,
        }
    }

    /// The literal `!pred`.
    pub fn negative(pred: PredId) -> Lit {
        Lit {
            pred,
            negated: true,
        }
    }

    /// Whether the literal holds under `values`, the value of every
    /// predicate in one cycle. A predicate outside `values` reads as false.
    pub fn holds(self, values: &[bool]) -> bool {
        values.get(self.pred.idx()).copied().unwrap_or(false) != self.negated
    }
}

/// A transition of the automaton.
#[derive(Clone, Debug)]
struct Edge {
    /// `None` is an epsilon move. `Some(lits)` consumes one cycle and is
    /// taken when every literal holds; an empty conjunction is `true`.
    guard: Option<Vec<Lit>>,
    /// The state entered.
    target: usize,
}

/// The live states of one evaluation attempt.
///
/// A run is always epsilon-closed: [`Nfa::start_run`] and [`Nfa::step`]
/// close it, so [`Nfa::accepts`] is a membership test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    live: BTreeSet<u32>,
}

impl Run {
    /// The live states, ascending. Useful for tests and debugging.
    pub fn states(&self) -> impl Iterator<Item = u32> + '_ {
        self.live.iter().copied()
    }

    /// True when no state is live: the attempt can never match.
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

/// A nondeterministic automaton whose consuming transitions are one clock
/// cycle each.
///
/// See the [module docs](self) for the constructions and their meaning.
#[derive(Clone, Debug)]
pub struct Nfa {
    states: Vec<Vec<Edge>>,
    start: usize,
    accept: usize,
}

/// Turns a state index into the `u32` a run set stores.
fn sid(i: usize) -> u32 {
    u32::try_from(i).expect("sequence automaton exceeds u32 states")
}

impl Nfa {
    /// An automaton with no states; `start` and `accept` are fixed up by
    /// the caller.
    fn blank() -> Nfa {
        Nfa {
            states: Vec::new(),
            start: 0,
            accept: 0,
        }
    }

    /// Adds a state and returns its index.
    fn add_state(&mut self) -> usize {
        self.states.push(Vec::new());
        self.states.len() - 1
    }

    /// Adds a consuming edge guarded by `lits`.
    fn add_edge(&mut self, from: usize, lits: Vec<Lit>, to: usize) {
        self.states[from].push(Edge {
            guard: Some(lits),
            target: to,
        });
    }

    /// Adds an epsilon edge.
    fn eps(&mut self, from: usize, to: usize) {
        self.states[from].push(Edge {
            guard: None,
            target: to,
        });
    }

    /// Copies `other`'s states in, returning its start and accept in the
    /// new numbering.
    fn absorb(&mut self, other: &Nfa) -> (usize, usize) {
        let off = self.states.len();
        for st in &other.states {
            let edges = st
                .iter()
                .map(|e| Edge {
                    guard: e.guard.clone(),
                    target: e.target + off,
                })
                .collect();
            self.states.push(edges);
        }
        (other.start + off, other.accept + off)
    }

    /// The number of states, for tests and diagnostics.
    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    /// Every state reachable from `set` through epsilon edges alone.
    fn closure(&self, set: impl IntoIterator<Item = usize>) -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        let mut stack: Vec<usize> = Vec::new();
        for s in set {
            if out.insert(sid(s)) {
                stack.push(s);
            }
        }
        while let Some(s) = stack.pop() {
            for e in &self.states[s] {
                if e.guard.is_none() && out.insert(sid(e.target)) {
                    stack.push(e.target);
                }
            }
        }
        out
    }

    /// The consuming edges leaving the epsilon closure of `s`.
    fn out_edges(&self, s: usize) -> Vec<(Vec<Lit>, usize)> {
        let mut out = Vec::new();
        for q in self.closure([s]) {
            for e in &self.states[q as usize] {
                if let Some(g) = &e.guard {
                    out.push((g.clone(), e.target));
                }
            }
        }
        out
    }

    /// True when the accepting state is epsilon-reachable from `s`.
    fn is_accepting(&self, s: usize) -> bool {
        self.closure([s]).contains(&sid(self.accept))
    }

    // ---- running ----

    /// The state of a fresh attempt, before the first cycle is consumed.
    pub fn start_run(&self) -> Run {
        Run {
            live: self.closure([self.start]),
        }
    }

    /// Advances every live state by one cycle under `values`, the value of
    /// each predicate in this cycle.
    pub fn step(&self, run: &Run, values: &[bool]) -> Run {
        let mut next: Vec<usize> = Vec::new();
        for s in &run.live {
            for e in &self.states[*s as usize] {
                if let Some(lits) = &e.guard
                    && lits.iter().all(|l| l.holds(values))
                {
                    next.push(e.target);
                }
            }
        }
        Run {
            live: self.closure(next),
        }
    }

    /// True when a match of the sequence ends in the cycle `run` describes.
    pub fn accepts(&self, run: &Run) -> bool {
        run.live.contains(&sid(self.accept))
    }

    /// True when no continuation can ever match.
    pub fn is_dead(&self, run: &Run) -> bool {
        run.live.is_empty()
    }

    // ---- constructions ----

    /// The one-cycle sequence `b`.
    pub fn predicate(p: PredId) -> Nfa {
        Nfa::letter(vec![Lit::positive(p)])
    }

    /// A one-cycle sequence whose letter is the conjunction `lits`.
    fn letter(lits: Vec<Lit>) -> Nfa {
        let mut n = Nfa::blank();
        let s = n.add_state();
        let a = n.add_state();
        n.add_edge(s, lits, a);
        n.start = s;
        n.accept = a;
        n
    }

    /// `true^k` for `k` in `[lo, hi]`, `hi` of `None` meaning `$`.
    ///
    /// This is the only construction that may match the empty word; it is
    /// always spliced between two sequences with epsilon edges, never used
    /// on its own.
    fn gap(lo: u32, hi: Option<u32>) -> Nfa {
        let mut n = Nfa::blank();
        let start = n.add_state();
        let mut cur = start;
        for _ in 0..lo {
            let nx = n.add_state();
            n.add_edge(cur, Vec::new(), nx);
            cur = nx;
        }
        let acc = n.add_state();
        n.eps(cur, acc);
        match hi {
            None => n.add_edge(cur, Vec::new(), cur),
            Some(h) => {
                for _ in 0..h.saturating_sub(lo) {
                    let nx = n.add_state();
                    n.add_edge(cur, Vec::new(), nx);
                    n.eps(nx, acc);
                    cur = nx;
                }
            }
        }
        n.start = start;
        n.accept = acc;
        n
    }

    /// `lhs ##[min:max] rhs`, with `max` of `None` meaning `$`.
    ///
    /// A `min` of zero is the fusion `##0`: the last cycle of `lhs` and the
    /// first cycle of `rhs` are the same cycle.
    pub fn concat(lhs: &Nfa, rhs: &Nfa, min: u32, max: Option<u32>) -> Nfa {
        if min > 0 {
            return Nfa::delayed(lhs, rhs, min, max);
        }
        let fused = Nfa::fuse(lhs, rhs);
        match max {
            Some(0) => fused,
            _ => Nfa::or(&fused, &Nfa::delayed(lhs, rhs, 1, max)),
        }
    }

    /// `lhs ##[min:max] rhs` for `min >= 1`.
    fn delayed(lhs: &Nfa, rhs: &Nfa, min: u32, max: Option<u32>) -> Nfa {
        let gap = Nfa::gap(min - 1, max.map(|m| m.saturating_sub(1)));
        let mut n = Nfa::blank();
        let (ls, la) = n.absorb(lhs);
        let (gs, ga) = n.absorb(&gap);
        let (rs, ra) = n.absorb(rhs);
        n.eps(la, gs);
        n.eps(ga, rs);
        n.start = ls;
        n.accept = ra;
        n
    }

    /// `lhs ##0 rhs`.
    fn fuse(lhs: &Nfa, rhs: &Nfa) -> Nfa {
        let mut n = Nfa::blank();
        let (ls, la) = n.absorb(lhs);
        let (rs, ra) = n.absorb(rhs);
        let mut finals: Vec<(usize, Vec<Lit>)> = Vec::new();
        for i in 0..lhs.states.len() {
            let s = i + ls;
            for e in n.states[s].clone() {
                if let Some(g) = e.guard
                    && n.closure([e.target]).contains(&sid(la))
                {
                    finals.push((s, g));
                }
            }
        }
        let inits = n.out_edges(rs);
        for (src, g1) in &finals {
            for (g2, dst) in &inits {
                n.add_edge(*src, merge_guards(g1, g2), *dst);
            }
        }
        n.start = ls;
        n.accept = ra;
        n
    }

    /// A leading delay, `##[min:max] seq`.
    pub fn lead(seq: &Nfa, min: u32, max: Option<u32>) -> Nfa {
        let gap = Nfa::gap(min, max);
        let mut n = Nfa::blank();
        let (gs, ga) = n.absorb(&gap);
        let (ss, sa) = n.absorb(seq);
        n.eps(ga, ss);
        n.start = gs;
        n.accept = sa;
        n
    }

    /// `seq[*min:max]`, consecutive repetition; `max` of `None` is `$`.
    ///
    /// # Panics
    ///
    /// Panics when `min` is zero: an empty match has no automaton here.
    pub fn repeat(seq: &Nfa, min: u32, max: Option<u32>) -> Nfa {
        assert!(min > 0, "consecutive repetition needs at least one copy");
        let mut n = Nfa::blank();
        let (start, mut cur) = n.absorb(seq);
        for _ in 1..min {
            let (s, a) = n.absorb(seq);
            n.eps(cur, s);
            cur = a;
        }
        let acc = n.add_state();
        n.eps(cur, acc);
        match max {
            None => {
                let (s, a) = n.absorb(seq);
                n.eps(cur, s);
                n.eps(a, s);
                n.eps(a, acc);
            }
            Some(m) => {
                for _ in min..m {
                    let (s, a) = n.absorb(seq);
                    n.eps(cur, s);
                    n.eps(a, acc);
                    cur = a;
                }
            }
        }
        n.start = start;
        n.accept = acc;
        n
    }

    /// `b[->min:max]`, goto repetition: the match ends on the `min`-th (up
    /// to `max`-th) cycle in which `b` holds, with any number of cycles in
    /// which it does not in between.
    ///
    /// # Panics
    ///
    /// Panics when `min` is zero.
    pub fn goto(p: PredId, min: u32, max: Option<u32>) -> Nfa {
        assert!(min > 0, "goto repetition needs at least one occurrence");
        let pos = vec![Lit::positive(p)];
        let neg = vec![Lit::negative(p)];
        let last = match max {
            Some(m) => m.saturating_sub(1),
            None => min,
        };
        let mut n = Nfa::blank();
        let waiting: Vec<usize> = (0..=last).map(|_| n.add_state()).collect();
        let acc = n.add_state();
        for (i, &st) in waiting.iter().enumerate() {
            let seen = sid(i) + 1;
            n.add_edge(st, neg.clone(), st);
            match waiting.get(i + 1) {
                Some(&nx) => n.add_edge(st, pos.clone(), nx),
                None => {
                    if max.is_none() {
                        n.add_edge(st, pos.clone(), st);
                    }
                }
            }
            if seen >= min {
                n.add_edge(st, pos.clone(), acc);
            }
        }
        n.start = waiting[0];
        n.accept = acc;
        n
    }

    /// `lhs or rhs`: either sequence matching is a match.
    pub fn or(lhs: &Nfa, rhs: &Nfa) -> Nfa {
        let mut n = Nfa::blank();
        let s = n.add_state();
        let (ls, la) = n.absorb(lhs);
        let (rs, ra) = n.absorb(rhs);
        let acc = n.add_state();
        n.eps(s, ls);
        n.eps(s, rs);
        n.eps(la, acc);
        n.eps(ra, acc);
        n.start = s;
        n.accept = acc;
        n
    }

    /// `lhs and rhs`: both match from the same cycle, the composite match
    /// ending where the later of the two ends.
    pub fn and(lhs: &Nfa, rhs: &Nfa) -> Nfa {
        Nfa::product(lhs, rhs, true)
    }

    /// `lhs intersect rhs`: both match from the same cycle and end in the
    /// same cycle.
    pub fn intersect(lhs: &Nfa, rhs: &Nfa) -> Nfa {
        Nfa::product(lhs, rhs, false)
    }

    /// `b throughout seq`: `seq` matches and `b` holds in every one of its
    /// cycles, which is exactly `b` conjoined onto every guard.
    pub fn throughout(p: PredId, seq: &Nfa) -> Nfa {
        let lit = Lit::positive(p);
        let mut n = seq.clone();
        for st in &mut n.states {
            for e in st {
                if let Some(g) = &mut e.guard
                    && !g.contains(&lit)
                {
                    g.push(lit);
                    g.sort_unstable();
                }
            }
        }
        n
    }

    /// `lhs within rhs`: `lhs` matches somewhere inside a match of `rhs`.
    pub fn within(lhs: &Nfa, rhs: &Nfa) -> Nfa {
        Nfa::intersect(&Nfa::pad(lhs), rhs)
    }

    /// `true[*0:$] ##1 seq ##1 true[*0:$]`.
    fn pad(seq: &Nfa) -> Nfa {
        let mut n = Nfa::blank();
        let s = n.add_state();
        n.add_edge(s, Vec::new(), s);
        let (ss, sa) = n.absorb(seq);
        n.eps(s, ss);
        let f = n.add_state();
        n.eps(sa, f);
        n.add_edge(f, Vec::new(), f);
        n.start = s;
        n.accept = f;
        n
    }

    /// The product used by [`Nfa::and`] (`wait` true) and
    /// [`Nfa::intersect`] (`wait` false).
    ///
    /// A product state pairs a state of each side, where `None` means that
    /// side has already matched. With `wait`, a side that matched may sit
    /// on `None` while the other one runs on; without it, the two sides
    /// must reach `None` in the same cycle.
    fn product(lhs: &Nfa, rhs: &Nfa, wait: bool) -> Nfa {
        let mut n = Nfa::blank();
        let mut ids: BTreeMap<ProductState, usize> = BTreeMap::new();
        let done: ProductState = (None, None);
        let start: ProductState = (Some(lhs.start), Some(rhs.start));
        ids.insert(done, n.add_state());
        ids.insert(start, n.add_state());
        let mut queue = vec![start];
        while let Some(key) = queue.pop() {
            let succ = product_successors(lhs, rhs, wait, key, done);
            let from = ids[&key];
            for (guard, to) in succ {
                let target = match ids.get(&to) {
                    Some(&id) => id,
                    None => {
                        let id = n.add_state();
                        ids.insert(to, id);
                        queue.push(to);
                        id
                    }
                };
                n.add_edge(from, guard, target);
            }
        }
        n.start = ids[&start];
        n.accept = ids[&done];
        n
    }
}

/// A state of the product automaton: one state per side, `None` once that
/// side has matched.
type ProductState = (Option<usize>, Option<usize>);

/// The union of two guards, sorted so equal guards compare equal.
fn merge_guards(a: &[Lit], b: &[Lit]) -> Vec<Lit> {
    let mut out = a.to_vec();
    out.extend_from_slice(b);
    out.sort_unstable();
    out.dedup();
    out
}

/// The successors of one product state; see [`Nfa::product`].
fn product_successors(
    lhs: &Nfa,
    rhs: &Nfa,
    wait: bool,
    key: ProductState,
    done: ProductState,
) -> Vec<(Vec<Lit>, ProductState)> {
    let mut succ: Vec<(Vec<Lit>, ProductState)> = Vec::new();
    match key {
        (Some(i), Some(j)) => {
            let right = rhs.out_edges(j);
            for (g1, t1) in lhs.out_edges(i) {
                for (g2, t2) in &right {
                    let guard = merge_guards(&g1, g2);
                    let da = lhs.is_accepting(t1);
                    let db = rhs.is_accepting(*t2);
                    succ.push((guard.clone(), (Some(t1), Some(*t2))));
                    if wait {
                        if da {
                            succ.push((guard.clone(), (None, Some(*t2))));
                        }
                        if db {
                            succ.push((guard.clone(), (Some(t1), None)));
                        }
                    }
                    if da && db {
                        succ.push((guard, done));
                    }
                }
            }
        }
        (None, Some(j)) => {
            for (g, t) in rhs.out_edges(j) {
                if rhs.is_accepting(t) {
                    succ.push((g.clone(), done));
                }
                succ.push((g, (None, Some(t))));
            }
        }
        (Some(i), None) => {
            for (g, t) in lhs.out_edges(i) {
                if lhs.is_accepting(t) {
                    succ.push((g.clone(), done));
                }
                succ.push((g, (Some(t), None)));
            }
        }
        (None, None) => {}
    }
    succ
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PredId = PredId(0);
    const B: PredId = PredId(1);
    const C: PredId = PredId(2);

    /// Cycle values for three predicates, one character each: `a`, `b` and
    /// `c` set that predicate alone, `.` sets none, `*` sets all, and the
    /// upper-case letters set the two predicates other than their own.
    fn trace(text: &str) -> Vec<[bool; 3]> {
        text.chars()
            .map(|c| match c {
                '.' => [false, false, false],
                '*' => [true, true, true],
                'a' => [true, false, false],
                'b' => [false, true, false],
                'c' => [false, false, true],
                'A' => [false, true, true],
                'B' => [true, false, true],
                'C' => [true, true, false],
                other => panic!("bad trace character `{other}`"),
            })
            .collect()
    }

    /// The 1-based cycles at which a single attempt started at cycle 0
    /// reaches the accepting state.
    fn ends(nfa: &Nfa, text: &str) -> Vec<usize> {
        let mut run = nfa.start_run();
        let mut out = Vec::new();
        for (i, values) in trace(text).iter().enumerate() {
            run = nfa.step(&run, values);
            if nfa.accepts(&run) {
                out.push(i + 1);
            }
        }
        out
    }

    fn none() -> Vec<usize> {
        Vec::new()
    }

    fn p(id: PredId) -> Nfa {
        Nfa::predicate(id)
    }

    #[test]
    fn single_letter() {
        let n = p(A);
        assert_eq!(ends(&n, "a..."), vec![1]);
        assert_eq!(ends(&n, ".aaa"), none());
        assert!(n.is_dead(&n.step(&n.start_run(), &[false, false, false])));
    }

    #[test]
    fn delay_fixed_and_range() {
        // a ##2 b
        let n = Nfa::concat(&p(A), &p(B), 2, Some(2));
        assert_eq!(ends(&n, "a.b."), vec![3]);
        assert_eq!(ends(&n, "ab.."), none());
        // a ##[1:3] b
        let n = Nfa::concat(&p(A), &p(B), 1, Some(3));
        assert_eq!(ends(&n, "abbb"), vec![2, 3, 4]);
        assert_eq!(ends(&n, "a..b"), vec![4]);
        assert_eq!(ends(&n, "a...b"), none());
        // a ##[2:$] b
        let n = Nfa::concat(&p(A), &p(B), 2, None);
        assert_eq!(ends(&n, "ab.b..b"), vec![4, 7]);
    }

    #[test]
    fn fusion() {
        // a ##0 b: one cycle in which both hold.
        let n = Nfa::concat(&p(A), &p(B), 0, Some(0));
        assert_eq!(ends(&n, "C..."), vec![1]);
        assert_eq!(ends(&n, "ab.."), none());
        // a ##[0:1] b
        let n = Nfa::concat(&p(A), &p(B), 0, Some(1));
        assert_eq!(ends(&n, "C..."), vec![1]);
        assert_eq!(ends(&n, "ab.."), vec![2]);
        // Fusion of a longer left sequence: (a ##1 b) ##0 c
        let ab = Nfa::concat(&p(A), &p(B), 1, Some(1));
        let n = Nfa::concat(&ab, &p(C), 0, Some(0));
        assert_eq!(ends(&n, "aA.."), vec![2]);
        assert_eq!(ends(&n, "ab.."), none());
    }

    #[test]
    fn leading_delay() {
        // ##2 b
        let n = Nfa::lead(&p(B), 2, Some(2));
        assert_eq!(ends(&n, "..b."), vec![3]);
        assert_eq!(ends(&n, ".b.."), none());
        // ##[0:2] b
        let n = Nfa::lead(&p(B), 0, Some(2));
        assert_eq!(ends(&n, "bbbb"), vec![1, 2, 3]);
    }

    #[test]
    fn consecutive_repetition() {
        let n = Nfa::repeat(&p(A), 3, Some(3));
        assert_eq!(ends(&n, "aaaa"), vec![3]);
        assert_eq!(ends(&n, "aa.a"), none());
        let n = Nfa::repeat(&p(A), 2, Some(4));
        assert_eq!(ends(&n, "aaaaa"), vec![2, 3, 4]);
        let n = Nfa::repeat(&p(A), 2, None);
        assert_eq!(ends(&n, "aaaaa"), vec![2, 3, 4, 5]);
        // A repeated sequence, not just a letter: (a ##1 b)[*2]
        let ab = Nfa::concat(&p(A), &p(B), 1, Some(1));
        let n = Nfa::repeat(&ab, 2, Some(2));
        assert_eq!(ends(&n, "abab"), vec![4]);
        assert_eq!(ends(&n, "ab.ab"), none());
    }

    #[test]
    fn goto_repetition() {
        let n = Nfa::goto(B, 1, Some(1));
        assert_eq!(ends(&n, "..b."), vec![3]);
        let n = Nfa::goto(B, 2, Some(2));
        assert_eq!(ends(&n, "b..b.b"), vec![4]);
        let n = Nfa::goto(B, 2, Some(3));
        assert_eq!(ends(&n, "b..b.b"), vec![4, 6]);
        let n = Nfa::goto(B, 2, None);
        assert_eq!(ends(&n, "b..bbb"), vec![4, 5, 6]);
        // Consecutive occurrences count too.
        let n = Nfa::goto(B, 3, Some(3));
        assert_eq!(ends(&n, "bbb"), vec![3]);
    }

    #[test]
    fn choice() {
        let n = Nfa::or(&p(A), &Nfa::concat(&p(B), &p(C), 1, Some(1)));
        assert_eq!(ends(&n, "a..."), vec![1]);
        assert_eq!(ends(&n, "bc.."), vec![2]);
        assert_eq!(ends(&n, ".c.."), none());
    }

    #[test]
    fn conjunction_waits_for_the_longer_side() {
        // a and (a ##2 c): ends where the longer one ends.
        let long = Nfa::concat(&p(A), &p(C), 2, Some(2));
        let n = Nfa::and(&p(A), &long);
        assert_eq!(ends(&n, "a.c."), vec![3]);
        // The short side must still match at cycle 1.
        assert_eq!(ends(&n, "..c."), none());
        // `intersect` needs equal lengths, so the same pair never matches.
        let n = Nfa::intersect(&p(A), &long);
        assert_eq!(ends(&n, "a.c."), none());
    }

    #[test]
    fn intersect_equal_length() {
        // a[*3] intersect (a ##2 c): the third cycle needs both a and c.
        let left = Nfa::repeat(&p(A), 3, Some(3));
        let right = Nfa::concat(&p(A), &p(C), 2, Some(2));
        let n = Nfa::intersect(&left, &right);
        assert_eq!(ends(&n, "aaB"), vec![3]);
        assert_eq!(ends(&n, "aac"), none());
    }

    #[test]
    fn throughout_and_within() {
        // a throughout (b ##2 c)
        let seq = Nfa::concat(&p(B), &p(C), 2, Some(2));
        let n = Nfa::throughout(A, &seq);
        assert_eq!(ends(&n, "CaB"), vec![3]);
        assert_eq!(ends(&n, "C.B"), none());
        // b within (a ##3 c): b in one of the four cycles.
        let outer = Nfa::concat(&p(A), &p(C), 3, Some(3));
        let n = Nfa::within(&p(B), &outer);
        assert_eq!(ends(&n, "aCb.c"), none());
        assert_eq!(ends(&n, "ab.c"), vec![4]);
        assert_eq!(ends(&n, "a..c"), none());
    }

    #[test]
    fn overlapping_attempts_in_lockstep() {
        // The classic: `##2 b` restarted every cycle. Several attempts are
        // live at once and each must report its own end.
        let n = Nfa::lead(&p(B), 2, Some(2));
        let values = trace("..b.b.");
        let mut runs: Vec<(usize, Run)> = Vec::new();
        let mut hits = Vec::new();
        for (cycle, v) in values.iter().enumerate() {
            runs.push((cycle, n.start_run()));
            let mut keep = Vec::new();
            for (start, run) in runs.drain(..) {
                let next = n.step(&run, v);
                if n.accepts(&next) {
                    hits.push((start, cycle));
                } else if !n.is_dead(&next) {
                    keep.push((start, next));
                }
            }
            runs = keep;
        }
        // Attempts starting at cycles 0 and 2 match at cycles 2 and 4.
        assert_eq!(hits, vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn literal_helpers() {
        let values = [true, false];
        assert!(Lit::positive(PredId(0)).holds(&values));
        assert!(!Lit::positive(PredId(1)).holds(&values));
        assert!(Lit::negative(PredId(1)).holds(&values));
        // An out-of-range predicate reads as false rather than panicking.
        assert!(!Lit::positive(PredId(7)).holds(&values));
        assert_eq!(PredId(3).idx(), 3);
        let n = Nfa::predicate(PredId(0));
        assert_eq!(n.state_count(), 2);
        assert_eq!(n.start_run().states().count(), 1);
        assert!(!n.start_run().is_empty());
    }
}
