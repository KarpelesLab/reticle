//! Four things the SDRAM controller's 7-series pins needed that the flow
//! could not build (`ip/memory/sdram_burst/README.md`, "The flow"), each
//! built here as the smallest design that met it.
//!
//! The ones that route need the Project X-Ray database and skip without it,
//! saying so; the flattening one needs nothing.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn reticle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reticle"))
}

fn scratch(name: &str) -> PathBuf {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("io-gaps-{name}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    out
}

/// `reticle fpga` on `source` for `device`, with `extra` options, writing into
/// a scratch directory of its own.
fn fpga(name: &str, source: &str, device: &str, extra: &[&str]) -> (Output, PathBuf) {
    let dir = scratch(name);
    let src = dir.join("source.v");
    std::fs::write(&src, source).unwrap();
    let out = reticle()
        .args(["fpga", "--device", device, "--report"])
        .args(extra)
        .arg("--output-dir")
        .arg(&dir)
        .arg(&src)
        .current_dir(&dir)
        .output()
        .expect("failed to run the reticle binary");
    (out, dir)
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// **An `IOBUF` instantiated on each bit of an inout vector maps to an
/// `IOBUF` a bit**, each with its own `T`. It was `I0033` at flattening: the
/// cell library's `IOBUF` is a module with an inout port, and an inout could
/// only be inlined onto a whole net. Would catch the refusal coming back, and
/// a bit's buffer losing its own enable to the other's. Would not catch a
/// buffer on the wrong pad: no constraints here, so the placer chooses.
#[test]
fn an_iobuf_on_each_bit_of_an_inout_vector_maps() {
    let source = "module top (input wire clk, input wire oe, input wire sel, input wire [1:0] d,\n\
                  output reg [1:0] q, inout wire [1:0] dq);\n\
                  wire [1:0] o;\n\
                  IOBUF b0 (.IO(dq[0]), .I(d[0]), .O(o[0]), .T(~oe));\n\
                  IOBUF b1 (.IO(dq[1]), .I(d[1]), .O(o[1]), .T(sel));\n\
                  always @(posedge clk) q <= o;\nendmodule\n";
    let (out, dir) = fpga("iobuf-bits", source, "xc7a100t-fgg484", &[]);
    assert!(out.status.success(), "{}", text(&out));
    let netlist = std::fs::read_to_string(dir.join("top.v")).unwrap();
    let buffers: Vec<&str> = netlist
        .split("IOBUF ")
        .skip(1)
        .filter(|cell| cell.starts_with("dq$io"))
        .collect();
    assert_eq!(buffers.len(), 2, "{netlist}");
    let enables: std::collections::BTreeSet<String> = buffers
        .iter()
        .filter_map(|cell| {
            let at = cell.find(".T(")?;
            let rest = &cell[at + 3..];
            Some(rest[..rest.find(')')?].to_owned())
        })
        .collect();
    assert_eq!(enables.len(), 2, "one enable for two buffers: {enables:?}");
}
