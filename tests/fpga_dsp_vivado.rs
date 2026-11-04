//! The `DSP48E1` against Vivado: its answers, its bits, and the netlists that
//! hold one.
//!
//! Everything under `testdata/fpga/vivado/dsp/` is what Vivado 2026.1 said, made
//! by `tools/vivado-parity/`; none of it needs Vivado to run, and none of it has
//! been near a board:
//!
//! - **What the block does.** `vectors/` holds what the vendor's model answered
//!   under `xsim` for 52 settings, and `dsp48e1_model.v`, written here, has to
//!   give the same answers. It would catch a register, an inverter, a shift or an
//!   ALU mode modelled wrong. It would not catch a mode no vector sets; the
//!   model's header lists those.
//! - **What a netlist does.** Every multiply mapped here, with that model, against
//!   the source it was mapped from, cycle by cycle. It would catch a register
//!   absorbed into the wrong stage, a control lost, an operand on the wrong pin or
//!   extended the wrong way, a cascade shifted wrongly. It would not catch
//!   something the source and the model both get wrong, or anything about a part.
//! - **What is written.** A block's features and what each of its inputs is on, in
//!   what Reticle's `.bit` decodes to, against Vivado's for the same design. It
//!   would catch any feature wrong, any tie on the wrong one of the three shapes.
//!   Which routes a signal takes it does not compare.
//!
//! The ones that need a whole place and route for each design are ignored, as the
//! ledger's is, except for one design of each kind.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> Option<PathBuf> {
    let dir = manifest().join("testdata/fpga/vivado/dsp");
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        None
    }
}

fn reticle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reticle"))
}

fn scratch(name: &str) -> PathBuf {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("dsp-{name}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    out
}

fn files_in(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == extension))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// The model against the vendor's
// ---------------------------------------------------------------------------

/// **The model answers as the vendor's does, in every setting that has vectors.**
#[test]
fn the_model_answers_as_the_vendors_does() {
    let Some(dir) = fixtures() else { return };
    let model = dir.join("dsp48e1_model.v");
    let vectors = files_in(&dir.join("vectors"), "v");
    assert!(vectors.len() >= 50, "{} settings", vectors.len());
    let mut wrong = Vec::new();
    for file in &vectors {
        let run = reticle()
            .args(["sim", "--top", "tb"])
            .arg(&model)
            .arg(file)
            .output()
            .expect("failed to run the reticle binary");
        let out = String::from_utf8_lossy(&run.stdout);
        if !out.lines().any(|l| l == "PASS") {
            let first = out.lines().next().unwrap_or("no output").to_owned();
            wrong.push(format!("{}: {first}", file.display()));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// ---------------------------------------------------------------------------
// A mapped netlist against the source
// ---------------------------------------------------------------------------

/// The testbench every design with `clk`, `si` and `so` gets: a sequence from a
/// fixed generator on `si`, with nothing printed until a run long enough for every
/// register to hold what the sequence put there, since the source's registers
/// start unknown and a mapped netlist's at zero. `si` changes a moment after the
/// edge and not on it: in a netlist the clock arrives through buffers, a delta
/// later, and a value changed at the edge beats it there and not in the source.
const SHIFT_TB: &str = r#"
module tb;
  reg clk = 0;
  reg [31:0] lfsr = 32'hace1ace1;
  wire si = lfsr[0];
  wire so;
  TOP dut (.clk(clk), .si(si), .so(so));
  integer n;
  initial begin
    for (n = 0; n < 900; n = n + 1) begin
      #5 clk = 1;
      #1;
      lfsr = {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
      #4 clk = 0;
      if (n > 300) $display("%b", so);
    end
    $finish;
  end
endmodule
"#;

/// The lines a simulation printed, with the simulator's own notes left out.
fn simulate(files: &[PathBuf]) -> Vec<String> {
    let mut run = reticle();
    run.args(["sim", "--top", "tb"]);
    for file in files {
        run.arg(file);
    }
    let out = run.output().expect("failed to run the reticle binary");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{files:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines().map(str::to_owned).collect()
}

/// A design's mapped netlist through `reticle fpga`, with no fabric.
fn map(source: &Path, top: &str, out: &Path) -> PathBuf {
    let run = reticle()
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--quiet",
            "--top",
            top,
        ])
        .arg("--output-dir")
        .arg(out)
        .arg(source)
        .output()
        .expect("failed to run the reticle binary");
    assert!(
        run.status.success(),
        "{top}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    out.join(format!("{top}.v"))
}

/// The cells a mapped netlist may name: a library of the small ones, and the model
/// of this one.
fn libraries(dir: &Path) -> Vec<PathBuf> {
    vec![
        manifest().join("examples/husky/sim/xilinx_cells.v"),
        dir.join("more_cells.v"),
        dir.join("dsp48e1_model.v"),
    ]
}

/// What a flow makes of a design, as the lines its `so` printed, against the same
/// from the source: panics on the first cycle they differ.
fn netlist_computes_the_source(dir: &Path, source: &Path, top: &str, out: &Path, netlist: &Path) {
    let tb = out.join("tb.v");
    std::fs::write(&tb, SHIFT_TB.replace("TOP", top)).unwrap();
    // `reticle sim` reads a signed memory's element as unsigned (found here, and
    // not in this file's way to fix), so that a filter's source simulates to
    // other answers than `xsim` gives. Where that is so, the answer is `xsim`'s.
    let want = match std::fs::read_to_string(dir.join("answers").join(format!("{top}.xsim"))) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(_) => simulate(&[source.to_owned(), tb.clone()]),
    };
    let mut files = libraries(dir);
    files.push(netlist.to_owned());
    files.push(tb);
    let got = simulate(&files);
    assert!(want.len() > 500, "{top}: {} lines", want.len());
    assert!(
        want.iter().any(|l| l == "1") && want.iter().any(|l| l == "0"),
        "{top} never moved, so this asks nothing of it"
    );
    if let Some(at) = want.iter().zip(&got).position(|(a, b)| a != b) {
        panic!(
            "{top}: differs at line {at}: source {} netlist {}",
            want[at], got[at]
        );
    }
    assert_eq!(want.len(), got.len(), "{top}");
}

/// **Every multiply Vivado was asked about, mapped here, computes what its source
/// does**, with nothing but its parity pin to show for it. Would catch a stage lost
/// or gained, which delays every answer; any one bit of a product wrong for one
/// operand pair in a run of 600. Two wrong bits at once cancel, which is what
/// `every_register_setting_computes_the_source` is for.
#[test]
fn every_mapped_design_computes_the_source() {
    let Some(dir) = fixtures() else { return };
    let designs = files_in(&dir.join("designs"), "v");
    let mut seen = 0;
    for source in designs {
        let top = source.file_stem().unwrap().to_str().unwrap().to_owned();
        // A shift register and a parity, written to be asked this; the sweep's
        // settings are by name, with nothing to map, and have the test below.
        if !source_infers(&source) || NEVER_KNOWN.contains(&top.as_str()) || top.starts_with("t_") {
            continue;
        }
        let out = scratch(&format!("map-{top}"));
        let netlist = map(&source, &top, &out);
        netlist_computes_the_source(&dir, &source, &top, &out, &netlist);
        seen += 1;
    }
    assert!(seen >= 40, "{seen} designs");
}

/// A design whose source has state nothing ever resets, so that it simulates as
/// unknown for ever, where a netlist's flip-flops start at zero: `acc <= acc + a * b`.
/// `every_register_setting_computes_the_source` has the same shape with a reset.
const NEVER_KNOWN: [&str; 1] = ["f_macc"];

/// True for a design that describes a multiply rather than naming the block.
fn source_infers(source: &Path) -> bool {
    !std::fs::read_to_string(source)
        .unwrap()
        .contains("DSP48E1 ")
}

/// **A block named in a design reaches the netlist with every connection and
/// parameter it was given**, read back through the model on both sides. Would catch
/// a parameter or a tied pin lost between the instance and the mapped netlist, or a
/// pin connected to the wrong port. It cannot say the model is right, since it is on
/// both sides; `the_model_answers_as_the_vendors_does` is for that.
#[test]
fn a_block_named_in_a_design_reaches_the_netlist_whole() {
    let Some(dir) = fixtures() else { return };
    for top in [
        "base",
        "areg2c1",
        "dport",
        "acasc",
        "inv_op0",
        "inv_clk",
        "tie_zero",
        "tie_one",
        "tie_zero_inv",
        "tie_one_inv",
        "unconnected",
        "comb_open",
    ] {
        let source = dir.join("designs").join(format!("{top}.v"));
        let out = scratch(&format!("named-{top}"));
        let netlist = map(&source, top, &out);
        let tb = out.join("tb.v");
        std::fs::write(&tb, SHIFT_TB.replace("TOP", top)).unwrap();
        let run = |design: &Path| {
            let mut files = libraries(&dir);
            files.push(design.to_owned());
            files.push(tb.clone());
            simulate(&files)
        };
        let (want, got) = (run(&source), run(&netlist));
        assert!(want.len() > 500, "{top}");
        let at = want
            .iter()
            .zip(&got)
            .position(|(x, y)| !x.contains('x') && x != y);
        assert_eq!(
            at,
            None,
            "{top}: {:?} against {:?}",
            at.map(|i| &want[i]),
            at.map(|i| &got[i])
        );
    }
}

/// One multiply with its operands, enable, reset and result on ports, so that a
/// result can be read whole: `areg` registers on `a`, `breg` on `b`, `mreg` after.
fn staged(
    a: u32,
    b: u32,
    signed: bool,
    stages: (u32, u32, u32),
    controls: &str,
    result: u32,
) -> String {
    let (areg, breg, mreg) = stages;
    let sign = if signed { "signed " } else { "" };
    let mut text = format!(
        "module staged (input wire clk, input wire en, input wire rst, input wire {sign}[{}:0] a, \
         input wire {sign}[{}:0] b, output wire {sign}[{}:0] y, output wire {sign}[{}:0] x);\n",
        a - 1,
        b - 1,
        result - 1,
        a - 1,
    );
    let edge = if controls == "arst" {
        "posedge clk or posedge rst"
    } else {
        "posedge clk"
    };
    let update = |q: &str, d: &str, width: u32| -> (String, String) {
        let decl = format!("  reg {sign}[{}:0] {q};\n", width - 1);
        let body = match controls {
            "en" => format!("    if (en) {q} <= {d};\n"),
            "rst" | "arst" => format!("    if (rst) {q} <= 0; else {q} <= {d};\n"),
            "rst1" => format!("    if (rst) {q} <= 1; else {q} <= {d};\n"),
            "both" => format!("    if (rst) {q} <= 0; else if (en) {q} <= {d};\n"),
            _ => format!("    {q} <= {d};\n"),
        };
        (decl, body)
    };
    let mut body = String::new();
    let mut prev = "a".to_owned();
    for i in 0..areg {
        let q = format!("qa{i}");
        let (d, s) = update(&q, &prev, a);
        text.push_str(&d);
        body.push_str(&s);
        prev = q;
    }
    let aq = prev;
    let mut prev = "b".to_owned();
    for i in 0..breg {
        let q = format!("qb{i}");
        let (d, s) = update(&q, &prev, b);
        text.push_str(&d);
        body.push_str(&s);
        prev = q;
    }
    let bq = prev;
    text.push_str(&format!(
        "  wire {sign}[{}:0] m0 = {aq} * {bq};\n",
        result - 1
    ));
    let mut prev = "m0".to_owned();
    for i in 0..mreg {
        let q = format!("qm{i}");
        let (d, s) = update(&q, &prev, result);
        text.push_str(&d);
        body.push_str(&s);
        prev = q;
    }
    text.push_str(&format!("  assign y = {prev};\n  assign x = {aq};\n"));
    if !body.is_empty() {
        text.push_str(&format!("  always @({edge}) begin\n{body}  end\n"));
    }
    text.push_str("endmodule\n");
    text
}

/// The testbench for [`staged`]: operands, enable and reset from a generator, the
/// result printed whole each cycle after a run that fills every stage. `en` starts
/// low and rises once before that, since the designs that use it as a clock would
/// otherwise see an edge at time zero in their source and none in a netlist.
fn staged_tb(a: u32, b: u32, result: u32) -> String {
    format!(
        r#"
module tb;
  reg clk = 0;
  reg [63:0] lfsr = 64'hace1ace1deadbeef;
  reg [63:0] other = 64'h0123456789abcdef;
  reg en = 0;
  reg rst = 0;
  wire [{a1}:0] a = lfsr[{a1}:0];
  wire [{b1}:0] b = other[{b1}:0];
  wire [{r1}:0] y;
  wire [{a1}:0] x;
  staged dut (.clk(clk), .en(en), .rst(rst), .a(a), .b(b), .y(y), .x(x));
  integer n;
  initial begin
    for (n = 0; n < 400; n = n + 1) begin
      #5 clk = 1;
      #1;
      lfsr = {{lfsr[62:0], lfsr[63] ^ lfsr[62] ^ lfsr[60] ^ lfsr[59]}};
      other = {{other[61:0], other[63] ^ other[60], other[62] ^ other[59]}};
      #1;
      if (n == 2) en = 1;
      if (n > 12) begin
        en = other[3] | other[9];
        rst = (other[14:12] == 3'b101);
      end
      #3 clk = 0;
      if (n > 12) $display("%h %h", y, x);
    end
    $finish;
  end
endmodule
"#,
        a1 = a - 1,
        b1 = b - 1,
        r1 = result - 1,
    )
}

/// **Each register setting, with an enable or a reset or both, against its source,
/// the whole result, every cycle.** What the parity test above cannot see: two
/// bits wrong at once, or a stage that only differs when the enable is low or a
/// reset arrives. The operand is also read outside the block, so a register
/// absorbed but needed elsewhere has to stay. Mutations of `fpga::dsp_absorb` are
/// what this test is for, and `docs/fpga-vivado-parity.md` lists the ones tried.
#[test]
fn every_register_setting_computes_the_source() {
    let Some(dir) = fixtures() else { return };
    let mut cases = Vec::new();
    for (areg, breg, mreg) in [
        (0, 0, 0),
        (1, 1, 0),
        (2, 2, 0),
        (3, 3, 0),
        (0, 0, 1),
        (0, 0, 2),
        (0, 0, 3),
        (1, 1, 1),
        (1, 1, 2),
        (2, 2, 1),
        (2, 2, 2),
        (3, 3, 2),
        (2, 0, 1),
        (0, 2, 1),
        (1, 3, 1),
    ] {
        for controls in ["", "en", "rst", "both", "arst", "rst1"] {
            cases.push((18, 18, true, (areg, breg, mreg), controls, 36));
        }
    }
    for stages in [(1, 1, 2), (2, 2, 2), (0, 0, 0), (1, 1, 1)] {
        for (a, b, signed, result) in [
            (25, 18, true, 43),
            (18, 25, true, 43),
            (24, 17, false, 41),
            (16, 16, false, 32),
            (12, 12, true, 24),
            (16, 16, true, 20),
            // Over several blocks.
            (26, 18, true, 44),
            (18, 18, false, 36),
            (32, 32, true, 64),
            (32, 32, false, 64),
            (32, 32, true, 32),
            (35, 35, true, 64),
            (48, 48, false, 64),
        ] {
            for controls in ["", "both"] {
                cases.push((a, b, signed, stages, controls, result));
            }
        }
    }
    let out = scratch("staged");
    let mut wrong = Vec::new();
    let mut blocks = 0;
    for (index, (a, b, signed, stages, controls, result)) in cases.iter().enumerate() {
        let here = out.join(format!("{index}"));
        std::fs::create_dir_all(&here).unwrap();
        let source = here.join("source.v");
        std::fs::write(&source, staged(*a, *b, *signed, *stages, controls, *result)).unwrap();
        let tb = here.join("tb.v");
        std::fs::write(&tb, staged_tb(*a, *b, *result)).unwrap();
        let netlist = map(&source, "staged", &here);
        blocks += usize::from(
            std::fs::read_to_string(&netlist)
                .unwrap()
                .contains("DSP48E1"),
        );
        let want = simulate(&[source.clone(), tb.clone()]);
        let mut files = libraries(&dir);
        files.push(netlist);
        files.push(tb);
        let got = simulate(&files);
        if want != got {
            let at = want.iter().zip(&got).position(|(x, y)| x != y);
            wrong.push(format!(
                "{a}x{b} signed={signed} stages={stages:?} controls=`{controls}` -> {result}: \
                 at {at:?}: {:?} against {:?}",
                at.map(|i| &want[i]),
                at.map(|i| &got[i])
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(
        blocks > cases.len() / 2,
        "{blocks} of {} took a block",
        cases.len()
    );
}

/// One sum of multiplies with everything on ports, written out for each shape the
/// block takes it in, and read whole every cycle against its source: a term's
/// flip-flop or none, an addend, a truncated or widened sum, an accumulator with a
/// reset so that it has a known value at all. Would catch a term extended the wrong
/// way, the top block's `P` confused for a term's, `OPMODE` selecting the wrong `Z`,
/// and an enable lost. A subtraction stays out of the block and is here to say so.
#[test]
fn every_sum_of_products_computes_the_source() {
    let Some(dir) = fixtures() else { return };
    // Unsigned, 17 bits, since one unsigned 18 is already two blocks.
    let head = |signed: bool, y: u32, w: u32| {
        let s = if signed { "signed " } else { "" };
        let top = w - 1;
        format!(
            "module staged (input wire clk, input wire en, input wire rst, input wire {s}[{top}:0] a, \
             input wire {s}[{top}:0] b, output wire {s}[{}:0] y, output wire {s}[{top}:0] x);\n  assign x = a;\n\
             reg {s}[{top}:0] a0, a1, a2, b0, b1, b2; reg {s}[{}:0] m0, m1, m2, q, r;\n",
            y - 1,
            y - 1
        )
    };
    let regs = "  always @(posedge clk) begin a0 <= a; a1 <= a0; a2 <= a1; b0 <= b; b1 <= b0; b2 <= b1; end\n";
    let mut cases: Vec<(String, String, u32, u32, usize)> = Vec::new();
    for (signed, y) in [
        (true, 40),
        (true, 48),
        (false, 40),
        (true, 30),
        (false, 30),
        (true, 70),
    ] {
        let w = if signed { 18 } else { 17 };
        let mut add = |name: &str, body: &str, blocks: usize| {
            cases.push((
                name.to_owned(),
                format!("{}{regs}{body}endmodule\n", head(signed, y, w)),
                w,
                y,
                blocks,
            ));
        };
        add(
            "two",
            "  always @(posedge clk) begin m0 <= a0 * b0; m1 <= a1 * b1; q <= m0 + m1; end\n  assign y = q;\n",
            2,
        );
        add(
            "three",
            "  always @(posedge clk) begin m0 <= a0 * b0; m1 <= a1 * b1; m2 <= a2 * b2; q <= m0 + m1 + m2; end\n  assign y = q;\n",
            3,
        );
        add(
            "comb",
            "  always @(posedge clk) begin q <= a0 * b0 + a1 * b1; end\n  assign y = q;\n",
            2,
        );
        add(
            "open",
            "  always @(posedge clk) begin m0 <= a0 * b0; m1 <= a1 * b1; end\n  assign y = m0 + m1;\n",
            2,
        );
        add(
            "addend",
            "  always @(posedge clk) begin r <= a2; q <= a0 * b0 + r; end\n  assign y = q;\n",
            1,
        );
        add(
            "addend2",
            "  always @(posedge clk) begin r <= a2; m0 <= a0 * b0; q <= m0 + r; end\n  assign y = q;\n",
            1,
        );
        add(
            "addend3",
            "  always @(posedge clk) begin m0 <= a0 * b0; q <= m0 + a2; end\n  assign y = q;\n",
            1,
        );
        add(
            "enable",
            "  always @(posedge clk) begin if (en) m0 <= a0 * b0; if (en) m1 <= a1 * b1; if (en) q <= m0 + m1; end\n  assign y = q;\n",
            2,
        );
        add(
            "reset",
            "  always @(posedge clk) begin if (rst) m0 <= 0; else m0 <= a0 * b0; if (rst) m1 <= 0; else m1 <= a1 * b1; if (rst) q <= 0; else q <= m0 + m1; end\n  assign y = q;\n",
            2,
        );
        add(
            "accumulate",
            "  initial q = 0;\n  always @(posedge clk) begin if (rst) q <= 0; else q <= q + a0 * b0; end\n  assign y = q;\n",
            1,
        );
        add(
            "accumulate2",
            "  initial q = 0;\n  always @(posedge clk) begin if (rst) q <= 0; else if (en) q <= a0 * b0 + q; end\n  assign y = q;\n",
            1,
        );
        add(
            "subtract",
            "  always @(posedge clk) begin q <= a2 - a0 * b0; end\n  assign y = q;\n",
            1,
        );
        add(
            "fans",
            "  always @(posedge clk) begin m0 <= a0 * b0; m1 <= a1 * b1; q <= m0 + m1; r <= m0; end\n  assign y = q + r;\n",
            2,
        );
    }
    let out = scratch("sums");
    let mut wrong = Vec::new();
    let mut folded = 0;
    for (index, (name, text, w, y, blocks)) in cases.iter().enumerate() {
        let here = out.join(format!("{index}"));
        std::fs::create_dir_all(&here).unwrap();
        let source = here.join("source.v");
        std::fs::write(&source, text).unwrap();
        let tb = here.join("tb.v");
        std::fs::write(&tb, staged_tb(*w, *w, *y)).unwrap();
        let netlist = map(&source, "staged", &here);
        let mapped = std::fs::read_to_string(&netlist).unwrap();
        let found = mapped.matches("DSP48E1 #(").count();
        assert_eq!(found, *blocks, "{name} {y}");
        folded += usize::from(!mapped.contains("CARRY4"));
        let want = simulate(&[source.clone(), tb.clone()]);
        let mut files = libraries(&dir);
        files.push(netlist);
        files.push(tb);
        let got = simulate(&files);
        // An accumulator is unknown in its source until its first reset, where a
        // netlist's flip-flops start at zero: that much is not a question. It has
        // to be known for most of the run for this to ask anything.
        let known = want.iter().filter(|l| !l.contains('x')).count();
        assert!(known * 2 > want.len(), "{name} y={y}: {known} known");
        let at = want
            .iter()
            .zip(&got)
            .position(|(x, y)| !x.contains('x') && x != y);
        if want.len() != got.len() || at.is_some() {
            wrong.push(format!(
                "{name} y={y}: at {at:?}: {:?} against {:?}",
                at.map(|i| &want[i]),
                at.map(|i| &got[i])
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(folded > 0);
}

/// **What a block's registers cannot be, stays out of it, and the multiply is still
/// right.** Each design here has a register the block could not stand in for, or
/// two that disagree: a reset on one of `A`'s two registers, which share `RSTA`; a
/// falling edge; a second clock; a reset to something other than zero; a value at
/// power up; a thing the source asked to keep; a product cut narrower than the sum
/// reads it, so that the block's sign is not the term's. Would catch any of those
/// conditions dropped from `fpga::dsp_absorb` or `fpga::dsp_fold`, which is what
/// each design was written for and `docs/fpga-vivado-parity.md` lists the mutations
/// tried. What it would not catch is a register kept out that could have gone in:
/// the cost of that is Vivado's comparison's to find.
#[test]
fn what_a_block_cannot_hold_stays_out_and_is_still_right() {
    let Some(dir) = fixtures() else { return };
    let head = "module staged (input wire clk, input wire en, input wire rst, \
                input wire signed [17:0] a, input wire signed [17:0] b, \
                output wire signed [39:0] y, output wire signed [17:0] x);\n  assign x = a;\n\
                reg signed [17:0] a0, a1, a2, b0, b1, b2; reg signed [39:0] m0, m1, q, r; \
                reg signed [19:0] n0, n1;\n";
    let plain = "  always @(posedge clk) begin a2 <= a; b2 <= b; end\n";
    let cases: [(&str, String); 21] = [
        (
            "reset_on_the_near_register",
            "  always @(posedge clk) begin a0 <= a; b0 <= b; if (rst) a1 <= 0; else a1 <= a0; \
             if (rst) b1 <= 0; else b1 <= b0; m0 <= a1 * b1; end\n  assign y = m0;\n"
                .to_owned(),
        ),
        (
            "reset_on_the_far_register",
            "  always @(posedge clk) begin if (rst) a0 <= 0; else a0 <= a; if (rst) b0 <= 0; \
             else b0 <= b; a1 <= a0; b1 <= b0; m0 <= a1 * b1; end\n  assign y = m0;\n"
                .to_owned(),
        ),
        (
            "enable_on_one_operand",
            "  always @(posedge clk) begin if (en) a0 <= a; b0 <= b; m0 <= a0 * b0; end\n  assign y = m0;\n"
                .to_owned(),
        ),
        (
            "an_enable_for_each_stage",
            "  always @(posedge clk) begin a0 <= a; if (en) a1 <= a0; b0 <= b; if (rst) b1 <= b0; \
             if (en ^ rst) m0 <= a1 * b1; if (!en) m1 <= m0; end\n  assign y = m1;\n"
                .to_owned(),
        ),
        (
            "reset_active_low",
            "  always @(posedge clk) begin if (!rst) a0 <= 0; else a0 <= a; if (!rst) b0 <= 0; else b0 <= b; \
             if (!rst) m0 <= 0; else m0 <= a0 * b0; end\n  assign y = m0;\n"
                .to_owned(),
        ),
        (
            "falling_edge_operand",
            format!("{plain}  always @(negedge clk) begin a0 <= a2; b0 <= b2; end\n  always @(posedge clk) m0 <= a0 * b0;\n  assign y = m0;\n"),
        ),
        (
            "falling_edge_after",
            format!("{plain}  always @(negedge clk) m0 <= a2 * b2;\n  assign y = m0;\n"),
        ),
        (
            "another_clock_on_an_operand",
            "  always @(posedge en) a0 <= a;\n  always @(posedge clk) begin b0 <= b; m0 <= a0 * b0; end\n  assign y = m0;\n"
                .to_owned(),
        ),
        (
            "another_clock_after",
            format!("{plain}  always @(posedge en) m0 <= a2 * b2;\n  always @(posedge clk) m1 <= m0;\n  assign y = m1;\n"),
        ),
        (
            "reset_to_one_on_an_operand",
            "  always @(posedge clk) begin if (rst) a0 <= 1; else a0 <= a; b0 <= b; m0 <= a0 * b0; end\n  assign y = m0;\n"
                .to_owned(),
        ),
        (
            "asynchronous_after",
            format!("{plain}  always @(posedge clk or posedge rst) if (rst) m0 <= 0; else m0 <= a2 * b2;\n  assign y = m0;\n"),
        ),
        (
            "kept_operand",
            "  (* keep = \"true\" *) reg signed [17:0] k;\n  always @(posedge clk) begin k <= a; b0 <= b; m0 <= k * b0; end\n  assign y = m0;\n"
                .to_owned(),
        ),
        (
            "read_elsewhere_after",
            format!("{plain}  always @(posedge clk) begin m0 <= a2 * b2; m1 <= m0; r <= m0 + 1; end\n  assign y = m1 ^ r;\n"),
        ),
        (
            "product_read_twice",
            format!("{plain}  wire signed [39:0] w = a2 * b2;\n  always @(posedge clk) begin m0 <= w; r <= w + 1; end\n  assign y = m0 ^ r;\n"),
        ),
        // Sums.
        (
            "two_clocks_in_a_sum",
            format!("{plain}  always @(posedge clk) m0 <= a2 * b2;\n  always @(posedge en) m1 <= a2 * b;\n  always @(posedge clk) q <= m0 + m1;\n  assign y = q;\n"),
        ),
        (
            "another_clock_on_a_sum",
            format!("{plain}  always @(posedge clk) begin m0 <= a2 * b2; m1 <= a * b; end\n  always @(posedge en) q <= m0 + m1;\n  assign y = q;\n"),
        ),
        (
            "a_cut_product_in_a_wider_sum",
            format!("{plain}  always @(posedge clk) begin n0 <= a2 * b2; n1 <= a * b; q <= n0 + n1; end\n  assign y = q;\n"),
        ),
        (
            "a_cut_product_with_an_addend",
            format!("{plain}  always @(posedge clk) begin n0 <= a2 * b2; q <= n0 + a; end\n  assign y = q;\n"),
        ),
        (
            "a_term_on_a_falling_edge",
            format!("{plain}  always @(negedge clk) m0 <= a2 * b2;\n  always @(posedge clk) begin m1 <= a * b; q <= m0 + m1; end\n  assign y = q;\n"),
        ),
        (
            "a_term_read_elsewhere",
            format!("{plain}  always @(posedge clk) begin m0 <= a2 * b2; m1 <= a * b; q <= m0 + m1; r <= m0; end\n  assign y = q ^ r;\n"),
        ),
        (
            "an_accumulator_with_an_asynchronous_reset",
            format!("{plain}  always @(posedge clk or posedge rst) if (rst) q <= 0; else q <= q + a2 * b2;\n  assign y = q;\n"),
        ),
    ];
    let out = scratch("cannot");
    let mut wrong = Vec::new();
    for (name, body) in &cases {
        let here = out.join(name);
        std::fs::create_dir_all(&here).unwrap();
        let source = here.join("source.v");
        std::fs::write(&source, format!("{head}{body}endmodule\n")).unwrap();
        let tb = here.join("tb.v");
        std::fs::write(&tb, staged_tb(18, 18, 40)).unwrap();
        let netlist = map(&source, "staged", &here);
        if *name == "kept_operand" {
            // In a copy or not the function is the same, so only this can tell:
            // what the source asked to keep stays one flip-flop in the fabric, and
            // the block has no register it could be.
            let mapped = std::fs::read_to_string(&netlist).unwrap();
            assert!(mapped.contains(".AREG(0)"), "the kept register went in");
        }
        let want = simulate(&[source.clone(), tb.clone()]);
        let mut files = libraries(&dir);
        files.push(netlist);
        files.push(tb);
        let got = simulate(&files);
        let known = want.iter().filter(|l| !l.contains('x')).count();
        assert!(
            known * 2 > want.len(),
            "{name}: {known} known of {}",
            want.len()
        );
        let at = want
            .iter()
            .zip(&got)
            .position(|(x, y)| !x.contains('x') && x != y);
        if want.len() != got.len() || at.is_some() {
            wrong.push(format!(
                "{name}: at {at:?}: {:?} against {:?}",
                at.map(|i| &want[i]),
                at.map(|i| &got[i])
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// ---------------------------------------------------------------------------
// The bits against Vivado's
// ---------------------------------------------------------------------------

/// The input pins of each block and the interconnect wire each one is on, from
/// `ppips_dsp_<side>.db`.
fn input_wires(root: &str, side: &str) -> BTreeMap<String, (String, String, u32)> {
    let path = format!("{root}/artix7/ppips_dsp_{side}.db");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        let mut parts = name.split('.').skip(1);
        let (Some(to), Some(from)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !(to.starts_with("DSP_0_") || to.starts_with("DSP_1_")) {
            continue;
        }
        let Some(rest) = from.strip_prefix("DSP_") else {
            continue;
        };
        let (kind, rest) = if let Some(rest) = rest.strip_prefix("IMUX") {
            ("IMUX", rest)
        } else if let Some(rest) = rest.strip_prefix("CTRL") {
            ("CTRL", rest)
        } else {
            continue;
        };
        let Some((number, row)) = rest.split_once('_') else {
            continue;
        };
        out.insert(
            to.to_owned(),
            (kind.to_owned(), number.to_owned(), row.parse().unwrap_or(0)),
        );
    }
    out
}

/// What `tools/vivado-parity/dsp_answers.py` makes of a decode: one block a list of
/// lines. See that script for what the lines mean.
fn summary(root: &str, decode: &str) -> Vec<Vec<String>> {
    let mut tiles: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for line in decode.lines() {
        let mut words = line.split_whitespace();
        if let (Some(a), Some(b), None) = (words.next(), words.next(), words.next()) {
            tiles.entry(a).or_default().insert(b);
        }
    }
    let mut blocks = Vec::new();
    for (tile, features) in &tiles {
        let Some(rest) = tile.strip_prefix("DSP_") else {
            continue;
        };
        let Some((side, place)) = rest.split_once("_X") else {
            continue;
        };
        let Some((x, y)) = place.split_once('Y') else {
            continue;
        };
        let y: u32 = y.parse().unwrap();
        let wires = input_wires(root, &side.to_lowercase());
        let used: BTreeSet<&str> = features
            .iter()
            .filter(|f| f.starts_with("DSP48.DSP_"))
            .map(|f| f.split('.').nth(1).unwrap())
            .collect();
        for block in used {
            let mut named: Vec<String> = Vec::new();
            let prefix = format!("DSP48.{block}.");
            let local = format!("{block}_");
            for feature in features {
                if let Some(name) = feature.strip_prefix(&prefix) {
                    named.push(name.to_owned());
                } else if let Some(rest) = feature.strip_prefix(&local) {
                    let (pin, source) = rest.split_once('.').unwrap();
                    named.push(format!("{pin}={}", &source[4..7]));
                }
            }
            named.sort();
            let mut states: BTreeMap<&str, Vec<String>> = BTreeMap::new();
            for state in ["one", "signal", "zero"] {
                states.insert(state, Vec::new());
            }
            for (pin, (kind, number, row)) in &wires {
                let Some(name) = pin.strip_prefix(&local) else {
                    continue;
                };
                let inter = format!("INT_{side}_X{x}Y{}", y + row);
                let wire = if side == "L" {
                    format!("{kind}_L{number}")
                } else {
                    format!("{kind}{number}")
                };
                let empty = BTreeSet::new();
                let there = tiles.get(inter.as_str()).unwrap_or(&empty);
                let arc = there.iter().find_map(|f| {
                    f.strip_prefix(wire.as_str())
                        .and_then(|r| r.strip_prefix('.'))
                });
                let state = match arc {
                    None => "one",
                    Some(fan)
                        if fan.starts_with("GFAN")
                            && there.contains(format!("{fan}.GND_WIRE").as_str()) =>
                    {
                        "zero"
                    }
                    Some(_) => "signal",
                };
                states.get_mut(state).unwrap().push(name.to_owned());
            }
            // The clock, by whether it comes off the global network.
            let row: u32 = std::fs::read_to_string(format!(
                "{root}/artix7/ppips_dsp_{}.db",
                side.to_lowercase()
            ))
            .unwrap_or_default()
            .lines()
            .find_map(|line| {
                let name = line.split_whitespace().next()?;
                let mut parts = name.split('.').skip(1);
                (parts.next()? == format!("{block}_CLK"))
                    .then(|| parts.next()?.rsplit('_').next()?.parse().ok())?
            })
            .unwrap_or(0);
            let empty = BTreeSet::new();
            let there = tiles
                .get(format!("INT_{side}_X{x}Y{}", y + row).as_str())
                .unwrap_or(&empty);
            let wire = if side == "L" { "CLK_L0." } else { "CLK0." };
            let network = there
                .iter()
                .find_map(|f| f.strip_prefix(wire))
                .is_some_and(|source| source.starts_with("GCLK"));
            let mut lines = vec![format!("features {}", named.join(" "))];
            lines.push(format!(
                "clock {}",
                if network { "network" } else { "none" }
            ));
            for (state, mut pins) in states {
                pins.sort();
                lines.push(format!("{state} {}", pins.join(" ")).trim_end().to_owned());
            }
            blocks.push(lines);
        }
    }
    blocks.sort();
    blocks
}

fn answers(text: &str) -> Vec<Vec<String>> {
    let mut blocks: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        if line == "block" {
            blocks.push(Vec::new());
        } else if let Some(block) = blocks.last_mut() {
            // Trailing spaces of an empty list.
            block.push(line.trim_end().to_owned());
        }
    }
    blocks.sort();
    blocks
}

/// The database's root, or `None` with a line saying it is absent.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a100t/tilegrid.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB")
        && Path::new(&format!("{root}/{probe}")).is_file()
    {
        return Some(root);
    }
    let home = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| format!("{d}/reticle"))
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| format!("{h}/.cache/reticle"))
        })?;
    let dir = format!("{home}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3");
    if Path::new(&format!("{dir}/{probe}")).is_file() {
        Some(dir)
    } else {
        eprintln!("skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db`");
        None
    }
}

/// Builds one design of `designs/` to a bitstream and gives its decode and its
/// routed netlist, or the build's complaint.
///
/// `asker` keeps two tests' builds of one design apart.
fn build(dir: &Path, top: &str, asker: &str) -> Result<(String, PathBuf, PathBuf), String> {
    let out = scratch(&format!("bit-{asker}-{top}"));
    let rcf = out.join(format!("{top}.rcf"));
    std::fs::write(
        &rcf,
        "create_clock -name clk -period 5 clk\nset_io -io_standard LVCMOS33 clk N11\n\
         set_io -io_standard LVCMOS33 si L4\nset_io -io_standard LVCMOS33 so L5\n",
    )
    .unwrap();
    let bit = out.join(format!("{top}.bit"));
    let routed = out.join(format!("{top}.routed.v"));
    let source = dir.join("designs").join(format!("{top}.v"));
    let run = reticle()
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--quiet",
            "--offline",
            "--top",
            top,
        ])
        .arg("--constraints")
        .arg(&rcf)
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg("--routed-netlist")
        .arg(&routed)
        .arg(&source)
        .output()
        .expect("failed to run the reticle binary");
    if !run.status.success() {
        return Err(String::from_utf8_lossy(&run.stderr).into_owned());
    }
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(
        err.is_empty()
            || !err.contains(" bit(s) unexplained")
            || err.contains(" 0 bit(s) unexplained"),
        "{top}: {err}"
    );
    let decoded = reticle()
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--offline",
            "--decode",
        ])
        .arg(&bit)
        .output()
        .expect("failed to run the reticle binary");
    assert!(decoded.status.success());
    let text = String::from_utf8_lossy(&decoded.stdout).into_owned();
    // **Every bit decodes**: nothing the decoder could not name, in a tile the
    // flow wrote. `HCLK` frames carry an ECC word of Vivado's and not this flow's.
    let unnamed: Vec<&str> = text.lines().filter(|l| l.starts_with("? ")).collect();
    assert!(unnamed.is_empty(), "{top}: {unnamed:?}");
    Ok((text, routed, out))
}

/// Where a design's blocks are not Vivado's, on purpose, and why; each is the same
/// function at a cost or with registers elsewhere, which the simulations hold it to.
const NOT_THE_SAME: [(&str, &str); 17] = [
    (
        "c_arst",
        "an asynchronous reset stays out of the block; Vivado takes `P` in anyway",
    ),
    ("f_preadd", "this flow does not use the pre-adder"),
    (
        "f_square",
        "`AREG` 2 and `P` here, `AREG` 1, `M` and `P` there: the same latency",
    ),
    (
        "f_two",
        "one block read twice here, two joined by `PCOUT` there",
    ),
    (
        "fir8",
        "the same blocks, with the shift register in the fabric here and in `ACOUT` there",
    ),
    (
        "l_sum30",
        "the same blocks, with the shift register in the fabric here and in `ACOUT` there",
    ),
    (
        "s_25x19",
        "two blocks here, one with the odd bit on `C` there",
    ),
    (
        "s_26x18",
        "two blocks here, one with the odd bit on `C` there",
    ),
    ("s_26x26", "four blocks here, three and some logic there"),
    (
        "s_32x32",
        "the same blocks, with `M` here where Vivado has `P`",
    ),
    (
        "s_32x32t",
        "the same blocks, with `M` here where Vivado has `P`",
    ),
    (
        "s_35x35",
        "the same blocks, with `M` here where Vivado has `P`",
    ),
    (
        "s_48x48",
        "the same blocks, with `M` here where Vivado has `P` and more",
    ),
    (
        "u_18x18",
        "two blocks here, one with the odd bit on `C` there",
    ),
    (
        "u_25x18",
        "the same count, with `M` here where Vivado has `P` and `C`",
    ),
    (
        "u_32x32",
        "the same blocks, with `M` here where Vivado has `P`",
    ),
    (
        "u_32x32t",
        "the same blocks, with `M` here where Vivado has `P`",
    ),
];

/// One design's blocks against Vivado's. `Err` says how they differ.
fn compare_with_vivado(root: &str, dir: &Path, top: &str) -> Result<(), String> {
    let (decode, _, _) = build(dir, top, "blocks")?;
    let want = std::fs::read_to_string(dir.join("answers").join(format!("{top}.txt")))
        .map_err(|e| e.to_string())?;
    let (want, got) = (answers(&want), summary(root, &decode));
    if want == got {
        return Ok(());
    }
    if want.len() != got.len() {
        return Err(format!("{} block(s) against {}", want.len(), got.len()));
    }
    // What differs, word by word: a list of 46 `MASK` bits hides one.
    let mut out = String::new();
    for (ours, theirs) in got.iter().zip(&want) {
        for (line, other) in ours.iter().zip(theirs) {
            let a: BTreeSet<&str> = line.split_whitespace().collect();
            let b: BTreeSet<&str> = other.split_whitespace().collect();
            let only_here: Vec<&&str> = a.difference(&b).collect();
            let only_there: Vec<&&str> = b.difference(&a).collect();
            if !only_here.is_empty() || !only_there.is_empty() {
                out.push_str(&format!(
                    "\n  {}: Reticle only {only_here:?}, Vivado only {only_there:?}",
                    line.split_whitespace().next().unwrap_or("")
                ));
            }
        }
    }
    Err(out)
}

/// **A block with a multiply, a register of each kind, an inferred multiply and a
/// by-name block with every input a constant, as Vivado wrote them.** One of each
/// kind, for the gate; the ignored test below is every design.
#[test]
fn the_blocks_of_a_few_designs_are_vivados() {
    let Some(dir) = fixtures() else { return };
    let Some(root) = chipdb() else { return };
    for top in [
        "mul18",
        "tie_zero",
        "mult_none",
        "comb_open",
        "site_r_odd_none",
        "n_pair",
    ] {
        if let Err(message) = compare_with_vivado(&root, &dir, top) {
            panic!("{top}: {message}");
        }
    }
}

/// **Every design there is an answer for, but the ones `NOT_THE_SAME` names**, and
/// each of those is checked still to differ, so that closing a gap has to say so.
#[test]
#[ignore = "places and routes each of about 150 designs, and needs the Project X-Ray database"]
fn the_blocks_of_every_design_are_vivados() {
    let Some(dir) = fixtures() else { return };
    let Some(root) = chipdb() else { return };
    let tops: Vec<String> = files_in(&dir.join("answers"), "txt")
        .iter()
        .map(|file| file.file_stem().unwrap().to_str().unwrap().to_owned())
        .collect();
    let wrong = in_parallel(&tops, |top| {
        let excused = NOT_THE_SAME.iter().any(|(name, _)| *name == top);
        match (compare_with_vivado(&root, &dir, top), excused) {
            (Ok(()), false) | (Err(_), true) => None,
            (Ok(()), true) => Some(format!("{top}: the same now; take it off the list")),
            (Err(message), false) => Some(format!("{top}: {message}")),
        }
    });
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// `check` of every name on a few threads, giving what each said was wrong. A whole
/// place and route of each is why this is here, and each is its own process.
fn in_parallel(names: &[String], check: impl Fn(&str) -> Option<String> + Sync) -> Vec<String> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let wrong = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(name) = names.get(at) else { break };
                    if let Some(message) = check(name) {
                        wrong.lock().unwrap().push(message);
                    }
                }
            });
        }
    });
    let mut out = wrong.into_inner().unwrap();
    out.sort();
    out
}

/// The routed netlist of one design against its source, by the same testbench.
fn routed_computes_the_source(dir: &Path, top: &str) {
    let (_, routed, out) = build(dir, top, "routed").unwrap_or_else(|e| panic!("{top}: {e}"));
    let source = dir.join("designs").join(format!("{top}.v"));
    netlist_computes_the_source(dir, &source, top, &out, &routed);
}

/// **What the part is given computes what was written**: the netlist the bitstream
/// is made from, tables paired and pins as the router left them, against the source,
/// with the block on its own and over two blocks and a carry chain. A wrong bit
/// here could not be a wrong mapping, which the tests above would have seen, so it
/// would catch `--routed-netlist` and the pin swaps. It says nothing of the frames,
/// which `the_blocks_of_a_few_designs_are_vivados` does for the block.
#[test]
fn the_routed_netlist_computes_the_source() {
    let Some(dir) = fixtures() else { return };
    let Some(_) = chipdb() else { return };
    for top in ["mul18", "u_18x18"] {
        routed_computes_the_source(&dir, top);
    }
}

/// As above for everything Reticle infers, which takes a while.
#[test]
#[ignore = "places and routes about 90 designs, and needs the Project X-Ray database"]
fn the_routed_netlist_of_every_design_computes_the_source() {
    let Some(dir) = fixtures() else { return };
    let Some(_) = chipdb() else { return };
    let tops: Vec<String> = files_in(&dir.join("designs"), "v")
        .iter()
        .filter(|source| source_infers(source))
        .map(|source| source.file_stem().unwrap().to_str().unwrap().to_owned())
        .filter(|top| !NEVER_KNOWN.contains(&top.as_str()))
        .collect();
    let wrong = in_parallel(&tops, |top| {
        std::panic::catch_unwind(|| routed_computes_the_source(&dir, top))
            .err()
            .map(|_| format!("{top}: see its panic above"))
    });
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
