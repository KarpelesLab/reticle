//! The C ABI: embedding Reticle in a tool written in another language.
//!
//! This module exports a flat `extern "C"` surface over the frontends, the
//! IR, synthesis and the simulator. The companion header is
//! [`reticle.h`][header], which lives next to this file and ships with the
//! crate; `tests/ffi_header.rs` checks that the header and the exported
//! symbols cannot drift apart.
//!
//! [header]: https://github.com/KarpelesLab/reticle/blob/master/src/ffi/reticle.h
//!
//! # Shape of the ABI
//!
//! The rules below are what make the surface small and hard to misuse.
//! They hold for every function without exception.
//!
//! - **Opaque handles.** `reticle_design`, `reticle_sim` and
//!   `reticle_diagnostics` are pointers to types C never sees the layout
//!   of. Each is produced by a named constructor and released by its
//!   `_free`; no struct crosses the boundary, so adding a field here can
//!   never break a compiled caller.
//! - **A status and an out-parameter.** Every fallible call returns an
//!   `int` from the `RETICLE_*` set and writes its result through a
//!   pointer argument. A non-zero status leaves every out-parameter
//!   untouched, so a caller that checks the status never reads a stale or
//!   half-written value.
//! - **Owned strings.** A `char **` out-parameter receives a
//!   NUL-terminated, heap-allocated, UTF-8 string that the caller releases
//!   with [`reticle_string_free`]. The two exceptions are
//!   [`reticle_version`] and [`reticle_status_message`], which return
//!   `const char *` pointing at static storage and must not be freed; the
//!   header says so on each.
//! - **No panics cross the boundary.** Unwinding out of an `extern "C"`
//!   function is undefined behaviour, so every entry point runs its body
//!   inside [`std::panic::catch_unwind`] and reports a panic as
//!   `RETICLE_ERR_PANIC`. [`reticle_self_test_panic`] exists so an
//!   embedder can check that for itself.
//! - **One fixed ABI.** Every symbol is exported whatever Cargo features
//!   are on. A call into a stage that was not compiled in returns
//!   `RETICLE_ERR_UNSUPPORTED`; [`reticle_features`] reports which stages
//!   are present. A caller therefore links against one header regardless
//!   of how the library was built.
//!
//! # Threading
//!
//! A handle is owned by its caller and carries no internal
//! synchronisation. Two threads may drive two different handles at the
//! same time; one handle must not be used from two threads at once.
//!
//! # Building
//!
//! ```sh
//! cargo rustc --release --features ffi --crate-type cdylib     # libreticle.so
//! cargo rustc --release --features ffi --crate-type staticlib  # libreticle.a
//! ```
//!
//! `docs/ffi.md` has the full recipe, including the link flags a static
//! build needs, and `examples/ffi/demo.c` is a worked example.
//!
//! # `unsafe`
//!
//! The crate denies `unsafe_code`; this module and [`crate::wasm`] are the
//! only places that opt back in, because a C ABI cannot be written without
//! raw pointers. Every `unsafe` block below states the invariant it relies
//! on. Nothing here dereferences a pointer it has not first checked for
//! null, and the safety contract a caller must meet is written on each
//! function.

// The one documented exception to the crate-wide `deny(unsafe_code)`; see
// the module docs above and `src/lib.rs`.
#![allow(unsafe_code)]
// C spells its types `reticle_design`, not `ReticleDesign`, and the header
// and the Rust definitions are easier to keep in step when they agree.
#![allow(non_camel_case_types)]

use std::ffi::{CStr, CString, c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};

mod design;
mod diagnostics;
// Shared with `crate::wasm`, which uses this copy when both features are
// on and includes the same file itself when only `wasm` is.
pub(crate) mod pipeline;
mod simulator;

pub use design::{
    reticle_design, reticle_design_emit, reticle_design_free, reticle_design_load_rtl,
    reticle_design_module_count, reticle_design_module_name, reticle_design_save_rtl,
    reticle_design_set_top, reticle_design_synth, reticle_design_top, reticle_elaborate_verilog,
    reticle_elaborate_verilog_source, reticle_elaborate_vhdl, reticle_elaborate_vhdl_source,
};
pub use diagnostics::{
    reticle_diagnostic_code, reticle_diagnostic_column, reticle_diagnostic_file,
    reticle_diagnostic_line, reticle_diagnostic_message, reticle_diagnostic_note,
    reticle_diagnostic_note_count, reticle_diagnostic_severity, reticle_diagnostics,
    reticle_diagnostics_count, reticle_diagnostics_error_count, reticle_diagnostics_free,
    reticle_diagnostics_render, reticle_diagnostics_warning_count,
};
pub use simulator::{
    reticle_sim, reticle_sim_create, reticle_sim_enable_vcd, reticle_sim_find_net,
    reticle_sim_free, reticle_sim_get, reticle_sim_get_u64, reticle_sim_net_count,
    reticle_sim_net_name, reticle_sim_net_width, reticle_sim_run, reticle_sim_run_for,
    reticle_sim_run_until, reticle_sim_set, reticle_sim_set_u64, reticle_sim_status,
    reticle_sim_take_messages, reticle_sim_take_output, reticle_sim_ticks_per_ns, reticle_sim_time,
    reticle_sim_vcd,
};

// ---------------------------------------------------------------------------
// Status codes
// ---------------------------------------------------------------------------

/// The call succeeded and every out-parameter was written.
pub const RETICLE_OK: c_int = 0;
/// A null pointer, a string that is not UTF-8, or an index out of range.
pub const RETICLE_ERR_INVALID: c_int = 1;
/// The stage ran and reported errors; the diagnostics handle has them.
pub const RETICLE_ERR_DIAGNOSTICS: c_int = 2;
/// A name (a net path, a module, an output format) does not exist.
pub const RETICLE_ERR_NOT_FOUND: c_int = 3;
/// The stage this call needs was not compiled into the library.
pub const RETICLE_ERR_UNSUPPORTED: c_int = 4;
/// The operation failed for a reason that is a bug rather than bad input.
pub const RETICLE_ERR_INTERNAL: c_int = 5;
/// The value has `x` or `z` bits and has no integer form.
pub const RETICLE_ERR_UNKNOWN_VALUE: c_int = 6;
/// A panic was caught at the boundary; the library state is unchanged for
/// this call but the handle should be treated as suspect.
pub const RETICLE_ERR_PANIC: c_int = 7;

/// The severity of a diagnostic, as reported by
/// [`reticle_diagnostic_severity`].
pub const RETICLE_SEVERITY_HELP: c_int = 0;
/// See [`RETICLE_SEVERITY_HELP`].
pub const RETICLE_SEVERITY_NOTE: c_int = 1;
/// See [`RETICLE_SEVERITY_HELP`].
pub const RETICLE_SEVERITY_WARNING: c_int = 2;
/// See [`RETICLE_SEVERITY_HELP`].
pub const RETICLE_SEVERITY_ERROR: c_int = 3;

/// The simulation may still process events.
pub const RETICLE_SIM_RUNNING: c_int = 0;
/// `$stop` was executed; the next run call resumes.
pub const RETICLE_SIM_STOPPED: c_int = 1;
/// `$finish` ran, or a failing assertion ended the run.
pub const RETICLE_SIM_FINISHED: c_int = 2;

// ---------------------------------------------------------------------------
// Panic and allocation plumbing
// ---------------------------------------------------------------------------

/// Runs `f` with unwinding stopped at the ABI boundary.
///
/// A panic escaping an `extern "C"` function is undefined behaviour, so
/// every entry point in this module funnels through here and reports
/// [`RETICLE_ERR_PANIC`] instead. The closure only ever captures raw
/// pointers and `Copy` scalars, which is why asserting unwind safety is
/// sound: there is no logically-inconsistent Rust value left behind for a
/// later call to observe.
fn guard(f: impl FnOnce() -> c_int) -> c_int {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(status) => status,
        Err(_) => RETICLE_ERR_PANIC,
    }
}

/// Counts the handles and strings this module has handed out but not yet
/// taken back, so the unit tests can assert that a sequence of calls
/// leaks nothing.
///
/// It is compiled only for `cargo test` of this crate: a release build
/// must not pay for a thread-local bump on every allocation.
#[cfg(test)]
pub(crate) mod live {
    use std::cell::Cell;

    thread_local! {
        static COUNT: Cell<isize> = const { Cell::new(0) };
    }

    /// Records one allocation handed to C.
    pub(crate) fn alloc() {
        COUNT.with(|c| c.set(c.get() + 1));
    }

    /// Records one allocation taken back from C.
    pub(crate) fn free() {
        COUNT.with(|c| c.set(c.get() - 1));
    }

    /// How many allocations are currently outstanding on this thread.
    pub(crate) fn count() -> isize {
        COUNT.with(Cell::get)
    }
}

#[cfg(test)]
use live::{alloc as track_alloc, free as track_free};

/// No-op counterparts of [`live`] for a non-test build.
#[cfg(not(test))]
fn track_alloc() {}

/// See [`track_alloc`].
#[cfg(not(test))]
fn track_free() {}

/// Moves `value` onto the heap and hands C a pointer to it.
pub(crate) fn into_handle<T>(value: T) -> *mut T {
    track_alloc();
    Box::into_raw(Box::new(value))
}

/// Takes a handle back and drops it; a null pointer is ignored.
///
/// # Safety
///
/// `ptr` must be null, or a pointer returned by [`into_handle`] for the
/// same `T` that has not already been passed here.
pub(crate) unsafe fn drop_handle<T>(ptr: *mut T) {
    if ptr.is_null() {
        return;
    }
    track_free();
    // SAFETY: the caller guarantees `ptr` came from `into_handle::<T>`
    // (that is, from `Box::into_raw`) and has not been freed, so
    // reconstructing the box is the matching deallocation.
    drop(unsafe { Box::from_raw(ptr) });
}

/// Borrows a handle for the duration of a call.
///
/// # Safety
///
/// `ptr` must be null or a live pointer from [`into_handle`] for the same
/// `T`, and no other reference to it may be live while the returned one is.
pub(crate) unsafe fn as_ref<'a, T>(ptr: *const T) -> Option<&'a T> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees the pointer is live, correctly typed
    // and not aliased by another reference for the returned lifetime.
    Some(unsafe { &*ptr })
}

/// Borrows a handle mutably for the duration of a call.
///
/// # Safety
///
/// As [`as_ref`], and no other reference to the value may exist at all.
pub(crate) unsafe fn as_mut<'a, T>(ptr: *mut T) -> Option<&'a mut T> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees the pointer is live, correctly typed
    // and unaliased for the returned lifetime.
    Some(unsafe { &mut *ptr })
}

/// Reads a borrowed C string as UTF-8.
///
/// Returns `None` for a null pointer or for bytes that are not UTF-8, both
/// of which the caller reports as [`RETICLE_ERR_INVALID`].
///
/// # Safety
///
/// `ptr` must be null or point at a NUL-terminated byte string that stays
/// valid and unmodified for the duration of the call.
pub(crate) unsafe fn str_arg<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees `ptr` is NUL-terminated and stable for
    // the call, which is exactly `CStr::from_ptr`'s contract.
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Reads an optional C string: a null pointer means "not given".
///
/// The outer `Option` separates "absent" from "present but invalid": the
/// result is `Some(None)` for null and `None` for bad UTF-8.
///
/// # Safety
///
/// As [`str_arg`].
pub(crate) unsafe fn opt_str_arg<'a>(ptr: *const c_char) -> Option<Option<&'a str>> {
    if ptr.is_null() {
        return Some(None);
    }
    // SAFETY: delegated to `str_arg`, whose contract this function repeats.
    unsafe { str_arg(ptr) }.map(Some)
}

/// Allocates a C string C must release with [`reticle_string_free`].
///
/// Reticle's own output never contains a NUL byte, but a source file can
/// (a Verilog string literal may hold one), and a NUL cannot be carried by
/// a C string. Rather than fail a whole emit over one byte, NULs are
/// dropped; the header documents that.
pub(crate) fn owned_string(s: &str) -> *mut c_char {
    let bytes: Vec<u8> = s.bytes().filter(|&b| b != 0).collect();
    let c = CString::new(bytes).expect("NUL bytes were filtered out");
    track_alloc();
    c.into_raw()
}

/// Writes an owned string through a `char **` out-parameter.
///
/// # Safety
///
/// `out` must be null or point at a writable `*mut c_char`.
pub(crate) unsafe fn write_string(out: *mut *mut c_char, s: &str) -> c_int {
    if out.is_null() {
        return RETICLE_ERR_INVALID;
    }
    let p = owned_string(s);
    // SAFETY: `out` is non-null and the caller guarantees it points at a
    // writable, suitably aligned `*mut c_char`.
    unsafe { *out = p };
    RETICLE_OK
}

/// Writes a scalar through an out-parameter, rejecting a null pointer.
///
/// # Safety
///
/// `out` must be null or point at a writable, aligned `T`.
pub(crate) unsafe fn write_out<T>(out: *mut T, value: T) -> c_int {
    if out.is_null() {
        return RETICLE_ERR_INVALID;
    }
    // SAFETY: `out` is non-null and the caller guarantees it is writable
    // and aligned for `T`.
    unsafe { *out = value };
    RETICLE_OK
}

/// Copies a source map file by file.
///
/// [`crate::source::SourceMap`] is not `Clone`, but its ids are handed out
/// in insertion order, so re-adding every file in that order reproduces
/// the map exactly, ids included. The simulator handle needs its own copy
/// to render the spans in its messages after the design handle that
/// produced them may have been freed.
#[cfg(feature = "sim")]
pub(crate) fn clone_source_map(map: &crate::source::SourceMap) -> crate::source::SourceMap {
    let mut out = crate::source::SourceMap::new();
    for (_, file) in map.files() {
        // The original map accepted these files, so re-adding them cannot
        // exceed either limit.
        out.add(file.name(), file.text())
            .expect("a copy of an accepted source map fits");
    }
    out
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// The library version, as a static NUL-terminated string.
///
/// The pointer is valid for the lifetime of the process and must **not**
/// be passed to [`reticle_string_free`].
#[unsafe(no_mangle)]
pub extern "C" fn reticle_version() -> *const c_char {
    // `concat!` appends the NUL at compile time, so this is a plain
    // pointer into static read-only data with no allocation involved.
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// A short, stable description of a `RETICLE_*` status code.
///
/// The pointer is static and must **not** be freed. An unrecognised code
/// yields `"unknown status"` rather than null, so a caller may print the
/// result unconditionally.
#[unsafe(no_mangle)]
pub extern "C" fn reticle_status_message(status: c_int) -> *const c_char {
    let text = match status {
        RETICLE_OK => "ok\0",
        RETICLE_ERR_INVALID => "invalid argument\0",
        RETICLE_ERR_DIAGNOSTICS => "the design was rejected; see the diagnostics\0",
        RETICLE_ERR_NOT_FOUND => "no such name\0",
        RETICLE_ERR_UNSUPPORTED => "this stage was not compiled into the library\0",
        RETICLE_ERR_INTERNAL => "internal error\0",
        RETICLE_ERR_UNKNOWN_VALUE => "the value has x or z bits\0",
        RETICLE_ERR_PANIC => "a panic was caught at the C boundary\0",
        _ => "unknown status\0",
    };
    text.as_ptr().cast()
}

/// Reports which stages this library was built with.
///
/// Writes a comma-separated list drawn from `verilog`, `vhdl`, `sim` and
/// `synth`, in that order, or the empty string when none are on. Every
/// entry point exists regardless; the ones whose stage is missing return
/// [`RETICLE_ERR_UNSUPPORTED`].
///
/// # Safety
///
/// `out` must point at a writable `char *`, which on success receives a
/// string owned by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_features(out: *mut *mut c_char) -> c_int {
    guard(|| {
        let mut names: Vec<&str> = Vec::new();
        if cfg!(feature = "verilog") {
            names.push("verilog");
        }
        if cfg!(feature = "vhdl") {
            names.push("vhdl");
        }
        if cfg!(feature = "sim") {
            names.push("sim");
        }
        if cfg!(feature = "synth") {
            names.push("synth");
        }
        // SAFETY: the caller guarantees `out` is writable; `write_string`
        // rejects a null pointer itself.
        unsafe { write_string(out, &names.join(",")) }
    })
}

/// Releases a string produced by any `char **` out-parameter.
///
/// A null pointer is ignored, so the usual C cleanup idiom is safe.
///
/// # Safety
///
/// `s` must be null, or a pointer written by one of this library's
/// `char **` out-parameters that has not already been freed. Pointers from
/// [`reticle_version`] and [`reticle_status_message`] are static and must
/// not be passed here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    track_free();
    // SAFETY: the caller guarantees `s` came from `CString::into_raw` in
    // `owned_string` and has not been freed, so reclaiming it as a
    // `CString` is the matching deallocation.
    drop(unsafe { CString::from_raw(s) });
}

/// Panics inside the boundary on purpose, and so always returns
/// [`RETICLE_ERR_PANIC`].
///
/// It exists so an embedder can prove, in its own test suite and against
/// its own build of the library, that a panic in Reticle becomes a status
/// code rather than an unwind through C. It has no other use.
#[unsafe(no_mangle)]
pub extern "C" fn reticle_self_test_panic() -> c_int {
    guard(|| panic!("reticle_self_test_panic: this panic is deliberate"))
}

// The unit tests come last so they follow every item they exercise.
#[cfg(test)]
mod tests;
