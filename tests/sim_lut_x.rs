//! An unknown input does not poison a lookup table whose output does not
//! depend on it.
//!
//! `CellKind::Lut` used to answer `x` as soon as any address bit was `x`,
//! whether the table's function read that bit or not. `CellKind::Mux` never
//! did — it hands on the input its select chose — so the same logic was
//! decidable before technology mapping and undecidable after it, and that
//! asymmetry cost a real test its coverage:
//! `tests/ip_library.rs`'s `usb_descriptors_survive_lookup_table_mapping`
//! had to take `BUF_RAM = 0` on every case, because one unwritten byte of
//! `usb_bulk_ep`'s packet buffer reached the byte multiplexer
//! `usb_dev_core` shares between its endpoints and, once that multiplexer
//! was `lut` cells, silenced the whole device.
//!
//! The rule the simulator implements now is in `sim::sched::lut_output`: the
//! output is the value every input assignment consistent with the unknown
//! bits produces, and `x` only when two such assignments disagree. The unit
//! tests beside that function check the arithmetic against an enumeration of
//! the assignments, for every three-input function and every address over
//! `{0, 1, x, z}`. **This file checks the thing the unit tests cannot: that
//! the rule survives the mapper.** The designs here are written in Verilog,
//! synthesised and mapped by `synth::techmap`, and the lookup tables the
//! simulator evaluates are whatever the mapper decided to build — the same
//! path the USB blocks take.
//!
//! What these tests would catch: a lookup-table evaluation that is
//! pessimistic (the defect), and one that is *optimistic*, which would be
//! worse — `an_unknown_the_function_reads_still_reaches_the_output` and
//! `an_unknown_select_is_still_unknown` fail if the output is ever invented.
//! What they would not catch: anything about how the mapper *covers* the
//! logic, which `every_block_maps_to_the_logic_it_was_mapped_from` answers
//! for; anything about `x` through a `Dff`, a `Pmux`, a `Tristate` or a
//! memory read port, which are evaluated elsewhere and are still pessimistic
//! or worse (`docs/simulation.md` says which); and the arithmetic of a table
//! wider than the mapper here emits, which the unit tests cover instead.

#![cfg(all(feature = "sim", feature = "synth", feature = "verilog"))]

use std::time::Instant;

use reticle::diag::Diagnostics;
use reticle::ir::{CellKind, Design, ModuleId};
use reticle::logic::{Bit, Logic};
use reticle::sim::{SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::techmap::{MapOptions, map_module};
use reticle::synth::{SynthOptions, run as synth_run};
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

/// Elaborates one Verilog source, synthesises it and maps it onto `k`-input
/// lookup tables, leaking the design the way `tests/sim_cosim.rs` does so a
/// `Simulator` can borrow it for the rest of the test.
fn mapped(top: &str, text: &str, k: u32) -> &'static Design {
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
    let mut design = elaborate(&[&file], &options, &mut diags)
        .unwrap_or_else(|| panic!("{top} produced no design"));
    assert!(
        !diags.has_errors(),
        "{top} does not elaborate:\n{}",
        diags.render(&map)
    );
    let module = design.top.expect("an elaborated top");
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    assert!(
        !diags.has_errors(),
        "{top} does not synthesise:\n{}",
        diags.render(&map)
    );
    map_module(&mut design.modules[module], &MapOptions::lut(k));
    Box::leak(Box::new(design))
}

/// Counts the cells of each kind in a module, so a test cannot pass on a
/// netlist that was never mapped.
fn cell_census(design: &Design, module: ModuleId) -> (usize, usize) {
    let mut luts = 0;
    let mut muxes = 0;
    for cell in design.modules[module].cells.values() {
        match cell.kind {
            CellKind::Lut { .. } => luts += 1,
            CellKind::Mux | CellKind::Pmux => muxes += 1,
            _ => {}
        }
    }
    (luts, muxes)
}

/// A byte multiplexer over two sources and the parity of both of them: the
/// shape `usb_dev_core` has between its endpoints, plus one output that
/// genuinely reads every bit.
const BYTE_MUX: &str = r#"
module byte_mux(
    input            sel,
    input      [7:0] driven,
    input      [7:0] never_driven,
    output     [7:0] chosen,
    output           parity
);
    assign chosen = sel ? never_driven : driven;
    assign parity = ^{driven, never_driven};
endmodule
"#;

/// The same multiplexer with an unwritten memory on the far side, which is
/// where the `x` comes from in the design this was found on: `usb_bulk_ep`'s
/// packet buffer with `BUF_RAM = 1`.
const BUFFERED_MUX: &str = r#"
module buffered_mux(
    input            clk,
    input            sel,
    input      [3:0] addr,
    input      [7:0] driven,
    output     [7:0] chosen
);
    reg [7:0] store [0:15];
    always @(posedge clk) begin
        if (0) store[addr] <= driven;
    end
    assign chosen = sel ? store[addr] : driven;
endmodule
"#;

fn sim_of(design: &'static Design) -> Simulator<'static> {
    Simulator::new(design, SimOptions::default()).expect("the design elaborates for simulation")
}

fn handle(sim: &Simulator<'_>, name: &str) -> reticle::sim::NetHandle {
    let path = format!("{}.{}", sim.top_name(), name);
    sim.net(&path)
        .unwrap_or_else(|| panic!("no net `{path}` in the simulation"))
}

#[test]
fn a_mapped_multiplexer_passes_a_byte_an_unknown_cannot_reach() {
    for k in [4u32, 6] {
        let design = mapped("byte_mux", BYTE_MUX, k);
        let module = design.top.expect("a top");
        let (luts, muxes) = cell_census(design, module);
        assert!(luts > 0, "LUT{k}: nothing was mapped onto lookup tables");
        assert_eq!(muxes, 0, "LUT{k}: a multiplexer cell survived mapping");

        let mut sim = sim_of(design);
        let sel = handle(&sim, "sel");
        let driven = handle(&sim, "driven");
        let chosen = handle(&sim, "chosen");
        // `never_driven` is left exactly as elaboration found it: `x`.
        sim.set(sel, Logic::from_bool(false));
        sim.set(driven, Logic::from_u64(0xA5, 8));
        sim.run_for(0);
        assert_eq!(
            sim.get(chosen).to_u64(),
            Some(0xA5),
            "LUT{k}: an unknown on the unselected input poisoned the byte \
             the select chose, which is {}",
            sim.get(chosen).to_binary_string()
        );

        // And the other way round, so the test cannot pass by the mapper
        // having dropped `never_driven` from the cone.
        sim.set(sel, Logic::from_bool(true));
        sim.set(driven, Logic::from_u64(0x3C, 8));
        sim.run_for(0);
        assert!(
            sim.get(chosen).has_unknown(),
            "LUT{k}: the selected input is unknown and the output is not"
        );
    }
}

#[test]
fn an_unknown_the_function_reads_still_reaches_the_output() {
    // The parity of both bytes depends on every bit of the undriven one, so
    // no restriction of any table makes it known. This is the test that
    // fails if the lookup-table rule is optimistic rather than optimal.
    for k in [4u32, 6] {
        let design = mapped("byte_mux", BYTE_MUX, k);
        let mut sim = sim_of(design);
        let sel = handle(&sim, "sel");
        let driven = handle(&sim, "driven");
        let parity = handle(&sim, "parity");
        sim.set(sel, Logic::from_bool(false));
        sim.set(driven, Logic::from_u64(0xA5, 8));
        sim.run_for(0);
        assert_eq!(
            sim.get(parity).bit(0),
            Bit::X,
            "LUT{k}: a parity over unknown bits came out known"
        );
    }
}

#[test]
fn an_unknown_select_is_still_unknown() {
    // Both sides known and different, the select unknown: the output has to
    // be `x` in the bits they disagree on and known in the bits they agree
    // on, which is the IEEE 1364-2005 §5.1.13 merge and is what the rule
    // gives for a multiplexer's table.
    for k in [4u32, 6] {
        let design = mapped("byte_mux", BYTE_MUX, k);
        let mut sim = sim_of(design);
        let sel = handle(&sim, "sel");
        let driven = handle(&sim, "driven");
        let never = handle(&sim, "never_driven");
        let chosen = handle(&sim, "chosen");
        sim.set(sel, Logic::x(1));
        sim.set(driven, Logic::from_u64(0b1111_0000, 8));
        sim.set(never, Logic::from_u64(0b1100_1100, 8));
        sim.run_for(0);
        let got = sim.get(chosen);
        // Bits 2, 3, 4 and 5 disagree between the two sources.
        for i in 0..8 {
            let agree = (0b1111_0000u32 >> i) & 1 == (0b1100_1100u32 >> i) & 1;
            let want = if agree {
                Bit::from_bool((0b1111_0000u32 >> i) & 1 == 1)
            } else {
                Bit::X
            };
            assert_eq!(
                got.bit(i),
                want,
                "LUT{k}: bit {i} of a merge over an unknown select is {}",
                got.to_binary_string()
            );
        }
    }
}

#[test]
fn a_mapped_multiplexer_survives_an_unwritten_memory() {
    // The shape `usb_descriptors_survive_lookup_table_mapping` was narrowed
    // to avoid: a memory nothing has written reads as `x`, that `x` goes
    // into a multiplexer the mapper turned into lookup tables, and the byte
    // the select actually chose has to come out anyway.
    for k in [4u32, 6] {
        let design = mapped("buffered_mux", BUFFERED_MUX, k);
        let module = design.top.expect("a top");
        let (luts, _) = cell_census(design, module);
        assert!(luts > 0, "LUT{k}: nothing was mapped onto lookup tables");

        let mut sim = sim_of(design);
        let sel = handle(&sim, "sel");
        let addr = handle(&sim, "addr");
        let driven = handle(&sim, "driven");
        let chosen = handle(&sim, "chosen");
        sim.set(sel, Logic::from_bool(false));
        sim.set(addr, Logic::from_u64(7, 4));
        sim.set(driven, Logic::from_u64(0x5A, 8));
        sim.run_for(0);
        assert_eq!(
            sim.get(chosen).to_u64(),
            Some(0x5A),
            "LUT{k}: an unwritten buffer silenced the byte beside it, \
             which came out {}",
            sim.get(chosen).to_binary_string()
        );

        // Reading the buffer itself is still unknown, which is the honest
        // answer and the reason the `x` is there at all.
        sim.set(sel, Logic::from_bool(true));
        sim.run_for(0);
        assert!(
            sim.get(chosen).has_unknown(),
            "LUT{k}: an unwritten buffer read as something"
        );
    }
}

/// What the X-optimal path costs, measured against the path it did not
/// change, in one process — never asserted, because CI machines differ.
///
/// `cargo test --release --features sim,synth,verilog --test sim_lut_x --
/// --ignored --nocapture` prints it.
#[test]
#[ignore = "a measurement, not a check"]
fn lookup_table_evaluation_rate() {
    // An 8x8 multiplier mapped onto LUT4 is a few hundred lookup tables of
    // real depth, so a vector touches most of them.
    const MUL8: &str = r#"
module mul8(input [7:0] a, input [7:0] b, output [15:0] p);
    assign p = a * b;
endmodule
"#;
    let design = mapped("mul8", MUL8, 4);
    let module = design.top.expect("a top");
    let (luts, _) = cell_census(design, module);

    // Five repetitions and the fastest kept: a slow one measures the
    // machine, as `tests/sim_compiled.rs` says.
    let time = |unknown_bits: u32| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let mut sim = sim_of(design);
            let a = handle(&sim, "a");
            let b = handle(&sim, "b");
            let p = handle(&sim, "p");
            let vectors = 2000u64;
            let start = Instant::now();
            for i in 0..vectors {
                sim.set(a, Logic::from_u64(i * 7 + 1, 8));
                let mut bv = Logic::from_u64(i * 13 + 3, 8);
                for j in 0..unknown_bits {
                    bv.set_bit(j, Bit::X);
                }
                sim.set(b, bv);
                sim.run_for(0);
                std::hint::black_box(sim.get(p));
            }
            best = best.min(start.elapsed().as_secs_f64() / vectors as f64);
        }
        best
    };

    println!("mul8 mapped onto LUT4: {luts} lookup tables");
    for unknown in [0u32, 1, 4, 8] {
        let per = time(unknown);
        println!(
            "  {unknown} unknown bits of `b`: {:8.1} vectors/s ({:6.1} us/vector)",
            1.0 / per,
            per * 1e6
        );
    }
}
