//! A `SLICEM`'s distributed RAM: one `RAM64X1D` on two of its lookup
//! tables.
//!
//! # What this declares
//!
//! One `lutram` bel per `SLICEM`, named `SLICEM_X0_RAM64X1D`, which is
//! the slice's **`D` lookup table holding the write port and its `C`
//! lookup table holding the read port**. Placing a cell there blocks the
//! two ordinary `lut` bels of the same letters, `SLICEM_X0_DLUT` and
//! `SLICEM_X0_CLUT`, because a lookup table's truth table and a RAM's
//! contents are the same `INIT` bits; `A`, `B`, the four flip-flops and
//! the whole `SLICEL` beside it in the same `CLBLM` stay usable.
//!
//! # Where each line came from, and what is checked versus quoted
//!
//! **Read from the database** (`artix7/segbits_clblm_{l,r}.db` and
//! `ppips_clblm_{l,r}.db`, and re-read by `tests/fpga_xray.rs`):
//!
//! - the mode is `<prefix>.DLUT.RAM` and `<prefix>.CLUT.RAM`, one bit
//!   each, per lookup table and not per slice;
//! - `SMALL` (32 words), `SRL`, `WA7USED`, `WA8USED`, `WEMUX.CE` and
//!   `CLKINV` stay **clear**: a 64-deep, non-shifting RAM whose write
//!   enable comes from the `WE` pin and whose clock is not inverted.
//!   Clear is what a zero-filled frame already says, so they cost nothing;
//! - `CLUT.DI1MUX` has two values and the one with **no bit set** is
//!   `DI_DMC31`, which is the `D` lookup table's data input — so the read
//!   port's copy is written from the same `DI` pin as the write port's,
//!   with no bit and no second wire;
//! - the pins' wires: `CLBLM_M_D1`..`D6` (fed from `IMUX40`, `45`, `38`,
//!   `44`, `47`, `43`), `CLBLM_M_C1`..`C6`, `CLBLM_M_DI` off `FAN3`,
//!   `CLBLM_M_WE` off `FAN4`, `CLBLM_M_CLK` off `CLK1`, and the read
//!   port's output `CLBLM_M_C`, which reaches `LOGIC_OUTS14`. All of them
//!   `always` lines, the same shape `sites.rs` already reads.
//!
//! **Quoted, not measured** — the part that would be silent if wrong:
//!
//! - **which lookup table is which port.** The write port is `D` and the
//!   read port is `C`. Two independent sources say so.
//!   *nextpnr-xilinx* `xilinx/pack_dram.cc` (`pack_dram`, the
//!   `RAM64X1D` group) puts the "write address input" cell at the
//!   topmost `z`, which `xilinx/fasm.cc`'s `write_luts_config` writes as
//!   `"ABCD"[3]` — the `D` lookup table — folds `SPO` into it, and puts
//!   `DPO` at the next `z` down, the `C`. *Project X-Ray's own fuzzer*
//!   `fuzzers/018-clb-ram/generate.py` tags a single `RAM64X1D` as
//!   occupying `(a, b, c, d) = (0, 0, 1, 1)` and says why: "D is always
//!   occupied first (due to WA/A sharing on D)". That fuzzer only keeps
//!   a sample when Vivado's own placement matched that tuple, so it is
//!   the nearest thing to a Vivado observation available here.
//! - **the address order.** `A<i>` reaches the `D` lookup table's input
//!   `<i + 1>` and `DPRA<i>` the `C` lookup table's input `<i + 1>`, with
//!   no permutation, and the `INIT` is copied unchanged into both lookup
//!   tables. That is nextpnr-xilinx's `dram_rules`: `RADR<i>` → `A<i+1>`,
//!   `WADR<i>` → `WA<i+1>`, and the cell's `INIT` assigned to every
//!   lookup table of the RAM, which `get_lut_init` writes with the
//!   identity physical-to-logical map. That the write address `WA1..6`
//!   *is* the `D` lookup table's `D1..6` is how Xilinx UG474 describes a
//!   `SLICEM`'s distributed RAM (paraphrased here, not measured), and it is
//!   the same sharing the fuzzer's comment above names.
//!
//! No Vivado bitstream with a distributed RAM is in the database
//! (`artix7/harness/` places switches and LEDs only), so none of the
//! quoted part has been compared with a vendor's output.
//! `examples/basys3/lutram.v` is the hardware test that would expose a
//! wrong permutation: a word written at a one-hot address and read back at
//! the same one-hot address shows on the LEDs only if the write and the
//! read port agree on which input is which address bit.

use std::collections::HashSet;

use super::{FeatureSet, sites};
use crate::fpga::arch::{BelDecl, ConfigEntry, WireRef};

/// The primitive a cell on the bel must be, as `xc7.dev` names it.
pub(super) const PRIMITIVE: &str = "RAM64X1D";

/// The lookup table holding the write port (and `SPO`).
pub(super) const WRITE_LUT: char = 'D';

/// The lookup table holding the read port (`DPO`).
pub(super) const READ_LUT: char = 'C';

/// The words a `RAM64X1D` holds, which is also the width of each lookup
/// table's `INIT`.
const WORDS: u32 = 64;

/// One distributed-RAM site, in feature and wire names, before the
/// database has been asked whether it has them.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LutRamSite {
    /// The bel's name: `SLICEM_X0_RAM64X1D`.
    pub name: String,
    /// Role and tile wire, in `xc7.dev`'s role names.
    pub pins: Vec<(String, String)>,
    /// The bels of the same tile a cell here makes unusable.
    pub blocks: Vec<String>,
    /// The features a `RAM64X1D` sets.
    pub mode: Vec<String>,
    /// Bit `n` of the cell's `INIT` and the features that carry it — one
    /// in each lookup table, since both hold a full copy of the contents.
    pub init: Vec<(u32, Vec<String>)>,
}

/// The distributed-RAM site of the `SLICEM` whose feature prefix is
/// `prefix`, in a tile of type `tile_type`; `None` for anything that is
/// not a `SLICEM`.
///
/// Only a `CLBLM` has one: `segbits_clbll_l.db`, whose two sites are both
/// `SLICEL`, has no `RAM` feature at all and `ppips_clbll_l.db` no `WE`
/// wire, which is the database's own statement that a `SLICEL` cannot be
/// a RAM. The `M` wires belong to the `SLICEM` (see
/// `sites::slice_wire_prefix`).
pub(super) fn ram64x1d(tile_type: &str, prefix: &str) -> Option<LutRamSite> {
    if !matches!(tile_type, "CLBLM_L" | "CLBLM_R") || !prefix.starts_with("SLICEM_") {
        return None;
    }
    let index = sites::prefix_index(prefix)?;
    let wires = sites::slice_wire_prefix(tile_type, index)?;
    if wires != "CLBLM_M" {
        return None;
    }
    let mut pins = Vec::new();
    for i in 0..6u32 {
        pins.push((format!("waddr{i}"), format!("{wires}_{WRITE_LUT}{}", i + 1)));
    }
    for i in 0..6u32 {
        pins.push((format!("raddr{i}"), format!("{wires}_{READ_LUT}{}", i + 1)));
    }
    pins.push(("din".to_owned(), format!("{wires}_DI")));
    pins.push(("we".to_owned(), format!("{wires}_WE")));
    pins.push(("wclk".to_owned(), format!("{wires}_CLK")));
    pins.push(("dout".to_owned(), format!("{wires}_{READ_LUT}")));

    let init = (0..WORDS)
        .map(|n| {
            (
                n,
                vec![
                    format!("{prefix}.{WRITE_LUT}LUT.INIT[{n:02}]"),
                    format!("{prefix}.{READ_LUT}LUT.INIT[{n:02}]"),
                ],
            )
        })
        .collect();
    Some(LutRamSite {
        name: format!("{prefix}_{PRIMITIVE}"),
        pins,
        blocks: vec![
            format!("{prefix}_{WRITE_LUT}LUT"),
            format!("{prefix}_{READ_LUT}LUT"),
        ],
        mode: vec![
            format!("{prefix}.{WRITE_LUT}LUT.RAM"),
            format!("{prefix}.{READ_LUT}LUT.RAM"),
        ],
        init,
    })
}

/// What [`bel`] could and could not resolve, for the load's coverage
/// report.
#[derive(Default)]
pub(super) struct Resolved {
    /// Pins whose wire the tile type declares.
    pub pins: usize,
    /// Pins whose wire it does not.
    pub pins_missing: usize,
}

/// The `lutram` bel for one `SLICEM`, or `None` when the database does
/// not describe all of it.
///
/// **All or nothing.** A RAM whose mode bit, any of whose 128 `INIT`
/// features, or any of whose 16 pin wires the tile type lacks is not
/// declared at all: a site the placer can fill and the bitstream cannot
/// configure would place, route, decode and hold nothing.
pub(super) fn bel(
    tile_type: &str,
    prefix: &str,
    features: &FeatureSet,
    wires: &HashSet<&str>,
    resolved: &mut Resolved,
) -> Option<BelDecl> {
    let site = ram64x1d(tile_type, prefix)?;
    let missing = site
        .pins
        .iter()
        .filter(|(_, wire)| !wires.contains(wire.as_str()))
        .count();
    resolved.pins += site.pins.len() - missing;
    resolved.pins_missing += missing;
    if missing > 0 {
        return None;
    }
    let mut mode = Vec::new();
    for name in &site.mode {
        mode.extend(features.feature(name)?.ones.iter().copied());
    }
    if mode.is_empty() {
        return None;
    }
    let mut config = vec![ConfigEntry::Cell {
        primitive: PRIMITIVE.to_owned(),
        bits: mode,
    }];
    for (index, names) in &site.init {
        for name in names {
            for at in &features.feature(name)?.ones {
                config.push(ConfigEntry::Param {
                    name: "INIT".to_owned(),
                    index: *index,
                    at: *at,
                });
            }
        }
    }
    let mut decl = BelDecl::new(site.name, "lutram");
    decl.pins = site
        .pins
        .into_iter()
        .map(|(role, wire)| (role, WireRef::local(wire)))
        .collect();
    decl.blocks = site.blocks;
    decl.config = config;
    Some(decl)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The permutation, pinned. This is the one table here that no
    /// database can check, so it is spelled out in full rather than
    /// computed from the same expression the code uses: `A0` is `D1`,
    /// `DPRA0` is `C1`, and so on up, and the output is the `C` lookup
    /// table's.
    ///
    /// It would catch a change to the order or to which lookup table is
    /// which port. It would **not** catch the order being wrong in the
    /// first place: it pins nextpnr-xilinx's reading, and only
    /// `examples/basys3/lutram.v` on a board can say the reading is right.
    #[test]
    fn the_write_port_is_the_d_lookup_table_and_the_read_port_the_c() {
        let site = ram64x1d("CLBLM_L", "SLICEM_X0").unwrap();
        let pins: Vec<(&str, &str)> = site
            .pins
            .iter()
            .map(|(r, w)| (r.as_str(), w.as_str()))
            .collect();
        assert_eq!(
            pins,
            vec![
                ("waddr0", "CLBLM_M_D1"),
                ("waddr1", "CLBLM_M_D2"),
                ("waddr2", "CLBLM_M_D3"),
                ("waddr3", "CLBLM_M_D4"),
                ("waddr4", "CLBLM_M_D5"),
                ("waddr5", "CLBLM_M_D6"),
                ("raddr0", "CLBLM_M_C1"),
                ("raddr1", "CLBLM_M_C2"),
                ("raddr2", "CLBLM_M_C3"),
                ("raddr3", "CLBLM_M_C4"),
                ("raddr4", "CLBLM_M_C5"),
                ("raddr5", "CLBLM_M_C6"),
                ("din", "CLBLM_M_DI"),
                ("we", "CLBLM_M_WE"),
                ("wclk", "CLBLM_M_CLK"),
                ("dout", "CLBLM_M_C"),
            ]
        );
        assert_eq!(site.blocks, vec!["SLICEM_X0_DLUT", "SLICEM_X0_CLUT"]);
        assert_eq!(site.mode, vec!["SLICEM_X0.DLUT.RAM", "SLICEM_X0.CLUT.RAM"]);
        // The contents, unpermuted, in both copies.
        assert_eq!(
            site.init[5],
            (
                5,
                vec![
                    "SLICEM_X0.DLUT.INIT[05]".to_owned(),
                    "SLICEM_X0.CLUT.INIT[05]".to_owned()
                ]
            )
        );
        assert_eq!(site.init.len(), 64);
    }

    /// Only a `CLBLM`'s `SLICEM` is offered: a `SLICEL`, either slice of a
    /// `CLBLL`, and a non-CLB tile get nothing.
    #[test]
    fn only_a_slicem_can_be_a_ram() {
        assert!(ram64x1d("CLBLM_R", "SLICEM_X0").is_some());
        assert!(ram64x1d("CLBLM_L", "SLICEL_X1").is_none());
        assert!(ram64x1d("CLBLL_L", "SLICEL_X0").is_none());
        assert!(ram64x1d("CLBLL_L", "SLICEM_X0").is_none());
        assert!(ram64x1d("LIOB33", "IOB_Y0").is_none());
    }
}
