//! Unit tests for the WebAssembly surface, driven the way JavaScript
//! drives it.
//!
//! The module is target-independent, so the whole protocol can be
//! exercised on the host: [`call`] allocates an argument with
//! [`reticle_wasm_alloc`], hands over the pointer and length, reads the
//! length-prefixed reply and frees both — exactly the sequence
//! `web/reticle.js` performs. A leak would show up as a failure in the
//! allocator, so the tests focus on the protocol and on the round trip
//! from Verilog through synthesis, emission and simulation.
//!
//! JSON is checked by looking for the fields rather than parsed: the crate
//! ships no JSON reader, and a substring assertion on a known-shaped
//! document is enough to catch a field that stopped being written.

use super::*;

/// A string argument, kept alive in the module's own linear memory for the
/// duration of a call.
struct Arg {
    ptr: *mut u8,
    len: usize,
}

impl Arg {
    /// Copies `text` into a fresh allocation, as the host would.
    fn new(text: &str) -> Arg {
        let bytes = text.as_bytes();
        let ptr = reticle_wasm_alloc(bytes.len());
        assert!(!ptr.is_null() || bytes.is_empty());
        if !bytes.is_empty() {
            // SAFETY: `ptr` is a fresh allocation of exactly `bytes.len()`
            // bytes, so the copy is in bounds and cannot overlap.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
        }
        Arg {
            ptr,
            len: bytes.len(),
        }
    }
}

impl Drop for Arg {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `reticle_wasm_alloc` with exactly
        // this length and is freed once.
        unsafe { reticle_wasm_free(self.ptr, self.len) };
    }
}

/// Reads a length-prefixed reply and frees its buffer.
fn reply(ptr: *mut u8) -> String {
    assert!(!ptr.is_null(), "the call answered with a null buffer");
    // SAFETY: every entry point answers with a buffer laid out as a
    // little-endian `u32` length followed by that many bytes, so the
    // header read and the body read are both in bounds.
    let (len, body) = unsafe {
        let mut header = [0u8; 4];
        std::ptr::copy_nonoverlapping(ptr, header.as_mut_ptr(), 4);
        let len = usize::try_from(u32::from_le_bytes(header)).expect("a length fits usize");
        let body = std::slice::from_raw_parts(ptr.add(4), len);
        (
            len,
            std::str::from_utf8(body)
                .expect("replies are UTF-8")
                .to_owned(),
        )
    };
    // SAFETY: the same pointer, with the size the protocol documents.
    unsafe { reticle_wasm_free(ptr, len + 4) };
    body
}

/// Elaborates one source and returns the reply.
fn elaborate(language: &str, name: &str, source: &str, top: &str) -> String {
    let (language, name, source, top) = (
        Arg::new(language),
        Arg::new(name),
        Arg::new(source),
        Arg::new(top),
    );
    // SAFETY: every pair points at its own live allocation of exactly that
    // length, which stays alive until after the call returns.
    reply(unsafe {
        reticle_wasm_elaborate(
            language.ptr,
            language.len,
            name.ptr,
            name.len,
            source.ptr,
            source.len,
            top.ptr,
            top.len,
        )
    })
}

/// Pulls a JSON string field out of a reply.
///
/// The replies this module builds escape with the crate's own JSON
/// writer, so a field's value ends at the first unescaped quote; that is
/// all this needs to find.
fn field(json: &str, key: &str) -> String {
    let needle = format!("\"{key}\":\"");
    let start = json
        .find(&needle)
        .unwrap_or_else(|| panic!("no `{key}` field in {json}"))
        + needle.len();
    let bytes = json.as_bytes();
    let mut out = String::new();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => break,
            b'\\' => {
                i += 1;
                out.push(match bytes[i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    other => char::from(other),
                });
            }
            other => out.push(char::from(other)),
        }
        i += 1;
    }
    out
}

/// A counter in the `.rtl` text format, for the calls that take a design.
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

#[test]
fn the_allocator_round_trips() {
    // A zero-length request still answers with a pointer that can be
    // handed straight back.
    let ptr = reticle_wasm_alloc(0);
    assert!(!ptr.is_null());
    // SAFETY: a zero length is documented as ignored.
    unsafe { reticle_wasm_free(ptr, 0) };

    let arg = Arg::new("hello, reticle");
    // SAFETY: `arg` owns `len` readable bytes for the duration.
    let seen = unsafe { std::slice::from_raw_parts(arg.ptr, arg.len) };
    assert_eq!(seen, b"hello, reticle");

    // Freeing null is a no-op, as the wrapper relies on.
    // SAFETY: that is the property under test.
    unsafe { reticle_wasm_free(std::ptr::null_mut(), 8) };
}

#[test]
fn version_and_features_are_plain_strings() {
    assert_eq!(reply(reticle_wasm_version()), crate::VERSION);
    let features = reply(reticle_wasm_features());
    assert_eq!(features.contains("verilog"), cfg!(feature = "verilog"));
    assert_eq!(features.contains("vhdl"), cfg!(feature = "vhdl"));
    assert_eq!(features.contains("sim"), cfg!(feature = "sim"));
    assert_eq!(features.contains("synth"), cfg!(feature = "synth"));
}

#[test]
fn an_unknown_language_is_reported_not_guessed() {
    let json = elaborate("cobol", "a.cob", "IDENTIFICATION DIVISION.", "");
    assert!(json.contains("\"ok\":false"), "{json}");
    assert!(json.contains("verilog"), "{json}");
    assert!(json.contains("\"errors\":1"), "{json}");
}

#[test]
fn a_bad_format_name_is_reported() {
    let (rtl, format) = (Arg::new(COUNTER_RTL), Arg::new("postscript"));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_emit(rtl.ptr, rtl.len, format.ptr, format.len) });
    assert!(json.contains("\"ok\":false"), "{json}");
    assert!(json.contains("blif"), "{json}");
}

#[test]
fn broken_rtl_comes_back_as_diagnostics() {
    let (rtl, format) = (Arg::new("this is not a design"), Arg::new("verilog"));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_emit(rtl.ptr, rtl.len, format.ptr, format.len) });
    assert!(json.contains("\"ok\":false"), "{json}");
    assert!(json.contains("\"severity\":\"error\""), "{json}");
    assert!(json.contains("\"line\":"), "{json}");
}

#[test]
fn emit_renders_a_design() {
    let (rtl, format) = (Arg::new(COUNTER_RTL), Arg::new("verilog"));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_emit(rtl.ptr, rtl.len, format.ptr, format.len) });
    assert!(json.contains("\"ok\":true"), "{json}");
    assert!(field(&json, "output").contains("module counter"), "{json}");
}

#[cfg(feature = "verilog")]
#[test]
fn verilog_goes_from_source_to_rtl() {
    let source = "module adder(input [7:0] a, b, output [7:0] y);\n\
                  assign y = a + b;\nendmodule\n";
    let json = elaborate("verilog", "adder.v", source, "adder");
    assert!(json.contains("\"ok\":true"), "{json}");
    assert!(json.contains("\"errors\":0"), "{json}");
    let rtl = field(&json, "rtl");
    assert!(rtl.contains("module adder"), "{rtl}");

    // And the `.rtl` it produced is what the other calls accept.
    let (arg, format) = (Arg::new(&rtl), Arg::new("verilog"));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_emit(arg.ptr, arg.len, format.ptr, format.len) });
    assert!(json.contains("\"ok\":true"), "{json}");
}

#[cfg(feature = "verilog")]
#[test]
fn a_verilog_error_carries_a_location() {
    let json = elaborate(
        "verilog",
        "bad.v",
        "module m;\n  wire w = ;\nendmodule\n",
        "",
    );
    assert!(json.contains("\"ok\":false"), "{json}");
    assert!(json.contains("\"file\":\"bad.v\""), "{json}");
    assert!(json.contains("\"line\":2"), "{json}");
    assert!(!field(&json, "rendered").is_empty(), "{json}");
}

#[cfg(not(feature = "verilog"))]
#[test]
fn verilog_reports_a_missing_frontend() {
    let json = elaborate("verilog", "m.v", "module m; endmodule", "");
    assert!(json.contains("\"ok\":false"), "{json}");
    assert!(json.contains("verilog"), "{json}");
}

#[cfg(feature = "vhdl")]
#[test]
fn vhdl_goes_from_source_to_rtl() {
    let source = "entity t is port (clk : in bit; q : out bit); end entity;\n\
                  architecture a of t is begin q <= clk; end architecture;\n";
    let json = elaborate("vhdl", "t.vhd", source, "t");
    assert!(json.contains("\"ok\":true"), "{json}");
    assert!(field(&json, "rtl").contains("module t"), "{json}");
}

#[cfg(feature = "synth")]
#[test]
fn synthesis_answers_with_a_netlist_and_a_report() {
    let rtl = Arg::new(COUNTER_RTL);
    // SAFETY: the pair points at a live allocation of its length.
    let json = reply(unsafe { reticle_wasm_synth(rtl.ptr, rtl.len, 4) });
    assert!(json.contains("\"ok\":true"), "{json}");
    assert!(field(&json, "report").contains("counter"), "{json}");
    assert!(field(&json, "rtl").contains("module counter"), "{json}");

    // An out-of-range LUT width is rejected before anything runs.
    // SAFETY: as above.
    let json = reply(unsafe { reticle_wasm_synth(rtl.ptr, rtl.len, 9) });
    assert!(json.contains("\"ok\":false"), "{json}");
}

#[cfg(not(feature = "synth"))]
#[test]
fn synthesis_reports_a_missing_stage() {
    let rtl = Arg::new(COUNTER_RTL);
    // SAFETY: the pair points at a live allocation of its length.
    let json = reply(unsafe { reticle_wasm_synth(rtl.ptr, rtl.len, 0) });
    assert!(json.contains("\"ok\":false"), "{json}");
    assert!(json.contains("synth"), "{json}");
}

#[cfg(feature = "sim")]
#[test]
fn simulation_answers_with_time_status_and_a_waveform() {
    let (rtl, top) = (Arg::new(COUNTER_RTL), Arg::new("counter"));
    // SAFETY: both pairs point at live allocations of their length.
    let json =
        reply(unsafe { reticle_wasm_simulate(rtl.ptr, rtl.len, top.ptr, top.len, 1_000, 1) });
    assert!(json.contains("\"ok\":true"), "{json}");
    assert!(json.contains("\"time\":1000"), "{json}");
    assert!(json.contains("\"status\":\"running\""), "{json}");
    assert!(field(&json, "vcd").contains("$var"), "{json}");

    // With no VCD asked for, the field is there but empty, so the wrapper
    // never has to test for its absence.
    // SAFETY: as above.
    let json = reply(unsafe { reticle_wasm_simulate(rtl.ptr, rtl.len, top.ptr, top.len, 10, 0) });
    assert_eq!(field(&json, "vcd"), "");
}

#[cfg(feature = "sim")]
#[test]
fn simulation_reports_a_top_that_is_not_there() {
    let (rtl, top) = (Arg::new(COUNTER_RTL), Arg::new("nope"));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_simulate(rtl.ptr, rtl.len, top.ptr, top.len, 10, 0) });
    assert!(json.contains("\"ok\":false"), "{json}");
}

#[cfg(not(feature = "sim"))]
#[test]
fn simulation_reports_a_missing_stage() {
    let (rtl, top) = (Arg::new(COUNTER_RTL), Arg::new(""));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_simulate(rtl.ptr, rtl.len, top.ptr, top.len, 10, 0) });
    assert!(json.contains("\"ok\":false"), "{json}");
    assert!(json.contains("sim"), "{json}");
}

#[cfg(all(feature = "verilog", feature = "synth", feature = "sim"))]
#[test]
fn the_whole_pipeline_round_trips() {
    // Source in, netlist and a simulation out, with every step going
    // through the wasm boundary and nothing but text in between.
    let source = "module blink(input clk, output reg q);\n\
                  initial q = 1'b0;\n\
                  always @(posedge clk) q <= ~q;\n\
                  endmodule\n";
    let json = elaborate("verilog", "blink.v", source, "blink");
    assert!(json.contains("\"ok\":true"), "{json}");
    let rtl = field(&json, "rtl");

    let arg = Arg::new(&rtl);
    // SAFETY: the pair points at a live allocation of its length.
    let json = reply(unsafe { reticle_wasm_synth(arg.ptr, arg.len, 0) });
    assert!(json.contains("\"ok\":true"), "{json}");
    let netlist = field(&json, "rtl");

    let (arg, format) = (Arg::new(&netlist), Arg::new("verilog"));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_emit(arg.ptr, arg.len, format.ptr, format.len) });
    assert!(json.contains("\"ok\":true"), "{json}");
    assert!(field(&json, "output").contains("module blink"), "{json}");

    let (arg, top) = (Arg::new(&netlist), Arg::new("blink"));
    // SAFETY: both pairs point at live allocations of their length.
    let json = reply(unsafe { reticle_wasm_simulate(arg.ptr, arg.len, top.ptr, top.len, 100, 0) });
    assert!(json.contains("\"ok\":true"), "{json}");
    assert!(json.contains("\"time\":100"), "{json}");
}
