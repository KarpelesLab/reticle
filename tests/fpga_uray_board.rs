//! Designs for the ZCU104, built by Reticle alone and written as
//! bitstreams the board loads through Linux's FPGA manager.
//!
//! Every test here is ignored because it writes files rather than checks
//! a property: set `RETICLE_ZCU104_OUT` to a directory and run it with
//! `--ignored`. Each design is held to the checks in
//! [`uray_board::Board::finish`] before it is written. What each one did
//! on the board is in `docs/fpga-uray.md`, with the Linux commands that
//! drove it.
//!
//! The designs talk to Linux over EMIO GPIO: output `k` is `gpio{594+k}`,
//! which Linux drives, and input `k` is the same number, which Linux
//! reads. A design never reads back on the line it drives, because Linux
//! reads an output line's own output register.

#![cfg(feature = "fpga")]

mod uray_board;

use std::path::PathBuf;

use reticle::fpga::uray::zcu104::clock_tile_types;
use uray_board::{Board, inputs, uraydb, wiringdb, write};

/// The output directory, or `None` having said why.
fn out_dir() -> Option<PathBuf> {
    match std::env::var("RETICLE_ZCU104_OUT") {
        Ok(dir) => Some(PathBuf::from(dir)),
        Err(_) => {
            eprintln!("skipped: set RETICLE_ZCU104_OUT to a directory to write into");
            None
        }
    }
}

/// EMIO output 0 routed straight to EMIO input 1, and nothing else.
///
/// The first design Reticle built for this part, and on 2026-10-09 the
/// first to run: input 1 follows output 0.
#[test]
#[ignore = "writes a bitstream for the board"]
fn emio_loopback() {
    let (Some(bits), Some(wiring), Some(out)) = (uraydb(), wiringdb(), out_dir()) else {
        return;
    };
    let inputs = inputs(&bits, &wiring);
    let mut board = Board::new(&inputs, None).unwrap();
    let route = board
        .connect(board.emio_out(0).unwrap(), board.emio_in(1).unwrap())
        .unwrap();
    eprintln!("{route:#?}");
    write(
        &board.finish("emio_loopback").unwrap(),
        &out,
        "emio_loopback",
    );
}

/// One flip-flop, every pin of it driven from Linux, to test what the
/// database's flip-flop features mean.
///
/// `AFF` of the slice beside interconnect `INT_X27Y206` takes its data
/// from the bypass `AX`. Its clock, clock enable and set/reset are the
/// slice's `CLK1`, `CKEN1` and `SRST1`, each routed from an EMIO output
/// through the interconnect, so Linux can clock it by hand. `AQ` goes to
/// EMIO input 1.
///
/// | EMIO out | pin |
/// |---|---|
/// | 0 | `AX` (data) |
/// | 2 | `CLK1` |
/// | 3 | `CKEN1` |
/// | 4 | `SRST1` |
///
/// The bits are the six Vivado sets on 350 `AFF`s of the base overlay
/// whose data comes from `AX`: the bypass select, clock enable and
/// set/reset in use, synchronous, initial value zero and reset value
/// zero. The two "in use" bits are reached by routing to the control
/// group's wires, as `uray::slice` models them; the rest are named. If
/// they mean what their names say, the flip-flop is an `FDRE` — and on
/// the ZCU104 on 2026-10-09 it was one, in all nine steps of the test.
#[test]
#[ignore = "writes a bitstream for the board"]
fn one_flip_flop() {
    let (Some(bits), Some(wiring), Some(out)) = (uraydb(), wiringdb(), out_dir()) else {
        return;
    };
    let inputs = inputs(&bits, &wiring);
    let mut board = Board::new(&inputs, None).unwrap();
    let slice = "CLEL_R_X27Y206";
    // The enable and reset reach the flip-flop through its control
    // group's wire, behind the pip that carries the group's "used" bit
    // (`uray::slice`): routing to the group wire turns that bit on.
    for (emio, pin) in [(0, "AX"), (2, "CLK1")] {
        let (from, to) = (
            board.emio_out(emio).unwrap(),
            board.slice_pin(slice, pin).unwrap(),
        );
        let route = board.connect(from, to).unwrap();
        eprintln!("EMIO {emio} -> {pin}: {route:?}");
    }
    for (emio, wire) in [(3, "RETICLE_SLICE_CE_ABCD1"), (4, "RETICLE_SLICE_SR_ABCD1")] {
        let (from, to) = (
            board.emio_out(emio).unwrap(),
            board.wire(slice, wire).unwrap(),
        );
        let route = board.connect(from, to).unwrap();
        eprintln!("EMIO {emio} -> {wire}: {route:?}");
    }
    let (from, to) = (
        board.slice_pin(slice, "AQ").unwrap(),
        board.emio_in(1).unwrap(),
    );
    let route = board.connect(from, to).unwrap();
    eprintln!("AQ -> EMIO in 1: {route:?}");
    for feature in [
        "SLICE_X0Y0.FFMUXA1.SP.BYP.OUT1",
        "SLICE_X0Y0.AFF.SYNC_ATTR=SYNC",
        "SLICE_X0Y0.AFF.FFINIT=INIT0",
        "SLICE_X0Y0.AFF.FFSR=SRLOW",
    ] {
        board.feature(slice, feature).unwrap();
    }
    write(
        &board.finish("one_flip_flop").unwrap(),
        &out,
        "one_flip_flop",
    );
}

/// Which leaf sites of the `RCLK_INT_L` tile `tile` Vivado fed from
/// horizontal distribution track `track` and switched on without a clock
/// enable, read from Vivado's bitstream.
fn leaves_on(inputs: &uray_board::Inputs, bitstream: &[u8], tile: &str, track: u32) -> Vec<u32> {
    use reticle::fpga::uray::{self, FrameLayout};
    let grid = &inputs.grid;
    let layout = FrameLayout::from_grid(grid);
    let bit = uray::read_bit(bitstream).unwrap();
    let decoded = uray::decode(grid, &layout, &bit.frames).unwrap();
    let index = grid.tiles().iter().position(|t| t.name == tile).unwrap();
    let set: std::collections::HashSet<_> = decoded.tiles[&index].iter().copied().collect();
    let kind = &grid.tiles()[index].kind;
    let on = |name: &str| {
        inputs.bits[kind]
            .features
            .iter()
            .find(|f| f.name == name)
            .is_some_and(|f| !f.ones.is_empty() && f.ones.iter().all(|b| set.contains(b)))
    };
    (0..32)
        .filter(|k| {
            on(&format!(
                "CLK_HDISTR_FT0_{track}->CLK_LEAF_SITES_{k}_CLK_IN"
            )) && on(&format!(
                "CLK_LEAF_SITES_{k}_CLK_IN->CLK_LEAF_SITES_{k}_CLK_LEAF"
            ))
        })
        .collect()
}

/// The slice tiles of the region nearest `near`, east of interconnect
/// column X27 (whose own slices tie the processor's inputs), skipping
/// `taken`.
fn slices_near(
    board: &Board<'_>,
    near: (u32, u32),
    count: usize,
    taken: &mut Vec<String>,
) -> Vec<String> {
    let grid = &board.db.grid;
    let int27 = grid
        .tiles()
        .iter()
        .find(|t| t.name == "INT_X27Y0")
        .unwrap()
        .grid
        .0;
    let mut candidates: Vec<(u32, String)> = grid
        .tiles()
        .iter()
        .filter(|t| matches!(t.kind.as_str(), "CLEL_R" | "CLEL_L" | "CLEM" | "CLEM_R"))
        .filter(|t| t.grid.0 > int27 + 1 && board.region.contains(t.grid.0, t.grid.1))
        .filter(|t| !taken.contains(&t.name))
        .map(|t| {
            (
                t.grid.0.abs_diff(near.0) + t.grid.1.abs_diff(near.1),
                t.name.clone(),
            )
        })
        .collect();
    candidates.sort();
    let chosen: Vec<String> = candidates.into_iter().take(count).map(|(_, n)| n).collect();
    taken.extend(chosen.iter().cloned());
    chosen
}

/// A ripple counter of `bits` stages, clocked by `clock`, its stages on
/// EMIO inputs `emio_base` upwards.
///
/// Each stage is flip-flop `AFF` of its own slice, so no two stages share
/// a clock pin. Its data comes from the bypass `AX`, which carries lookup
/// table `A` inverting `AQ` (contents `0x5555…`: the output is the
/// inverse of `A1` whatever the other inputs read). Each stage after the
/// first is clocked by the one before, so the count runs down. The flip-
/// flops carry no clock enable or set/reset feature: if that leaves the
/// enable on and the reset off, the counter counts.
fn ripple_counter(board: &mut Board<'_>, clock: u32, slices: &[String], emio_base: u32) {
    let mut clock = clock;
    for (i, slice) in slices.iter().enumerate() {
        let pin = |board: &Board<'_>, name: &str| board.slice_pin(slice, name).unwrap();
        let clk = pin(board, "CLK1");
        board.connect(clock, clk).unwrap();
        board.lut(slice, 'A', 0x5555_5555_5555_5555).unwrap();
        let (q, a1, o, ax) = (
            pin(board, "AQ"),
            pin(board, "A1"),
            pin(board, "A_O"),
            pin(board, "AX"),
        );
        board.connect(q, a1).unwrap();
        board.connect(o, ax).unwrap();
        for feature in [
            "SLICE_X0Y0.FFMUXA1.SP.BYP.OUT1",
            "SLICE_X0Y0.AFF.FFINIT=INIT0",
            "SLICE_X0Y0.AFF.FFSR=SRLOW",
        ] {
            board.feature(slice, feature).unwrap();
        }
        let k = emio_base + u32::try_from(i).unwrap();
        let emio = board.emio_in(k).unwrap();
        board.connect(q, emio).unwrap();
        clock = q;
    }
}

/// Three 32-bit ripple counters, each clocked by a leaf of the clock
/// network Vivado built for the base overlay, to find which leaves carry
/// the processor's 100 MHz `PL_CLK0`.
///
/// The clock-distribution tiles (`RCLK_*` and the processor's clock
/// buffer tiles) are replayed verbatim from Vivado's bitstream: the
/// database does not describe this die's processor clock buffers, so the
/// network is borrowed whole. The leaves are chosen from what Vivado
/// switched on beside the processor, on horizontal distribution tracks 14
/// (where Vivado's buffer for `PL_CLK0` sends it, by the one feature of
/// that buffer the database explains), 11 and 22. Linux reads each
/// counter as one 32-bit EMIO bank, twice, a measured interval apart.
#[test]
#[ignore = "writes a bitstream for the board"]
fn three_clock_counters() {
    let (Some(bits), Some(wiring), Some(out)) = (uraydb(), wiringdb(), out_dir()) else {
        return;
    };
    let Some(oracle) = uray_board::reference() else {
        return;
    };
    let inputs = inputs(&bits, &wiring);
    let mut board = Board::new(&inputs, None).unwrap();
    // The fabric's clock rows and the processor's clock buffers, leaves
    // included: these counters are clocked by leaves Vivado turned on.
    let types = clock_tile_types(&inputs.grid, true);
    let replayed = board.replay(&oracle, &types).unwrap();
    eprintln!(
        "replayed {replayed} bits of {} clock tile types",
        types.len()
    );

    let mut taken = Vec::new();
    for (counter, (tile, track)) in [
        ("RCLK_INT_L_X29Y89", 14u32),
        ("RCLK_INT_L_X29Y89", 11),
        ("RCLK_INT_L_X30Y89", 22),
    ]
    .into_iter()
    .enumerate()
    {
        let leaves = leaves_on(&inputs, &oracle, tile, track);
        eprintln!("{tile} track {track}: leaves {leaves:?}");
        let leaf = board
            .wire(tile, &format!("CLK_LEAF_SITES_{}_CLK_LEAF", leaves[0]))
            .unwrap();
        // The first stage sits beside an interconnect tile the leaf
        // reaches: a leaf serves one half of its clock region's column.
        let reach = board.node_tiles(leaf);
        let first = inputs
            .grid
            .tiles()
            .iter()
            .filter(|t| matches!(t.kind.as_str(), "CLEL_R" | "CLEL_L" | "CLEM" | "CLEM_R"))
            .filter(|t| !taken.contains(&t.name))
            .find(|t| {
                reach
                    .iter()
                    .any(|r| r.1 == t.grid.1 && r.0.abs_diff(t.grid.0) == 1)
            })
            .map(|t| t.name.clone())
            .expect("a free slice beside the leaf's reach");
        taken.push(first.clone());
        let near = inputs.grid.tiles()[board.tile(&first).unwrap()].grid;
        let mut slices = vec![first];
        slices.extend(slices_near(&board, near, 31, &mut taken));
        let base = 32 * u32::try_from(counter).unwrap();
        ripple_counter(&mut board, leaf, &slices, base);
        eprintln!("counter {counter}: {} .. {}", slices[0], slices[31]);
    }
    write(
        &board.finish("three_clock_counters").unwrap(),
        &out,
        "three_clock_counters",
    );
}

/// A 32-bit counter written in Verilog and taken through Reticle's whole
/// flow — elaboration, synthesis, technology mapping, placement, routing
/// and the bitstream — onto the processor's side of the fabric, clocked by
/// `PL_CLK0` and read by Linux on EMIO inputs 0..31.
///
/// The clock network is Vivado's, replayed, without its leaves: the
/// router reaches each flip-flop from the `PL_CLK0` global wire through a
/// leaf of track 14 it turns on itself, by the leaf's documented features.
/// If the count runs at 100 MHz, everything between Verilog and the part
/// works.
#[test]
#[ignore = "writes a bitstream for the board"]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn verilog_counter() {
    use reticle::diag::Diagnostics;
    use reticle::fpga::{Constraints, FpgaOptions, synthesize_for, target};
    use reticle::source::SourceMap;

    let (Some(bits), Some(wiring), Some(out)) = (uraydb(), wiringdb(), out_dir()) else {
        return;
    };
    let Some(oracle) = uray_board::reference() else {
        return;
    };
    let verilog = "module top(input wire clk, output reg [31:0] count);\n\
                   always @(posedge clk) count <= count + 32'd1;\n\
                   endmodule\n";
    let mut rcf = String::from("set_io clk PL_CLK0\n");
    for k in 0..32 {
        rcf.push_str(&format!("set_io count[{k}] EMIO_I{k}\n"));
    }

    let mut map = SourceMap::new();
    let source = map.add("counter.v", verilog).unwrap();
    let rcf_file = map.add("counter.rcf", &rcf).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::SystemVerilog,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let mut design = reticle::verilog::elaborate_file(
        &ast,
        &reticle::verilog::ElabOptions::default(),
        &mut diags,
    )
    .unwrap();
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let top = design.top.unwrap();
    let device = target("xczu7ev-ffvc1156").unwrap();
    let mut constraints = Constraints::parse(&rcf, rcf_file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap();
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let inputs = inputs(&bits, &wiring);
    let written = reticle::fpga::uray::zcu104::implement(
        &inputs,
        &design,
        top,
        device,
        &constraints,
        &reticle::fpga::uray::zcu104::ImplementOptions {
            oracle: &oracle,
            place: &reticle::fpga::PlaceOptions::default(),
            east: reticle::fpga::uray::zcu104::DEFAULT_EAST,
        },
    )
    .unwrap();
    write(&written, &out, "verilog_counter");
}
