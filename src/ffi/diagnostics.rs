//! The `reticle_diagnostics` handle: a frozen snapshot of one stage's
//! diagnostics.
//!
//! Diagnostics carry [`crate::source::Span`]s, which only mean something
//! next to the [`crate::source::SourceMap`] they were produced against.
//! Rather than make C keep the two in step, the handle is built by
//! resolving everything up front: the rustc-style rendering, and per
//! diagnostic its severity, code, message, file, line, column and notes.
//! The snapshot therefore outlives the design that produced it and holds
//! no borrow, which is what lets a caller free handles in any order.

use std::ffi::{c_char, c_int};

use crate::diag::{Diagnostics, Severity};
use crate::source::SourceMap;

use super::{
    RETICLE_ERR_INVALID, RETICLE_SEVERITY_ERROR, RETICLE_SEVERITY_HELP, RETICLE_SEVERITY_NOTE,
    RETICLE_SEVERITY_WARNING, as_ref, drop_handle, guard, into_handle, write_out, write_string,
};

/// One resolved diagnostic.
struct Item {
    severity: c_int,
    code: String,
    message: String,
    file: String,
    line: u32,
    column: u32,
    notes: Vec<String>,
}

/// An opaque, immutable list of diagnostics.
///
/// Created by any call that runs a stage, and released with
/// [`reticle_diagnostics_free`].
pub struct reticle_diagnostics {
    rendered: String,
    items: Vec<Item>,
    errors: usize,
    warnings: usize,
}

impl reticle_diagnostics {
    /// Resolves `diags` against `map` into a standalone snapshot.
    ///
    /// The list is sorted by source position first, so two runs over the
    /// same input produce the same order whatever order the stages
    /// reported in.
    pub(crate) fn snapshot(mut diags: Diagnostics, map: &SourceMap) -> reticle_diagnostics {
        diags.sort();
        let rendered = diags.render(map);
        let errors = diags.error_count();
        let warnings = diags.warning_count();
        let items = diags
            .iter()
            .map(|d| {
                let (file, line, column) = match d.primary_span() {
                    Some(span) => {
                        let (name, loc) = map.locate(span);
                        (name.to_owned(), loc.line, loc.col)
                    }
                    None => (String::new(), 0, 0),
                };
                Item {
                    severity: severity_code(d.severity),
                    code: d.code.unwrap_or_default().to_owned(),
                    message: d.message.clone(),
                    file,
                    line,
                    column,
                    notes: d.notes.clone(),
                }
            })
            .collect();
        reticle_diagnostics {
            rendered,
            items,
            errors,
            warnings,
        }
    }
}

/// Maps a [`Severity`] to its `RETICLE_SEVERITY_*` code.
fn severity_code(severity: Severity) -> c_int {
    match severity {
        Severity::Help => RETICLE_SEVERITY_HELP,
        Severity::Note => RETICLE_SEVERITY_NOTE,
        Severity::Warning => RETICLE_SEVERITY_WARNING,
        Severity::Error => RETICLE_SEVERITY_ERROR,
    }
}

/// Publishes a snapshot through an optional `reticle_diagnostics **`
/// out-parameter, dropping it when the caller did not ask for it.
///
/// # Safety
///
/// `out` must be null or point at a writable `reticle_diagnostics *`.
pub(crate) unsafe fn publish(out: *mut *mut reticle_diagnostics, snapshot: reticle_diagnostics) {
    if out.is_null() {
        return;
    }
    // SAFETY: `out` is non-null and the caller guarantees it points at a
    // writable, aligned slot for one handle pointer.
    unsafe { *out = into_handle(snapshot) };
}

/// Borrows the `index`th item of a handle, or reports why it cannot.
///
/// # Safety
///
/// `diags` must be null or a live handle from this library.
unsafe fn item<'a>(diags: *const reticle_diagnostics, index: usize) -> Result<&'a Item, c_int> {
    // SAFETY: delegated to `as_ref`; the caller guarantees the pointer is
    // null or a live, unaliased handle.
    let d = unsafe { as_ref(diags) }.ok_or(RETICLE_ERR_INVALID)?;
    d.items.get(index).ok_or(RETICLE_ERR_INVALID)
}

/// Releases a diagnostics handle. A null pointer is ignored.
///
/// # Safety
///
/// `diags` must be null, or a handle from this library that has not
/// already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostics_free(diags: *mut reticle_diagnostics) {
    // SAFETY: the caller guarantees `diags` is null or a live handle that
    // this module allocated with `into_handle`.
    unsafe { drop_handle(diags) };
}

/// Writes the whole list rendered as rustc-style text, with source
/// excerpts.
///
/// The result is empty when there are no diagnostics, and each entry ends
/// with a blank line, so it can be printed straight to `stderr`.
///
/// # Safety
///
/// `diags` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostics_render(
    diags: *const reticle_diagnostics,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: the caller guarantees both pointers meet the contract
        // above; `as_ref` and `write_string` reject null themselves.
        let Some(d) = (unsafe { as_ref(diags) }) else {
            return RETICLE_ERR_INVALID;
        };
        unsafe { write_string(out, &d.rendered) }
    })
}

/// Writes how many diagnostics the handle holds.
///
/// # Safety
///
/// `diags` must be a live handle and `out` must point at a writable
/// `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostics_count(
    diags: *const reticle_diagnostics,
    out: *mut usize,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; both helpers reject null pointers.
        let Some(d) = (unsafe { as_ref(diags) }) else {
            return RETICLE_ERR_INVALID;
        };
        unsafe { write_out(out, d.items.len()) }
    })
}

/// Writes how many of the diagnostics are errors.
///
/// # Safety
///
/// As [`reticle_diagnostics_count`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostics_error_count(
    diags: *const reticle_diagnostics,
    out: *mut usize,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; both helpers reject null pointers.
        let Some(d) = (unsafe { as_ref(diags) }) else {
            return RETICLE_ERR_INVALID;
        };
        unsafe { write_out(out, d.errors) }
    })
}

/// Writes how many of the diagnostics are warnings.
///
/// # Safety
///
/// As [`reticle_diagnostics_count`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostics_warning_count(
    diags: *const reticle_diagnostics,
    out: *mut usize,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; both helpers reject null pointers.
        let Some(d) = (unsafe { as_ref(diags) }) else {
            return RETICLE_ERR_INVALID;
        };
        unsafe { write_out(out, d.warnings) }
    })
}

/// Writes the `RETICLE_SEVERITY_*` code of one diagnostic.
///
/// # Safety
///
/// `diags` must be a live handle and `out` must point at a writable `int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_severity(
    diags: *const reticle_diagnostics,
    index: usize,
    out: *mut c_int,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` rejects a null handle and an index
        // that is out of range.
        match unsafe { item(diags, index) } {
            Ok(it) => unsafe { write_out(out, it.severity) },
            Err(status) => status,
        }
    })
}

/// Writes the stable short code of one diagnostic, such as `V0007`.
///
/// The string is empty when the diagnostic has no code.
///
/// # Safety
///
/// `diags` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_code(
    diags: *const reticle_diagnostics,
    index: usize,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` and `write_string` reject bad input.
        match unsafe { item(diags, index) } {
            Ok(it) => unsafe { write_string(out, &it.code) },
            Err(status) => status,
        }
    })
}

/// Writes the one-line headline of one diagnostic.
///
/// # Safety
///
/// As [`reticle_diagnostic_code`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_message(
    diags: *const reticle_diagnostics,
    index: usize,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` and `write_string` reject bad input.
        match unsafe { item(diags, index) } {
            Ok(it) => unsafe { write_string(out, &it.message) },
            Err(status) => status,
        }
    })
}

/// Writes the name of the file a diagnostic points into.
///
/// The string is empty when the diagnostic has no span.
///
/// # Safety
///
/// As [`reticle_diagnostic_code`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_file(
    diags: *const reticle_diagnostics,
    index: usize,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` and `write_string` reject bad input.
        match unsafe { item(diags, index) } {
            Ok(it) => unsafe { write_string(out, &it.file) },
            Err(status) => status,
        }
    })
}

/// Writes the 1-based line of a diagnostic, or 0 when it has no span.
///
/// # Safety
///
/// `diags` must be a live handle and `out` must point at a writable
/// `uint32_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_line(
    diags: *const reticle_diagnostics,
    index: usize,
    out: *mut u32,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` rejects bad input.
        match unsafe { item(diags, index) } {
            Ok(it) => unsafe { write_out(out, it.line) },
            Err(status) => status,
        }
    })
}

/// Writes the 1-based column of a diagnostic, counted in characters, or 0
/// when it has no span.
///
/// # Safety
///
/// As [`reticle_diagnostic_line`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_column(
    diags: *const reticle_diagnostics,
    index: usize,
    out: *mut u32,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` rejects bad input.
        match unsafe { item(diags, index) } {
            Ok(it) => unsafe { write_out(out, it.column) },
            Err(status) => status,
        }
    })
}

/// Writes how many trailing note lines a diagnostic carries.
///
/// # Safety
///
/// `diags` must be a live handle and `out` must point at a writable
/// `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_note_count(
    diags: *const reticle_diagnostics,
    index: usize,
    out: *mut usize,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` rejects bad input.
        match unsafe { item(diags, index) } {
            Ok(it) => unsafe { write_out(out, it.notes.len()) },
            Err(status) => status,
        }
    })
}

/// Writes one note line of one diagnostic.
///
/// # Safety
///
/// `diags` must be a live handle and `out` must point at a writable
/// `char *`, which on success receives a string the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn reticle_diagnostic_note(
    diags: *const reticle_diagnostics,
    index: usize,
    note: usize,
    out: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as documented; `item` and `write_string` reject bad input.
        let it = match unsafe { item(diags, index) } {
            Ok(it) => it,
            Err(status) => return status,
        };
        match it.notes.get(note) {
            Some(text) => unsafe { write_string(out, text) },
            None => RETICLE_ERR_INVALID,
        }
    })
}

/// Builds an empty snapshot, for a stage that reported nothing.
pub(crate) fn empty() -> reticle_diagnostics {
    reticle_diagnostics {
        rendered: String::new(),
        items: Vec::new(),
        errors: 0,
        warnings: 0,
    }
}

/// Builds a snapshot holding a single error with no span.
///
/// Used where a stage fails with a plain message rather than a span-backed
/// diagnostic (an emitter refusing a design, say), so a caller always has
/// one place to look for the reason.
pub(crate) fn single_error(message: impl Into<String>) -> reticle_diagnostics {
    let message = message.into();
    reticle_diagnostics {
        rendered: format!("error: {message}\n\n"),
        items: vec![Item {
            severity: RETICLE_SEVERITY_ERROR,
            code: String::new(),
            message,
            file: String::new(),
            line: 0,
            column: 0,
            notes: Vec::new(),
        }],
        errors: 1,
        warnings: 0,
    }
}
