//! Branching heuristics: VSIDS variable ordering and phase saving.
//!
//! VSIDS (Variable State Independent Decaying Sum, from Chaff) keeps one
//! activity per variable, bumped for every variable that takes part in a
//! conflict analysis and decayed geometrically afterwards. The decay is
//! implemented MiniSat-style by growing the bump increment instead of
//! touching every activity, with a rescale when the increment gets large.
//! Unassigned variables are kept in a binary max-heap keyed on activity so
//! the next decision is a pop.
//!
//! Phase saving (Pipatsrisawat & Darwiche 2007) remembers the last value a
//! variable was assigned and reuses it as the decision polarity, which keeps
//! restarts from throwing away the partial assignment they interrupt.

use super::Var;

const NOT_IN_HEAP: usize = usize::MAX;

/// The variable ordering state of a solver.
pub(super) struct VarOrder {
    heap: Vec<Var>,
    pos: Vec<usize>,
    activity: Vec<f64>,
    inc: f64,
    decay: f64,
    phase: Vec<bool>,
}

impl VarOrder {
    pub(super) fn new(decay: f64) -> VarOrder {
        VarOrder {
            heap: Vec::new(),
            pos: Vec::new(),
            activity: Vec::new(),
            inc: 1.0,
            decay,
            phase: Vec::new(),
        }
    }

    /// Registers a fresh variable and puts it in the heap.
    pub(super) fn add_var(&mut self, v: Var) {
        debug_assert_eq!(v.idx(), self.activity.len());
        self.activity.push(0.0);
        self.pos.push(NOT_IN_HEAP);
        self.phase.push(false);
        self.insert(v);
    }

    /// The saved phase: the value to try first for `v`.
    pub(super) fn phase(&self, v: Var) -> bool {
        self.phase[v.idx()]
    }

    pub(super) fn save_phase(&mut self, v: Var, value: bool) {
        self.phase[v.idx()] = value;
    }

    /// Increases the activity of `v` by the current increment.
    pub(super) fn bump(&mut self, v: Var) {
        self.activity[v.idx()] += self.inc;
        if self.activity[v.idx()] > 1e100 {
            for a in &mut self.activity {
                *a *= 1e-100;
            }
            self.inc *= 1e-100;
        }
        if self.pos[v.idx()] != NOT_IN_HEAP {
            self.sift_up(self.pos[v.idx()]);
        }
    }

    /// Decays every activity (by growing the increment).
    pub(super) fn decay(&mut self) {
        self.inc /= self.decay;
    }

    pub(super) fn in_heap(&self, v: Var) -> bool {
        self.pos[v.idx()] != NOT_IN_HEAP
    }

    /// Puts `v` back in the heap if it is not already there.
    pub(super) fn insert(&mut self, v: Var) {
        if self.in_heap(v) {
            return;
        }
        let i = self.heap.len();
        self.heap.push(v);
        self.pos[v.idx()] = i;
        self.sift_up(i);
    }

    /// Pops the most active variable, or `None` when the heap is empty.
    pub(super) fn pop_max(&mut self) -> Option<Var> {
        let top = *self.heap.first()?;
        let last = self.heap.pop().expect("non-empty heap");
        self.pos[top.idx()] = NOT_IN_HEAP;
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.pos[last.idx()] = 0;
            self.sift_down(0);
        }
        Some(top)
    }

    fn act(&self, v: Var) -> f64 {
        self.activity[v.idx()]
    }

    fn sift_up(&mut self, mut i: usize) {
        let v = self.heap[i];
        while i > 0 {
            let p = (i - 1) / 2;
            let pv = self.heap[p];
            if self.act(pv) >= self.act(v) {
                break;
            }
            self.heap[i] = pv;
            self.pos[pv.idx()] = i;
            i = p;
        }
        self.heap[i] = v;
        self.pos[v.idx()] = i;
    }

    fn sift_down(&mut self, mut i: usize) {
        let v = self.heap[i];
        let n = self.heap.len();
        loop {
            let l = 2 * i + 1;
            if l >= n {
                break;
            }
            let r = l + 1;
            let child = if r < n && self.act(self.heap[r]) > self.act(self.heap[l]) {
                r
            } else {
                l
            };
            let cv = self.heap[child];
            if self.act(cv) <= self.act(v) {
                break;
            }
            self.heap[i] = cv;
            self.pos[cv.idx()] = i;
            i = child;
        }
        self.heap[i] = v;
        self.pos[v.idx()] = i;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_orders_by_activity() {
        let mut order = VarOrder::new(0.95);
        for i in 0..6 {
            order.add_var(Var::new(i));
        }
        order.bump(Var::new(3));
        order.bump(Var::new(3));
        order.decay();
        order.bump(Var::new(1));
        // One decay makes the increment 1/0.95 < 2, so 3 stays first.
        assert_eq!(order.pop_max(), Some(Var::new(3)));
        assert_eq!(order.pop_max(), Some(Var::new(1)));
        assert!(!order.in_heap(Var::new(1)));
        order.insert(Var::new(1));
        order.insert(Var::new(1));
        assert_eq!(order.pop_max(), Some(Var::new(1)));
        let mut rest = Vec::new();
        while let Some(v) = order.pop_max() {
            rest.push(v.index());
        }
        rest.sort_unstable();
        assert_eq!(rest, vec![0, 2, 4, 5]);
        assert_eq!(order.pop_max(), None);
    }

    #[test]
    fn rescale_keeps_order() {
        let mut order = VarOrder::new(0.5);
        for i in 0..3 {
            order.add_var(Var::new(i));
        }
        for _ in 0..400 {
            order.decay();
        }
        order.bump(Var::new(2));
        order.bump(Var::new(0));
        order.bump(Var::new(0));
        assert_eq!(order.pop_max(), Some(Var::new(0)));
        assert_eq!(order.pop_max(), Some(Var::new(2)));
        assert_eq!(order.pop_max(), Some(Var::new(1)));
    }
}
