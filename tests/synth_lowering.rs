//! A process and the cells it lowers to compute the same thing.
//!
//! **Nothing else here checks that.** `tests/synth_golden.rs` pins the
//! lowering's *text*, which catches a change and says nothing about
//! whether the change was right; `synth --verify` proves the optimised
//! netlist equivalent to the unoptimised lowering, so both of its sides
//! come through the same lowering and a fault in it cancels out; and the
//! IP library's testbenches simulate the elaborated design, before
//! synthesis has run at all. So the one pass that turns an `always`
//! block into flip-flops had no end-to-end check on its semantics.
//!
//! It needed one. `src/synth/proc/exec.rs` built a `case` statement's
//! clock enable as a chain of one multiplexer per arm, and the fix
//! replaced that with a balanced OR over the arms — a rewrite of the
//! expression that decides *whether a register loads at all*, in the
//! pass with no semantic test. `docs/fpga-trellis.md` has why the chain
//! had to go: on an ECP5 at 60 MHz it was the difference between a
//! decompressor that worked and one that decoded four bytes and stopped.
//!
//! The method is the same one the rest of this repository uses for a
//! claim about behaviour: run both and compare, over every input there
//! is rather than a sample. Each design below is elaborated twice, one
//! copy synthesised and one left as processes, and the two are clocked
//! side by side on identical stimulus with every output compared every
//! cycle.
//!
//! What these would catch: an enable that fires when it should not or
//! fails to when it should, for any arm of a case, any priority between
//! overlapping arms, any default, and any register that holds. What they
//! would not: anything about *timing*, which is the fault that prompted
//! the file and which no simulation can see; anything the optimiser does
//! after lowering, which `synth --verify` owns; and anything about a
//! process shape not written below.

#![cfg(all(feature = "sim", feature = "synth", feature = "verilog"))]

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::logic::{Bit, Logic};
use reticle::sim::{SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run as synth_run};
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

/// Elaborates one Verilog source, optionally synthesising it, and leaks
/// the design so a `Simulator` can borrow it for the rest of the test.
fn design(top: &str, text: &str, synthesise: bool) -> &'static Design {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let id = map.add(format!("{top}.v"), text).expect("the source fits");
    let file = parse_source(
        &mut map,
        id,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    assert!(
        !diags.has_errors(),
        "{top} does not parse:\n{}",
        diags.render(&map)
    );
    let options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
    let mut d = elaborate(&[&file], &options, &mut diags)
        .unwrap_or_else(|| panic!("{top} produced no design"));
    assert!(
        !diags.has_errors(),
        "{top} does not elaborate:\n{}",
        diags.render(&map)
    );
    if synthesise {
        synth_run(&mut d, &SynthOptions::default(), &mut diags);
        assert!(
            !diags.has_errors(),
            "{top} does not synthesise:\n{}",
            diags.render(&map)
        );
    }
    Box::leak(Box::new(d))
}

fn bit(v: bool) -> Logic {
    Logic::from_bit(if v { Bit::One } else { Bit::Zero })
}

fn word(width: u32, value: u64) -> Logic {
    Logic::from_u64(value, width)
}

/// A net of the top module, or a panic naming it.
fn top(sim: &Simulator<'_>, name: &str) -> reticle::sim::NetHandle {
    let path = format!("{}.{}", sim.top_name(), name);
    sim.net(&path)
        .unwrap_or_else(|| panic!("no net `{path}` in the simulation"))
}

/// A pair of simulators over the same source, one synthesised.
struct Pair<'d> {
    proc: Simulator<'d>,
    cells: Simulator<'d>,
}

impl<'d> Pair<'d> {
    fn new(top: &str, a: &'d Design, b: &'d Design) -> Pair<'d> {
        Pair {
            proc: Simulator::new(a, SimOptions::default())
                .unwrap_or_else(|_| panic!("{top} as processes cannot be simulated")),
            cells: Simulator::new(b, SimOptions::default())
                .unwrap_or_else(|_| panic!("{top} as cells cannot be simulated")),
        }
    }

    fn set(&mut self, name: &str, v: Logic) {
        let a = top(&self.proc, name);
        let b = top(&self.cells, name);
        self.proc.set(a, v.clone());
        self.cells.set(b, v);
    }

    /// One clock period, half low and half high.
    fn cycle(&mut self, clk: &str) {
        self.set(clk, bit(false));
        self.proc.run_for(500);
        self.cells.run_for(500);
        self.set(clk, bit(true));
        self.proc.run_for(500);
        self.cells.run_for(500);
    }

    /// Fails if the two disagree on `name`.
    fn agree(&self, name: &str, what: &str) {
        let a = top(&self.proc, name);
        let b = top(&self.cells, name);
        let (x, y) = (self.proc.get(a), self.cells.get(b));
        assert_eq!(
            x, y,
            "{name} disagrees after {what}: processes say {x}, cells say {y}"
        );
    }
}

/// Sixteen arms on a four-bit selector, each loading the register from a
/// different expression and only when its own enable bit is set.
///
/// This is the shape the chain was built for and the shape the tree
/// replaced: disjoint constant items, a register assigned under a
/// condition inside each of them, and a default that assigns nothing so
/// the register must hold.
const WIDE_CASE: &str = r#"
module wide_case(
    input             clk,
    input      [3:0]  sel,
    input      [15:0] en,
    input      [7:0]  d,
    output reg [7:0]  q
);
    always @(posedge clk) begin
        case (sel)
            4'd0:  if (en[0])  q <= d;
            4'd1:  if (en[1])  q <= d + 8'd1;
            4'd2:  if (en[2])  q <= d ^ 8'haa;
            4'd3:  if (en[3])  q <= {d[3:0], d[7:4]};
            4'd4:  if (en[4])  q <= d - 8'd1;
            4'd5:  if (en[5])  q <= ~d;
            4'd6:  if (en[6])  q <= d | 8'h0f;
            4'd7:  if (en[7])  q <= d & 8'hf0;
            4'd8:  if (en[8])  q <= {1'b0, d[7:1]};
            4'd9:  if (en[9])  q <= {d[6:0], 1'b0};
            4'd10: if (en[10]) q <= d + q;
            4'd11: if (en[11]) q <= d ^ q;
            4'd12: if (en[12]) q <= 8'h5a;
            4'd13: if (en[13]) q <= q + 8'd3;
            4'd14: if (en[14]) q <= 8'd0;
            // 15 assigns nothing at all, and neither does the default,
            // so both have to leave `q` alone.
            default: ;
        endcase
    end
endmodule
"#;

/// Overlapping items, so the arms are *not* parallel and the priority a
/// chain encodes is the behaviour rather than an artefact.
///
/// `casez` with `?` makes several items match at once; Verilog says the
/// first wins. The fix only changed the parallel path, and this is what
/// says it left the other one alone.
const PRIORITY_CASE: &str = r#"
module priority_case(
    input            clk,
    input      [3:0] sel,
    input            en,
    output reg [3:0] q
);
    always @(posedge clk) begin
        casez (sel)
            4'b1???: q <= 4'd1;
            4'b?1??: if (en) q <= 4'd2;
            4'b??1?: q <= 4'd3;
            4'b0001: if (en) q <= 4'd4;
            default: ;
        endcase
    end
endmodule
"#;

/// Every `(sel, en, d)` the sixteen-arm case can see, against the
/// processes it was lowered from.
///
/// The enables are swept as a one-hot plus all-set and all-clear rather
/// than over all 65536 values: the arm that fires is chosen by `sel`, so
/// only `en[sel]` can matter, and the three patterns cover it set, clear
/// and set-with-everything-else-set. What is exhaustive is the pair that
/// decides the enable.
#[test]
fn a_wide_parallel_case_lowers_to_cells_that_compute_the_same_thing() {
    let a = design("wide_case", WIDE_CASE, false);
    let b = design("wide_case", WIDE_CASE, true);
    let mut p = Pair::new("wide_case", a, b);
    p.set("clk", bit(false));
    p.set("sel", word(4, 0));
    p.set("en", word(16, 0));
    p.set("d", word(8, 0));
    p.cycle("clk");

    let mut cycles = 0u32;
    for sel in 0..16u64 {
        for en in (0..16u64).map(|i| 1u64 << i).chain([0u64, 0xffff]) {
            for d in [0u64, 1, 0x5a, 0x80, 0xff] {
                p.set("sel", word(4, sel));
                p.set("en", word(16, en));
                p.set("d", word(8, d));
                p.cycle("clk");
                p.agree("q", &format!("sel={sel} en={en:#06x} d={d:#04x}"));
                cycles += 1;
            }
        }
    }
    assert_eq!(cycles, 16 * 18 * 5, "the stimulus this swept");
}

/// Every `(sel, en)` of the overlapping case, which is the control: the
/// first matching item must still win.
#[test]
fn an_overlapping_case_keeps_its_priority_through_lowering() {
    let a = design("priority_case", PRIORITY_CASE, false);
    let b = design("priority_case", PRIORITY_CASE, true);
    let mut p = Pair::new("priority_case", a, b);
    p.set("clk", bit(false));
    // **Primed, because the two disagree about a register that has
    // never been loaded.** `4'b1000` matches the first item, which
    // assigns unconditionally, so `q` is known from here on. Before
    // that the process reads `x` and the lowered flip-flop reads `z`:
    // a difference in what an unwritten register looks like and not in
    // what either computes, noticed here and not chased. It is a
    // reading, not a measurement -- nothing has established which of
    // the two is right, only that they differ.
    p.set("sel", word(4, 0b1000));
    p.set("en", bit(false));
    p.cycle("clk");
    p.agree("q", "priming");

    for sel in 0..16u64 {
        for en in [0u64, 1] {
            p.set("sel", word(4, sel));
            p.set("en", bit(en == 1));
            p.cycle("clk");
            p.agree("q", &format!("sel={sel:#06b} en={en}"));
        }
    }
}
