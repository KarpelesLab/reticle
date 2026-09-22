//! The `reticle_sim` handle: a running simulation.
//!
//! [`crate::sim::Simulator`] borrows the design it runs, which a C caller
//! cannot express: there is no way to tell C that one handle must outlive
//! another. The handle therefore takes its own copy of the design and of
//! the source map, and keeps the simulator that borrows them in the same
//! allocation. That is the one self-referential structure in the crate,
//! and the invariants that make it sound are spelled out on [`new`].
//!
//! Nets are addressed by index rather than by an opaque per-net pointer:
//! the handle snapshots the simulator's net list at creation, so
//! [`reticle_sim_find_net`] turns a hierarchical path into an index once
//! and every later get or set is a bounds-checked array lookup. There is
//! nothing for C to free per net and nothing to dangle.
//!
//! [`new`]: reticle_sim::new

use std::ffi::{c_char, c_int};

#[cfg(not(feature = "sim"))]
use super::RETICLE_ERR_UNSUPPORTED;
use super::design::reticle_design;
use super::diagnostics::{self, reticle_diagnostics};
#[cfg(feature = "sim")]
use super::into_handle;
use super::{
    RETICLE_ERR_INVALID, RETICLE_OK, as_mut, as_ref, drop_handle, guard, str_arg, write_out,
    write_string,
};

/// An opaque running simulation.
///
/// Created by [`reticle_sim_create`] and released with
/// [`reticle_sim_free`].
#[cfg(feature = "sim")]
pub struct reticle_sim {
    /// Declared first so it is dropped first: it borrows `design`.
    sim: crate::sim::Simulator<'static>,
    /// Boxed, so the `Design` has a heap address that does not move when
    /// the handle itself is moved into its `Box`.
    #[allow(dead_code, reason = "owns the storage `sim` borrows")]
    design: Box<crate::ir::Design>,
    /// A copy of the sources, for rendering the simulator's messages after
    /// the design handle may have been freed.
    map: crate::source::SourceMap,
    /// Every net's hierarchical path and handle, in hierarchy order.
    nets: Vec<(String, crate::sim::NetHandle)>,
}

/// The same handle in a build without the `sim` feature: it can never be
/// constructed, and every entry point reports
/// [`RETICLE_ERR_UNSUPPORTED`].
#[cfg(not(feature = "sim"))]
pub struct reticle_sim {
    _never: (),
}

#[cfg(feature = "sim")]
impl reticle_sim {
    /// Elaborates `design` for simulation into a standalone handle.
    ///
    /// The handle owns a clone of the design and of its source map, so it
    /// stays valid after the [`reticle_design`] it came from is freed.
    fn new(
        design: &reticle_design,
        top: Option<&str>,
    ) -> Result<reticle_sim, crate::diag::Diagnostics> {
        use crate::sim::{SimOptions, Simulator};

        let owned = Box::new(design.design.clone());
        // SAFETY: `owned` is a `Box`, so the `Design` it holds lives at a
        // fixed heap address that is not affected by later moves of the
        // box or of the handle. The reference is stored only in `sim`,
        // which is declared before `design` in `reticle_sim` and so is
        // dropped first; nothing ever moves out of or reassigns `design`.
        // The `'static` lifetime is therefore a claim about the address
        // outliving the borrow, which the field order guarantees, and not
        // a claim that the design lives forever.
        let borrowed: &'static crate::ir::Design = unsafe { &*std::ptr::from_ref(owned.as_ref()) };

        let options = SimOptions {
            top: top.map(str::to_owned),
            ..SimOptions::default()
        };
        let sim = Simulator::new(borrowed, options)?;
        let nets = sim.nets();
        Ok(reticle_sim {
            sim,
            design: owned,
            map: super::clone_source_map(&design.map),
            nets,
        })
    }

    /// How many nets the simulation has.
    fn net_count(&self) -> Result<usize, c_int> {
        Ok(self.nets.len())
    }

    /// The hierarchical path of one net.
    fn net_name(&self, index: usize) -> Result<&str, c_int> {
        self.nets
            .get(index)
            .map(|(name, _)| name.as_str())
            .ok_or(RETICLE_ERR_INVALID)
    }

    /// The index of the net at a hierarchical path.
    fn find_net(&self, path: &str) -> Result<usize, c_int> {
        self.nets
            .iter()
            .position(|(name, _)| name == path)
            .ok_or(super::RETICLE_ERR_NOT_FOUND)
    }

    /// The handle of the net at `index`.
    fn handle(&self, index: usize) -> Result<crate::sim::NetHandle, c_int> {
        self.nets
            .get(index)
            .map(|&(_, h)| h)
            .ok_or(RETICLE_ERR_INVALID)
    }

    /// The declared width of one net, in bits.
    fn net_width(&self, index: usize) -> Result<u32, c_int> {
        Ok(self.sim.get(self.handle(index)?).width())
    }

    /// One net's value, as a Verilog literal such as `8'h2a`.
    fn get(&self, index: usize) -> Result<String, c_int> {
        Ok(self.sim.get(self.handle(index)?).to_verilog_literal())
    }

    /// One net's value as an integer, if every bit is known.
    fn get_u64(&self, index: usize) -> Result<u64, c_int> {
        self.sim
            .get(self.handle(index)?)
            .to_u64()
            .ok_or(super::RETICLE_ERR_UNKNOWN_VALUE)
    }

    /// Writes a net from a Verilog literal, resized to the net's width.
    fn set(&mut self, index: usize, literal: &str) -> Result<(), c_int> {
        let handle = self.handle(index)?;
        let width = self.sim.get(handle).width();
        let value = crate::logic::Logic::parse_verilog(literal).map_err(|_| RETICLE_ERR_INVALID)?;
        self.sim.set(handle, value.resize(width));
        Ok(())
    }

    /// Writes a net from an integer, truncated to the net's width.
    fn set_u64(&mut self, index: usize, value: u64) -> Result<(), c_int> {
        let handle = self.handle(index)?;
        let width = self.sim.get(handle).width();
        self.sim
            .set(handle, crate::logic::Logic::from_u64(value, width));
        Ok(())
    }

    /// Advances the simulation by `ticks`.
    fn run_for(&mut self, ticks: u64) -> Result<(), c_int> {
        self.sim.run_for(ticks);
        Ok(())
    }

    /// Advances the simulation to `time`.
    fn run_until(&mut self, time: u64) -> Result<(), c_int> {
        self.sim.run_until(time);
        Ok(())
    }

    /// Runs until the event queue drains or the design stops.
    fn run(&mut self) -> Result<(), c_int> {
        self.sim.run();
        Ok(())
    }

    /// The current simulation time, in ticks.
    fn time(&self) -> Result<u64, c_int> {
        Ok(self.sim.time())
    }

    /// The current `RETICLE_SIM_*` status.
    fn status(&self) -> Result<c_int, c_int> {
        use crate::sim::Status;
        Ok(match self.sim.status() {
            Status::Running => super::RETICLE_SIM_RUNNING,
            Status::Stopped => super::RETICLE_SIM_STOPPED,
            Status::Finished => super::RETICLE_SIM_FINISHED,
        })
    }

    /// How many ticks make one nanosecond in this design's timescale.
    fn ticks_per_ns(&self) -> Result<u64, c_int> {
        use crate::ir::{Delay, TimeUnit};
        Ok(self.sim.ticks(Delay::new(1, TimeUnit::Ns)))
    }

    /// Takes the `$display` / `$write` output produced so far.
    fn take_output(&mut self) -> Result<String, c_int> {
        Ok(self.sim.take_output())
    }

    /// Starts VCD capture, discarding any capture already running.
    fn enable_vcd(&mut self) -> Result<(), c_int> {
        self.sim.enable_vcd();
        Ok(())
    }

    /// The VCD text captured so far; empty when capture is off.
    fn vcd(&self) -> Result<String, c_int> {
        Ok(self.sim.vcd().unwrap_or_default().to_owned())
    }

    /// Takes the simulator's own diagnostics as a snapshot.
    fn take_messages(&mut self) -> Result<reticle_diagnostics, c_int> {
        let diags = self.sim.take_messages();
        Ok(reticle_diagnostics::snapshot(diags, &self.map))
    }
}

#[cfg(not(feature = "sim"))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the signatures mirror the `sim` build's"
)]
impl reticle_sim {
    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn net_count(&self) -> Result<usize, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn net_name(&self, _index: usize) -> Result<&str, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn find_net(&self, _path: &str) -> Result<usize, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn net_width(&self, _index: usize) -> Result<u32, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn get(&self, _index: usize) -> Result<String, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn get_u64(&self, _index: usize) -> Result<u64, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn set(&mut self, _index: usize, _literal: &str) -> Result<(), c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn set_u64(&mut self, _index: usize, _value: u64) -> Result<(), c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn run_for(&mut self, _ticks: u64) -> Result<(), c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn run_until(&mut self, _time: u64) -> Result<(), c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn run(&mut self) -> Result<(), c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn time(&self) -> Result<u64, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn status(&self) -> Result<c_int, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn ticks_per_ns(&self) -> Result<u64, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn take_output(&mut self) -> Result<String, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn enable_vcd(&mut self) -> Result<(), c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn vcd(&self) -> Result<String, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }

    /// See the `sim` build; always [`RETICLE_ERR_UNSUPPORTED`] here.
    fn take_messages(&mut self) -> Result<reticle_diagnostics, c_int> {
        Err(RETICLE_ERR_UNSUPPORTED)
    }
}

/// Elaborates a design for simulation.
///
/// `top` names the module to instantiate as the root, or is null to use
/// the design's own top. The simulator takes its own copy of the design,
/// so `design` may be freed immediately afterwards.
///
/// On failure (no top, a recursive hierarchy) the reason is reported
/// through `out_diags`, which may be null. Warnings raised during
/// elaboration are not errors and are collected by
/// [`reticle_sim_take_messages`] instead.
///
/// # Safety
///
/// `design` must be a live design handle, `top` must be null or a
/// NUL-terminated string, `out_sim` must point at a writable handle slot,
/// and `out_diags` must be null or point at one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_create(
    design: *const reticle_design,
    top: *const c_char,
    out_sim: *mut *mut reticle_sim,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        if out_sim.is_null() {
            return RETICLE_ERR_INVALID;
        }
        // SAFETY: as documented; `as_ref` and `opt_str_arg` reject a null
        // or non-UTF-8 argument themselves.
        let Some(d) = (unsafe { as_ref(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        let Some(top) = (unsafe { super::opt_str_arg(top) }) else {
            return RETICLE_ERR_INVALID;
        };
        create(d, top, out_sim, out_diags)
    })
}

/// The `sim`-enabled body of [`reticle_sim_create`].
#[cfg(feature = "sim")]
fn create(
    design: &reticle_design,
    top: Option<&str>,
    out_sim: *mut *mut reticle_sim,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    match reticle_sim::new(design, top) {
        Ok(handle) => {
            // SAFETY: `out_diags` is null or a writable handle slot.
            unsafe { diagnostics::publish(out_diags, diagnostics::empty()) };
            // SAFETY: `out_sim` was checked non-null by the caller and is,
            // per the contract, a writable and aligned handle slot.
            unsafe { *out_sim = into_handle(handle) };
            RETICLE_OK
        }
        Err(diags) => {
            let snapshot = reticle_diagnostics::snapshot(diags, &design.map);
            // SAFETY: `out_diags` is null or a writable handle slot.
            unsafe { diagnostics::publish(out_diags, snapshot) };
            super::RETICLE_ERR_DIAGNOSTICS
        }
    }
}

/// The body of [`reticle_sim_create`] without the `sim` feature.
#[cfg(not(feature = "sim"))]
fn create(
    _design: &reticle_design,
    _top: Option<&str>,
    _out_sim: *mut *mut reticle_sim,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    // SAFETY: `out_diags` is null or a writable handle slot, per the
    // contract of `reticle_sim_create`.
    unsafe {
        diagnostics::publish(
            out_diags,
            diagnostics::single_error("this library was built without the `sim` feature"),
        )
    };
    RETICLE_ERR_UNSUPPORTED
}

/// Releases a simulation handle. A null pointer is ignored.
///
/// # Safety
///
/// `sim` must be null, or a handle from this library that has not already
/// been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_free(sim: *mut reticle_sim) {
    // SAFETY: the caller guarantees `sim` is null or a live handle that
    // this module allocated with `into_handle`.
    unsafe { drop_handle(sim) };
}

/// Borrows a simulation handle, or reports why it cannot.
///
/// # Safety
///
/// `sim` must be null or a live handle.
unsafe fn borrow<'a>(sim: *const reticle_sim) -> Result<&'a reticle_sim, c_int> {
    // SAFETY: delegated to `as_ref`, whose contract this repeats.
    unsafe { as_ref(sim) }.ok_or(RETICLE_ERR_INVALID)
}

/// Borrows a simulation handle mutably.
///
/// # Safety
///
/// `sim` must be null or a live, unaliased handle.
unsafe fn borrow_mut<'a>(sim: *mut reticle_sim) -> Result<&'a mut reticle_sim, c_int> {
    // SAFETY: delegated to `as_mut`, whose contract this repeats.
    unsafe { as_mut(sim) }.ok_or(RETICLE_ERR_INVALID)
}

/// Writes how many nets the simulation has.
///
/// Net indices run from 0 to this count and stay valid for the life of
/// the handle.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_net_count(sim: *const reticle_sim, out: *mut usize) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(reticle_sim::net_count) {
            Ok(value) => unsafe { write_out(out, value) },
            Err(status) => status,
        }
    })
}

/// Writes the hierarchical path of the net at `index`, such as
/// `counter.q`.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_net_name(
    sim: *const reticle_sim,
    index: usize,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(|s| s.net_name(index)) {
            Ok(name) => unsafe { write_string(out, name) },
            Err(status) => status,
        }
    })
}

/// Writes the index of the net at a hierarchical path.
///
/// Returns [`RETICLE_ERR_NOT_FOUND`](super::RETICLE_ERR_NOT_FOUND) when
/// the simulation has no such net.
///
/// # Safety
///
/// `sim` must be a live handle, `path` a NUL-terminated string, and `out`
/// must point at a writable `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_find_net(
    sim: *const reticle_sim,
    path: *const c_char,
    out: *mut usize,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null and non-UTF-8.
        let Some(path) = (unsafe { str_arg(path) }) else {
            return RETICLE_ERR_INVALID;
        };
        match unsafe { borrow(sim) }.and_then(|s| s.find_net(path)) {
            Ok(index) => unsafe { write_out(out, index) },
            Err(status) => status,
        }
    })
}

/// Writes the width of the net at `index`, in bits.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `uint32_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_net_width(
    sim: *const reticle_sim,
    index: usize,
    out: *mut u32,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(|s| s.net_width(index)) {
            Ok(width) => unsafe { write_out(out, width) },
            Err(status) => status,
        }
    })
}

/// Writes one net's value as a Verilog literal, such as `8'h2a` or
/// `4'bx0z1`.
///
/// Every value is representable this way, including one with unknown
/// bits, which is why this and not an integer is the general accessor.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_get(
    sim: *const reticle_sim,
    index: usize,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(|s| s.get(index)) {
            Ok(text) => unsafe { write_string(out, &text) },
            Err(status) => status,
        }
    })
}

/// Writes one net's value as an unsigned integer.
///
/// Returns
/// [`RETICLE_ERR_UNKNOWN_VALUE`](super::RETICLE_ERR_UNKNOWN_VALUE) when
/// the value has `x` or `z` bits, and
/// [`RETICLE_ERR_INVALID`] when it is wider than 64 bits; use
/// [`reticle_sim_get`] for those.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `uint64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_get_u64(
    sim: *const reticle_sim,
    index: usize,
    out: *mut u64,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(|s| s.get_u64(index)) {
            Ok(value) => unsafe { write_out(out, value) },
            Err(status) => status,
        }
    })
}

/// Writes a net from a Verilog literal such as `1'b1` or `8'hff`.
///
/// The value is resized to the net's width, and the write takes effect
/// immediately; its fanout is evaluated by the next run call. A net driven
/// by continuous assignment is overwritten again when its driver
/// re-evaluates.
///
/// # Safety
///
/// `sim` must be a live handle and `literal` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_set(
    sim: *mut reticle_sim,
    index: usize,
    literal: *const c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null and non-UTF-8.
        let Some(literal) = (unsafe { str_arg(literal) }) else {
            return RETICLE_ERR_INVALID;
        };
        match unsafe { borrow_mut(sim) }.and_then(|s| s.set(index, literal)) {
            Ok(()) => RETICLE_OK,
            Err(status) => status,
        }
    })
}

/// Writes a net from an unsigned integer, truncated to the net's width.
///
/// # Safety
///
/// `sim` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_set_u64(
    sim: *mut reticle_sim,
    index: usize,
    value: u64,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `borrow_mut` rejects a null pointer.
        match unsafe { borrow_mut(sim) }.and_then(|s| s.set_u64(index, value)) {
            Ok(()) => RETICLE_OK,
            Err(status) => status,
        }
    })
}

/// Advances the simulation by `ticks` ticks of its precision.
///
/// Use [`reticle_sim_ticks_per_ns`] to turn a wall-clock delay into
/// ticks. The call returns early when `$finish` or `$stop` runs.
///
/// # Safety
///
/// `sim` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_run_for(sim: *mut reticle_sim, ticks: u64) -> c_int {
    guard(|| {
        // SAFETY: as documented; `borrow_mut` rejects a null pointer.
        match unsafe { borrow_mut(sim) }.and_then(|s| s.run_for(ticks)) {
            Ok(()) => RETICLE_OK,
            Err(status) => status,
        }
    })
}

/// Advances the simulation to absolute time `time`, in ticks.
///
/// # Safety
///
/// `sim` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_run_until(sim: *mut reticle_sim, time: u64) -> c_int {
    guard(|| {
        // SAFETY: as documented; `borrow_mut` rejects a null pointer.
        match unsafe { borrow_mut(sim) }.and_then(|s| s.run_until(time)) {
            Ok(()) => RETICLE_OK,
            Err(status) => status,
        }
    })
}

/// Runs until the event queue drains, `$finish` runs or `$stop` pauses.
///
/// A self-driving testbench needs nothing else; a design with a free
/// running clock and no `$finish` would never return, so drive those with
/// [`reticle_sim_run_for`].
///
/// # Safety
///
/// `sim` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_run(sim: *mut reticle_sim) -> c_int {
    guard(|| {
        // SAFETY: as documented; `borrow_mut` rejects a null pointer.
        match unsafe { borrow_mut(sim) }.and_then(reticle_sim::run) {
            Ok(()) => RETICLE_OK,
            Err(status) => status,
        }
    })
}

/// Writes the current simulation time, in ticks.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `uint64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_time(sim: *const reticle_sim, out: *mut u64) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(reticle_sim::time) {
            Ok(value) => unsafe { write_out(out, value) },
            Err(status) => status,
        }
    })
}

/// Writes the current `RETICLE_SIM_*` status.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable `int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_status(sim: *const reticle_sim, out: *mut c_int) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(reticle_sim::status) {
            Ok(value) => unsafe { write_out(out, value) },
            Err(status) => status,
        }
    })
}

/// Writes how many ticks make one nanosecond in this design's timescale.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `uint64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_ticks_per_ns(sim: *const reticle_sim, out: *mut u64) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(reticle_sim::ticks_per_ns) {
            Ok(value) => unsafe { write_out(out, value) },
            Err(status) => status,
        }
    })
}

/// Takes the `$display` and `$write` output produced since the last call.
///
/// The buffer is cleared, so a long run can be drained incrementally
/// without it growing without bound.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_take_output(
    sim: *mut reticle_sim,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow_mut(sim) }.and_then(reticle_sim::take_output) {
            Ok(text) => unsafe { write_string(out, &text) },
            Err(status) => status,
        }
    })
}

/// Starts VCD capture: a header and a snapshot of every net now, then
/// every later change.
///
/// Calling it again restarts the capture from the current time.
///
/// # Safety
///
/// `sim` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_enable_vcd(sim: *mut reticle_sim) -> c_int {
    guard(|| {
        // SAFETY: as documented; `borrow_mut` rejects a null pointer.
        match unsafe { borrow_mut(sim) }.and_then(reticle_sim::enable_vcd) {
            Ok(()) => RETICLE_OK,
            Err(status) => status,
        }
    })
}

/// Writes the VCD text captured so far, or the empty string when capture
/// was never enabled.
///
/// The capture keeps running; call this again later for a longer dump.
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_vcd(sim: *const reticle_sim, out: *mut *mut c_char) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null pointers.
        match unsafe { borrow(sim) }.and_then(reticle_sim::vcd) {
            Ok(text) => unsafe { write_string(out, &text) },
            Err(status) => status,
        }
    })
}

/// Takes the simulator's own diagnostics: unresolved instances, port width
/// mismatches, failing assertions and `$error` calls.
///
/// The simulator's list is cleared, so each call reports only what is new.
/// `out` receives a handle the caller frees with
/// [`reticle_diagnostics_free`](super::reticle_diagnostics_free).
///
/// # Safety
///
/// `sim` must be a live handle and `out` must point at a writable
/// diagnostics handle slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_sim_take_messages(
    sim: *mut reticle_sim,
    out: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        if out.is_null() {
            return RETICLE_ERR_INVALID;
        }
        // SAFETY: as documented; `borrow_mut` rejects a null pointer.
        match unsafe { borrow_mut(sim) }.and_then(reticle_sim::take_messages) {
            Ok(snapshot) => {
                // SAFETY: `out` was checked non-null just above.
                unsafe { diagnostics::publish(out, snapshot) };
                RETICLE_OK
            }
            Err(status) => status,
        }
    })
}
