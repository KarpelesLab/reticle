//! Restart policies.
//!
//! A restart throws away the current partial assignment (keeping learnt
//! clauses, activities and saved phases) so the search can escape a bad
//! prefix of decisions. Two policies are offered:
//!
//! - [`RestartPolicy::Luby`]: restart after `100 * luby(i)` conflicts, where
//!   `luby` is the Luby, Sinclair & Zuckerman sequence `1 1 2 1 1 2 4 1 1 2
//!   1 1 2 4 8 ...`. This is MiniSat 2.2's default and is robust on random
//!   and combinatorial instances.
//! - [`RestartPolicy::Glucose`]: restart when the average LBD of the last
//!   50 learnt clauses is worse than the overall average by a factor `K`
//!   (Audemard & Simon 2009). This is more aggressive and tends to help on
//!   structured (industrial) UNSAT instances.

/// How the solver decides when to restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Luby sequence with a base of 100 conflicts (MiniSat's default).
    #[default]
    Luby,
    /// LBD-average based dynamic restarts (Glucose).
    Glucose,
}

const LUBY_BASE: u64 = 100;
const GLUCOSE_QUEUE: usize = 50;
/// Glucose's `K`, scaled by 10 to stay in integers: restart when
/// `recent_avg * 0.8 > global_avg`.
const GLUCOSE_K_TENTHS: u64 = 8;
/// Minimum conflicts before Glucose restarts are considered at all, so the
/// short-term average has a chance to settle.
const GLUCOSE_MIN_CONFLICTS: u64 = 50;

/// Per-solve restart bookkeeping.
pub(super) struct RestartState {
    policy: RestartPolicy,
    /// Conflicts since the last restart.
    since_restart: u64,
    /// Luby: conflicts allowed in the current run.
    budget: u64,
    /// Glucose: ring buffer of recent LBDs.
    recent: Vec<u32>,
    recent_next: usize,
    recent_sum: u64,
    global_sum: u64,
    global_count: u64,
}

impl RestartState {
    pub(super) fn new(policy: RestartPolicy) -> RestartState {
        RestartState {
            policy,
            since_restart: 0,
            budget: LUBY_BASE,
            recent: Vec::with_capacity(GLUCOSE_QUEUE),
            recent_next: 0,
            recent_sum: 0,
            global_sum: 0,
            global_count: 0,
        }
    }

    pub(super) fn policy(&self) -> RestartPolicy {
        self.policy
    }

    pub(super) fn set_policy(&mut self, policy: RestartPolicy) {
        self.policy = policy;
    }

    /// Starts run number `run` (0-based) of a `solve` call.
    pub(super) fn begin_run(&mut self, run: u64) {
        self.since_restart = 0;
        self.budget = LUBY_BASE * luby(run);
        self.recent.clear();
        self.recent_next = 0;
        self.recent_sum = 0;
    }

    /// Records a learnt clause's LBD.
    pub(super) fn on_conflict(&mut self, lbd: u32) {
        self.since_restart += 1;
        self.global_sum += u64::from(lbd);
        self.global_count += 1;
        if self.recent.len() < GLUCOSE_QUEUE {
            self.recent.push(lbd);
        } else {
            self.recent_sum -= u64::from(self.recent[self.recent_next]);
            self.recent[self.recent_next] = lbd;
            self.recent_next = (self.recent_next + 1) % GLUCOSE_QUEUE;
        }
        self.recent_sum += u64::from(lbd);
    }

    /// Whether the current run should be abandoned.
    pub(super) fn due(&self) -> bool {
        match self.policy {
            RestartPolicy::Luby => self.since_restart >= self.budget,
            RestartPolicy::Glucose => {
                if self.since_restart < GLUCOSE_MIN_CONFLICTS
                    || self.recent.len() < GLUCOSE_QUEUE
                    || self.global_count == 0
                {
                    return false;
                }
                // recent_avg * K > global_avg, cross-multiplied:
                // recent_sum * K * global_count > global_sum * QUEUE.
                let queue = u64::try_from(GLUCOSE_QUEUE).expect("small constant");
                let lhs = self.recent_sum * GLUCOSE_K_TENTHS * self.global_count;
                let rhs = self.global_sum * queue * 10;
                lhs > rhs
            }
        }
    }
}

/// The Luby sequence value at index `i` (0-based): `1 1 2 1 1 2 4 ...`.
///
/// Each value is a power of two, so the result is exact. MiniSat's
/// formulation: find the finite subsequence containing `i` and its size,
/// then descend until `i` is the last element of a subsequence.
pub(super) fn luby(i: u64) -> u64 {
    let mut size: u64 = 1;
    let mut seq: u32 = 0;
    while size < i + 1 {
        seq += 1;
        size = 2 * size + 1;
    }
    let mut x = i;
    while size - 1 != x {
        size = (size - 1) >> 1;
        seq -= 1;
        x %= size;
    }
    1 << seq
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luby_sequence() {
        let got: Vec<u64> = (0..15).map(luby).collect();
        assert_eq!(got, vec![1, 1, 2, 1, 1, 2, 4, 1, 1, 2, 1, 1, 2, 4, 8]);
    }

    #[test]
    fn luby_budget_grows() {
        let mut r = RestartState::new(RestartPolicy::Luby);
        r.begin_run(0);
        for _ in 0..99 {
            r.on_conflict(3);
            assert!(!r.due());
        }
        r.on_conflict(3);
        assert!(r.due());
        r.begin_run(2);
        for _ in 0..199 {
            r.on_conflict(3);
        }
        assert!(!r.due());
        r.on_conflict(3);
        assert!(r.due());
    }

    #[test]
    fn glucose_restarts_on_bad_lbds() {
        let mut r = RestartState::new(RestartPolicy::Glucose);
        r.begin_run(0);
        for _ in 0..200 {
            r.on_conflict(2);
        }
        assert!(!r.due(), "good clauses keep searching");
        for _ in 0..50 {
            r.on_conflict(20);
        }
        assert!(r.due(), "a run of bad clauses restarts");
    }
}
