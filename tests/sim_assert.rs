//! Concurrent assertions end to end: the Verilog bridge and overlapping
//! attempts on a real simulation.
//!
//! The unit tests next to the code cover every sequence operator in
//! isolation; this file checks the two seams a unit test cannot reach —
//! property text taken straight from the Verilog front end's `RawTokens`,
//! and many attempts of one property running at once over a clocked
//! design.

#![cfg(all(feature = "sim", feature = "verilog"))]

use reticle::diag::Diagnostics;
use reticle::ir::{Design, Polarity};
use reticle::sim::assertion::property::{
    DirectiveKind, PropertyExpr, Sequence, from_assertion, parse_directive,
};
use reticle::sim::{SimOptions, Simulator};
use reticle::source::{SourceMap, Span};
use reticle::verilog::ast::{Assertion, Item, ItemKind};
use reticle::verilog::{Dialect, NoIncludes, parse_source};

/// A clocked design in which `%a` is sampled high in cycles 0 and 1 and
/// `%b` only in cycle 2, so of the two overlapping attempts of
/// `a |-> ##2 b` the first one passes and the second one fails.
const PULSES: &str = "\
top pulses

module pulses
  timescale 1 ns / 1 ps
  net %clk u1 reg
  net %a u1 reg
  net %b u1 reg
  net %step u4 reg
  process clkgen free
    wait for 8'd5
    %clk = not(%clk)
  end
  process drive seq posedge %clk
    %step <= add(%step, 4'd1)
  end
  process stim initial
    %clk = 1'd0
    %a = 1'd0
    %b = 1'd0
    %step = 4'd0
    wait for 8'd2
    %a = 1'd1
    wait for 8'd20
    %a = 1'd0
    %b = 1'd1
    wait for 8'd10
    %b = 1'd0
    wait for 8'd30
    finish
  end
end
";

fn design(text: &str, name: &str) -> (SourceMap, Span, Design) {
    let mut map = SourceMap::new();
    let file = map.add(name, text).expect("source fits");
    let design = Design::parse_text(text, file).expect("the design parses");
    let span = Span::new(file, 0, 0);
    (map, span, design)
}

/// Every `assert`, `assume` and `cover property` item of a Verilog module,
/// in order.
fn assertions(items: &[Item]) -> Vec<&Assertion> {
    let mut out = Vec::new();
    for item in items {
        let ItemKind::Module(m) = &item.kind else {
            continue;
        };
        for entry in &m.items {
            if let ItemKind::Assertion(a) = &entry.kind {
                out.push(a);
            }
        }
    }
    out
}

#[test]
fn verilog_property_tokens_parse_into_directives() {
    let source = "\
module fifo(input logic clk, input logic rst, input logic full, input logic push);
  no_push: assert property (@(posedge clk) disable iff (rst) full |-> !push);
  pushes: cover property (@(posedge clk) push ##1 !push);
  assume property (@(negedge clk) full |=> full or !push);
  restrict property (@(posedge clk) full);
  always_comb assert (full != push);
endmodule
";
    let mut map = SourceMap::new();
    let id = map.add("fifo.sv", source).expect("source fits");
    let mut diags = Diagnostics::new();
    let file = parse_source(
        &mut map,
        id,
        Dialect::SystemVerilog,
        &mut NoIncludes,
        &mut diags,
    );
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let items = assertions(&file.items);
    assert_eq!(items.len(), 4, "four item-level assertions");

    // The concurrent ones convert; `restrict` does not.
    let first = from_assertion(items[0])
        .expect("concurrent")
        .expect("parses");
    assert_eq!(first.name.as_deref(), Some("no_push"));
    assert_eq!(first.kind, DirectiveKind::Assert);
    assert_eq!(first.clock.as_ref().expect("clock").polarity, Polarity::Pos);
    assert_eq!(first.clock.as_ref().expect("clock").net.name, "clk");
    assert!(first.disable.is_some(), "disable iff (rst)");
    assert!(matches!(first.property, PropertyExpr::Implies { .. }));
    // The span points inside the source the property was written in.
    assert_eq!(first.span.file, id);
    assert!(first.span.start < first.span.end);

    let cover = from_assertion(items[1])
        .expect("concurrent")
        .expect("parses");
    assert_eq!(cover.kind, DirectiveKind::Cover);
    assert!(matches!(
        cover.property,
        PropertyExpr::Seq(Sequence::Delay { .. })
    ));

    let assume = from_assertion(items[2])
        .expect("concurrent")
        .expect("parses");
    assert_eq!(assume.kind, DirectiveKind::Assume);
    assert_eq!(
        assume.clock.as_ref().expect("clock").polarity,
        Polarity::Neg
    );
    assert!(assume.name.is_none());

    assert!(from_assertion(items[3]).is_none(), "`restrict` is skipped");
}

#[test]
fn overlapping_attempts_over_a_clocked_design() {
    let (_map, span, d) = design(PULSES, "pulses.rtl");
    let mut sim = Simulator::new(&d, SimOptions::default()).expect("elaborates");
    // Two attempts have a matching antecedent, one cycle apart; the `b`
    // pulse only satisfies the first of them.
    let id = sim
        .add_assertion_text("late_b: assert property (@(posedge clk) a |-> ##2 b)", span)
        .expect("binds");
    let cover = sim
        .add_assertion_text("both: cover property (@(posedge clk) a ##1 a)", span)
        .expect("binds");
    sim.run();
    assert!(sim.finished());
    let results = sim.assertion_results();
    let late = &results[id.index()];
    assert_eq!(late.name, "late_b");
    // Clocking events at 5, 15, ... 55 ns: six cycles, `a` sampled high in
    // the first two and `b` in the third.
    assert_eq!(late.cycles, 6);
    assert_eq!(late.attempts, 6);
    assert_eq!(late.passes, 1, "the attempt of cycle 0 sees `b` at cycle 2");
    assert_eq!(late.failures, 1, "the attempt of cycle 1 does not");
    assert_eq!(late.vacuous, 4);
    assert_eq!(late.incomplete, 0);
    // The failure names the attempt it belongs to, which is what tracking
    // overlapping attempts is for: the attempt that started one cycle
    // later than the one that passed.
    let reported: Vec<&str> = sim
        .messages()
        .iter()
        .filter(|d| d.message.contains("late_b"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].contains("started at cycle 1"), "{reported:?}");
    assert!(reported[0].contains("failed at cycle 3"), "{reported:?}");
    // Every report carries the sampled values of the nets involved.
    let notes = sim
        .messages()
        .iter()
        .find(|d| d.message.contains("late_b"))
        .expect("a report")
        .notes
        .join(" ");
    assert!(notes.contains("pulses.a = "), "{notes}");
    assert!(notes.contains("pulses.b = "), "{notes}");
    // `a ##1 a` matches once: the attempt that starts at cycle 1.
    assert_eq!(results[cover.index()].passes, 1);
}

#[test]
fn binding_errors_are_diagnostics() {
    let (_map, span, d) = design(PULSES, "pulses.rtl");
    let mut sim = Simulator::new(&d, SimOptions::default()).expect("elaborates");
    let err = sim
        .add_assertion_text("assert property (a |-> b)", span)
        .expect_err("no clocking event");
    assert!(err.message.contains("no clocking event"), "{}", err.message);
    let err = sim
        .add_assertion_text("assert property (@(posedge clk) zz |-> b)", span)
        .expect_err("unknown net");
    assert!(err.message.contains("unknown net `zz`"), "{}", err.message);
    let err = sim
        .add_assertion_text("assert property (@(posedge nope) a |-> b)", span)
        .expect_err("unknown clock");
    assert!(err.message.contains("unknown clock net"), "{}", err.message);
    assert_eq!(sim.assertion_count(), 0);
    // A syntax error is a diagnostic with a span too.
    let err = parse_directive("a |-> ", span).expect_err("truncated");
    assert_eq!(err.primary_span().map(|s| s.file), Some(span.file));
}
