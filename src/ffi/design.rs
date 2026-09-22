//! The `reticle_design` handle: an elaborated design plus the sources it
//! came from.
//!
//! A handle owns both an [`crate::ir::Design`] and the
//! [`crate::source::SourceMap`] its spans point into, because every later
//! stage (synthesis, the simulator, an emitter) can report a diagnostic
//! that needs resolving back to a line. Keeping the two together is what
//! lets the C side pass one pointer around instead of two.
//!
//! # Sans-I/O
//!
//! The library never opens a file, so "a list of files" here is a list of
//! *(name, text)* pairs: the embedder reads the bytes, Reticle gets the
//! text, and the name is what diagnostics print. The same rule applies to
//! Verilog `` `include ``: includes are not resolved, and one is reported
//! as a diagnostic rather than silently reaching the filesystem.

use std::ffi::{c_char, c_int};

use crate::diag::Diagnostics;
use crate::ir::Design;
use crate::ir::emit::{self, Format};
use crate::source::SourceMap;

use super::diagnostics::{self, reticle_diagnostics};
use super::pipeline::{self, Language};
use super::{
    RETICLE_ERR_DIAGNOSTICS, RETICLE_ERR_INVALID, RETICLE_ERR_NOT_FOUND, RETICLE_ERR_UNSUPPORTED,
    RETICLE_OK, as_mut, as_ref, drop_handle, guard, into_handle, opt_str_arg, str_arg, write_out,
    write_string,
};

/// An opaque elaborated design.
///
/// Produced by [`reticle_elaborate_verilog`], [`reticle_elaborate_vhdl`],
/// their single-source shorthands or [`reticle_design_load_rtl`], and
/// released with [`reticle_design_free`].
pub struct reticle_design {
    pub(crate) design: Design,
    pub(crate) map: SourceMap,
}

/// Collects `count` pairs of C strings into borrowed `(name, text)` pairs.
///
/// # Safety
///
/// `names` and `sources` must each point at `count` readable, valid
/// `const char *` values, and every one of those must be a NUL-terminated
/// string that stays valid for the call.
unsafe fn source_list<'a>(
    names: *const *const c_char,
    sources: *const *const c_char,
    count: usize,
) -> Option<Vec<(&'a str, &'a str)>> {
    if names.is_null() || sources.is_null() || count == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        // SAFETY: the caller guarantees both arrays hold `count` readable
        // elements, so `i < count` is in bounds for each; `str_arg` then
        // checks each element for null and for valid UTF-8.
        let name = unsafe { str_arg(*names.add(i)) }?;
        let text = unsafe { str_arg(*sources.add(i)) }?;
        out.push((name, text));
    }
    Some(out)
}

/// Shared body of the two `reticle_elaborate_*` entry points.
///
/// # Safety
///
/// As the entry points that call it: the two arrays must hold `count`
/// valid C strings, `top` must be null or a C string, and the two
/// out-parameters must be null or writable.
unsafe fn elaborate(
    names: *const *const c_char,
    sources: *const *const c_char,
    count: usize,
    top: *const c_char,
    out_design: *mut *mut reticle_design,
    out_diags: *mut *mut reticle_diagnostics,
    language: Language,
) -> c_int {
    if out_design.is_null() {
        return RETICLE_ERR_INVALID;
    }
    // SAFETY: the caller guarantees the arrays are readable for `count`
    // elements and hold NUL-terminated strings; `source_list` and
    // `opt_str_arg` reject nulls and non-UTF-8 themselves.
    let Some(list) = (unsafe { source_list(names, sources, count) }) else {
        return RETICLE_ERR_INVALID;
    };
    let Some(top) = (unsafe { opt_str_arg(top) }) else {
        return RETICLE_ERR_INVALID;
    };

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let design = pipeline::elaborate(language, &list, top, &mut map, &mut diags);
    let unsupported = !language.enabled();
    let failed = diags.has_errors() || design.is_none();
    let snapshot = reticle_diagnostics::snapshot(diags, &map);
    // SAFETY: `out_diags` is null or writable, per the contract.
    unsafe { diagnostics::publish(out_diags, snapshot) };

    match design {
        Some(design) if !failed => {
            // SAFETY: `out_design` was checked non-null above and the
            // caller guarantees it is a writable, aligned slot.
            unsafe { *out_design = into_handle(reticle_design { design, map }) };
            RETICLE_OK
        }
        _ if unsupported => RETICLE_ERR_UNSUPPORTED,
        _ => RETICLE_ERR_DIAGNOSTICS,
    }
}

/// Parses and elaborates a list of Verilog / SystemVerilog sources.
///
/// `names` and `sources` are parallel arrays of `count` C strings: the
/// display name of each file and its text. All of them form one
/// compilation, so a module in one may instantiate a module in another.
/// The SystemVerilog rules are selected when the first name ends in `.sv`
/// or `.svh`. `top` names the root module, or is null to let elaboration
/// pick the module nothing instantiates.
///
/// On success `*out_design` receives a handle the caller frees with
/// [`reticle_design_free`]. `out_diags` may be null; when it is not, it
/// receives a handle with every diagnostic, including warnings on success,
/// which the caller frees with
/// [`reticle_diagnostics_free`](super::reticle_diagnostics_free).
///
/// # Safety
///
/// `names` and `sources` must each point at `count` NUL-terminated
/// strings that stay valid for the call, `top` must be null or such a
/// string, `out_design` must point at a writable handle slot, and
/// `out_diags` must be null or point at one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_elaborate_verilog(
    names: *const *const c_char,
    sources: *const *const c_char,
    count: usize,
    top: *const c_char,
    out_design: *mut *mut reticle_design,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        // SAFETY: the contract of this function is exactly `elaborate`'s.
        unsafe {
            elaborate(
                names,
                sources,
                count,
                top,
                out_design,
                out_diags,
                Language::Verilog,
            )
        }
    })
}

/// Parses and elaborates one Verilog source held in a string.
///
/// The shorthand for [`reticle_elaborate_verilog`] with a single file.
///
/// # Safety
///
/// `name` and `source` must be NUL-terminated strings valid for the call,
/// and the remaining arguments are as [`reticle_elaborate_verilog`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_elaborate_verilog_source(
    name: *const c_char,
    source: *const c_char,
    top: *const c_char,
    out_design: *mut *mut reticle_design,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        // SAFETY: `&name` and `&source` are one-element arrays of the two
        // pointers the caller supplied, which is what `elaborate` reads.
        unsafe {
            elaborate(
                &raw const name,
                &raw const source,
                1,
                top,
                out_design,
                out_diags,
                Language::Verilog,
            )
        }
    })
}

/// Parses, analyses and elaborates a list of VHDL sources.
///
/// Every source is compiled into the `work` library against the bundled
/// `std` and `ieee` libraries, under VHDL-2008. `top` names the root
/// entity, or is null to let elaboration pick one.
///
/// Arguments and ownership are as [`reticle_elaborate_verilog`].
///
/// # Safety
///
/// As [`reticle_elaborate_verilog`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_elaborate_vhdl(
    names: *const *const c_char,
    sources: *const *const c_char,
    count: usize,
    top: *const c_char,
    out_design: *mut *mut reticle_design,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        // SAFETY: the contract of this function is exactly `elaborate`'s.
        unsafe {
            elaborate(
                names,
                sources,
                count,
                top,
                out_design,
                out_diags,
                Language::Vhdl,
            )
        }
    })
}

/// Parses, analyses and elaborates one VHDL source held in a string.
///
/// # Safety
///
/// As [`reticle_elaborate_verilog_source`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_elaborate_vhdl_source(
    name: *const c_char,
    source: *const c_char,
    top: *const c_char,
    out_design: *mut *mut reticle_design,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        // SAFETY: `&name` and `&source` are one-element arrays of the two
        // pointers the caller supplied, which is what `elaborate` reads.
        unsafe {
            elaborate(
                &raw const name,
                &raw const source,
                1,
                top,
                out_design,
                out_diags,
                Language::Vhdl,
            )
        }
    })
}

/// Loads a design written in the `.rtl` IR text format.
///
/// `name` is the display name used in diagnostics and `text` is the file's
/// contents. The `.rtl` reader is always compiled in, whatever frontends
/// the build has.
///
/// # Safety
///
/// `name` and `text` must be NUL-terminated strings valid for the call,
/// `out_design` must point at a writable handle slot, and `out_diags` must
/// be null or point at one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_load_rtl(
    name: *const c_char,
    text: *const c_char,
    out_design: *mut *mut reticle_design,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        if out_design.is_null() {
            return RETICLE_ERR_INVALID;
        }
        // SAFETY: as documented; `str_arg` rejects null and non-UTF-8.
        let (Some(name), Some(text)) = (unsafe { str_arg(name) }, unsafe { str_arg(text) }) else {
            return RETICLE_ERR_INVALID;
        };
        let mut map = SourceMap::new();
        let Ok(id) = map.add(name, text) else {
            return RETICLE_ERR_INVALID;
        };
        match Design::parse_text(text, id) {
            Ok(design) => {
                // SAFETY: `out_diags` is null or a writable handle slot.
                unsafe { diagnostics::publish(out_diags, diagnostics::empty()) };
                // SAFETY: `out_design` was checked non-null above.
                unsafe { *out_design = into_handle(reticle_design { design, map }) };
                RETICLE_OK
            }
            Err(diags) => {
                let snapshot = reticle_diagnostics::snapshot(diags, &map);
                // SAFETY: `out_diags` is null or a writable handle slot.
                unsafe { diagnostics::publish(out_diags, snapshot) };
                RETICLE_ERR_DIAGNOSTICS
            }
        }
    })
}

/// Writes the design back out in the `.rtl` IR text format.
///
/// The round trip is exact: reading the result with
/// [`reticle_design_load_rtl`] yields the same design.
///
/// # Safety
///
/// `design` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_save_rtl(
    design: *const reticle_design,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; both helpers reject null pointers.
        let Some(d) = (unsafe { as_ref(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        unsafe { write_string(out, &d.design.to_text()) }
    })
}

/// Renders the design in one of the netlist formats.
///
/// `format` is a name or extension accepted by the CLI's `--format`:
/// `verilog`, `vhdl`, `json` (Yosys), `blif` or `edif`; an unrecognised
/// one is [`RETICLE_ERR_NOT_FOUND`]. A construct the target cannot express
/// is reported through `out_diags`, which may be null.
///
/// # Safety
///
/// `design` must be a live handle, `format` a NUL-terminated string, `out`
/// a writable `char *` slot, and `out_diags` null or a writable handle
/// slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_emit(
    design: *const reticle_design,
    format: *const c_char,
    out: *mut *mut c_char,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null and non-UTF-8.
        let Some(d) = (unsafe { as_ref(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        let Some(name) = (unsafe { str_arg(format) }) else {
            return RETICLE_ERR_INVALID;
        };
        let Some(format) = Format::from_name(name) else {
            return RETICLE_ERR_NOT_FOUND;
        };
        match emit::emit(&d.design, format) {
            Ok(text) => {
                // SAFETY: `out_diags` is null or a writable handle slot.
                unsafe { diagnostics::publish(out_diags, diagnostics::empty()) };
                // SAFETY: `out` is null or a writable `char *` slot.
                unsafe { write_string(out, &text) }
            }
            Err(error) => {
                let mut diags = Diagnostics::new();
                diags.error(error.span, error.message);
                let snapshot = reticle_diagnostics::snapshot(diags, &d.map);
                // SAFETY: `out_diags` is null or a writable handle slot.
                unsafe { diagnostics::publish(out_diags, snapshot) };
                RETICLE_ERR_DIAGNOSTICS
            }
        }
    })
}

/// Writes the name of the top module, or the empty string when the design
/// has no top.
///
/// # Safety
///
/// `design` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_top(
    design: *const reticle_design,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; both helpers reject null pointers.
        let Some(d) = (unsafe { as_ref(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        let name = d
            .design
            .top
            .map(|id| d.design.module(id).name.as_str().to_owned())
            .unwrap_or_default();
        unsafe { write_string(out, &name) }
    })
}

/// Makes the module called `name` the top of the hierarchy.
///
/// Returns [`RETICLE_ERR_NOT_FOUND`] when the design has no such module,
/// leaving the current top alone.
///
/// # Safety
///
/// `design` must be a live handle and `name` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_set_top(
    design: *mut reticle_design,
    name: *const c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; the helpers reject null and non-UTF-8.
        let Some(d) = (unsafe { as_mut(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        let Some(name) = (unsafe { str_arg(name) }) else {
            return RETICLE_ERR_INVALID;
        };
        match d.design.module_by_name(name) {
            Some(id) => {
                d.design.top = Some(id);
                RETICLE_OK
            }
            None => RETICLE_ERR_NOT_FOUND,
        }
    })
}

/// Writes how many modules the design holds.
///
/// # Safety
///
/// `design` must be a live handle and `out` must point at a writable
/// `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_module_count(
    design: *const reticle_design,
    out: *mut usize,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; both helpers reject null pointers.
        let Some(d) = (unsafe { as_ref(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        unsafe { write_out(out, d.design.modules.len()) }
    })
}

/// Writes the name of the `index`th module, in the order they were
/// elaborated.
///
/// # Safety
///
/// `design` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_module_name(
    design: *const reticle_design,
    index: usize,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; both helpers reject null pointers.
        let Some(d) = (unsafe { as_ref(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        match d.design.modules.iter().nth(index) {
            Some((_, module)) => unsafe { write_string(out, module.name.as_str()) },
            None => RETICLE_ERR_INVALID,
        }
    })
}

/// Synthesises the design in place.
///
/// Generic synthesis (process lowering, flip-flop / memory / FSM
/// inference, optimisation, cellification) always runs. `lut_inputs` then
/// selects technology mapping: 0 leaves the netlist generic, and 2 to 8
/// map what is left onto `k`-input LUTs. Any other value is
/// [`RETICLE_ERR_INVALID`] and the design is untouched.
///
/// `out_report` may be null; when it is not, it receives the pass log and
/// the cell report as one string the caller owns. `out_diags` may be null.
///
/// # Safety
///
/// `design` must be a live handle, and `out_report` and `out_diags` must
/// each be null or point at a writable slot of the matching type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_synth(
    design: *mut reticle_design,
    lut_inputs: u32,
    out_report: *mut *mut c_char,
    out_diags: *mut *mut reticle_diagnostics,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `as_mut` rejects a null pointer.
        let Some(d) = (unsafe { as_mut(design) }) else {
            return RETICLE_ERR_INVALID;
        };
        if lut_inputs != 0 && !(2..=8).contains(&lut_inputs) {
            return RETICLE_ERR_INVALID;
        }
        let mut diags = Diagnostics::new();
        let Some(report) = pipeline::synth(&mut d.design, &d.map, lut_inputs, &mut diags) else {
            // SAFETY: `out_diags` is null or a writable handle slot.
            unsafe {
                diagnostics::publish(
                    out_diags,
                    diagnostics::single_error("this library was built without the `synth` feature"),
                )
            };
            return RETICLE_ERR_UNSUPPORTED;
        };
        let failed = diags.has_errors();
        let snapshot = reticle_diagnostics::snapshot(diags, &d.map);
        // SAFETY: `out_diags` is null or a writable handle slot.
        unsafe { diagnostics::publish(out_diags, snapshot) };
        if !out_report.is_null() {
            // SAFETY: `out_report` is non-null and, per the contract, a
            // writable `char *` slot.
            unsafe { write_string(out_report, &report) };
        }
        if failed {
            RETICLE_ERR_DIAGNOSTICS
        } else {
            RETICLE_OK
        }
    })
}

/// Releases a design handle. A null pointer is ignored.
///
/// Handles are independent: a simulator created from a design keeps its
/// own copy, so the two may be freed in either order.
///
/// # Safety
///
/// `design` must be null, or a handle from this library that has not
/// already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_design_free(design: *mut reticle_design) {
    // SAFETY: the caller guarantees `design` is null or a live handle that
    // this module allocated with `into_handle`.
    unsafe { drop_handle(design) };
}
