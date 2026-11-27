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

/// **An `ODDR` on a buffer's three-state enable is refused before placing,
/// with the port and the ball.** The `OLOGIC` has the register (its `TFF`),
/// but this flow builds an `ODDR` on the data path only, and the build used to
/// place, then fail in the router with "no wire joining them". Would catch the
/// refusal lost through the `not` cell a tri-state's enable is made of, which
/// is how it first failed to fire, and the ball left out. Would not catch an
/// `ODDR` on the enable through anything other than inversions.
#[test]
fn an_oddr_on_a_three_state_enable_is_refused_with_its_ball() {
    let source = "module top (input wire clk, input wire en, input wire d, output wire q,\n\
                  inout wire pad);\n  wire t, o;\n\
                  ODDR #(.DDR_CLK_EDGE(\"SAME_EDGE\"), .INIT(1'b1)) ot (\n\
                  .Q(t), .C(clk), .CE(1'b1), .D1(~en), .D2(~en), .R(1'b0), .S(1'b0));\n\
                  ODDR #(.DDR_CLK_EDGE(\"SAME_EDGE\"), .INIT(1'b0)) od (\n\
                  .Q(o), .C(clk), .CE(1'b1), .D1(d), .D2(d), .R(1'b0), .S(1'b0));\n\
                  assign pad = t ? 1'bz : o;\n  assign q = pad;\nendmodule\n";
    let dir = scratch("oddr-enable");
    let xdc = dir.join("pins.xdc");
    std::fs::write(
        &xdc,
        "set_property -dict {PACKAGE_PIN D1 IOSTANDARD LVCMOS33} [get_ports pad]\n",
    )
    .unwrap();
    let xdc = xdc.to_string_lossy().into_owned();
    let (out, _) = fpga(
        "oddr-enable-build",
        source,
        "xc7a100t-fgg484",
        &["--constraints", &xdc],
    );
    let log = text(&out);
    assert!(!out.status.success(), "{log}");
    assert!(
        log.contains(
            "the three-state enable of port `pad` (ball D1) is the output of the `ODDR` `ot`"
        ),
        "{log}"
    );
}

/// The Project X-Ray database, or `None` with a line saying what is missing:
/// `RETICLE_CHIPDB`, then the pinned copy in the per-user cache, as
/// `tests/fpga_xray_tristate.rs` looks.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a100t/tilegrid.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but `{root}/{probe}` is not there");
        return None;
    }
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")));
    let dir = cache.map(|c| format!("{c}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3"));
    match dir {
        Some(dir) if Path::new(&format!("{dir}/{probe}")).is_file() => Some(dir),
        _ => {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` \
                 or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// A design built to a bitstream for the XC7A100T-FGG484 with `xdc`: the
/// build's output, and the lines `--decode` reads back from the bitstream.
fn bitstream(name: &str, source: &str, xdc: &str, root: &str) -> (Output, Vec<String>) {
    let dir = scratch(name);
    let constraints = dir.join("pins.xdc");
    std::fs::write(&constraints, xdc).unwrap();
    let src = dir.join("source.v");
    std::fs::write(&src, source).unwrap();
    let bit = dir.join("top.bit");
    let out = reticle()
        .env("RETICLE_CHIPDB", root)
        .args([
            "fpga",
            "--device",
            "xc7a100t-fgg484",
            "--report",
            "--constraints",
        ])
        .arg(&constraints)
        .arg("--bitstream")
        .arg(&bit)
        .arg("--output-dir")
        .arg(&dir)
        .arg(&src)
        .output()
        .expect("failed to run the reticle binary");
    if !out.status.success() {
        return (out, Vec::new());
    }
    let decoded = reticle()
        .env("RETICLE_CHIPDB", root)
        .args(["fpga", "--device", "xc7a100t-fgg484", "--decode"])
        .arg(&bit)
        .output()
        .expect("failed to run the reticle binary");
    let lines = String::from_utf8_lossy(&decoded.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    (out, lines)
}

/// **An `ODDR` on the pad of a single-pad IO tile is refused before placing,
/// with the ball; a plain output there still builds.** Both of bank 35's
/// (`F4`, `IO_0`; `L6`, `IO_25`), clocked from a global buffer and from the
/// fabric. It used to place the `ODDR` in another tile and fail in the router;
/// the clock pips such a register needs have no bits under the tile's own wire
/// names (`xray::check_single_pad_registers`). Would catch the refusal lost, or
/// moved after placing, and a plain output on such a ball refused with it.
/// Would not catch the refusal being needless, should the pips turn out to be
/// the two-pad tile's after all: that needs a Vivado bitstream nobody here has.
#[test]
fn an_io_register_on_a_single_pad_tile_is_refused_with_its_ball() {
    let Some(root) = chipdb() else { return };
    let fabric = "module top (input wire clk, input wire d, output wire q);\n\
                  ODDR #(.DDR_CLK_EDGE(\"SAME_EDGE\"), .INIT(1'b0)) od (\n\
                  .Q(q), .C(clk), .CE(1'b1), .D1(d), .D2(d), .R(1'b0), .S(1'b0));\nendmodule\n";
    let global = "module top (input wire clk, input wire d, output wire q);\n\
                  wire g;\n  BUFG bg (.I(clk), .O(g));\n\
                  ODDR #(.DDR_CLK_EDGE(\"SAME_EDGE\"), .INIT(1'b0)) od (\n\
                  .Q(q), .C(g), .CE(1'b1), .D1(d), .D2(d), .R(1'b0), .S(1'b0));\nendmodule\n";
    let plain = "module top (input wire clk, input wire d, output reg q);\n\
                 always @(posedge clk) q <= d;\nendmodule\n";
    let mut wrong = Vec::new();
    for ball in ["F4", "L6"] {
        let xdc = format!(
            "set_property -dict {{PACKAGE_PIN H4 IOSTANDARD LVCMOS33}} [get_ports clk]\n\
             set_property -dict {{PACKAGE_PIN B2 IOSTANDARD LVCMOS33}} [get_ports d]\n\
             set_property -dict {{PACKAGE_PIN {ball} IOSTANDARD LVCMOS33}} [get_ports q]\n"
        );
        for (clock, source) in [("fabric", fabric), ("global", global)] {
            let name = format!("single-{ball}-{clock}");
            let (out, _) = bitstream(&name, source, &xdc, &root);
            let log = text(&out);
            let said = format!(
                "`od` (ODDR) is the IO register of ball {ball}, which is on a single-pad IO tile"
            );
            if out.status.success() || !log.contains(&said) {
                wrong.push(format!("{name}: not refused as it should be:\n{log}"));
            }
            if log.contains("placed on real sites") || log.contains("router") {
                wrong.push(format!("{name}: refused after placing:\n{log}"));
            }
        }
        let name = format!("single-{ball}-plain");
        let (out, _) = bitstream(&name, plain, &xdc, &root);
        let log = text(&out);
        if !out.status.success() || !log.contains(" 0 bit(s) unexplained") {
            wrong.push(format!("{name}: a plain output did not build:\n{log}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
