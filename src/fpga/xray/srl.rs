//! A `SLICEM`'s shift registers: `SRLC32E`, `SRL16E` and `SRLC16E` on one of
//! its four lookup tables.
//!
//! # What this declares
//!
//! Two bels per lane of a `SLICEM`, `<prefix>_<L>SRL32` and `<prefix>_<L>SRL16`,
//! of kinds `srl32` and `srl16`. A cell on either takes the lane's lookup table
//! (they share `INIT`) and excludes the slice's `RAM64X1D`, because `WEMUX.CE`
//! is one bit for the slice and a RAM wants it clear. The flip-flops of the
//! slice stay usable, with one clock between them.
//!
//! # Where each line came from
//!
//! **Checked**: decoded from what Vivado 2026.1 wrote for the same cells, with
//! its site pips and `get_bel_pins` beside them
//! (`testdata/fpga/vivado/slice_cells/`, `tests/fpga_xray_slice_cells.rs`).
//!
//! - **The address is not the lookup table's inputs from 1.** `A[i]` is
//!   `<L>(i + 2)`. `<L>1` is not used and each stored bit is written twice, bit
//!   `k` at `INIT[2k]` and `INIT[2k + 1]`, so the address reads the same word
//!   whatever `<L>1` reads. `32'hF00FA5C3` came out `FF0000FFCC33F00F`.
//! - **A sixteen-bit form on the six-input table sets `SMALL`**, its contents
//!   are the upper half (`16'hA5C3` is `CC33F00F00000000`), and its data comes
//!   in on `DI2`, which is the slice's **`<L>X`**, not `<L>I`, with no `DI1MUX`
//!   bit. `<L>6` is unrouted and reads one, which is what selects that half.
//! - **The enable arrives on the slice's `CE` wire, with `WEMUX.CE` set and
//!   `CEUSEDMUX` clear.** `lutram.rs`'s `RAM64X1D` takes `WE` with `WEMUX` clear,
//!   so this is not what that precedent would have said. `WEMUX` reads the wire
//!   *ahead* of `CEUSEDMUX`: Vivado puts a shift register whose enable is routed
//!   beside a flip-flop with no enable, and that flip-flop is not enabled by it
//!   (`CEUSEDMUX:1` beside `WEMUX:CE`). So the pin here is `<shared>_FAN<n>`,
//!   behind the mux, and not `<wires>_CE`.
//! - **An enable tied high is not routed**: the wire reads one undriven, as every
//!   `FAN_ALT` does by default. That is a claim on the wire, which is what
//!   [`CONSTANT_ENABLE`] is for: a route through `FAN_ALT<n>` to a bounce wire
//!   would otherwise write that signal into this shift register's enable.
//! - **A zero on the address is `GND_WIRE` through a `GFAN`**, a one is left
//!   unrouted (`srl16e_fixed0`, `srlc32e_fixed31`); `bram.rs`'s ties do both.
//! - **`IS_CLK_INVERTED` is `CLKINV`**, as for a flip-flop, and **unlike a latch**.
//! - **`Q31` leaves a slice only from the `A` lane**, on the `D` lane's output
//!   mux: `DOUTMUX.MC31` and the `DMUX` wire, whichever lane it looks like it
//!   should be. Vivado said so by placing `SRLC32E` on `A6LUT` with its `Q31`
//!   net on the site pin `DMUX`. **Inside a slice it runs `D` to `C` to `B` to
//!   `A`**: `<L>LUT.DI1MUX` clear reads the next lane's `MC31`, with no bit of its
//!   own, and that is what `srlc32e_cascade4` is, `DLUT` the first stage.
//!   So a cell whose `Q31` is read by anything but the next stage of a chain has
//!   to be on `A` (`place::confine_to_pins`), and a chain is held to its lanes as a
//!   macro (`place::srl_cascades`).
//!
//! # A database entry that is wrong
//!
//! `segbits_clblm_*.db` says `DOUTMUX.MC31` is
//! `30_10 !30_51 30_52 !30_56 30_57`. **`30_10` is `A5FFMUX.IN_B`**, and every
//! bitstream Vivado wrote with that site pip here has `30_52` and `30_57` and
//! not `30_10`: six of the 79 designs, and those were the only slice bits in them
//! the database could not name. The fuzzer that found the feature must have
//! always had that flip-flop in use. [`correct_features`] takes the bit out.
//!
//! # What is not done
//!
//! - **The five-input half.** Vivado puts a sixteen-bit form it can register on
//!   `A5LUT`, with the lower half of `INIT`, `DI1` from `<L>I`, and its `Q` on
//!   `O5`, and it can hold two in one table. This flow always uses the six-input
//!   one, which is one of Vivado's own answers (`srl16e_1`, `srl16e_two`).
//!
//! **Nothing here has been on a board.**

use std::collections::HashSet;

use super::sites::{self, PassThrough};
use super::{ConfigBit, FeatureSet};
use crate::fpga::arch::{BelDecl, ConfigEntry, WireRef};

/// The lanes of a slice, in UG474's order.
const LANES: [char; 4] = ['A', 'B', 'C', 'D'];

/// The role of a shift register's enable, which a constant on it has to claim.
pub(super) const CONSTANT_ENABLE: &str = "ce";

/// The wire a lane's cascade output is on, invented as `sites` invents the
/// others: the database gives `MC31` no tile wire.
fn mc31_wire(wires: &str, letter: char) -> String {
    format!("{wires}_MUX_{letter}MC31")
}

/// The wire a lane's `DI1` is on: the output of `<L>LUT.DI1MUX` on `A`, `B` and
/// `C`, and the slice's `DI` on `D`, which has no such mux.
fn di1_wire(wires: &str, letter: char) -> String {
    if letter == 'D' {
        format!("{wires}_DI")
    } else {
        format!("{wires}_MUX_{letter}DI1")
    }
}

/// The hops a `SLICEM`'s shift registers add to its tile: `<L>I` into `DI1`,
/// the next lane's `MC31` into it with no bit at all, and the `A` lane's `MC31`
/// onto the `D` lane's output mux.
pub(super) fn pass_throughs(wires: &str, prefix: &str) -> Vec<PassThrough> {
    let mut out = vec![PassThrough {
        to: format!("{wires}_DMUX"),
        from: mc31_wire(wires, 'A'),
        features: vec![format!("{prefix}.DOUTMUX.MC31")],
    }];
    for (letter, next) in [('A', 'B'), ('B', 'C'), ('C', 'D')] {
        out.push(PassThrough {
            to: di1_wire(wires, letter),
            from: format!("{wires}_{letter}I"),
            features: vec![format!("{prefix}.{letter}LUT.DI1MUX.{letter}I")],
        });
        out.push(PassThrough {
            to: di1_wire(wires, letter),
            from: mc31_wire(wires, next),
            features: Vec::new(),
        });
    }
    out
}

/// The hops of [`pass_throughs`] for a tile type, if it has a `SLICEM`.
pub(super) fn tile_pass_throughs(tile_type: &str) -> Vec<PassThrough> {
    if matches!(tile_type, "CLBLM_L" | "CLBLM_R") {
        pass_throughs("CLBLM_M", "SLICEM_X0")
    } else {
        Vec::new()
    }
}

/// The two entries of a `segbits` file this flow knows better than, from what
/// Vivado wrote; see the module documentation.
pub(super) fn correct_features(tile_type: &str, set: FeatureSet) -> FeatureSet {
    if !matches!(tile_type, "CLBLM_L" | "CLBLM_R") {
        return set;
    }
    let mut all = set.into_features();
    for feature in &mut all {
        if feature.name.ends_with(".DOUTMUX.MC31") {
            feature.ones.retain(|bit| *bit != ConfigBit::new(30, 10));
        }
    }
    FeatureSet::new(all)
}

/// A bel's bits: its mode's and each stored bit's, or `None` when the database
/// has not got one of the features.
fn config(
    primitives: &[&str],
    mode: &[String],
    prefix: &str,
    letter: char,
    bits: u32,
    first: u32,
    features: &FeatureSet,
) -> Option<Vec<ConfigEntry>> {
    let mut mode_bits = Vec::new();
    for name in mode {
        mode_bits.extend(features.feature(name)?.ones.iter().copied());
    }
    let mut out: Vec<ConfigEntry> = primitives
        .iter()
        .map(|primitive| ConfigEntry::Cell {
            primitive: (*primitive).to_owned(),
            bits: mode_bits.clone(),
        })
        .collect();
    for k in 0..bits {
        for at in [first + 2 * k, first + 2 * k + 1] {
            let name = format!("{prefix}.{letter}LUT.INIT[{at:02}]");
            for bit in &features.feature(&name)?.ones {
                out.push(ConfigEntry::Param {
                    name: "INIT".to_owned(),
                    index: k,
                    at: *bit,
                });
            }
        }
    }
    for bit in &features.feature(&format!("{prefix}.CLKINV"))?.ones {
        out.push(ConfigEntry::Param {
            name: "IS_CLK_INVERTED".to_owned(),
            index: 0,
            at: *bit,
        });
    }
    Some(out)
}

/// The `srl32` and `srl16` bels of the `SLICEM` whose feature prefix is
/// `prefix`, all or nothing per bel: a site the placer can fill and the bitstream
/// cannot configure would place, route, decode and hold nothing.
pub(super) fn bels(
    tile_type: &str,
    prefix: &str,
    features: &FeatureSet,
    wires: &HashSet<&str>,
) -> Vec<BelDecl> {
    let mut out = Vec::new();
    if !matches!(tile_type, "CLBLM_L" | "CLBLM_R") || !prefix.starts_with("SLICEM_") {
        return out;
    }
    let Some(index) = sites::prefix_index(prefix) else {
        return out;
    };
    let Some(slice) = sites::slice_wire_prefix(tile_type, index) else {
        return out;
    };
    if slice != "CLBLM_M" {
        return out;
    }
    let enable = sites::slice_enable_source(tile_type, index);
    for letter in LANES {
        let ram = format!("{prefix}_RAM64X1D");
        let lut = format!("{prefix}_{letter}LUT");
        let shared = |extra: &str| {
            vec![
                lut.clone(),
                ram.clone(),
                format!("{prefix}_{letter}{extra}"),
            ]
        };
        let pins: Vec<(String, String)> = vec![
            ("clk".to_owned(), format!("{slice}_CLK")),
            ("ce".to_owned(), enable.clone()),
            ("q".to_owned(), format!("{slice}_{letter}")),
        ];
        let mut long = pins.clone();
        long.push(("q31".to_owned(), mc31_wire(slice, letter)));
        long.push(("d".to_owned(), di1_wire(slice, letter)));
        for i in 0..5u32 {
            long.push((format!("a{i}"), format!("{slice}_{letter}{}", i + 2)));
        }
        let mut short = pins;
        // The sixteen-bit form's data is not `DI1`, so it starts a chain or stands
        // alone, and the only `Q15` that goes anywhere is the `A` lane's.
        if letter == 'A' {
            short.push(("q31".to_owned(), mc31_wire(slice, letter)));
        }
        short.push(("d".to_owned(), format!("{slice}_{letter}X")));
        for i in 0..4u32 {
            short.push((format!("a{i}"), format!("{slice}_{letter}{}", i + 2)));
        }
        let srl = format!("{prefix}.{letter}LUT.SRL");
        let small = format!("{prefix}.{letter}LUT.SMALL");
        let we = format!("{prefix}.WEMUX.CE");
        let forms = [
            (
                "srl32",
                "SRL32",
                long,
                config(
                    &["SRLC32E"],
                    &[srl.clone(), we.clone()],
                    prefix,
                    letter,
                    32,
                    0,
                    features,
                ),
                shared("SRL16"),
            ),
            (
                "srl16",
                "SRL16",
                short,
                config(
                    &["SRL16E", "SRLC16E"],
                    &[srl.clone(), small, we.clone()],
                    prefix,
                    letter,
                    16,
                    32,
                    features,
                ),
                shared("SRL32"),
            ),
        ];
        for (kind, suffix, pins, config, blocks) in forms {
            let Some(config) = config else { continue };
            if pins.iter().any(|(_, wire)| !wires.contains(wire.as_str())) {
                continue;
            }
            let mut decl = BelDecl::new(format!("{prefix}_{letter}{suffix}"), kind);
            decl.pins = pins
                .into_iter()
                .map(|(role, wire)| (role, WireRef::local(wire)))
                .collect();
            decl.blocks = blocks;
            decl.config = config;
            out.push(decl);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wires a lane's cascade and data are on, spelled out rather than
    /// computed: `D` has no `DI1MUX`, and the others read `<L>I` through theirs.
    /// Would catch a lane's data on the wrong wire; would not catch the reading
    /// being wrong in the first place, which `tests/fpga_xray_slice_cells.rs`
    /// holds against Vivado.
    #[test]
    fn the_d_lane_has_no_data_mux() {
        assert_eq!(di1_wire("CLBLM_M", 'D'), "CLBLM_M_DI");
        assert_eq!(di1_wire("CLBLM_M", 'A'), "CLBLM_M_MUX_ADI1");
        let hops = pass_throughs("CLBLM_M", "SLICEM_X0");
        assert_eq!(hops.len(), 7);
        // The cascade is the absence of a bit.
        assert!(hops.iter().any(|h| h.to == "CLBLM_M_MUX_ADI1"
            && h.from == "CLBLM_M_MUX_BMC31"
            && h.features.is_empty()));
        assert_eq!(hops[0].to, "CLBLM_M_DMUX");
        assert_eq!(hops[0].from, "CLBLM_M_MUX_AMC31");
        assert_eq!(hops[0].features, ["SLICEM_X0.DOUTMUX.MC31"]);
    }

    /// `DOUTMUX.MC31` loses the `A5FFMUX.IN_B` bit the database gave it and keeps
    /// the two that are its own, and nothing else is touched. Would catch the
    /// correction being lost, which would have left a Vivado image's mux unnamed
    /// and made this flow set a five-input flip-flop's data mux with no reason.
    #[test]
    fn a_wrong_database_entry_is_corrected() {
        let bit = ConfigBit::new;
        let set = FeatureSet::new(vec![
            crate::fpga::xray::Feature {
                name: "SLICEM_X0.DOUTMUX.MC31".to_owned(),
                ones: vec![bit(30, 10), bit(30, 52), bit(30, 57)],
                zeros: vec![bit(30, 51), bit(30, 56)],
            },
            crate::fpga::xray::Feature {
                name: "SLICEM_X0.A5FFMUX.IN_B".to_owned(),
                ones: vec![bit(30, 10)],
                zeros: Vec::new(),
            },
        ]);
        let fixed = correct_features("CLBLM_L", set);
        assert_eq!(
            fixed.feature("SLICEM_X0.DOUTMUX.MC31").unwrap().ones,
            vec![bit(30, 52), bit(30, 57)]
        );
        assert_eq!(
            fixed.feature("SLICEM_X0.A5FFMUX.IN_B").unwrap().ones,
            vec![bit(30, 10)]
        );
    }
}
