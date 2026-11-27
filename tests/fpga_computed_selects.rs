//! Selects and part-writes at a computed place, through the 7-series flow:
//! how many lookup tables each takes, and that the mapped netlist computes what
//! the source does.
//!
//! A place that is arithmetic on narrow indices, `x[128 * r + 32 * w +: 32]` or
//! `n[8 * i + 8 +: 8] = d`, used to be built as shifters across the whole
//! vector, because the amount is a sum the mapper cannot see through:
//! `h264_deblock` as first written was 205,796 lookup tables for that reason.
//! Each design here is measured against a bound near what a multiplexer of the
//! element width needs, and simulated, source against netlist, through `reticle
//! sim` with the device's cell models: the source through the simulator's own
//! reading of the select, which shares nothing with synthesis.
//!
//! **Would catch** a select or write that went back to a shifter (the bounds are
//! a tenth or less of what the shifters took), and any value placed at the wrong
//! bit for the values the stimulus reaches. **Would not catch** a wrong bit
//! only for an index value the stimulus never makes; the indices here are two
//! to four bits, and a run of 60 cycles of a 64-bit LFSR reaches all of them
//! with high likelihood but is a sample, not a proof. Bits the source reads as
//! `x`, a select past the end, are not compared.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::Command;

const DEVICE: &str = "xc7a100t-ftg256";

fn reticle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reticle"))
}

fn scratch(name: &str) -> PathBuf {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("selects-{name}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    out
}

/// One design: its source, the width of its `x` and `v`, and the most lookup
/// tables it may take (with what the shifters took, measured before).
struct Case {
    name: &'static str,
    body: &'static str,
    /// Width of the input `x` and of the output `v`.
    x: u32,
    v: u32,
    most: usize,
}

/// Every design has `clk`, a 64-bit `s` of stimulus, a vector `x` and a
/// registered output `v`.
const CASES: &[Case] = &[
    Case {
        // 263 lookup tables as shifters.
        name: "read_biased",
        body: "always @(posedge clk) v <= x[8 * s[3:0] + 8 +: 8];",
        x: 128,
        v: 8,
        most: 40,
    },
    Case {
        // 1,440 as shifters.
        name: "read_sum",
        body: "always @(posedge clk) v <= x[128 * s[1:0] + 32 * s[3:2] +: 32];",
        x: 512,
        v: 32,
        most: 160,
    },
    Case {
        // 255 as shifters.
        name: "read_down",
        body: "always @(posedge clk) v <= x[8 * s[3:0] + 7 -: 8];",
        x: 128,
        v: 8,
        most: 40,
    },
    Case {
        name: "read_bit",
        body: "always @(posedge clk) v <= x[s[3:0] + s[7:4]];",
        // 22 as shifters, and still: 256 values name 31 places, so the sum
        // goes to the shifter (`names_distinct_places`).
        x: 32,
        v: 1,
        most: 24,
    },
    Case {
        // 3,349 as shifters.
        name: "write_biased",
        body: "reg [135:0] n;\n  always @* begin n = x; n[8 * s[3:0] + 8 +: 8] = s[63:56]; end\n  \
               always @(posedge clk) v <= n;",
        x: 136,
        v: 136,
        most: 140,
    },
    Case {
        // 49,713 as shifters.
        name: "write_sum",
        body: "reg [511:0] n;\n  always @* begin n = x; n[128 * s[1:0] + 32 * s[3:2] +: 32] = s[63:32]; end\n  \
               always @(posedge clk) v <= n;",
        x: 512,
        v: 512,
        most: 600,
    },
    Case {
        name: "write_bit",
        body: "reg [31:0] n;\n  always @* begin n = x; n[s[3:0] + s[7:4]] = s[8]; end\n  \
               always @(posedge clk) v <= n;",
        // 49 as shifts, and still; 199 a value at a time.
        x: 32,
        v: 32,
        most: 56,
    },
    Case {
        // 3,349 as shifters.
        name: "write_register",
        body: "initial v = 0;\n  always @(posedge clk) v[8 * s[3:0] + 8 +: 8] <= s[63:56];",
        x: 8,
        v: 136,
        most: 40,
    },
];

fn source(case: &Case) -> String {
    format!(
        "module top (input wire clk, input wire [63:0] s, input wire [{}:0] x, output reg [{}:0] v);\n  \
         {}\nendmodule\n",
        case.x - 1,
        case.v - 1,
        case.body
    )
}

fn testbench(case: &Case) -> String {
    let words = case.x.div_ceil(64);
    format!(
        r#"
module tb;
  reg clk = 0;
  reg [63:0] s = 64'hace1ace1deadbeef;
  reg [{w}:0] wide = 0;
  wire [{v1}:0] v;
  top dut (.clk(clk), .s(s), .x(wide[{x1}:0]), .v(v));
  integer n;
  initial begin
    #120;
    for (n = 0; n < 60; n = n + 1) begin
      s = {{s[62:0], s[63] ^ s[62] ^ s[60] ^ s[59]}};
      wide = {{wide, s}};
      #5 clk = 1;
      #5 clk = 0;
      if (n > {words}) $display("%b", v);
    end
    $finish;
  end
endmodule
"#,
        w = words * 64 - 1,
        v1 = case.v - 1,
        x1 = case.x - 1,
    )
}

fn simulate(files: &[PathBuf]) -> Vec<String> {
    let out = reticle()
        .args(["sim", "--top", "tb", "--device", DEVICE])
        .args(files)
        .output()
        .expect("failed to run the reticle binary");
    assert!(
        out.status.success(),
        "{files:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// **The reads are proved, where they stay in range.** `reticle synth --verify`
/// proves the synthesised netlist against the lowering before it, which still
/// holds the selects as selects, so for a read it is an independent answer;
/// within range, because a select past the end reads `x` there, a free value,
/// and a netlist's zeros differ from it (as the shifter's did). For a write the
/// lowering is where the change is, so this says nothing of writes; the
/// simulation below is what checks those. Would catch a slice taken at the
/// wrong place for any index value, which the simulation only samples.
#[test]
fn the_reads_are_proved_where_they_stay_in_range() {
    let dir = scratch("proved");
    let src = dir.join("reads.v");
    std::fs::write(
        &src,
        "module reads (input wire [5:0] i, input wire [1:0] r, input wire [1:0] w,\n\
         input wire [511:0] s, output wire [7:0] y, output wire [7:0] z, output wire [31:0] u);\n\
         assign y = s[8 * (63 - i) +: 8];\n  assign z = s[8 * i + 7 -: 8];\n\
         assign u = s[128 * r + 32 * w +: 32];\nendmodule\n",
    )
    .unwrap();
    let out = reticle()
        .args(["synth", "--verify", "--report"])
        .arg(&src)
        .output()
        .expect("failed to run the reticle binary");
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(log.contains("synthesis of reads: EQUIVALENT"), "{log}");
}

/// **Each select and write at a computed place is a multiplexer, and right.**
#[test]
fn a_place_computed_from_narrow_indices_is_a_multiplexer() {
    let mut wrong = Vec::new();
    for case in CASES {
        let dir = scratch(case.name);
        let src = dir.join("source.v");
        std::fs::write(&src, source(case)).unwrap();
        let run = reticle()
            .args(["fpga", "--device", DEVICE, "--quiet", "--top", "top"])
            .arg("--output-dir")
            .arg(&dir)
            .arg(&src)
            .output()
            .expect("failed to run the reticle binary");
        assert!(
            run.status.success(),
            "{}: {}",
            case.name,
            String::from_utf8_lossy(&run.stderr)
        );
        let netlist = dir.join("top.v");
        let mapped = std::fs::read_to_string(&netlist).unwrap();
        let luts = mapped
            .lines()
            .filter(|l| {
                let l = l.trim_start();
                l.starts_with("LUT") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit)
            })
            .count();
        if luts > case.most {
            wrong.push(format!(
                "{}: {luts} lookup tables, at most {}",
                case.name, case.most
            ));
        }
        let tb = dir.join("tb.v");
        std::fs::write(&tb, testbench(case)).unwrap();
        let want = simulate(&[src, tb.clone()]);
        let got = simulate(&[netlist, tb]);
        assert!(want.len() > 40, "{}: {} lines", case.name, want.len());
        let compared = want.iter().filter(|l| !l.contains('x')).count();
        assert!(compared * 2 > want.len(), "{}: {compared} known", case.name);
        if want.len() != got.len() {
            wrong.push(format!(
                "{}: {} lines against {}",
                case.name,
                want.len(),
                got.len()
            ));
            continue;
        }
        for (at, (w, g)) in want.iter().zip(&got).enumerate() {
            let differs = w.chars().zip(g.chars()).any(|(a, b)| a != 'x' && a != b);
            if differs {
                wrong.push(format!("{}: cycle {at}: source {w} netlist {g}", case.name));
                break;
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
