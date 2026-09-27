//! `keep` from the front ends through synthesis.
//!
//! The defect these tests exist for: `(* keep *) reg zero_probe = 1'b0;`
//! folded to `assign %z = 1'd0` and said nothing, because the attribute
//! lands on the net and the rule that deleted the flip-flop read the cell.
//! So every case here starts from source text rather than from a hand-built
//! module, and asks what is in the netlist afterwards.
//!
//! [`reticle::synth::keep`] holds the contract these pin.

#![cfg(all(feature = "synth", feature = "verilog"))]

use reticle::diag::{Diagnostics, Severity};
use reticle::ir::{CellKind, Design};
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run as synth};
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

/// Elaborates one Verilog source and synthesises it with the default
/// pipeline, returning the design and everything synthesis reported.
fn build(text: &str) -> (Design, Diagnostics, SourceMap) {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let id = map.add("t.v", text).expect("fits");
    let file = parse_source(
        &mut map,
        id,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    let mut design = elaborate(
        &[&file],
        &ElabOptions::new(Dialect::Verilog2005),
        &mut diags,
    )
    .expect("a design");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let mut synth_diags = Diagnostics::new();
    synth(&mut design, &SynthOptions::default(), &mut synth_diags);
    assert!(!synth_diags.has_errors(), "{}", synth_diags.render(&map));
    (design, synth_diags, map)
}

/// The flip-flops of the top module, as `(cell name, the net its `q`
/// drives)`.
fn flops(design: &Design) -> Vec<(String, String)> {
    let m = design.module(design.top.expect("a top"));
    let mut out: Vec<(String, String)> = m
        .cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
        .map(|(_, c)| {
            let q = c.output("q").expect("a q");
            (
                c.name.as_str().to_owned(),
                m.nets[q].name.as_str().to_owned(),
            )
        })
        .collect();
    out.sort();
    out
}

/// The nets of the top module, by name.
fn nets(design: &Design) -> Vec<String> {
    let m = design.module(design.top.expect("a top"));
    let mut out: Vec<String> = m
        .nets
        .iter()
        .map(|(_, n)| n.name.as_str().to_owned())
        .collect();
    out.sort();
    out
}

/// The `S0034` warnings, as their messages.
fn keep_warnings(diags: &Diagnostics) -> Vec<String> {
    diags
        .iter()
        .filter(|d| d.code == Some("S0034"))
        .map(|d| {
            assert_eq!(d.severity, Severity::Warning, "{}", d.message);
            d.message.clone()
        })
        .collect()
}

/// The measurement this was found by, in the smallest design that has it: a
/// register whose data input is the constant zero, whose value leaves the
/// module, and which has to still be a flip-flop when the netlist is
/// written — because what it measures is the value the *fabric* releases it
/// into, which no amount of IR reasoning can supply.
#[test]
fn a_kept_register_fed_a_constant_is_still_a_flip_flop() {
    let (design, diags, _) = build(
        "module m(input clk, input [7:0] d, output [7:0] y);
             (* keep *) reg zero_probe = 1'b0;
             always @(posedge clk) zero_probe <= 1'b0;
             assign y = d ^ {8{zero_probe}};
         endmodule",
    );
    assert_eq!(
        flops(&design),
        [("zero_probe$ff".to_owned(), "zero_probe".to_owned())]
    );
    assert!(
        keep_warnings(&diags).is_empty(),
        "{:?}",
        keep_warnings(&diags)
    );
}

/// The same register without the attribute folds away, so the test above is
/// measuring the attribute and not something that would have survived
/// anyway.
#[test]
fn the_same_register_without_the_attribute_folds_away() {
    let (design, _, _) = build(
        "module m(input clk, input [7:0] d, output [7:0] y);
             reg zero_probe = 1'b0;
             always @(posedge clk) zero_probe <= 1'b0;
             assign y = d ^ {8{zero_probe}};
         endmodule",
    );
    assert!(flops(&design).is_empty());
}

/// Every spelling the vendors invented, and a register nothing reads at
/// all: `dont_touch` and `mark_debug` were already exempt from the linter's
/// unused rules, so honouring them here is what makes the two agree.
#[test]
fn every_spelling_keeps_a_register_nothing_reads() {
    for spelling in [
        "keep",
        "KEEP",
        "dont_touch",
        "mark_debug",
        "preserve",
        "noprune",
        "syn_keep",
        "syn_preserve",
    ] {
        let (design, _, _) = build(&format!(
            "module m(input clk, input d);
                 (* {spelling} *) reg watched;
                 always @(posedge clk) watched <= d;
             endmodule"
        ));
        assert_eq!(
            flops(&design),
            [("watched$ff".to_owned(), "watched".to_owned())],
            "{spelling}"
        );
    }
}

/// A `keep` switched off keeps nothing; the register goes exactly as it
/// would without the attribute. The textual form is the one that matters,
/// since `keep = "false"` used to read as true.
#[test]
fn a_keep_that_is_off_keeps_nothing() {
    for off in [
        "(* keep = 0 *)",
        "(* keep = \"false\" *)",
        "(* keep = \"no\" *)",
    ] {
        let (design, _, _) = build(&format!(
            "module m(input clk, input d);
                 {off} reg watched;
                 always @(posedge clk) watched <= d;
             endmodule"
        ));
        assert!(flops(&design).is_empty(), "{off}");
    }
}

/// Two registers that are identical in every respect are normally merged
/// into one. Kept, they are two, because a probe that has been aliased to
/// another probe is not a probe.
#[test]
fn kept_registers_are_not_merged_with_each_other() {
    let (design, _, _) = build(
        "module m(input clk, input d, output y1, output y2);
             (* keep *) reg a;
             (* keep *) reg b;
             always @(posedge clk) begin a <= d; b <= d; end
             assign y1 = a;
             assign y2 = b;
         endmodule",
    );
    assert_eq!(
        flops(&design),
        [
            ("a$ff".to_owned(), "a".to_owned()),
            ("b$ff".to_owned(), "b".to_owned()),
        ]
    );
}

/// An attribute on the `always` block keeps the registers the block infers.
/// The process does not survive its own lowering, so the promise has to
/// travel onto the cells that replace it.
#[test]
fn a_keep_on_an_always_block_keeps_what_it_infers() {
    let (design, diags, _) = build(
        "module m(input clk, input [7:0] d, output [7:0] y);
             reg probe = 1'b0;
             (* keep *) always @(posedge clk) probe <= 1'b0;
             assign y = d ^ {8{probe}};
         endmodule",
    );
    assert_eq!(
        flops(&design),
        [("probe$ff".to_owned(), "probe".to_owned())]
    );
    assert!(keep_warnings(&diags).is_empty());
}

/// A kept wire is in the netlist with its own name, and nothing substitutes
/// its value into its readers — but its combinational driver may still fold
/// to a constant, because that leaves the net with the value it always had.
/// This is the documented limit of the promise, not an oversight.
#[test]
fn a_kept_wire_survives_and_its_gate_may_still_fold() {
    let (design, _, _) = build(
        "module m(input [7:0] d, output [7:0] y);
             (* keep *) wire [7:0] masked = d & 8'h00;
             assign y = masked;
         endmodule",
    );
    assert!(
        nets(&design).contains(&"masked".to_owned()),
        "{:?}",
        nets(&design)
    );
    let m = design.module(design.top.expect("a top"));
    let masked = m
        .nets
        .iter()
        .find(|(_, n)| n.name == "masked")
        .map(|(id, _)| id)
        .expect("the kept net");
    let text = m.to_text();
    assert!(
        m.assigns
            .iter()
            .any(|a| matches!(a.target, reticle::ir::Lvalue::Net(n) if n == masked)),
        "the kept net keeps a driver\n{text}"
    );
}

/// A `keep` on a continuous assignment keeps the assignment: it is not
/// removed, not merged with an identical one, and not propagated into its
/// readers.
#[test]
fn a_keep_on_an_assignment_keeps_it() {
    let (design, _, _) = build(
        "module m(input [7:0] a, input [7:0] b, output [7:0] y);
             (* keep *) wire [7:0] s1 = a + b;
             wire [7:0] s2 = a + b;
             assign y = s1 ^ s2;
         endmodule",
    );
    assert!(
        nets(&design).contains(&"s1".to_owned()),
        "{:?}",
        nets(&design)
    );
}

/// A keep in a place synthesis cannot honour is `S0034`, with a note that
/// says what to write instead. Four placements, four warnings, no silence.
#[test]
fn a_keep_that_cannot_be_honoured_is_a_warning() {
    let (_, diags, _) = build(
        "(* keep *) module m(input clk, input d, output q);
             (* keep_signal *) reg r;
             (* keep *) initial r = 1'b0;
             always @(posedge clk) r <= d;
             assign q = r;
             (* keep *) leaf u0 (.a(d));
         endmodule
         module leaf(input a);
         endmodule",
    );
    let warnings = keep_warnings(&diags);
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`keep` on module `m` keeps nothing")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`keep_signal` on `r` is not an attribute Reticle honours")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`keep` on an `initial` block keeps nothing")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("does not keep a hierarchy boundary")),
        "{warnings:?}"
    );
}

/// The VHDL front end puts an attribute specification on the same net a
/// Verilog attribute lands on, so the fix covers both languages at once.
#[cfg(feature = "vhdl")]
#[test]
fn a_vhdl_attribute_specification_keeps_the_register_too() {
    use reticle::ir::validate::validate;
    use reticle::vhdl::sema::Design as VhdlDesign;
    use reticle::vhdl::{ElabOptions as VhdlOptions, Standard, elaborate as vhdl_elaborate};

    let source = "
library ieee;
use ieee.std_logic_1164.all;

entity probe is
  port (clk : in std_logic; d : in std_logic; y : out std_logic);
end entity;

architecture rtl of probe is
  attribute keep : boolean;
  signal zero_probe : std_logic;
  attribute keep of zero_probe : signal is true;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      zero_probe <= '0';
    end if;
  end process;
  y <= d xor zero_probe;
end architecture;
";
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut vhdl = VhdlDesign::with_stdlib(&mut map, Standard::Vhdl2008, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let id = map.add("probe.vhd", source).expect("fits");
    vhdl.add_source(&map, id, "work", &mut diags);
    let analysis = vhdl.analyze(&map, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let mut design = vhdl_elaborate(&analysis, &VhdlOptions::new(), &mut diags).expect("a design");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let mut synth_diags = Diagnostics::new();
    synth(&mut design, &SynthOptions::default(), &mut synth_diags);
    assert!(validate(&design).is_empty());
    assert_eq!(
        flops(&design),
        [("zero_probe$ff".to_owned(), "zero_probe".to_owned())]
    );
    assert!(keep_warnings(&synth_diags).is_empty());
}
