//! `synth::opt::RegisterLoads`, which the 7 series' `--synthesis vivado` asks
//! for, proved by the post-synthesis check on the two shapes it was written
//! for: a hold behind a net, and a load under an enable that decides a
//! multiplexer in it.
//!
//! The proof is `synth::verify`'s: the synthesised module against the source
//! lowered and nothing else, sequentially, its registers matched by name. It
//! reads a register with an enable as `en ? d : q`, which is the function
//! both rules leave as it was, so it is the proof of exactly what they claim.
//! The registers have a reset so that the induction has a state to start from.

#![cfg(all(feature = "synth", feature = "formal", feature = "verilog"))]

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, SynthStats, run};

/// The reference samples of NewAE's `sad`, four bytes of them: a shift under
/// an `if` inside an `if`, written as a `for` loop over part selects, which
/// is what leaves the register's own bits rearranged in the arm its enable
/// never loads.
const SHIFT_LOOP: &str = "
module shift_loop (input wire clk, input wire rst, input wire a, input wire b,
                   input wire c, input wire [7:0] din, output wire [7:0] o);
  reg [31:0] r;
  integer d;
  always @(posedge clk)
    if (rst) begin
      r <= 32'd0;
      d = 0;
    end
    else if (a || b || c) begin
      for (d = 0; d < 4; d = d + 1)
        if (a || b) begin
          if (d == 3) r[d*8 +: 8] <= c ? r[0 +: 8] : din;
          else r[d*8 +: 8] <= r[(d+1)*8 +: 8];
        end
    end
  assign o = r[7:0];
endmodule
";

/// The analyser FIFO's write pointer: its next value a net that its flags
/// read too, the hold in it.
const HOLD_NET: &str = "
module hold_net (input wire clk, input wire rst, input wire w,
                 output reg [3:0] q, output wire [3:0] n);
  reg [3:0] inc;
  wire [3:0] next = w ? inc : q;
  always @(posedge clk)
    if (rst) begin
      q <= 4'd0;
      inc <= 4'd1;
    end else begin
      q <= next;
      if (w) inc <= inc + 4'd1;
    end
  assign n = next;
endmodule
";

fn elaborate(text: &str, top: &str) -> Design {
    let mut map = SourceMap::new();
    let source = map.add(format!("{top}.v"), text.to_owned()).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::Verilog2005,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let options = reticle::verilog::ElabOptions {
        top: Some(top.to_owned()),
        ..reticle::verilog::ElabOptions::default()
    };
    let design = reticle::verilog::elaborate_file(&ast, &options, &mut diags)
        .expect("the design elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    design
}

/// `top` synthesised with the rules on or off, its synthesis proved.
fn synthesise(text: &str, top: &str, register_loads: bool) -> SynthStats {
    let mut design = elaborate(text, top);
    let mut diags = Diagnostics::new();
    let options = SynthOptions {
        register_loads,
        verify_equivalence: true,
        ..SynthOptions::default()
    };
    let stats = run(&mut design, &options, &mut diags);
    assert!(!diags.has_errors(), "{top}: {}", stats.render(None));
    stats
}

/// How many times the rules fired, over every round.
fn fired(stats: &SynthStats, key: &str) -> u64 {
    stats
        .passes
        .iter()
        .filter(|(name, _)| name == "register_loads")
        .map(|(_, s)| s.get(key))
        .sum()
}

/// **Both shapes are rewritten, and both are proved equivalent to their
/// source.** Would catch either rule loading something the register did not
/// (the check finds a cycle where the two differ, which is an error here),
/// or either no longer firing on the shape it was written for; would not
/// catch a rule that fires and changes nothing, which the figures pinned in
/// `tests/fpga_xray_synthesis.rs` are for.
#[test]
fn register_loads_are_proved_on_the_shapes_they_were_written_for() {
    for (text, top, key) in [
        (
            SHIFT_LOOP,
            "shift_loop",
            "loads simplified under the enable",
        ),
        (HOLD_NET, "hold_net", "enables lifted through a net"),
    ] {
        let stats = synthesise(text, top, true);
        assert!(fired(&stats, key) > 0, "{top}: `{key}` never fired");
        assert_eq!(stats.verification.len(), 1, "{top}");
        assert!(
            stats.verification[0].passed(),
            "{top}: {}\n{:?}",
            stats.verification[0].render(),
            stats.verification[0]
                .diags
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );
    }
}

/// **Off, the pass does not run at all**, so a flow that does not ask for it
/// — every one but the 7 series' under `--synthesis vivado` — synthesises as
/// it did. Would catch the option ignored.
#[test]
fn register_loads_are_off_by_default() {
    assert!(!SynthOptions::default().register_loads);
    let stats = synthesise(SHIFT_LOOP, "shift_loop", false);
    assert!(
        stats
            .passes
            .iter()
            .all(|(name, _)| name != "register_loads")
    );
}
