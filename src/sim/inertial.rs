//! Inertial delay: an assignment that cancels what it has already
//! scheduled.
//!
//! A delayed assignment schedules an update on the time wheel. Under a
//! *transport* delay every update scheduled is applied. Under an
//! *inertial* one (VHDL's default for `s <= v after t`, LRM 10.5.2.2, and
//! a Verilog continuous assignment's, IEEE 1364-2005 §6.1.3) scheduling a
//! new value removes the updates the same assignment still has pending
//! whose value differs from the new one, so a pulse shorter than the delay
//! never reaches the target. The IR marks such a delay with
//! [`crate::ir::Delay::inertial`].
//!
//! The time wheel's entries are not touched. Each inertial assignment —
//! a continuous one by its driver, a process's by the process and the
//! signal it writes first — keeps the updates it has pending; scheduling
//! a new one moves the pending ones it removes to a cancelled list, and an
//! update about to be applied that is on that list is dropped instead.
//! An update removed and an update of another assignment with the same
//! time, target and value cannot be told apart; the second is dropped too,
//! which writes nothing different.

use std::collections::{HashMap, HashSet};

use crate::logic::Logic;

use super::Simulator;
use super::elab::{DriverId, ProcId, SigId};
use super::value::Store;

/// Whose pending updates these are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Owner {
    /// A continuous assignment, by its driver.
    Driver(DriverId),
    /// A process's assignment, by the process and the signal it writes.
    Proc(ProcId, SigId),
}

/// The pending and cancelled updates of the inertial assignments.
#[derive(Debug, Default)]
pub(crate) struct InertialState {
    /// The drivers whose delay is inertial.
    drivers: HashSet<DriverId>,
    /// Updates scheduled and not yet applied, as time and value.
    pending: HashMap<Owner, Vec<(u64, Logic)>>,
    /// Process updates removed before their time, as time, signal and
    /// value.
    cancelled_nba: Vec<(u64, SigId, Logic)>,
    /// Driver updates removed before their time.
    cancelled_driver: Vec<(u64, DriverId, Logic)>,
}

impl InertialState {
    /// Marks a continuous assignment's driver as inertial.
    pub(crate) fn mark_driver(&mut self, d: DriverId) {
        self.drivers.insert(d);
    }

    /// Records a new update of `owner` at `at`, cancelling the pending
    /// ones whose value differs; returns those cancelled.
    fn schedule(&mut self, owner: Owner, now: u64, at: u64, value: &Logic) -> Vec<(u64, Logic)> {
        let list = self.pending.entry(owner).or_default();
        // What is due now or earlier has been applied (or dropped).
        list.retain(|(t, _)| *t > now);
        let mut cancelled = Vec::new();
        list.retain(|(t, v)| {
            if v == value {
                true
            } else {
                cancelled.push((*t, v.clone()));
                false
            }
        });
        list.push((at, value.clone()));
        cancelled
    }
}

/// The signal a list of stores writes first, which names a process's
/// assignment.
fn first_signal(stores: &[Store]) -> Option<SigId> {
    stores.iter().find_map(|s| match s {
        Store::Sig { sig, .. } => Some(*sig),
        _ => None,
    })
}

impl Simulator<'_> {
    /// True when a continuous assignment's delay is inertial.
    pub(crate) fn inertial_driver(&self, d: DriverId) -> bool {
        self.inertial.drivers.contains(&d)
    }

    /// A process's inertial assignment scheduled `value` for `at`.
    pub(crate) fn inertial_nba_scheduled(
        &mut self,
        pid: ProcId,
        stores: &[Store],
        value: &Logic,
        at: u64,
    ) {
        let Some(sig) = first_signal(stores) else {
            return;
        };
        let now = self.now;
        for (t, v) in self
            .inertial
            .schedule(Owner::Proc(pid, sig), now, at, value)
        {
            self.inertial.cancelled_nba.push((t, sig, v));
        }
    }

    /// An inertial driver scheduled `value` for `at`.
    pub(crate) fn inertial_driver_scheduled(&mut self, d: DriverId, value: &Logic, at: u64) {
        let now = self.now;
        for (t, v) in self.inertial.schedule(Owner::Driver(d), now, at, value) {
            self.inertial.cancelled_driver.push((t, d, v));
        }
    }

    /// True when the process update about to be applied now was cancelled;
    /// it is then taken off the cancelled list.
    pub(crate) fn inertial_nba_cancelled(&mut self, stores: &[Store], value: &Logic) -> bool {
        if self.inertial.cancelled_nba.is_empty() {
            return false;
        }
        let Some(sig) = first_signal(stores) else {
            return false;
        };
        let now = self.now;
        let found = self
            .inertial
            .cancelled_nba
            .iter()
            .position(|(t, s, v)| *t == now && *s == sig && v == value);
        match found {
            Some(i) => {
                self.inertial.cancelled_nba.swap_remove(i);
                true
            }
            None => false,
        }
    }

    /// True when the driver update about to be applied now was cancelled.
    pub(crate) fn inertial_driver_cancelled(&mut self, d: DriverId, value: &Logic) -> bool {
        if self.inertial.cancelled_driver.is_empty() {
            return false;
        }
        let now = self.now;
        let found = self
            .inertial
            .cancelled_driver
            .iter()
            .position(|(t, x, v)| *t == now && *x == d && v == value);
        match found {
            Some(i) => {
                self.inertial.cancelled_driver.swap_remove(i);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Would catch a new update that keeps a pending one of another value
    /// (the pulse would get through), or one that cancels an update of the
    /// same value, which inertial delay keeps; would not catch the
    /// simulator dropping a cancelled update, which the VHDL fixtures do.
    #[test]
    fn a_new_value_cancels_pending_updates_of_another() {
        let mut s = InertialState::default();
        let d = Owner::Driver(DriverId::at(0));
        let one = Logic::from_bool(true);
        let zero = Logic::from_bool(false);
        assert!(s.schedule(d, 0, 30, &one).is_empty());
        let gone = s.schedule(d, 20, 50, &zero);
        assert_eq!(gone, vec![(30, one.clone())]);
        assert!(s.schedule(d, 25, 55, &zero).is_empty());
        // Due by now: no longer pending, nothing to cancel.
        assert!(s.schedule(d, 60, 90, &one).is_empty());
    }
}
