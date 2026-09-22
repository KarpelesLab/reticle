//! The co-simulation API: the public methods of [`Simulator`].
//!
//! A Rust testbench looks like:
//!
//! ```
//! use reticle::ir::{Delay, Design, TimeUnit};
//! use reticle::logic::Logic;
//! use reticle::sim::{SimOptions, Simulator};
//! use reticle::source::SourceMap;
//!
//! let text = "\
//! module counter
//!   net %clk u1 wire
//!   net %q u8 reg
//!   port clk in %clk
//!   port q out %q
//!   process initial
//!     %q = 8'd0
//!   end
//!   process seq posedge %clk
//!     %q <= add(%q, 8'd1)
//!   end
//! end
//! ";
//! let mut map = SourceMap::new();
//! let file = map.add("counter.rtl", text).unwrap();
//! let design = Design::parse_text(text, file).unwrap();
//! let mut sim = Simulator::new(&design, SimOptions::default()).unwrap();
//! let clk = sim.net("counter.clk").unwrap();
//! let q = sim.net("counter.q").unwrap();
//! let half = sim.ticks(Delay::new(5, TimeUnit::Ns));
//! sim.run_for(0);
//! for _ in 0..3 {
//!     sim.set(clk, Logic::from_bool(true));
//!     sim.run_for(half);
//!     sim.set(clk, Logic::from_bool(false));
//!     sim.run_for(half);
//! }
//! assert_eq!(sim.get(q).to_u64(), Some(3));
//! ```
//!
//! Times are in ticks of the simulation precision ([`Simulator::ticks`]
//! converts a [`Delay`]). [`Simulator::set`] writes a net as a process
//! would (it takes effect immediately; a continuously driven net gets
//! overwritten again when its driver re-evaluates, so use
//! [`Simulator::force`] for those). All run methods return when the
//! requested time is reached, `$finish` executes, or `$stop` pauses; a
//! paused simulation resumes with the next run call.

use std::fmt;
use std::io;

use crate::diag::Diagnostics;
use crate::ir::{Delay, Design, NetKind};
use crate::logic::Logic;

use super::elab::{InstId, MemId, SigId};
use super::fst::FstCapture;
use super::value::resolve_all;
use super::{SimOptions, Simulator, Status};

/// A net of the running simulation, looked up by hierarchical name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NetHandle(pub(crate) SigId);

/// A memory of the running simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemHandle(pub(crate) MemId);

impl<'d> Simulator<'d> {
    /// Elaborates `design` for simulation.
    ///
    /// Returns the elaboration diagnostics on error (no top module, a
    /// recursive hierarchy, a missing module id). Warnings (unresolved or
    /// black-box instances, port width mismatches) are kept in
    /// [`Simulator::messages`].
    pub fn new(design: &'d Design, options: SimOptions) -> Result<Self, Diagnostics> {
        Self::elaborate(design, options)
    }

    /// The design being simulated.
    pub fn design(&self) -> &'d Design {
        self.design
    }

    /// Femtoseconds per tick: the finest `timescale` precision in the
    /// design, or 1 ps.
    pub fn precision_fs(&self) -> u64 {
        self.precision_fs
    }

    /// Converts a delay to ticks (rounded, at least one for a non-zero
    /// delay).
    pub fn ticks(&self, delay: Delay) -> u64 {
        self.delay_ticks(delay)
    }

    /// The current time in ticks.
    pub fn time(&self) -> u64 {
        self.now
    }

    /// The run state.
    pub fn status(&self) -> Status {
        self.status
    }

    /// True once `$finish` (or a failing assertion) ended the run.
    pub fn finished(&self) -> bool {
        self.status == Status::Finished
    }

    /// The name of the top instance, the first component of every
    /// hierarchical name.
    pub fn top_name(&self) -> &str {
        &self.instances[0].name
    }

    /// Finds the instance at `path` (components without the net name).
    fn find_instance(&self, parts: &[&str]) -> Option<InstId> {
        let (first, rest) = parts.split_first()?;
        if *first != self.instances[0].name {
            return None;
        }
        let mut cur = InstId(0);
        for name in rest {
            cur = *self.instances[cur.idx()]
                .children
                .iter()
                .find(|c| self.instances[c.idx()].name == *name)?;
        }
        Some(cur)
    }

    /// Looks up a net by hierarchical name, `top.inst.net`.
    pub fn net(&self, path: &str) -> Option<NetHandle> {
        let parts: Vec<&str> = path.split('.').collect();
        let (name, scope) = parts.split_last()?;
        let inst = self.find_instance(scope)?;
        let state = &self.instances[inst.idx()];
        let nid = state.m.net_by_name(name)?;
        Some(NetHandle(state.nets[nid.index()]))
    }

    /// Looks up a memory by hierarchical name.
    pub fn memory(&self, path: &str) -> Option<MemHandle> {
        let parts: Vec<&str> = path.split('.').collect();
        let (name, scope) = parts.split_last()?;
        let inst = self.find_instance(scope)?;
        let state = &self.instances[inst.idx()];
        let mid = state.m.memory_by_name(name)?;
        Some(MemHandle(state.mems[mid.index()]))
    }

    /// Every net with its hierarchical name, in hierarchy order.
    pub fn nets(&self) -> Vec<(String, NetHandle)> {
        let mut out = Vec::new();
        for state in &self.instances {
            for (nid, net) in state.m.nets.iter() {
                out.push((
                    format!("{}.{}", state.path, net.name),
                    NetHandle(state.nets[nid.index()]),
                ));
            }
        }
        out
    }

    /// The hierarchical name of the net a handle was created from (the
    /// first net mapped to the signal).
    pub fn net_name(&self, net: NetHandle) -> &str {
        &self.signals[net.0.idx()].name
    }

    /// The current value of a net (the forced value while forced).
    pub fn get(&self, net: NetHandle) -> Logic {
        self.signals[net.0.idx()].effective().clone()
    }

    /// Writes a net as a process would: immediately, and its fanout is
    /// scheduled for the next run call.
    pub fn set(&mut self, net: NetHandle, value: Logic) {
        self.write_signal(net.0, value);
    }

    /// Overrides a net until [`Simulator::release`]: readers see `value`
    /// whatever its drivers do.
    pub fn force(&mut self, net: NetHandle, value: Logic) {
        let sig = net.0;
        let s = &mut self.signals[sig.idx()];
        let value = value.resize(s.width()).with_signed(s.value.is_signed());
        let old = s.effective().clone();
        s.forced = Some(value.clone());
        if old != value {
            self.propagate(sig, &old, &value);
        }
    }

    /// Removes a force. A wire returns to what its drivers resolve to; a
    /// register keeps the forced value until the next assignment.
    pub fn release(&mut self, net: NetHandle) {
        let sig = net.0;
        let Some(forced) = self.signals[sig.idx()].forced.take() else {
            return;
        };
        let s = &self.signals[sig.idx()];
        let new = if s.kind == NetKind::Wire && !self.sig_drivers[sig.idx()].is_empty() {
            let contribs = self.sig_drivers[sig.idx()].iter().filter_map(|d| {
                self.drivers[d.idx()]
                    .contrib
                    .iter()
                    .find(|(x, _)| *x == sig)
                    .map(|(_, v)| v)
            });
            resolve_all(s.width(), contribs)
        } else {
            forced.clone()
        };
        self.signals[sig.idx()].value = new.clone();
        if new != forced {
            self.propagate(sig, &forced, &new);
        }
    }

    /// One element of a memory, or `None` when out of range.
    pub fn get_mem(&self, mem: MemHandle, index: u64) -> Option<Logic> {
        let i = usize::try_from(index).ok()?;
        self.memories[mem.0.idx()].data.get(i).cloned()
    }

    /// Writes one element of a memory; false when out of range.
    pub fn set_mem(&mut self, mem: MemHandle, index: u64, value: Logic) -> bool {
        self.write_mem(mem.0, index, &value)
    }

    /// The number of elements of a memory.
    pub fn mem_len(&self, mem: MemHandle) -> usize {
        self.memories[mem.0.idx()].data.len()
    }

    /// Calls `callback` with the time and new value whenever the net
    /// changes.
    pub fn on_change(&mut self, net: NetHandle, callback: impl FnMut(u64, &Logic) + 'static) {
        let index = self.callbacks.len();
        self.callbacks.push(Box::new(callback));
        self.sig_callbacks[net.0.idx()].push(index);
    }

    /// Resumes after `$stop`.
    fn resume(&mut self) {
        if self.status == Status::Stopped {
            self.status = Status::Running;
        }
    }

    /// Runs every time slot up to and including `time`, then sets the
    /// clock to `time`.
    pub fn run_until(&mut self, time: u64) {
        self.resume();
        loop {
            if self.status != Status::Running {
                return;
            }
            self.run_slot();
            if self.status != Status::Running {
                return;
            }
            match self.next_time() {
                Some(t) if t <= time => self.advance_to(t),
                _ => break,
            }
        }
        if self.now < time {
            self.now = time;
        }
    }

    /// Runs for `ticks` ticks from the current time.
    pub fn run_for(&mut self, ticks: u64) {
        let target = self.now.saturating_add(ticks);
        self.run_until(target);
    }

    /// Runs for a delay.
    pub fn run_for_delay(&mut self, delay: Delay) {
        let ticks = self.ticks(delay);
        self.run_for(ticks);
    }

    /// Runs one time slot: the current one if it has pending events, else
    /// the next scheduled one. Returns false when nothing was left to run.
    pub fn step(&mut self) -> bool {
        self.resume();
        if self.status != Status::Running {
            return false;
        }
        if self.has_current_events() {
            self.run_slot();
            return true;
        }
        match self.next_time() {
            Some(t) => {
                self.advance_to(t);
                self.run_slot();
                true
            }
            None => false,
        }
    }

    /// Runs until `$finish`, `$stop` or no event is left.
    pub fn run(&mut self) {
        while self.step() && self.status == Status::Running {}
    }

    /// The text produced by `$display` and friends so far.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Takes the output text, leaving it empty.
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.output)
    }

    /// Runtime diagnostics: assertion reports, unknown tasks, stuck
    /// processes, `unique`/`priority` violations, elaboration warnings.
    pub fn messages(&self) -> &Diagnostics {
        &self.messages
    }

    /// Takes the diagnostics, leaving none.
    pub fn take_messages(&mut self) -> Diagnostics {
        std::mem::take(&mut self.messages)
    }

    /// Starts VCD capture: the header and a snapshot of every net are
    /// written now, and every later change is recorded. Calling it again
    /// restarts the capture.
    pub fn enable_vcd(&mut self) {
        self.vcd = Some(self.build_vcd());
    }

    /// The VCD text captured so far, if capture is enabled.
    pub fn vcd(&self) -> Option<&str> {
        self.vcd.as_ref().map(|v| v.text())
    }

    /// Writes the captured VCD text to `out`; a no-op when capture is off.
    pub fn dump_vcd(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        match &self.vcd {
            Some(v) => out.write_str(v.text()),
            None => Ok(()),
        }
    }

    /// Starts FST capture, the binary counterpart of
    /// [`Simulator::enable_vcd`].
    ///
    /// A snapshot of every net is taken now and every later change is
    /// recorded through the same callback mechanism as
    /// [`Simulator::on_change`], so the capture holds exactly the changes a
    /// VCD of the same run would. The returned [`FstCapture`] is the handle
    /// on it: hold it and pass it to [`Simulator::dump_fst`]. Calling this
    /// again starts a second, independent capture.
    ///
    /// See [`sim::fst`](crate::sim::fst) for the file layout.
    pub fn enable_fst(&mut self) -> FstCapture {
        self.build_fst()
    }

    /// Writes `capture` to `out` as a complete FST file, ending at the
    /// current simulation time.
    ///
    /// The capture is left running, so a long run may be dumped more than
    /// once.
    ///
    /// # Errors
    ///
    /// Propagates the failure of `out`.
    pub fn dump_fst(&self, capture: &FstCapture, out: &mut dyn io::Write) -> io::Result<()> {
        out.write_all(&capture.to_bytes(self.now))
    }
}
