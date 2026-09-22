//! Unit tests for the C ABI, driven from the Rust side of the boundary.
//!
//! Every entry point is called the way C would call it: a raw pointer in,
//! a status out, and a matching `_free` afterwards. Each test wraps itself
//! in [`NoLeaks`], which snapshots the module's allocation counter
//! ([`super::live`]) and asserts on drop that the test handed back
//! everything it took, so a forgotten `reticle_string_free` fails the test
//! rather than quietly growing the heap.

use std::ffi::{CStr, CString, c_char, c_int};

use super::*;

/// Asserts on drop that the allocation counter is back where it started.
struct NoLeaks(isize);

impl NoLeaks {
    /// Snapshots the counter for this thread.
    fn start() -> NoLeaks {
        NoLeaks(live::count())
    }
}

impl Drop for NoLeaks {
    fn drop(&mut self) {
        // A panicking test is already failing; a second panic from here
        // would hide its message.
        if !std::thread::panicking() {
            assert_eq!(
                live::count(),
                self.0,
                "the test leaked {} allocation(s) across the C boundary",
                live::count() - self.0
            );
        }
    }
}

/// Builds a C string argument.
fn c(text: &str) -> CString {
    CString::new(text).expect("test strings hold no NUL")
}

/// Reads and frees a string the library handed out.
fn take(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null(), "the out-parameter was not written");
    // SAFETY: `ptr` came from a `char **` out-parameter, so it is a live
    // NUL-terminated UTF-8 string this module allocated.
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("the library only produces UTF-8")
        .to_owned();
    // SAFETY: the same pointer, freed exactly once.
    unsafe { reticle_string_free(ptr) };
    text
}

/// A counter in the `.rtl` text format, used by the tests that need a
/// design without depending on a frontend being compiled in.
const COUNTER_RTL: &str = "\
module counter
  net %clk u1 wire
  net %q u8 reg
  port clk in %clk
  port q out %q
  process initial
    %q = 8'd0
  end
  process seq posedge %clk
    %q <= add(%q, 8'd1)
  end
end
";

/// Loads [`COUNTER_RTL`] into a handle, asserting it parsed.
fn counter() -> *mut reticle_design {
    let (name, text) = (c("counter.rtl"), c(COUNTER_RTL));
    let mut design = std::ptr::null_mut();
    // SAFETY: both strings are live, and `design` is a writable slot.
    let status = unsafe {
        reticle_design_load_rtl(
            name.as_ptr(),
            text.as_ptr(),
            &raw mut design,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, RETICLE_OK);
    design
}

#[test]
fn version_is_the_crate_version() {
    let _guard = NoLeaks::start();
    // SAFETY: `reticle_version` returns a static NUL-terminated string.
    let text = unsafe { CStr::from_ptr(reticle_version()) };
    assert_eq!(text.to_str().unwrap(), crate::VERSION);
}

#[test]
fn every_status_has_a_message() {
    let _guard = NoLeaks::start();
    for status in [
        RETICLE_OK,
        RETICLE_ERR_INVALID,
        RETICLE_ERR_DIAGNOSTICS,
        RETICLE_ERR_NOT_FOUND,
        RETICLE_ERR_UNSUPPORTED,
        RETICLE_ERR_INTERNAL,
        RETICLE_ERR_UNKNOWN_VALUE,
        RETICLE_ERR_PANIC,
        999,
    ] {
        // SAFETY: the returned pointer is static and NUL-terminated.
        let message = unsafe { CStr::from_ptr(reticle_status_message(status)) };
        assert!(!message.to_str().unwrap().is_empty());
    }
}

#[test]
fn features_names_the_compiled_stages() {
    let _guard = NoLeaks::start();
    let mut out = std::ptr::null_mut();
    // SAFETY: `out` is a writable `char *` slot.
    assert_eq!(unsafe { reticle_features(&raw mut out) }, RETICLE_OK);
    let names = take(out);
    assert_eq!(names.contains("verilog"), cfg!(feature = "verilog"));
    assert_eq!(names.contains("vhdl"), cfg!(feature = "vhdl"));
    assert_eq!(names.contains("sim"), cfg!(feature = "sim"));
    assert_eq!(names.contains("synth"), cfg!(feature = "synth"));

    // A null out-parameter is rejected rather than written through.
    // SAFETY: passing null is exactly what the call must survive.
    assert_eq!(
        unsafe { reticle_features(std::ptr::null_mut()) },
        RETICLE_ERR_INVALID
    );
}

#[test]
fn a_panic_becomes_a_status() {
    let _guard = NoLeaks::start();
    // The default hook would print the backtrace of a panic the test
    // expects; silence it for the duration.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let status = reticle_self_test_panic();
    std::panic::set_hook(hook);
    assert_eq!(status, RETICLE_ERR_PANIC);
}

#[test]
fn freeing_null_is_a_no_op() {
    let _guard = NoLeaks::start();
    // SAFETY: every `_free` documents that null is ignored; that is the
    // property under test.
    unsafe {
        reticle_string_free(std::ptr::null_mut());
        reticle_design_free(std::ptr::null_mut());
        reticle_sim_free(std::ptr::null_mut());
        reticle_diagnostics_free(std::ptr::null_mut());
    }
}

#[test]
fn rtl_round_trips_through_a_handle() {
    let _guard = NoLeaks::start();
    let design = counter();

    let mut out = std::ptr::null_mut();
    // SAFETY: `design` is live and `out` is a writable slot.
    assert_eq!(
        unsafe { reticle_design_save_rtl(design, &raw mut out) },
        RETICLE_OK
    );
    assert_eq!(take(out), COUNTER_RTL);

    // A `.rtl` file names no top, so the handle reports the empty string
    // until one is chosen.
    // SAFETY: as above.
    assert_eq!(
        unsafe { reticle_design_top(design, &raw mut out) },
        RETICLE_OK
    );
    assert_eq!(take(out), "");

    let mut count = 0usize;
    // SAFETY: as above.
    assert_eq!(
        unsafe { reticle_design_module_count(design, &raw mut count) },
        RETICLE_OK
    );
    assert_eq!(count, 1);

    // SAFETY: as above.
    assert_eq!(
        unsafe { reticle_design_module_name(design, 0, &raw mut out) },
        RETICLE_OK
    );
    assert_eq!(take(out), "counter");
    // SAFETY: as above; index 1 is past the end.
    assert_eq!(
        unsafe { reticle_design_module_name(design, 1, &raw mut out) },
        RETICLE_ERR_INVALID
    );

    let name = c("counter");
    // SAFETY: `design` is live and `name` is a NUL-terminated string.
    assert_eq!(
        unsafe { reticle_design_set_top(design, name.as_ptr()) },
        RETICLE_OK
    );
    // SAFETY: as above.
    assert_eq!(
        unsafe { reticle_design_top(design, &raw mut out) },
        RETICLE_OK
    );
    assert_eq!(take(out), "counter");

    let missing = c("nope");
    // SAFETY: as above.
    assert_eq!(
        unsafe { reticle_design_set_top(design, missing.as_ptr()) },
        RETICLE_ERR_NOT_FOUND
    );

    // SAFETY: `design` is live and has not been freed.
    unsafe { reticle_design_free(design) };
}

#[test]
fn loading_broken_rtl_reports_diagnostics() {
    let _guard = NoLeaks::start();
    let (name, text) = (c("broken.rtl"), c("module m\n  this is not rtl\nend\n"));
    let mut design = std::ptr::null_mut();
    let mut diags = std::ptr::null_mut();
    // SAFETY: both strings are live and both out-parameters are writable.
    let status = unsafe {
        reticle_design_load_rtl(
            name.as_ptr(),
            text.as_ptr(),
            &raw mut design,
            &raw mut diags,
        )
    };
    assert_eq!(status, RETICLE_ERR_DIAGNOSTICS);
    assert!(design.is_null(), "no design is produced on failure");

    let mut count = 0usize;
    // SAFETY: `diags` is live and `count` is writable.
    assert_eq!(
        unsafe { reticle_diagnostics_count(diags, &raw mut count) },
        RETICLE_OK
    );
    assert!(count > 0);
    // SAFETY: `diags` is live.
    unsafe { reticle_diagnostics_free(diags) };
}

#[test]
fn emit_renders_every_format() {
    let _guard = NoLeaks::start();
    let design = counter();

    // The netlist formats describe cells and cannot express a behavioural
    // process, and BLIF wants single-output logic, so they only apply once
    // the design has been synthesised and mapped to LUTs.
    #[cfg(feature = "synth")]
    let formats: &[&str] = {
        // SAFETY: `design` is live; both null slots mean "do not report".
        let status =
            unsafe { reticle_design_synth(design, 4, std::ptr::null_mut(), std::ptr::null_mut()) };
        assert_eq!(status, RETICLE_OK);
        &["verilog", "vhdl", "json", "blif", "edif"]
    };
    #[cfg(not(feature = "synth"))]
    let formats: &[&str] = &["verilog", "vhdl"];

    for &name in formats {
        let format = c(name);
        let mut out = std::ptr::null_mut();
        // SAFETY: `design` and `format` are live, `out` is writable and a
        // null diagnostics slot means "do not report".
        let status = unsafe {
            reticle_design_emit(design, format.as_ptr(), &raw mut out, std::ptr::null_mut())
        };
        assert_eq!(status, RETICLE_OK, "emitting {name}");
        assert!(!take(out).is_empty(), "{name} produced nothing");
    }

    let bogus = c("postscript");
    let mut out = std::ptr::null_mut();
    // SAFETY: as above.
    let status =
        unsafe { reticle_design_emit(design, bogus.as_ptr(), &raw mut out, std::ptr::null_mut()) };
    assert_eq!(status, RETICLE_ERR_NOT_FOUND);

    // SAFETY: `design` is live and has not been freed.
    unsafe { reticle_design_free(design) };
}

#[test]
fn null_handles_are_rejected() {
    let _guard = NoLeaks::start();
    let mut out = std::ptr::null_mut();
    let mut n = 0usize;
    let no_design: *const reticle_design = std::ptr::null();
    let no_diags: *const reticle_diagnostics = std::ptr::null();
    let no_sim: *const reticle_sim = std::ptr::null();
    // SAFETY: every one of these takes a null handle, which each entry
    // point must reject rather than dereference.
    unsafe {
        assert_eq!(
            reticle_design_save_rtl(no_design, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_design_top(no_design, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_design_module_count(no_design, &raw mut n),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_design_module_name(no_design, 0, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_design_set_top(std::ptr::null_mut(), std::ptr::null()),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_design_synth(std::ptr::null_mut(), 0, &raw mut out, std::ptr::null_mut()),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_design_emit(
                no_design,
                std::ptr::null(),
                &raw mut out,
                std::ptr::null_mut()
            ),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostics_count(no_diags, &raw mut n),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostics_render(no_diags, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_sim_net_count(no_sim, &raw mut n),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_sim_create(
                no_design,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_design_load_rtl(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_elaborate_verilog(
                std::ptr::null(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            RETICLE_ERR_INVALID
        );
    }
}

#[test]
fn every_diagnostic_accessor_rejects_a_bad_index() {
    let _guard = NoLeaks::start();
    let (name, text) = (c("broken.rtl"), c("not a design at all\n"));
    let mut design = std::ptr::null_mut();
    let mut diags = std::ptr::null_mut();
    // SAFETY: the strings are live and both out-parameters are writable.
    unsafe {
        reticle_design_load_rtl(
            name.as_ptr(),
            text.as_ptr(),
            &raw mut design,
            &raw mut diags,
        )
    };

    let mut severity: c_int = 0;
    let mut line = 0u32;
    let mut column = 0u32;
    let mut notes = 0usize;
    let mut out = std::ptr::null_mut();
    // SAFETY: `diags` is live; index 999 is past the end of every list.
    unsafe {
        assert_eq!(
            reticle_diagnostic_severity(diags, 999, &raw mut severity),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_code(diags, 999, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_message(diags, 999, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_file(diags, 999, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_line(diags, 999, &raw mut line),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_column(diags, 999, &raw mut column),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_note_count(diags, 999, &raw mut notes),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_note(diags, 999, 0, &raw mut out),
            RETICLE_ERR_INVALID
        );
        assert_eq!(
            reticle_diagnostic_note(diags, 0, 999, &raw mut out),
            RETICLE_ERR_INVALID
        );
    }

    // And the item at index 0 answers every one of them.
    // SAFETY: `diags` is live and index 0 exists.
    unsafe {
        assert_eq!(
            reticle_diagnostic_severity(diags, 0, &raw mut severity),
            RETICLE_OK
        );
        assert_eq!(severity, RETICLE_SEVERITY_ERROR);
        assert_eq!(reticle_diagnostic_line(diags, 0, &raw mut line), RETICLE_OK);
        assert_eq!(
            reticle_diagnostic_column(diags, 0, &raw mut column),
            RETICLE_OK
        );
        assert_eq!(
            reticle_diagnostic_note_count(diags, 0, &raw mut notes),
            RETICLE_OK
        );
        assert_eq!(
            reticle_diagnostic_message(diags, 0, &raw mut out),
            RETICLE_OK
        );
        assert!(!take(out).is_empty());
        assert_eq!(reticle_diagnostic_code(diags, 0, &raw mut out), RETICLE_OK);
        take(out);
        assert_eq!(reticle_diagnostic_file(diags, 0, &raw mut out), RETICLE_OK);
        assert_eq!(take(out), "broken.rtl");
        assert_eq!(reticle_diagnostics_render(diags, &raw mut out), RETICLE_OK);
        assert!(take(out).contains("broken.rtl"));
    }

    let mut errors = 0usize;
    let mut warnings = 0usize;
    // SAFETY: `diags` is live and both counters are writable.
    unsafe {
        assert_eq!(
            reticle_diagnostics_error_count(diags, &raw mut errors),
            RETICLE_OK
        );
        assert_eq!(
            reticle_diagnostics_warning_count(diags, &raw mut warnings),
            RETICLE_OK
        );
    }
    assert!(errors > 0);

    // SAFETY: `diags` is live and has not been freed.
    unsafe { reticle_diagnostics_free(diags) };
}

#[cfg(feature = "verilog")]
mod verilog {
    use super::*;

    /// A two-module design, to prove a list of sources elaborates as one
    /// compilation.
    const ADDER: &str = "module adder(input [7:0] a, b, output [7:0] y);\n\
                         assign y = a + b;\nendmodule\n";
    const TOP: &str = "module top(input [7:0] a, b, output [7:0] y);\n\
                       adder u(.a(a), .b(b), .y(y));\nendmodule\n";

    #[test]
    fn one_source_elaborates() {
        let _guard = NoLeaks::start();
        let (name, text) = (c("counter.v"), c(ADDER));
        let mut design = std::ptr::null_mut();
        let mut diags = std::ptr::null_mut();
        // SAFETY: the strings are live and both slots are writable.
        let status = unsafe {
            reticle_elaborate_verilog_source(
                name.as_ptr(),
                text.as_ptr(),
                std::ptr::null(),
                &raw mut design,
                &raw mut diags,
            )
        };
        assert_eq!(status, RETICLE_OK);

        let mut out = std::ptr::null_mut();
        // SAFETY: `design` is live and `out` is writable.
        assert_eq!(
            unsafe { reticle_design_top(design, &raw mut out) },
            RETICLE_OK
        );
        assert_eq!(take(out), "adder");

        // SAFETY: both handles are live and have not been freed.
        unsafe {
            reticle_diagnostics_free(diags);
            reticle_design_free(design);
        }
    }

    #[test]
    fn a_list_of_sources_elaborates_together() {
        let _guard = NoLeaks::start();
        let names = [c("top.v"), c("adder.v")];
        let texts = [c(TOP), c(ADDER)];
        let name_ptrs = [names[0].as_ptr(), names[1].as_ptr()];
        let text_ptrs = [texts[0].as_ptr(), texts[1].as_ptr()];
        let top = c("top");
        let mut design = std::ptr::null_mut();
        // SAFETY: both arrays hold two live C strings and `design` is a
        // writable slot.
        let status = unsafe {
            reticle_elaborate_verilog(
                name_ptrs.as_ptr(),
                text_ptrs.as_ptr(),
                2,
                top.as_ptr(),
                &raw mut design,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, RETICLE_OK);

        let mut count = 0usize;
        // SAFETY: `design` is live and `count` is writable.
        assert_eq!(
            unsafe { reticle_design_module_count(design, &raw mut count) },
            RETICLE_OK
        );
        assert_eq!(count, 2, "both modules elaborated");

        // SAFETY: `design` is live and has not been freed.
        unsafe { reticle_design_free(design) };
    }

    #[test]
    fn a_syntax_error_is_reported_with_a_location() {
        let _guard = NoLeaks::start();
        let (name, text) = (c("bad.v"), c("module m;\n  wire w = ;\nendmodule\n"));
        let mut design = std::ptr::null_mut();
        let mut diags = std::ptr::null_mut();
        // SAFETY: the strings are live and both slots are writable.
        let status = unsafe {
            reticle_elaborate_verilog_source(
                name.as_ptr(),
                text.as_ptr(),
                std::ptr::null(),
                &raw mut design,
                &raw mut diags,
            )
        };
        assert_eq!(status, RETICLE_ERR_DIAGNOSTICS);
        assert!(design.is_null());

        let mut out = std::ptr::null_mut();
        let mut line = 0u32;
        // SAFETY: `diags` is live and both slots are writable.
        unsafe {
            assert_eq!(reticle_diagnostic_file(diags, 0, &raw mut out), RETICLE_OK);
            assert_eq!(take(out), "bad.v");
            assert_eq!(reticle_diagnostic_line(diags, 0, &raw mut line), RETICLE_OK);
        }
        assert_eq!(line, 2);
        // SAFETY: `diags` is live and has not been freed.
        unsafe { reticle_diagnostics_free(diags) };
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let _guard = NoLeaks::start();
        // A lone 0xff byte is not UTF-8, so the string cannot be read.
        let text = CString::new(vec![0xffu8, 0xfe]).unwrap();
        let name = c("bad.v");
        let mut design = std::ptr::null_mut();
        // SAFETY: both pointers are live C strings; only the contents are
        // invalid, which is what the call must detect.
        let status = unsafe {
            reticle_elaborate_verilog_source(
                name.as_ptr(),
                text.as_ptr(),
                std::ptr::null(),
                &raw mut design,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, RETICLE_ERR_INVALID);
    }
}

#[cfg(not(feature = "verilog"))]
#[test]
fn verilog_reports_unsupported_when_absent() {
    let _guard = NoLeaks::start();
    let (name, text) = (c("m.v"), c("module m; endmodule"));
    let mut design = std::ptr::null_mut();
    // SAFETY: both strings are live and `design` is a writable slot.
    let status = unsafe {
        reticle_elaborate_verilog_source(
            name.as_ptr(),
            text.as_ptr(),
            std::ptr::null(),
            &raw mut design,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, RETICLE_ERR_UNSUPPORTED);
}

#[cfg(feature = "vhdl")]
#[test]
fn vhdl_elaborates_from_a_string() {
    let _guard = NoLeaks::start();
    let source = "entity t is port (clk : in bit; q : out bit); end entity;\n\
                  architecture a of t is begin q <= clk; end architecture;\n";
    let (name, text) = (c("t.vhd"), c(source));
    let mut design = std::ptr::null_mut();
    let mut diags = std::ptr::null_mut();
    // SAFETY: both strings are live and both slots are writable.
    let status = unsafe {
        reticle_elaborate_vhdl_source(
            name.as_ptr(),
            text.as_ptr(),
            std::ptr::null(),
            &raw mut design,
            &raw mut diags,
        )
    };
    assert_eq!(status, RETICLE_OK);

    let mut out = std::ptr::null_mut();
    // SAFETY: `design` is live and `out` is writable.
    assert_eq!(
        unsafe { reticle_design_top(design, &raw mut out) },
        RETICLE_OK
    );
    assert_eq!(take(out), "t");

    // The list form takes the same design.
    let names = [name.as_ptr()];
    let texts = [text.as_ptr()];
    let mut second = std::ptr::null_mut();
    // SAFETY: both one-element arrays hold live C strings.
    let status = unsafe {
        reticle_elaborate_vhdl(
            names.as_ptr(),
            texts.as_ptr(),
            1,
            std::ptr::null(),
            &raw mut second,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, RETICLE_OK);

    // SAFETY: all three handles are live and have not been freed.
    unsafe {
        reticle_design_free(second);
        reticle_diagnostics_free(diags);
        reticle_design_free(design);
    }
}

#[cfg(not(feature = "vhdl"))]
#[test]
fn vhdl_reports_unsupported_when_absent() {
    let _guard = NoLeaks::start();
    let (name, text) = (c("t.vhd"), c("entity t is end entity;"));
    let mut design = std::ptr::null_mut();
    // SAFETY: both strings are live and `design` is a writable slot.
    let status = unsafe {
        reticle_elaborate_vhdl_source(
            name.as_ptr(),
            text.as_ptr(),
            std::ptr::null(),
            &raw mut design,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, RETICLE_ERR_UNSUPPORTED);
}

#[cfg(feature = "synth")]
#[test]
fn synthesis_rewrites_the_design() {
    let _guard = NoLeaks::start();
    let design = counter();
    let mut report = std::ptr::null_mut();
    let mut diags = std::ptr::null_mut();
    // SAFETY: `design` is live and both slots are writable.
    let status = unsafe { reticle_design_synth(design, 0, &raw mut report, &raw mut diags) };
    assert_eq!(status, RETICLE_OK);
    assert!(take(report).contains("counter"));
    // SAFETY: `diags` is live and has not been freed.
    unsafe { reticle_diagnostics_free(diags) };

    // A LUT width outside 2..=8 is rejected before anything runs.
    // SAFETY: `design` is live; the two null slots mean "do not report".
    let status =
        unsafe { reticle_design_synth(design, 9, std::ptr::null_mut(), std::ptr::null_mut()) };
    assert_eq!(status, RETICLE_ERR_INVALID);

    // SAFETY: `design` is live; mapping to 4-input LUTs is in range.
    let status =
        unsafe { reticle_design_synth(design, 4, std::ptr::null_mut(), std::ptr::null_mut()) };
    assert_eq!(status, RETICLE_OK);

    // SAFETY: `design` is live and has not been freed.
    unsafe { reticle_design_free(design) };
}

#[cfg(not(feature = "synth"))]
#[test]
fn synthesis_reports_unsupported_when_absent() {
    let _guard = NoLeaks::start();
    let design = counter();
    let mut diags = std::ptr::null_mut();
    // SAFETY: `design` is live and `diags` is a writable slot.
    let status = unsafe { reticle_design_synth(design, 0, std::ptr::null_mut(), &raw mut diags) };
    assert_eq!(status, RETICLE_ERR_UNSUPPORTED);
    // SAFETY: both handles are live and have not been freed.
    unsafe {
        reticle_diagnostics_free(diags);
        reticle_design_free(design);
    }
}

#[cfg(feature = "sim")]
mod simulation {
    use super::*;

    /// Creates a simulator over [`COUNTER_RTL`], freeing the design first
    /// to prove the simulator does not borrow it.
    fn start() -> *mut reticle_sim {
        let design = counter();
        let mut sim = std::ptr::null_mut();
        let mut diags = std::ptr::null_mut();
        // SAFETY: `design` is live and both slots are writable.
        let status =
            unsafe { reticle_sim_create(design, std::ptr::null(), &raw mut sim, &raw mut diags) };
        assert_eq!(status, RETICLE_OK);
        // SAFETY: both handles are live; the simulator keeps its own copy,
        // so freeing the design here must leave it working.
        unsafe {
            reticle_diagnostics_free(diags);
            reticle_design_free(design);
        }
        sim
    }

    #[test]
    fn a_counter_counts() {
        let _guard = NoLeaks::start();
        let sim = start();

        let mut count = 0usize;
        // SAFETY: `sim` is live and `count` is writable.
        assert_eq!(
            unsafe { reticle_sim_net_count(sim, &raw mut count) },
            RETICLE_OK
        );
        assert_eq!(count, 2);

        let mut out = std::ptr::null_mut();
        // SAFETY: as above.
        assert_eq!(
            unsafe { reticle_sim_net_name(sim, 0, &raw mut out) },
            RETICLE_OK
        );
        assert!(take(out).starts_with("counter."));

        let clk_path = c("counter.clk");
        let q_path = c("counter.q");
        let missing = c("counter.nope");
        let (mut clk, mut q, mut nowhere) = (0usize, 0usize, 0usize);
        // SAFETY: `sim` and the paths are live and the slots are writable.
        unsafe {
            assert_eq!(
                reticle_sim_find_net(sim, clk_path.as_ptr(), &raw mut clk),
                RETICLE_OK
            );
            assert_eq!(
                reticle_sim_find_net(sim, q_path.as_ptr(), &raw mut q),
                RETICLE_OK
            );
            assert_eq!(
                reticle_sim_find_net(sim, missing.as_ptr(), &raw mut nowhere),
                RETICLE_ERR_NOT_FOUND
            );
        }

        let mut width = 0u32;
        // SAFETY: `sim` is live and `width` is writable.
        assert_eq!(
            unsafe { reticle_sim_net_width(sim, q, &raw mut width) },
            RETICLE_OK
        );
        assert_eq!(width, 8);

        let mut per_ns = 0u64;
        // SAFETY: as above.
        assert_eq!(
            unsafe { reticle_sim_ticks_per_ns(sim, &raw mut per_ns) },
            RETICLE_OK
        );
        assert!(per_ns > 0);

        // Settle the initial block, then clock the counter three times.
        // SAFETY: `sim` is live throughout.
        unsafe {
            assert_eq!(reticle_sim_run_for(sim, 0), RETICLE_OK);
            let one = c("1'b1");
            let zero = c("1'b0");
            for _ in 0..3 {
                assert_eq!(reticle_sim_set(sim, clk, one.as_ptr()), RETICLE_OK);
                assert_eq!(reticle_sim_run_for(sim, per_ns), RETICLE_OK);
                assert_eq!(reticle_sim_set(sim, clk, zero.as_ptr()), RETICLE_OK);
                assert_eq!(reticle_sim_run_for(sim, per_ns), RETICLE_OK);
            }
        }

        let mut value = 0u64;
        // SAFETY: `sim` is live and `value` is writable.
        assert_eq!(
            unsafe { reticle_sim_get_u64(sim, q, &raw mut value) },
            RETICLE_OK
        );
        assert_eq!(value, 3);

        // SAFETY: as above.
        assert_eq!(unsafe { reticle_sim_get(sim, q, &raw mut out) }, RETICLE_OK);
        assert_eq!(take(out), "8'h03");

        // An integer write goes in and comes back out.
        // SAFETY: `sim` is live.
        assert_eq!(unsafe { reticle_sim_set_u64(sim, q, 200) }, RETICLE_OK);
        // SAFETY: as above.
        assert_eq!(
            unsafe { reticle_sim_get_u64(sim, q, &raw mut value) },
            RETICLE_OK
        );
        assert_eq!(value, 200);

        let mut time = 0u64;
        let mut status: c_int = -1;
        // SAFETY: `sim` is live and both slots are writable.
        unsafe {
            assert_eq!(reticle_sim_time(sim, &raw mut time), RETICLE_OK);
            assert_eq!(reticle_sim_status(sim, &raw mut status), RETICLE_OK);
        }
        assert_eq!(time, per_ns * 6);
        assert_eq!(status, RETICLE_SIM_RUNNING);

        // SAFETY: `sim` is live and has not been freed.
        unsafe { reticle_sim_free(sim) };
    }

    #[test]
    fn bad_net_indices_and_literals_are_rejected() {
        let _guard = NoLeaks::start();
        let sim = start();
        let mut out = std::ptr::null_mut();
        let mut value = 0u64;
        let mut width = 0u32;
        let junk = c("not a literal");
        let one = c("1'b1");
        // SAFETY: `sim` and both strings are live; index 99 is past the end.
        unsafe {
            assert_eq!(reticle_sim_get(sim, 99, &raw mut out), RETICLE_ERR_INVALID);
            assert_eq!(
                reticle_sim_get_u64(sim, 99, &raw mut value),
                RETICLE_ERR_INVALID
            );
            assert_eq!(
                reticle_sim_net_width(sim, 99, &raw mut width),
                RETICLE_ERR_INVALID
            );
            assert_eq!(
                reticle_sim_net_name(sim, 99, &raw mut out),
                RETICLE_ERR_INVALID
            );
            assert_eq!(reticle_sim_set(sim, 99, one.as_ptr()), RETICLE_ERR_INVALID);
            assert_eq!(reticle_sim_set_u64(sim, 99, 1), RETICLE_ERR_INVALID);
            assert_eq!(reticle_sim_set(sim, 0, junk.as_ptr()), RETICLE_ERR_INVALID);
        }
        // SAFETY: `sim` is live and has not been freed.
        unsafe { reticle_sim_free(sim) };
    }

    #[test]
    fn an_unknown_value_has_no_integer_form() {
        let _guard = NoLeaks::start();
        let sim = start();
        let q_path = c("counter.q");
        let mut q = 0usize;
        // SAFETY: `sim` and `q_path` are live and `q` is writable.
        assert_eq!(
            unsafe { reticle_sim_find_net(sim, q_path.as_ptr(), &raw mut q) },
            RETICLE_OK
        );
        let unknown = c("8'bxxxxxxxx");
        let mut value = 0u64;
        // SAFETY: `sim` and `unknown` are live and `value` is writable.
        unsafe {
            assert_eq!(reticle_sim_set(sim, q, unknown.as_ptr()), RETICLE_OK);
            assert_eq!(
                reticle_sim_get_u64(sim, q, &raw mut value),
                RETICLE_ERR_UNKNOWN_VALUE
            );
        }
        // SAFETY: `sim` is live and has not been freed.
        unsafe { reticle_sim_free(sim) };
    }

    #[test]
    fn output_vcd_and_messages_are_drained() {
        let _guard = NoLeaks::start();
        let sim = start();
        let mut out = std::ptr::null_mut();

        // VCD is empty until capture is enabled, and has a header after.
        // SAFETY: `sim` is live and `out` is writable.
        unsafe {
            assert_eq!(reticle_sim_vcd(sim, &raw mut out), RETICLE_OK);
            assert_eq!(take(out), "");
            assert_eq!(reticle_sim_enable_vcd(sim), RETICLE_OK);
            assert_eq!(reticle_sim_run_for(sim, 10), RETICLE_OK);
            assert_eq!(reticle_sim_vcd(sim, &raw mut out), RETICLE_OK);
        }
        assert!(take(out).contains("$var"));

        // SAFETY: `sim` is live and `out` is writable.
        unsafe {
            assert_eq!(reticle_sim_take_output(sim, &raw mut out), RETICLE_OK);
        }
        // The counter has no `$display`, so there is nothing to drain.
        assert_eq!(take(out), "");

        let mut diags = std::ptr::null_mut();
        // SAFETY: `sim` is live and `diags` is a writable slot.
        unsafe {
            assert_eq!(reticle_sim_take_messages(sim, &raw mut diags), RETICLE_OK);
            assert_eq!(
                reticle_sim_take_messages(sim, std::ptr::null_mut()),
                RETICLE_ERR_INVALID
            );
        }
        // SAFETY: both handles are live and have not been freed.
        unsafe {
            reticle_diagnostics_free(diags);
            reticle_sim_free(sim);
        }
    }

    #[test]
    fn run_returns_when_the_queue_drains() {
        let _guard = NoLeaks::start();
        let sim = start();
        // SAFETY: `sim` is live; the counter has no free-running clock, so
        // the queue drains after the initial block.
        unsafe {
            assert_eq!(reticle_sim_run(sim), RETICLE_OK);
            assert_eq!(reticle_sim_run_until(sim, 100), RETICLE_OK);
        }
        let mut time = 0u64;
        // SAFETY: `sim` is live and `time` is writable.
        assert_eq!(unsafe { reticle_sim_time(sim, &raw mut time) }, RETICLE_OK);
        assert_eq!(time, 100);
        // SAFETY: `sim` is live and has not been freed.
        unsafe { reticle_sim_free(sim) };
    }
}

#[cfg(not(feature = "sim"))]
#[test]
fn the_simulator_reports_unsupported_when_absent() {
    let _guard = NoLeaks::start();
    let design = counter();
    let mut sim = std::ptr::null_mut();
    let mut diags = std::ptr::null_mut();
    // SAFETY: `design` is live and both slots are writable.
    let status =
        unsafe { reticle_sim_create(design, std::ptr::null(), &raw mut sim, &raw mut diags) };
    assert_eq!(status, RETICLE_ERR_UNSUPPORTED);
    assert!(sim.is_null());
    // SAFETY: both handles are live and have not been freed.
    unsafe {
        reticle_diagnostics_free(diags);
        reticle_design_free(design);
    }
}
