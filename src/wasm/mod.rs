//! The WebAssembly surface: the compiler as plain functions over linear
//! memory.
//!
//! Reticle has no dependencies, and that includes the bindgen toolchains,
//! so this module speaks the lowest common denominator a browser
//! understands: `extern "C"` functions, an allocator pair
//! ([`reticle_wasm_alloc`] / [`reticle_wasm_free`]), and buffers in the
//! module's own memory. `cargo build --target wasm32-unknown-unknown` is
//! the whole build step; there is no post-processing and no generated
//! glue. `web/reticle.js` is a hand-written wrapper that turns this into a
//! comfortable JavaScript API, and `web/index.html` is a playground built
//! on it.
//!
//! # The calling convention
//!
//! A string **argument** is a pair: a pointer into linear memory and a
//! byte length, in that order, holding UTF-8. JavaScript allocates it with
//! [`reticle_wasm_alloc`], copies the bytes in, calls, and frees it.
//!
//! A **result** is one pointer to a buffer this module allocated, laid out
//! as a little-endian `u32` byte length followed by that many bytes of
//! UTF-8 JSON. The caller reads the length, decodes the bytes, then
//! returns the buffer with `reticle_wasm_free(ptr, 4 + len)`. A null
//! result means the allocation failed and is the only case the wrapper has
//! to special-case.
//!
//! Every operation answers with a JSON object of the same shape, so one
//! decoder in JavaScript handles all of them:
//!
//! ```json
//! {
//!   "ok": true,
//!   "diagnostics": [
//!     {"severity": "warning", "code": "V0007", "message": "…",
//!      "file": "top.v", "line": 3, "column": 12, "notes": ["…"]}
//!   ],
//!   "errors": 0,
//!   "warnings": 1,
//!   "rendered": "…rustc-style text…"
//! }
//! ```
//!
//! plus whatever that call produces: `rtl`, `output`, `report`, `vcd`,
//! `time` or `status`.
//!
//! # Designs are passed as text
//!
//! There are no handles here. [`reticle_wasm_elaborate`] answers with the
//! design in the `.rtl` text format, and [`reticle_wasm_synth`],
//! [`reticle_wasm_emit`] and [`reticle_wasm_simulate`] take that text
//! back. Nothing in JavaScript owns a pointer it has to release later,
//! which removes the whole class of bugs a handle API has in a garbage
//! collected host, at the cost of re-parsing a design between steps. The
//! `.rtl` round trip is exact, so nothing is lost.
//!
//! # `unsafe`
//!
//! The crate denies `unsafe_code`; this module and [`crate::ffi`] are the
//! only places that opt back in, because reading a buffer the host wrote
//! cannot be expressed safely. Every block states its invariant. The
//! module is not wasm-specific — it compiles and is tested on the host
//! target too, which is how the round trip in `tests` exercises it.

// The one documented exception to the crate-wide `deny(unsafe_code)`; see
// the module docs above and `src/lib.rs`.
#![allow(unsafe_code)]

use std::alloc::{Layout, alloc, dealloc};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::diag::{Diagnostics, Severity};
use crate::ir::Design;
use crate::ir::emit::json_string;
use crate::source::SourceMap;

// The stage calls are shared with the C ABI rather than written twice.
// The two features are independent, so there is no common parent module
// to hold the file: with `ffi` also on this uses the copy that module
// already compiled, and otherwise it pulls the same file in directly.
// Exactly one of the two is ever active, so the file is compiled once.
#[cfg(feature = "ffi")]
use crate::ffi::pipeline;
#[cfg(not(feature = "ffi"))]
#[path = "../ffi/pipeline.rs"]
mod pipeline;

use pipeline::Language;

/// Every buffer this module hands out is a plain byte array, so one-byte
/// alignment is all the layout ever needs.
const ALIGN: usize = 1;

/// The pointer returned for a zero-length allocation.
///
/// `std::alloc::alloc` may not be called with a zero-sized layout, and a
/// caller that asks for nothing still wants a pointer it can pass back to
/// [`reticle_wasm_free`]; an aligned, non-null, never-dereferenced address
/// is the usual answer.
fn dangling() -> *mut u8 {
    std::ptr::NonNull::<u8>::dangling().as_ptr()
}

/// Allocates `len` bytes in the module's linear memory.
///
/// The host writes its UTF-8 argument here and passes the pointer and the
/// length to one of the entry points below. Returns null when `len` cannot
/// be allocated. The buffer is released with [`reticle_wasm_free`] and the
/// *same* length; there is no header and no way to recover it otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn reticle_wasm_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return dangling();
    }
    let Ok(layout) = Layout::from_size_align(len, ALIGN) else {
        return std::ptr::null_mut();
    };
    // SAFETY: `layout` has a non-zero size, which is `alloc`'s only
    // requirement. A null return is the documented out-of-memory answer
    // and is passed straight back to the host.
    unsafe { alloc(layout) }
}

/// Releases a buffer from [`reticle_wasm_alloc`] or from an entry point.
///
/// `len` must be exactly what was allocated: the length passed to
/// [`reticle_wasm_alloc`], or `4 + n` for a result buffer whose header
/// says `n`. A null pointer or a zero length is ignored.
///
/// # Safety
///
/// `ptr` must be null, or a pointer this module returned that has not
/// already been freed, and `len` must be its exact allocated size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_wasm_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    let Ok(layout) = Layout::from_size_align(len, ALIGN) else {
        return;
    };
    // SAFETY: the caller guarantees `ptr` came from this module's
    // allocator with exactly this size and one-byte alignment, which is
    // the layout reconstructed here.
    unsafe { dealloc(ptr, layout) };
}

/// Borrows a `(pointer, length)` argument as a string.
///
/// A null pointer or a zero length is the empty string, which is how the
/// wrapper passes "not given" without a second flag. Bytes that are not
/// UTF-8 are rejected.
///
/// # Safety
///
/// `ptr` must be null or point at `len` readable bytes that stay valid and
/// unmodified for the duration of the call.
unsafe fn arg<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() || len == 0 {
        return Some("");
    }
    // SAFETY: the caller guarantees `len` bytes at `ptr` are readable and
    // stable for the call, which is `from_raw_parts`'s contract.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes).ok()
}

/// Copies `body` into a freshly allocated length-prefixed result buffer.
///
/// The layout is a little-endian `u32` length followed by the bytes. The
/// host frees it with `reticle_wasm_free(ptr, 4 + len)`.
fn response(body: &str) -> *mut u8 {
    let bytes = body.as_bytes();
    let Ok(len) = u32::try_from(bytes.len()) else {
        // A body of four gigabytes cannot happen in a 32-bit address
        // space; answering null is still better than truncating a length.
        return std::ptr::null_mut();
    };
    let total = bytes.len() + 4;
    let Ok(layout) = Layout::from_size_align(total, ALIGN) else {
        return std::ptr::null_mut();
    };
    // SAFETY: `total` is at least 4, so the layout has a non-zero size.
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        return ptr;
    }
    // SAFETY: `ptr` is a fresh allocation of `4 + bytes.len()` bytes, so
    // the header write and the copy that follows it are both in bounds,
    // and the source and destination cannot overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(len.to_le_bytes().as_ptr(), ptr, 4);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.add(4), bytes.len());
    }
    ptr
}

/// Runs `f` and turns a panic into an error response, since unwinding out
/// of an `extern "C"` function is undefined behaviour.
///
/// The closure captures only raw pointers and `Copy` scalars, so asserting
/// unwind safety leaves no inconsistent value behind for a later call.
fn guard(f: impl FnOnce() -> String) -> *mut u8 {
    let body = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(body) => body,
        Err(_) => Reply::failed("a panic was caught at the WebAssembly boundary").json(),
    };
    response(&body)
}

// ---------------------------------------------------------------------------
// Replies
// ---------------------------------------------------------------------------

/// One operation's answer, built up and then rendered as JSON.
///
/// JSON is assembled by hand: the crate ships no serialiser, the shape is
/// fixed, and [`json_string`] already escapes exactly what JSON needs.
struct Reply {
    ok: bool,
    diagnostics: String,
    errors: usize,
    warnings: usize,
    rendered: String,
    fields: Vec<(&'static str, String)>,
}

impl Reply {
    /// An empty successful reply.
    fn new() -> Reply {
        Reply {
            ok: true,
            diagnostics: "[]".to_owned(),
            errors: 0,
            warnings: 0,
            rendered: String::new(),
            fields: Vec::new(),
        }
    }

    /// A failed reply carrying one message and nothing else, for the
    /// errors that never reach a compiler stage (a bad argument, a name
    /// that is not a format, a panic).
    fn failed(message: &str) -> Reply {
        let mut reply = Reply::new();
        reply.ok = false;
        reply.errors = 1;
        reply.rendered = format!("error: {message}\n");
        reply.diagnostics = format!(
            "[{{\"severity\":\"error\",\"code\":\"\",\"message\":{},\
             \"file\":\"\",\"line\":0,\"column\":0,\"notes\":[]}}]",
            json_string(message)
        );
        reply
    }

    /// Records the diagnostics a stage produced, resolved against `map`.
    ///
    /// They are sorted by source position first, so the same input always
    /// yields the same list.
    fn with_diagnostics(mut self, mut diags: Diagnostics, map: &SourceMap) -> Reply {
        diags.sort();
        self.rendered = diags.render(map);
        self.errors = diags.error_count();
        self.warnings = diags.warning_count();

        let mut out = String::from("[");
        for (i, d) in diags.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let (file, line, column) = match d.primary_span() {
                Some(span) => {
                    let (name, loc) = map.locate(span);
                    (name.to_owned(), loc.line, loc.col)
                }
                None => (String::new(), 0, 0),
            };
            let severity = match d.severity {
                Severity::Help => "help",
                Severity::Note => "note",
                Severity::Warning => "warning",
                Severity::Error => "error",
            };
            let _ = write!(
                out,
                "{{\"severity\":\"{severity}\",\"code\":{},\"message\":{},\
                 \"file\":{},\"line\":{line},\"column\":{column},\"notes\":[",
                json_string(d.code.unwrap_or_default()),
                json_string(&d.message),
                json_string(&file),
            );
            for (j, note) in d.notes.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                out.push_str(&json_string(note));
            }
            out.push_str("]}");
        }
        out.push(']');
        self.diagnostics = out;
        self
    }

    /// Marks the reply as a failure.
    fn fail(mut self) -> Reply {
        self.ok = false;
        self
    }

    /// Adds a string-valued field to the reply.
    fn text(mut self, key: &'static str, value: impl Into<String>) -> Reply {
        self.fields.push((key, json_string(&value.into())));
        self
    }

    /// Adds a number-valued field to the reply.
    ///
    /// Only the simulation reply has one (`time`), so this is compiled
    /// with the stage that uses it.
    #[cfg(feature = "sim")]
    fn number(mut self, key: &'static str, value: u64) -> Reply {
        self.fields.push((key, value.to_string()));
        self
    }

    /// Renders the whole reply as one JSON object.
    fn json(self) -> String {
        let mut out = format!(
            "{{\"ok\":{},\"errors\":{},\"warnings\":{},\"rendered\":{},\"diagnostics\":{}",
            self.ok,
            self.errors,
            self.warnings,
            json_string(&self.rendered),
            self.diagnostics,
        );
        for (key, value) in &self.fields {
            let _ = write!(out, ",\"{key}\":{value}");
        }
        out.push('}');
        out
    }
}

/// Parses a `.rtl` design, or returns the reply that explains why not.
fn load_rtl(text: &str) -> Result<(Design, SourceMap), Reply> {
    let mut map = SourceMap::new();
    let Ok(id) = map.add("design.rtl", text) else {
        return Err(Reply::failed("the design text is too large"));
    };
    match Design::parse_text(text, id) {
        Ok(design) => Ok((design, map)),
        Err(diags) => Err(Reply::new().fail().with_diagnostics(diags, &map)),
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// The library version, as a length-prefixed plain string (not JSON).
///
/// Free it with `reticle_wasm_free(ptr, 4 + len)` like any other result.
#[unsafe(no_mangle)]
pub extern "C" fn reticle_wasm_version() -> *mut u8 {
    response(crate::VERSION)
}

/// The stages this module was built with, as a length-prefixed
/// comma-separated string (not JSON).
///
/// Drawn from `verilog`, `vhdl`, `sim` and `synth`, in that order. The
/// playground uses it to grey out a button whose stage is missing.
#[unsafe(no_mangle)]
pub extern "C" fn reticle_wasm_features() -> *mut u8 {
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
    response(&names.join(","))
}

/// Parses and elaborates one source into a design.
///
/// `language` is `verilog` or `vhdl`, `name` is the display name
/// diagnostics print, `source` is the text and `top` names the root module
/// or entity (empty to let elaboration choose). On success the reply has
/// an `rtl` field holding the design in the `.rtl` text format, which the
/// other entry points take back.
///
/// # Safety
///
/// Each `(pointer, length)` pair must be null-with-zero or point at that
/// many readable bytes, valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_wasm_elaborate(
    language: *const u8,
    language_len: usize,
    name: *const u8,
    name_len: usize,
    source: *const u8,
    source_len: usize,
    top: *const u8,
    top_len: usize,
) -> *mut u8 {
    guard(|| {
        // SAFETY: the caller guarantees every pair is readable for its
        // length and stable for the call; `arg` rejects non-UTF-8 itself.
        let (Some(language), Some(name), Some(source), Some(top)) = (unsafe {
            (
                arg(language, language_len),
                arg(name, name_len),
                arg(source, source_len),
                arg(top, top_len),
            )
        }) else {
            return Reply::failed("an argument is not valid UTF-8").json();
        };
        let Some(language) = Language::from_name(language) else {
            return Reply::failed("`language` must be `verilog` or `vhdl`").json();
        };
        let name = if name.is_empty() {
            match language {
                Language::Verilog => "input.v",
                Language::Vhdl => "input.vhd",
            }
        } else {
            name
        };
        let top = (!top.is_empty()).then_some(top);

        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let design = pipeline::elaborate(language, &[(name, source)], top, &mut map, &mut diags);
        let reply = Reply::new().with_diagnostics(diags, &map);
        match design {
            Some(design) if reply.errors == 0 => reply.text("rtl", design.to_text()).json(),
            _ => reply.fail().json(),
        }
    })
}

/// Synthesises a design given in the `.rtl` text format.
///
/// `lut_inputs` is 0 for a generic netlist, or 2 to 8 to map the logic
/// that is left onto `k`-input LUTs. The reply has the synthesised design
/// in `rtl` and the pass log and cell counts in `report`.
///
/// # Safety
///
/// As [`reticle_wasm_elaborate`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_wasm_synth(
    rtl: *const u8,
    rtl_len: usize,
    lut_inputs: u32,
) -> *mut u8 {
    guard(|| {
        // SAFETY: the caller guarantees the pair is readable for its
        // length and stable for the call.
        let Some(text) = (unsafe { arg(rtl, rtl_len) }) else {
            return Reply::failed("the design is not valid UTF-8").json();
        };
        if !pipeline::synth_enabled() {
            return Reply::failed("this build has no `synth` stage").json();
        }
        if lut_inputs != 0 && !(2..=8).contains(&lut_inputs) {
            return Reply::failed("`lut_inputs` must be 0, or 2 to 8").json();
        }
        let (mut design, map) = match load_rtl(text) {
            Ok(pair) => pair,
            Err(reply) => return reply.json(),
        };
        let mut diags = Diagnostics::new();
        let report = pipeline::synth(&mut design, &map, lut_inputs, &mut diags).unwrap_or_default();
        let reply = Reply::new().with_diagnostics(diags, &map);
        if reply.errors > 0 {
            return reply.fail().text("report", report).json();
        }
        reply
            .text("rtl", design.to_text())
            .text("report", report)
            .json()
    })
}

/// Renders a design in one of the netlist formats.
///
/// `format` is `verilog`, `vhdl`, `json`, `blif` or `edif`. The rendered
/// text comes back in `output`.
///
/// # Safety
///
/// As [`reticle_wasm_elaborate`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_wasm_emit(
    rtl: *const u8,
    rtl_len: usize,
    format: *const u8,
    format_len: usize,
) -> *mut u8 {
    guard(|| {
        use crate::ir::emit::{self, Format};

        // SAFETY: the caller guarantees both pairs are readable for their
        // lengths and stable for the call.
        let (Some(text), Some(format)) = (unsafe { (arg(rtl, rtl_len), arg(format, format_len)) })
        else {
            return Reply::failed("an argument is not valid UTF-8").json();
        };
        let Some(format) = Format::from_name(format) else {
            return Reply::failed("`format` must be verilog, vhdl, json, blif or edif").json();
        };
        let (design, map) = match load_rtl(text) {
            Ok(pair) => pair,
            Err(reply) => return reply.json(),
        };
        match emit::emit(&design, format) {
            Ok(output) => Reply::new().text("output", output).json(),
            Err(error) => {
                let mut diags = Diagnostics::new();
                diags.error(error.span, error.message);
                Reply::new().fail().with_diagnostics(diags, &map).json()
            }
        }
    })
}

/// Simulates a design given in the `.rtl` text format.
///
/// `top` names the root module (empty for the design's own top) and
/// `ticks` is how far to run, in ticks of the simulation precision; 0 runs
/// until the event queue drains or the design calls `$finish`, which is
/// what a self-driving testbench wants. With `vcd` non-zero the reply also
/// carries the waveform.
///
/// The reply has the `$display` output in `output`, the final time in
/// `time`, one of `running`, `stopped` or `finished` in `status`, and the
/// simulator's own messages in `diagnostics`.
///
/// # Safety
///
/// As [`reticle_wasm_elaborate`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_wasm_simulate(
    rtl: *const u8,
    rtl_len: usize,
    top: *const u8,
    top_len: usize,
    ticks: u64,
    vcd: u32,
) -> *mut u8 {
    guard(|| {
        // SAFETY: the caller guarantees both pairs are readable for their
        // lengths and stable for the call.
        let (Some(text), Some(top)) = (unsafe { (arg(rtl, rtl_len), arg(top, top_len)) }) else {
            return Reply::failed("an argument is not valid UTF-8").json();
        };
        let (design, map) = match load_rtl(text) {
            Ok(pair) => pair,
            Err(reply) => return reply.json(),
        };
        simulate(
            &design,
            &map,
            (!top.is_empty()).then_some(top),
            ticks,
            vcd != 0,
        )
    })
}

/// The `sim`-enabled body of [`reticle_wasm_simulate`].
#[cfg(feature = "sim")]
fn simulate(
    design: &Design,
    map: &SourceMap,
    top: Option<&str>,
    ticks: u64,
    want_vcd: bool,
) -> String {
    use crate::sim::{SimOptions, Simulator, Status};

    let options = SimOptions {
        top: top.map(str::to_owned),
        ..SimOptions::default()
    };
    let mut sim = match Simulator::new(design, options) {
        Ok(sim) => sim,
        Err(diags) => return Reply::new().fail().with_diagnostics(diags, map).json(),
    };
    if want_vcd {
        sim.enable_vcd();
    }
    if ticks == 0 {
        sim.run();
    } else {
        sim.run_for(ticks);
    }

    let status = match sim.status() {
        Status::Running => "running",
        Status::Stopped => "stopped",
        Status::Finished => "finished",
    };
    let output = sim.take_output();
    let vcd = sim.vcd().unwrap_or_default().to_owned();
    let time = sim.time();
    let messages = sim.take_messages();
    let reply = Reply::new().with_diagnostics(messages, map);
    let failed = reply.errors > 0;
    let reply = reply
        .text("output", output)
        .number("time", time)
        .text("status", status)
        .text("vcd", vcd);
    if failed {
        reply.fail().json()
    } else {
        reply.json()
    }
}

/// The body of [`reticle_wasm_simulate`] without the `sim` feature.
#[cfg(not(feature = "sim"))]
fn simulate(
    _design: &Design,
    _map: &SourceMap,
    _top: Option<&str>,
    _ticks: u64,
    _want_vcd: bool,
) -> String {
    Reply::failed("this build has no `sim` stage").json()
}

// The unit tests come last so they follow every item they exercise.
#[cfg(test)]
mod tests;
