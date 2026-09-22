//! The scheduler: event regions, the time wheel and signal propagation.
//!
//! One time slot is processed by [`Simulator::run_slot`], which loops over
//! the regions of IEEE 1364-2005 §11.4 until nothing is left:
//!
//! ```text
//! loop {
//!     if active is not empty   { pop one event, execute it; continue }
//!     if inactive is not empty { move all to active; continue }
//!     if nba is not empty      { apply every update in order; continue }
//!     break
//! }
//! run the monitor region ($strobe, $monitor)
//! ```
//!
//! Events in the active region are process activations, driver and cell
//! evaluations and delayed blocking updates. Executing one may change
//! signals; [`Simulator::write_signal`] compares old and new values and,
//! on a change, records the VCD sample, runs `on_change` callbacks, and
//! schedules the signal's fanout: drivers and combinational cells on any
//! change, sequential cells and processes on the matching edge (§9.7.1),
//! and processes blocked in a `wait` whose edge list mentions the signal.
//!
//! Each pass through active/inactive/NBA is one delta cycle in VHDL terms
//! (IEEE 1076-2008 §14.7.5): non-blocking updates become visible together,
//! processes sensitive to them resume in the next pass, and the slot is
//! over when a pass adds nothing. Future work lives in a `BTreeMap` from
//! time to events ([`Timed`]); [`Simulator::advance_to`] moves the clock
//! and unpacks that time's entries into their regions.
//!
//! A slot that processes more than `SimOptions::max_slot_events` events
//! is a zero-delay oscillation and ends the run with an error.

use crate::diag::Diagnostic;
use crate::ir::{CellKind, Lvalue, Polarity};
use crate::logic::{Bit, Logic};

use super::elab::{
    CellRef, Consumer, DriverId, DriverSource, InstId, MemId, ProcId, SigId, Target,
};
use super::eval::{bits_binary, index_value};
use super::process::ProcStatus;
use super::value::{Store, Value, merge_unknown, replace_bits, resolve_all};
use super::{Simulator, Status};

/// An entry of the active or inactive region.
#[derive(Clone, Debug)]
pub(crate) enum Event {
    /// Run or resume a process.
    Proc(ProcId),
    /// Re-evaluate a driver.
    Driver(DriverId),
    /// Apply a driver value that was delayed.
    DriverApply(DriverId, Logic, Vec<Store>),
    /// Trigger a sequential cell.
    Cell(CellRef),
}

/// An entry of the time wheel.
#[derive(Clone, Debug)]
pub(crate) enum Timed {
    /// Resume a delayed process.
    Wake(ProcId),
    /// A non-blocking assignment whose delay elapsed.
    Nba(Vec<Store>, Logic),
    /// A driver update whose transport delay elapsed.
    Driver(DriverId, Logic, Vec<Store>),
}

/// The LSB of a value, `X` for an empty vector.
pub(crate) fn bit0(l: &Logic) -> Bit {
    l.get(0).unwrap_or(Bit::X)
}

/// True for a positive edge between two samples: `0→1`, `x/z→1`, `0→x/z`.
pub(crate) fn is_posedge(old: Bit, new: Bit) -> bool {
    (old != Bit::One && new == Bit::One) || (old == Bit::Zero && !new.is_known())
}

/// True for a negative edge: `1→0`, `x/z→0`, `1→x/z`.
pub(crate) fn is_negedge(old: Bit, new: Bit) -> bool {
    (old != Bit::Zero && new == Bit::Zero) || (old == Bit::One && !new.is_known())
}

/// Whether a change from `old` to `new` satisfies `polarity`.
pub(crate) fn edge_matches(polarity: Polarity, old: &Logic, new: &Logic) -> bool {
    match polarity {
        Polarity::Any => true,
        Polarity::Pos => is_posedge(bit0(old), bit0(new)),
        Polarity::Neg => is_negedge(bit0(old), bit0(new)),
    }
}

impl<'d> Simulator<'d> {
    /// True when the current time slot has unprocessed events.
    pub(crate) fn has_current_events(&self) -> bool {
        !self.active.is_empty() || !self.inactive.is_empty() || !self.nba.is_empty()
    }

    /// The time of the earliest pending timed event.
    pub(crate) fn next_time(&self) -> Option<u64> {
        self.timed.keys().next().copied()
    }

    /// Moves the clock to `t` and unpacks the events scheduled there.
    pub(crate) fn advance_to(&mut self, t: u64) {
        self.now = t;
        let Some(list) = self.timed.remove(&t) else {
            return;
        };
        for ev in list {
            match ev {
                Timed::Wake(pid) => {
                    self.procs[pid.idx()].status = ProcStatus::Queued;
                    self.active.push_back(Event::Proc(pid));
                }
                Timed::Nba(stores, value) => self.nba.push((stores, value)),
                Timed::Driver(d, value, stores) => {
                    self.active.push_back(Event::DriverApply(d, value, stores));
                }
            }
        }
    }

    /// Schedules a timed event `ticks` from now.
    pub(crate) fn schedule_after(&mut self, ticks: u64, ev: Timed) {
        let t = self.now.saturating_add(ticks);
        self.timed.entry(t).or_default().push(ev);
    }

    /// Runs the current time slot to quiescence, then the concurrent
    /// assertions and the monitor region.
    pub(crate) fn run_slot(&mut self) {
        self.sample_assertions();
        let mut count = 0u64;
        loop {
            if self.status != Status::Running {
                return;
            }
            if let Some(ev) = self.active.pop_front() {
                count += 1;
                if count > self.options.max_slot_events {
                    self.messages.push(
                        Diagnostic::error(format!(
                            "time slot at {} did not settle after {} events; zero-delay loop?",
                            self.now, self.options.max_slot_events
                        ))
                        .with_note("a combinational feedback loop or a delta-cycle oscillation"),
                    );
                    self.status = Status::Finished;
                    return;
                }
                self.dispatch(ev);
                continue;
            }
            if !self.inactive.is_empty() {
                let moved: Vec<Event> = self.inactive.drain(..).collect();
                self.active.extend(moved);
                continue;
            }
            if !self.nba.is_empty() {
                let items = std::mem::take(&mut self.nba);
                for (stores, value) in items {
                    self.apply_stores(&stores, &value);
                }
                continue;
            }
            break;
        }
        self.run_assertions();
        self.monitor_region();
    }

    /// Executes one active-region event.
    fn dispatch(&mut self, ev: Event) {
        match ev {
            Event::Proc(pid) => self.run_process(pid),
            Event::Driver(d) => {
                self.drivers[d.idx()].scheduled = false;
                self.eval_driver(d);
            }
            Event::DriverApply(d, value, stores) => self.apply_driver(d, value, stores),
            Event::Cell(c) => {
                self.cells[c.idx()].scheduled = false;
                self.trigger_cell(c);
            }
        }
    }

    /// Queues a consumer in the active region unless it already is.
    pub(crate) fn schedule_consumer(&mut self, consumer: Consumer) {
        match consumer {
            Consumer::Driver(d) => {
                let dr = &mut self.drivers[d.idx()];
                if !dr.scheduled {
                    dr.scheduled = true;
                    self.active.push_back(Event::Driver(d));
                }
            }
            Consumer::Cell(c) => {
                let cell = &mut self.cells[c.idx()];
                if !cell.scheduled {
                    cell.scheduled = true;
                    self.active.push_back(Event::Cell(c));
                }
            }
            Consumer::Proc(p) => {
                let proc_ = &mut self.procs[p.idx()];
                if proc_.status == ProcStatus::Idle {
                    proc_.status = ProcStatus::Queued;
                    self.active.push_back(Event::Proc(p));
                }
            }
        }
    }

    /// Wakes a waiting process.
    pub(crate) fn wake(&mut self, pid: ProcId) {
        let p = &mut self.procs[pid.idx()];
        p.wait_gen = p.wait_gen.wrapping_add(1);
        p.status = ProcStatus::Queued;
        self.active.push_back(Event::Proc(pid));
    }

    /// Sets a signal's resolved value (a procedural write or a driver
    /// resolution result) and propagates the change.
    pub(crate) fn write_signal(&mut self, sig: SigId, value: Logic) {
        let s = &mut self.signals[sig.idx()];
        let width = s.value.width();
        let value = if value.width() == width {
            value
        } else {
            value.resize(width)
        };
        let value = value.with_signed(s.value.is_signed());
        if s.value == value {
            return;
        }
        let old = std::mem::replace(&mut s.value, value.clone());
        if s.forced.is_some() {
            return;
        }
        self.propagate(sig, &old, &value);
    }

    /// Notifies everything that depends on `sig` of a change.
    pub(crate) fn propagate(&mut self, sig: SigId, old: &Logic, new: &Logic) {
        if self.coverage.is_some() {
            self.cover_toggle(sig, old, new);
        }
        if !self.assert_clocks.is_empty() {
            self.note_assertion_edge(sig, old, new);
        }
        if !self.breaks.is_empty() {
            self.check_breakpoints(sig, new);
        }
        if let Some(vcd) = &mut self.vcd {
            vcd.change(
                self.now,
                sig,
                new,
                self.signals[sig.idx()].ty == crate::ir::Type::Real,
            );
        }
        let cbs = self.sig_callbacks[sig.idx()].clone();
        for i in cbs {
            (self.callbacks[i])(self.now, new);
        }
        for i in 0..self.sig_fanout[sig.idx()].len() {
            let f = self.sig_fanout[sig.idx()][i];
            if edge_matches(f.polarity, old, new) {
                self.schedule_consumer(f.consumer);
            }
        }
        if self.sig_waiters[sig.idx()].is_empty() {
            return;
        }
        let waiters = std::mem::take(&mut self.sig_waiters[sig.idx()]);
        let mut keep = Vec::new();
        for (pid, generation) in waiters {
            let p = &self.procs[pid.idx()];
            if p.wait_gen != generation || !p.status.is_waiting() {
                continue;
            }
            if p.wait_matches(sig, old, new) {
                self.wake(pid);
            } else {
                keep.push((pid, generation));
            }
        }
        self.sig_waiters[sig.idx()].extend(keep);
    }

    /// Records the breakpoints a change on `sig` fires.
    fn check_breakpoints(&mut self, sig: SigId, new: &Logic) {
        for b in &self.breaks {
            if b.sig != sig {
                continue;
            }
            let hit = match &b.value {
                Some(want) => want.clone().as_unsigned() == new.clone().as_unsigned(),
                None => true,
            };
            if hit && !self.break_hits.contains(&b.id) {
                self.break_hits.push(b.id);
            }
        }
    }

    /// Writes one memory element and schedules the memory's readers.
    pub(crate) fn write_mem(&mut self, mem: MemId, addr: u64, value: &Logic) -> bool {
        let m = &mut self.memories[mem.idx()];
        let Ok(i) = usize::try_from(addr) else {
            return false;
        };
        if i >= m.data.len() {
            return false;
        }
        let value = value
            .resize(m.elem_width)
            .with_signed(m.data[i].is_signed());
        if m.data[i] == value {
            return true;
        }
        m.data[i] = value;
        let consumers = m.fanout.clone();
        for c in consumers {
            self.schedule_consumer(c);
        }
        true
    }

    /// The width of one store.
    fn store_width(&self, store: &Store) -> u32 {
        match store {
            Store::Sig { width, .. } | Store::Skip { width } => *width,
            Store::Mem { mem, .. } => self.memories[mem.idx()].elem_width,
        }
    }

    /// The bits to store for `value` into `target`: a real written to a
    /// `real` net keeps its IEEE 754 encoding, everything else converts
    /// through [`Value::to_logic`].
    pub(crate) fn logic_for_target(&self, target: &Target, value: Value) -> Logic {
        if let (Value::Real(r), Target::Sig { sig, .. }) = (&value, target)
            && self.signals[sig.idx()].ty == crate::ir::Type::Real
        {
            return Logic::from_u64(r.to_bits(), 64);
        }
        value.to_logic()
    }

    /// Evaluates the dynamic parts of a target into concrete stores.
    pub(crate) fn resolve_target(&mut self, inst: InstId, target: &Target) -> Vec<Store> {
        let mut out = Vec::new();
        self.resolve_into(inst, target, &mut out);
        out
    }

    fn resolve_into(&mut self, inst: InstId, target: &Target, out: &mut Vec<Store>) {
        match target {
            Target::Sig { sig, lo, width } => out.push(Store::Sig {
                sig: *sig,
                lo: *lo,
                width: *width,
            }),
            Target::Index {
                sig,
                index,
                elem,
                count,
            } => {
                let i = self.eval_logic(inst, *index);
                match index_value(&i) {
                    Some(i) if i >= 0 && u64::try_from(i).is_ok_and(|i| i < *count) => {
                        let lo = u32::try_from(i).unwrap_or(u32::MAX).saturating_mul(*elem);
                        out.push(Store::Sig {
                            sig: *sig,
                            lo,
                            width: *elem,
                        });
                    }
                    _ => out.push(Store::Skip { width: *elem }),
                }
            }
            Target::Mem { mem, addr } => {
                let a = self.eval_logic(inst, *addr);
                let size = self.memories[mem.idx()].data.len();
                match a.to_u64() {
                    Some(a) if usize::try_from(a).is_ok_and(|a| a < size) => {
                        out.push(Store::Mem { mem: *mem, addr: a });
                    }
                    _ => out.push(Store::Skip {
                        width: self.memories[mem.idx()].elem_width,
                    }),
                }
            }
            Target::Concat(parts) => {
                for p in parts {
                    self.resolve_into(inst, p, out);
                }
            }
        }
    }

    /// Splits `value` over `stores` (most significant first), returning the
    /// part for each store.
    fn split_value(&self, stores: &[Store], value: &Logic) -> Vec<Logic> {
        let total: u32 = stores
            .iter()
            .fold(0u32, |acc, s| acc.saturating_add(self.store_width(s)));
        let value = if value.width() == total {
            value.clone()
        } else {
            value.resize(total)
        };
        let mut offset = total;
        let mut parts = Vec::with_capacity(stores.len());
        for s in stores {
            let w = self.store_width(s);
            offset -= w;
            parts.push(if w == 0 {
                Logic::zero(0)
            } else {
                value.slice(offset + w - 1, offset)
            });
        }
        parts
    }

    /// Applies a procedural assignment: every store is written as if by a
    /// process (last write wins).
    pub(crate) fn apply_stores(&mut self, stores: &[Store], value: &Logic) {
        let parts = self.split_value(stores, value);
        for (store, part) in stores.iter().zip(parts) {
            match store {
                Store::Sig { sig, lo, width } => {
                    let current = &self.signals[sig.idx()].value;
                    let new = if *lo == 0 && *width == current.width() {
                        part
                    } else {
                        replace_bits(current, *lo, &part)
                    };
                    self.write_signal(*sig, new);
                }
                Store::Mem { mem, addr } => {
                    self.write_mem(*mem, *addr, &part);
                }
                Store::Skip { .. } => {}
            }
        }
    }

    /// Evaluates a driver and applies (or schedules) its value.
    pub(crate) fn eval_driver(&mut self, d: DriverId) {
        let (inst, source, target, delay) = {
            let dr = &self.drivers[d.idx()];
            (dr.inst, dr.source, dr.target.clone(), dr.delay)
        };
        let value = match source {
            DriverSource::Expr(e) => {
                let v = self.eval(inst, e);
                self.logic_for_target(&target, v)
            }
            DriverSource::Sig(s) => self.signals[s.idx()].effective().clone(),
            DriverSource::Cell(c) => match self.eval_comb_cell(c) {
                Some(v) => v,
                None => return,
            },
        };
        let stores = self.resolve_target(inst, &target);
        if delay > 0 {
            self.schedule_after(delay, Timed::Driver(d, value, stores));
        } else {
            self.apply_driver(d, value, stores);
        }
    }

    /// Applies a driver's value: wires receive a contribution and are
    /// re-resolved over all their drivers; registers and memories are
    /// written directly.
    pub(crate) fn apply_driver(&mut self, d: DriverId, value: Logic, stores: Vec<Store>) {
        let parts = self.split_value(&stores, &value);
        let mut wire_updates: Vec<(SigId, Logic)> = Vec::new();
        for (store, part) in stores.iter().zip(parts) {
            match store {
                Store::Sig { sig, lo, width } => {
                    let s = &self.signals[sig.idx()];
                    if s.kind == crate::ir::NetKind::Wire {
                        let entry = match wire_updates.iter().position(|(x, _)| x == sig) {
                            Some(i) => &mut wire_updates[i].1,
                            None => {
                                wire_updates.push((*sig, Logic::z(s.width())));
                                &mut wire_updates.last_mut().expect("just pushed").1
                            }
                        };
                        *entry = replace_bits(entry, *lo, &part);
                    } else {
                        let new = if *lo == 0 && *width == s.value.width() {
                            part
                        } else {
                            replace_bits(&s.value, *lo, &part)
                        };
                        self.write_signal(*sig, new);
                    }
                }
                Store::Mem { mem, addr } => {
                    self.write_mem(*mem, *addr, &part);
                }
                Store::Skip { .. } => {}
            }
        }
        for (sig, contrib) in wire_updates {
            let dr = &mut self.drivers[d.idx()];
            match dr.contrib.iter_mut().find(|(s, _)| *s == sig) {
                Some((_, old)) => {
                    if *old == contrib {
                        continue;
                    }
                    *old = contrib;
                }
                None => dr.contrib.push((sig, contrib)),
            }
            let width = self.signals[sig.idx()].width();
            let resolved = {
                let contribs = self.sig_drivers[sig.idx()].iter().filter_map(|dd| {
                    self.drivers[dd.idx()]
                        .contrib
                        .iter()
                        .find(|(s, _)| *s == sig)
                        .map(|(_, v)| v)
                });
                resolve_all(width, contribs)
            };
            self.write_signal(sig, resolved);
        }
    }

    /// The input `port` of cell `c`, evaluated.
    fn cell_input(&mut self, c: CellRef, port: &str) -> Option<Logic> {
        let (inst, e) = {
            let cs = &self.cells[c.idx()];
            (cs.inst, cs.cell.input(port)?)
        };
        Some(self.eval_logic(inst, e))
    }

    /// The current value of output `port` of cell `c`.
    fn cell_output_value(&self, c: CellRef, port: &str) -> Option<Logic> {
        let cs = &self.cells[c.idx()];
        let net = cs.cell.output(port)?;
        let sig = self.instances[cs.inst.idx()].nets[net.index()];
        Some(self.signals[sig.idx()].effective().clone())
    }

    /// The function of a combinational cell; `None` keeps the previous
    /// output (a latch holding).
    pub(crate) fn eval_comb_cell(&mut self, c: CellRef) -> Option<Logic> {
        use crate::ir::BinaryOp as B;
        let kind = self.cells[c.idx()].cell.kind.clone();
        let binary = |sim: &mut Self, op: B| {
            let a = sim.cell_input(c, "a")?;
            let b = sim.cell_input(c, "b")?;
            Some(bits_binary(op, a, b))
        };
        match kind {
            CellKind::Not => Some(self.cell_input(c, "a")?.not()),
            CellKind::Buf => self.cell_input(c, "a"),
            CellKind::And => binary(self, B::And),
            CellKind::Or => binary(self, B::Or),
            CellKind::Xor => binary(self, B::Xor),
            CellKind::Add => binary(self, B::Add),
            CellKind::Sub => binary(self, B::Sub),
            CellKind::Mul => binary(self, B::Mul),
            CellKind::Div => binary(self, B::Div),
            CellKind::Mod => binary(self, B::Mod),
            CellKind::Shl => binary(self, B::Shl),
            CellKind::Shr => binary(self, B::Shr),
            CellKind::Sshr => binary(self, B::Sshr),
            CellKind::Eq => binary(self, B::Eq),
            CellKind::Ne => binary(self, B::Ne),
            CellKind::Lt => binary(self, B::Lt),
            CellKind::Le => binary(self, B::Le),
            CellKind::Gt => binary(self, B::Gt),
            CellKind::Ge => binary(self, B::Ge),
            CellKind::ReduceAnd => Some(self.cell_input(c, "a")?.reduce_and()),
            CellKind::ReduceOr => Some(self.cell_input(c, "a")?.reduce_or()),
            CellKind::ReduceXor => Some(self.cell_input(c, "a")?.reduce_xor()),
            CellKind::Mux => {
                let a = self.cell_input(c, "a")?;
                let b = self.cell_input(c, "b")?;
                let s = self.cell_input(c, "s")?;
                Some(match s.truth() {
                    Bit::One => b,
                    Bit::Zero => a,
                    _ => merge_unknown(&a, &b),
                })
            }
            CellKind::Pmux => {
                let a = self.cell_input(c, "a")?;
                let b = self.cell_input(c, "b")?;
                let s = self.cell_input(c, "s")?;
                let w = a.width();
                if s.has_unknown() {
                    return Some(Logic::x(w));
                }
                if s.is_zero() {
                    return Some(a);
                }
                let ones: Vec<u32> = (0..s.width()).filter(|i| s.bit(*i) == Bit::One).collect();
                match ones.as_slice() {
                    [i] if (i + 1).saturating_mul(w) <= b.width() && w > 0 => {
                        Some(b.slice((i + 1) * w - 1, i * w))
                    }
                    _ => Some(Logic::x(w)),
                }
            }
            CellKind::Lut { init, .. } => {
                let a = self.cell_input(c, "a")?;
                Some(match a.to_u64().and_then(|i| u32::try_from(i).ok()) {
                    Some(i) if i < init.width() => Logic::from_bit(init.bit(i)),
                    _ => Logic::x(1),
                })
            }
            CellKind::Tristate => {
                let a = self.cell_input(c, "a")?;
                let en = self.cell_input(c, "en")?;
                Some(match en.truth() {
                    Bit::One => a,
                    Bit::Zero => Logic::z(a.width()),
                    _ => Logic::x(a.width()),
                })
            }
            CellKind::Dlatch => {
                let d = self.cell_input(c, "d")?;
                let en = self.cell_input(c, "en")?;
                match en.truth() {
                    Bit::One => Some(d),
                    Bit::Zero => None,
                    _ => {
                        let q = self.cell_output_value(c, "q")?;
                        Some(merge_unknown(&d, &q))
                    }
                }
            }
            CellKind::MemRdPort {
                mem,
                clocked: false,
            } => {
                let addr = self.cell_input(c, "addr")?;
                let inst = self.cells[c.idx()].inst;
                let mid = self.instances[inst.idx()].mems[mem.index()];
                let m = &self.memories[mid.idx()];
                Some(
                    addr.to_u64()
                        .and_then(|a| usize::try_from(a).ok())
                        .and_then(|a| m.data.get(a).cloned())
                        .unwrap_or_else(|| Logic::x(m.elem_width)),
                )
            }
            _ => None,
        }
    }

    /// Reacts to a change on a sequential cell's inputs.
    pub(crate) fn trigger_cell(&mut self, c: CellRef) {
        let kind = self.cells[c.idx()].cell.kind.clone();
        let inst = self.cells[c.idx()].inst;
        match kind {
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                let clk = bit0(&self.cell_input(c, "clk").unwrap_or_else(|| Logic::x(1)));
                let last = std::mem::replace(&mut self.cells[c.idx()].last_clk, clk);
                let edge = if clk_pos {
                    is_posedge(last, clk)
                } else {
                    is_negedge(last, clk)
                };
                let Some(q) = self.cells[c.idx()].cell.output("q") else {
                    return;
                };
                let target = self.target_from_lvalue(inst, &Lvalue::Net(q));
                let stores = self.resolve_target(inst, &target);
                if let Some(rst) = reset {
                    let r = self
                        .cell_input(c, "rst")
                        .map(|l| bit0(&l))
                        .unwrap_or(Bit::X);
                    let active = if rst.active_high {
                        r == Bit::One
                    } else {
                        r == Bit::Zero
                    };
                    if active && (rst.asynchronous || edge) {
                        self.nba.push((stores, rst.value.clone()));
                        return;
                    }
                }
                if !edge {
                    return;
                }
                if has_enable && self.cell_input(c, "en").map(|l| l.truth()) != Some(Bit::One) {
                    return;
                }
                if let Some(d) = self.cell_input(c, "d") {
                    self.nba.push((stores, d));
                }
            }
            CellKind::MemRdPort { mem, clocked: true } => {
                let clk = bit0(&self.cell_input(c, "clk").unwrap_or_else(|| Logic::x(1)));
                let last = std::mem::replace(&mut self.cells[c.idx()].last_clk, clk);
                if !is_posedge(last, clk) {
                    return;
                }
                if self.cell_input(c, "en").map(|l| l.truth()) == Some(Bit::Zero) {
                    return;
                }
                let Some(addr) = self.cell_input(c, "addr") else {
                    return;
                };
                let Some(data) = self.cells[c.idx()].cell.output("data") else {
                    return;
                };
                let mid = self.instances[inst.idx()].mems[mem.index()];
                let m = &self.memories[mid.idx()];
                let value = addr
                    .to_u64()
                    .and_then(|a| usize::try_from(a).ok())
                    .and_then(|a| m.data.get(a).cloned())
                    .unwrap_or_else(|| Logic::x(m.elem_width));
                let target = self.target_from_lvalue(inst, &Lvalue::Net(data));
                let stores = self.resolve_target(inst, &target);
                self.nba.push((stores, value));
            }
            CellKind::MemWrPort { mem, clocked } => {
                if clocked {
                    let clk = bit0(&self.cell_input(c, "clk").unwrap_or_else(|| Logic::x(1)));
                    let last = std::mem::replace(&mut self.cells[c.idx()].last_clk, clk);
                    if !is_posedge(last, clk) {
                        return;
                    }
                }
                if self.cell_input(c, "en").map(|l| l.truth()) != Some(Bit::One) {
                    return;
                }
                let (Some(addr), Some(data)) =
                    (self.cell_input(c, "addr"), self.cell_input(c, "data"))
                else {
                    return;
                };
                let Some(a) = addr.to_u64() else {
                    return;
                };
                let mid = self.instances[inst.idx()].mems[mem.index()];
                if clocked {
                    self.nba
                        .push((vec![Store::Mem { mem: mid, addr: a }], data));
                } else {
                    self.write_mem(mid, a, &data);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        assert!(is_posedge(Bit::Zero, Bit::One));
        assert!(is_posedge(Bit::X, Bit::One));
        assert!(is_posedge(Bit::Zero, Bit::Z));
        assert!(!is_posedge(Bit::One, Bit::One));
        assert!(!is_posedge(Bit::X, Bit::Z));
        assert!(is_negedge(Bit::One, Bit::Zero));
        assert!(is_negedge(Bit::One, Bit::X));
        assert!(is_negedge(Bit::Z, Bit::Zero));
        assert!(!is_negedge(Bit::Zero, Bit::X));
        let zero = Logic::zero(1);
        let one = Logic::ones(1);
        assert!(edge_matches(Polarity::Any, &zero, &one));
        assert!(edge_matches(Polarity::Pos, &zero, &one));
        assert!(!edge_matches(Polarity::Neg, &zero, &one));
        assert_eq!(bit0(&Logic::zero(0)), Bit::X);
    }
}
