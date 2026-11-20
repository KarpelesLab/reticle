//! The in-fabric MCU-flash responder and its control interface, simulated so
//! CI runs their HDL testbenches.
//!
//! `testdata/fpga/cynthion/smartcard_serial_mcu.v` answers a Ledger secure
//! element's timed MCU-flash reads (UNSEC_CHUNK_READ_EXT kind 2) from a 32 KB
//! image in block RAM, so the element's boot-time probes are met in a few
//! clocks rather than a USB round trip. Two pieces have HDL testbenches:
//!
//!   * `smartcard_mcu_resp_tb.v` drives the responder — the SEPROXYHAL
//!     framing, the trigger, the exact answer bytes, the end-of-flash clamp,
//!     the disabled pass-through and the reframe on each ATR;
//!   * `smartcard_mcu_ctrl_tb.v` drives the control interface — the image
//!     upload, the CRC (asserted against Python's `zlib.crc32`, an independent
//!     implementation), the status and notice records, the LOAD-while-powered
//!     refusal and the USB-reset clear.
//!
//! Each prints one PASS line, which is what these tests assert. They skip when
//! `testdata/` or `ip/` is not in this copy of the crate, the way the other
//! testbench-driven tests here do: the files are part of the repository and
//! excluded from the published `.crate`.

#![cfg(all(feature = "verilog", feature = "sim"))]

use reticle::diag::Diagnostics;
use reticle::sim::{SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

/// Elaborates the sources (the testbench first), runs to `$finish`, and
/// returns what it printed. `None` when a source is missing from this crate.
fn run_tb(top: &str, sources: &[&str]) -> Option<String> {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for name in sources {
        let text = match std::fs::read_to_string(name) {
            Ok(t) => t,
            Err(_) => {
                eprintln!("skipped: `{name}` is not in this copy of the crate");
                return None;
            }
        };
        let id = map.add(*name, &text).expect("fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    let refs: Vec<_> = files.iter().collect();
    let mut opts = ElabOptions::new(Dialect::Verilog2005);
    opts.top = Some(top.to_string());
    let design = elaborate(&refs, &opts, &mut diags).expect("the testbench elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let mut sim = Simulator::new(&design, SimOptions::default()).expect("it simulates");
    sim.run();
    assert!(sim.finished(), "`{top}` did not reach `$finish`");
    Some(sim.output().to_string())
}

#[test]
fn the_responder_answers_kind2_reads_from_the_image() {
    let Some(out) = run_tb(
        "smartcard_mcu_resp_tb",
        &[
            "testdata/fpga/cynthion/smartcard_mcu_resp_tb.v",
            "testdata/fpga/cynthion/smartcard_mcu_resp.v",
        ],
    ) else {
        return;
    };
    assert_eq!(
        out.as_str(),
        "PASS: kind-2 reads are answered from the image, clamped, silent when disabled, and reframed on each ATR\n",
        "`smartcard_mcu_resp_tb.v` disagrees with `smartcard_mcu_resp.v`"
    );
}

#[test]
fn the_control_interface_loads_crcs_and_clears() {
    let Some(out) = run_tb(
        "smartcard_mcu_ctrl_tb",
        &[
            "testdata/fpga/cynthion/smartcard_mcu_ctrl_tb.v",
            "testdata/fpga/cynthion/smartcard_mcu_ctrl.v",
            "ip/memory/fifo_sync/rtl/fifo_sync.v",
        ],
    ) else {
        return;
    };
    assert_eq!(
        out.as_str(),
        "PASS: upload, CRC, status, notices, the powered-LOAD refusal, the per-LOAD counter reset and the USB-reset clear all hold\n",
        "`smartcard_mcu_ctrl_tb.v` disagrees with `smartcard_mcu_ctrl.v`"
    );
}
