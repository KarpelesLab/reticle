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

use uray_board::{Board, inputs, uraydb, wiringdb};

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
    let mut board = Board::new(&inputs, &bits);
    let route = board.connect(board.emio_out(0), board.emio_in(1));
    eprintln!("{route:#?}");
    board.finish(&out, "emio_loopback");
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
/// The features are the six Vivado sets on 350 `AFF`s of the base overlay
/// whose data comes from `AX`: the bypass select, clock enable and
/// set/reset in use, synchronous, initial value zero and reset value
/// zero. If they mean what their names say, the flip-flop is an `FDRE`.
#[test]
#[ignore = "writes a bitstream for the board"]
fn one_flip_flop() {
    let (Some(bits), Some(wiring), Some(out)) = (uraydb(), wiringdb(), out_dir()) else {
        return;
    };
    let inputs = inputs(&bits, &wiring);
    let mut board = Board::new(&inputs, &bits);
    let slice = "CLEL_R_X27Y206";
    for (emio, pin) in [(0, "AX"), (2, "CLK1"), (3, "CKEN1"), (4, "SRST1")] {
        let route = board.connect(board.emio_out(emio), board.slice_pin(slice, pin));
        eprintln!("EMIO {emio} -> {pin}: {route:?}");
    }
    let route = board.connect(board.slice_pin(slice, "AQ"), board.emio_in(1));
    eprintln!("AQ -> EMIO in 1: {route:?}");
    for feature in [
        "SLICE_X0Y0.FFMUXA1.SP.BYP.OUT1",
        "SLICE_X0Y0.AFF.CE_ACTIVE=TRUE",
        "SLICE_X0Y0.AFF.SR_ACTIVE=TRUE",
        "SLICE_X0Y0.AFF.SYNC_ATTR=SYNC",
        "SLICE_X0Y0.AFF.FFINIT=INIT0",
        "SLICE_X0Y0.AFF.FFSR=SRLOW",
    ] {
        board.feature(slice, feature);
    }
    board.finish(&out, "one_flip_flop");
}
