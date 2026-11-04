//! A `SLICEM`'s distributed RAM: every shape Vivado puts on one, each a bel
//! of its own kind.
//!
//! # What this declares
//!
//! Per `SLICEM`, one bel per shape below, each blocking the lookup tables
//! (and the wide multiplexers) it takes and every other shape of the slice,
//! since all of them write through the `D` lookup table's inputs. A shape's
//! second place does not block its first: Vivado puts two `RAM32X1D` of one
//! address on the `C` and `D` tables' upper halves and lower halves, and pins
//! on the same wire make the placer give the two the same address.
//!
//! | Bel | Kind | Tables | Holds |
//! |---|---|---|---|
//! | `RAM64X1D` | `lutram` | `D` write, `C` read | `RAM64X1D` |
//! | `RAM32X1D`, `RAM32X1D_LO` | `lutram_d32` | the same, the upper halves or the lower | `RAM32X1D` |
//! | `RAM64X1S` | `lutram_s64` | `D` | `RAM64X1S` |
//! | `RAM32X1S` | `lutram_s32` | `D`'s upper half | `RAM32X1S` |
//! | `RAM64X2S` | `lutram_s64x2` | `D`, and `C` written at `D`'s inputs | `RAM64X2S` |
//! | `RAM32X2S` | `lutram_s32x2` | their upper halves | `RAM32X2S` |
//! | `RAM128X1S` | `lutram_s128` | `D` low, `C` high, `F7BMUX` | `RAM128X1S` |
//! | `RAM128X1D` | `lutram_d128` | `D`/`C` write, `B`/`A` read, both `F7` | `RAM128X1D` |
//! | `RAM256X1S` | `lutram_s256` | all four, both `F7`, `F8` | `RAM256X1S` |
//! | `RAM32M` | `lutram_m32` | all four, both halves | `RAM32M` |
//! | `RAM64M` | `lutram_m64` | all four | `RAM64M` |
//!
//! Each also holds its `_1`, which is the same cell on a falling edge: the
//! flow names a cell whose `IS_WCLK_INVERTED` is one so, since the slice's
//! flip-flops share its `CLKINV` (`place::slice_mode`).
//!
//! # Where each line came from: Vivado's answer
//!
//! `testdata/fpga/vivado/lutram/` has one design per cell, and per question
//! about inference, and `vivado.txt` there what Vivado 2026.1 made of each:
//! its placement report, each pin's bel pin and site pin, and the slice's
//! features decoded from its bitstream through this repository's database
//! (`tools/vivado-parity/lutram.sh`). Every line of the tables here is read off
//! those, and `tests/fpga_xray_lutram_vivado.rs` holds Reticle's slices to them.
//!
//! - **Write enable on `CE`, with `WEMUX.CE`**, in every one of Vivado's RAMs.
//!   The slice has a `WE` pin too (`CLBLM_M_WE`, fed from `FAN4`), and this
//!   flow used it, with `WEMUX.CE` clear. Asked to route that, Vivado said
//!   `Element WEMUX is not routable` (`docs/fpga-vivado-parity.md`), so the
//!   pin is Vivado's `CE` now. That changes what a slice's flip-flops may share
//!   it with: `CE` is theirs too, after `CEUSEDMUX`, so a flip-flop beside a
//!   RAM has the RAM's write enable or none (`sites::slice_pass_throughs`).
//! - **The write address is the `D` table's inputs** (`WA1`..`WA6` are
//!   `D1`..`D6`), `A<i>` is input `<i + 1>` of each table, and the contents are
//!   unpermuted: `RADR0` is `A1`, `WADR0` is `D1`, and a 64-word table's
//!   `INIT[n]` is the cell's `INIT[n]`, in every design. This was quoted from
//!   nextpnr-xilinx and Project X-Ray's fuzzer before; Vivado agrees.
//! - **A 32-word table is a `SMALL` table's half**: its upper half (`O6`) is
//!   written from `DI2`, the slice's `<L>X`, and holds `INIT[63:32]`; its lower
//!   (`O5`) from `DI1` and holds `INIT[31:0]`. Input 6 is left unrouted, which
//!   the interconnect reads as one (`ppips_int_l.db`'s default), as Vivado
//!   leaves it. A `RAM32M` port's bit 0 is its lower half, bit 1 its upper, and
//!   `INIT_A[2k]` and `INIT_A[2k + 1]` are word `k` of the two.
//! - **`DI1`** is `<L>I` with `<L>LUT.DI1MUX.<L>I`, or with the mux clear the
//!   next table up's (`D` has only `DI`): a shape whose tables all hold the
//!   same data leaves every mux clear, as a `RAM128X1D` does.
//! - **The wide ones** read their seventh address bit as `WA7` on `CX`
//!   (`WA7USED`) and their eighth as `WA8` on `BX` (`WA8USED`), the same wires
//!   as the multiplexers' selects: `F7BMUX` on `CX`, `F7AMUX` on `AX`, `F8MUX`
//!   on `BX`. `RAM128X1S` is `D` below `C`; `RAM128X1D`'s read port is `B`
//!   below `A`; `RAM256X1S` is `D`, `C`, `B`, `A` from the bottom.
//!
//! None of it has been on a part. That Vivado wrote these bits is the
//! evidence; whether a slice so configured remembers anything is not
//! measured. `examples/basys3/lutram.v` is the hardware test that would.
//!
//! # The model before
//!
//! [`LutRamModel::Legacy`] is the one `RAM64X1D` this module declared until
//! 2026-11-04, on `WE` and with `WEMUX.CE` clear, and nothing else: what a
//! build measured on a board with it keeps.

use std::collections::HashSet;

use super::{FeatureSet, sites};
use crate::fpga::arch::{BelDecl, ConfigEntry, WireRef};

/// Which distributed RAM a `SLICEM` offers; see the module docs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LutRamModel {
    /// Every shape Vivado uses, written as Vivado writes it.
    #[default]
    Vivado,
    /// One `RAM64X1D` per slice on its `WE` pin, as before 2026-11-04.
    Legacy,
}

/// The primitive the legacy model's one bel holds, as `xc7.dev` names it.
pub(super) const PRIMITIVE: &str = "RAM64X1D";

/// One distributed-RAM bel, in feature and wire names, before the database
/// has been asked whether it has them.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LutRamSite {
    /// The bel's name: `SLICEM_X0_RAM32M`.
    pub name: String,
    /// The kind of site, which `xc7.dev`'s `site` clause names.
    pub kind: &'static str,
    /// Role and tile wire, in `xc7.dev`'s role names. A role may name
    /// several wires, which the router reaches alike.
    pub pins: Vec<(String, String)>,
    /// The bels of the same tile a cell here makes unusable.
    pub blocks: Vec<String>,
    /// Each primitive the bel holds and the features a cell of it sets.
    pub cells: Vec<(String, Vec<String>)>,
    /// A parameter's bit and the features that carry it.
    pub init: Vec<(String, u32, Vec<String>)>,
}

/// Which half of a lookup table a 32-word shape uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Half {
    /// The whole table: 64 words.
    Whole,
    /// The upper half, `O6`, written from `DI2`.
    Upper,
    /// The lower half, `O5`, written from `DI1`.
    Lower,
}

/// The shapes, in declaration order: bel name tail, kind, the primitive, the
/// tables it takes (the first the one it writes through, for a single-port
/// shape), the half, and the wide multiplexers. A tail's `_LO` is a second
/// place for the same shape in the slice, the lower halves of the `D` and `C`
/// tables, which does not block the first.
///
/// The name of a lone shape's usual place sorts before its second's, since
/// the placer tries a tile's sites in that order, and Vivado puts a lone
/// `RAM32X1D` on the upper halves. That is a preference and no rule: the
/// placer may still choose the lower.
/// One entry of [`SHAPES`].
type Shape = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Half,
    &'static [&'static str],
);

const SHAPES: [Shape; 12] = [
    ("RAM64X1D", "lutram", "RAM64X1D", "DC", Half::Whole, &[]),
    ("RAM32X1D", "lutram_d32", "RAM32X1D", "DC", Half::Upper, &[]),
    (
        "RAM32X1D_LO",
        "lutram_d32",
        "RAM32X1D",
        "DC",
        Half::Lower,
        &[],
    ),
    ("RAM64X1S", "lutram_s64", "RAM64X1S", "D", Half::Whole, &[]),
    ("RAM32X1S", "lutram_s32", "RAM32X1S", "D", Half::Upper, &[]),
    (
        "RAM64X2S",
        "lutram_s64x2",
        "RAM64X2S",
        "DC",
        Half::Whole,
        &[],
    ),
    (
        "RAM32X2S",
        "lutram_s32x2",
        "RAM32X2S",
        "DC",
        Half::Upper,
        &[],
    ),
    (
        "RAM128X1S",
        "lutram_s128",
        "RAM128X1S",
        "DC",
        Half::Whole,
        &["F7B"],
    ),
    (
        "RAM128X1D",
        "lutram_d128",
        "RAM128X1D",
        "DCBA",
        Half::Whole,
        &["F7A", "F7B"],
    ),
    (
        "RAM256X1S",
        "lutram_s256",
        "RAM256X1S",
        "DCBA",
        Half::Whole,
        &["F7A", "F7B", "F8"],
    ),
    ("RAM32M", "lutram_m32", "RAM32M", "DCBA", Half::Upper, &[]),
    ("RAM64M", "lutram_m64", "RAM64M", "DCBA", Half::Whole, &[]),
];

/// The shape a bel tail is a place of: `RAM32X1D_LO` is a `RAM32X1D`'s.
fn shape_of(tail: &str) -> &str {
    tail.trim_end_matches("_LO")
}

/// The site kinds [`SHAPES`] declares, for the test that holds `xc7.dev` to
/// them.
#[cfg(test)]
pub(super) fn site_kinds() -> Vec<(&'static str, &'static str)> {
    let mut out: Vec<(&str, &str)> = SHAPES.iter().map(|s| (s.2, s.1)).collect();
    out.dedup();
    out
}

/// Every distributed-RAM bel of the `SLICEM` whose feature prefix is
/// `prefix`, in a tile of type `tile_type`; none for anything that is not a
/// `SLICEM`.
///
/// Only a `CLBLM` has one: `segbits_clbll_l.db`, whose two sites are both
/// `SLICEL`, has no `RAM` feature at all and `ppips_clbll_l.db` no `WE`
/// wire, which is the database's own statement that a `SLICEL` cannot be
/// a RAM. The `M` wires belong to the `SLICEM` (see
/// `sites::slice_wire_prefix`).
pub(super) fn lutram_sites(tile_type: &str, prefix: &str, model: LutRamModel) -> Vec<LutRamSite> {
    if !matches!(tile_type, "CLBLM_L" | "CLBLM_R") || !prefix.starts_with("SLICEM_") {
        return Vec::new();
    }
    let Some(index) = sites::prefix_index(prefix) else {
        return Vec::new();
    };
    let Some(w) = sites::slice_wire_prefix(tile_type, index) else {
        return Vec::new();
    };
    if w != "CLBLM_M" {
        return Vec::new();
    }
    if model == LutRamModel::Legacy {
        return vec![legacy(w, prefix)];
    }
    let names: Vec<String> = SHAPES.iter().map(|s| format!("{prefix}_{}", s.0)).collect();
    SHAPES
        .iter()
        .enumerate()
        .map(|(at, shape)| {
            let (tail, kind, primitive, luts, half, muxes) = *shape;
            let mut site = build(w, prefix, primitive, luts, half);
            site.name = names[at].clone();
            site.kind = kind;
            for lut in luts.chars() {
                site.blocks.push(format!("{prefix}_{lut}LUT"));
            }
            // And every shift register of the slice, when the fabric declares
            // them (`super::srl`, which blocks the legacy `RAM64X1D` so, by
            // name, and knows no other shape): a RAM's write enable and a
            // shift register's clock enable are the one `CE` of the slice,
            // and nothing here keeps the two on one signal.
            for lane in ['A', 'B', 'C', 'D'] {
                site.blocks.push(format!("{prefix}_{lane}SRL32"));
                site.blocks.push(format!("{prefix}_{lane}SRL16"));
            }
            for mux in muxes {
                site.blocks.push(format!("{prefix}_{mux}MUX"));
            }
            // Every other shape, but this one's other place.
            for (other, name) in SHAPES.iter().zip(&names) {
                if shape_of(other.0) != shape_of(tail) {
                    site.blocks.push(name.clone());
                }
            }
            site
        })
        .collect()
}

/// The one `RAM64X1D` of [`LutRamModel::Legacy`], exactly as it was.
fn legacy(w: &str, prefix: &str) -> LutRamSite {
    let mut pins = Vec::new();
    for i in 0..6u32 {
        pins.push((format!("waddr{i}"), format!("{w}_D{}", i + 1)));
    }
    for i in 0..6u32 {
        pins.push((format!("raddr{i}"), format!("{w}_C{}", i + 1)));
    }
    pins.push(("din".to_owned(), format!("{w}_DI")));
    pins.push(("we".to_owned(), format!("{w}_WE")));
    pins.push(("wclk".to_owned(), format!("{w}_CLK")));
    pins.push(("dout".to_owned(), format!("{w}_C")));
    let init = (0..64u32)
        .map(|n| {
            (
                "INIT".to_owned(),
                n,
                vec![
                    format!("{prefix}.DLUT.INIT[{n:02}]"),
                    format!("{prefix}.CLUT.INIT[{n:02}]"),
                ],
            )
        })
        .collect();
    LutRamSite {
        name: format!("{prefix}_{PRIMITIVE}"),
        kind: "lutram",
        pins,
        blocks: vec![format!("{prefix}_DLUT"), format!("{prefix}_CLUT")],
        cells: vec![(
            PRIMITIVE.to_owned(),
            vec![format!("{prefix}.DLUT.RAM"), format!("{prefix}.CLUT.RAM")],
        )],
        init,
    }
}

/// The wire of `<L>I`, a table's own `DI1`: the `D` table's is `DI`.
fn di1(w: &str, lut: char) -> String {
    format!("{w}_{lut}I")
}

/// One shape's pins, modes and contents; the caller names and blocks it.
fn build(w: &str, prefix: &str, primitive: &str, luts: &str, half: Half) -> LutRamSite {
    let mut pins: Vec<(String, String)> = Vec::new();
    let mut pin = |role: String, wire: String| pins.push((role, wire));
    let input = |lut: char, i: u32| format!("{w}_{lut}{}", i + 1);
    let out6 = |lut: char| format!("{w}_{lut}");
    let out5 = |lut: char| sites::lut5_wire(w, lut);
    let words = if half == Half::Whole { 6 } else { 5 };
    let mut mode: Vec<String> = vec![format!("{prefix}.WEMUX.CE")];
    for lut in luts.chars() {
        mode.push(format!("{prefix}.{lut}LUT.RAM"));
        if half != Half::Whole {
            mode.push(format!("{prefix}.{lut}LUT.SMALL"));
        }
    }
    let mut init: Vec<(String, u32, Vec<String>)> = Vec::new();
    let table = |lut: char, bit: u32| format!("{prefix}.{lut}LUT.INIT[{bit:02}]");
    // Where word `n` of a table-sized part sits in its table's `INIT`.
    let at = |n: u32| match half {
        Half::Whole | Half::Lower => n,
        Half::Upper => 32 + n,
    };
    match primitive {
        "RAM64X1D" | "RAM32X1D" => {
            for i in 0..words {
                pin(format!("waddr{i}"), input('D', i));
                pin(format!("raddr{i}"), input('C', i));
            }
            match half {
                Half::Upper => {
                    pin("din".to_owned(), format!("{w}_DX"));
                    pin("din".to_owned(), format!("{w}_CX"));
                }
                _ => pin("din".to_owned(), di1(w, 'D')),
            }
            let (spo, dout) = if half == Half::Lower {
                (out5('D'), out5('C'))
            } else {
                (out6('D'), out6('C'))
            };
            pin("dout".to_owned(), dout);
            pin("spo".to_owned(), spo);
            for n in 0..(1 << words) {
                init.push((
                    "INIT".to_owned(),
                    n,
                    vec![table('D', at(n)), table('C', at(n))],
                ));
            }
        }
        "RAM64X1S" | "RAM32X1S" => {
            for i in 0..words {
                pin(format!("waddr{i}"), input('D', i));
            }
            match half {
                Half::Upper => pin("din".to_owned(), format!("{w}_DX")),
                _ => pin("din".to_owned(), di1(w, 'D')),
            }
            pin("spo".to_owned(), out6('D'));
            for n in 0..(1 << words) {
                init.push(("INIT".to_owned(), n, vec![table('D', at(n))]));
            }
        }
        "RAM64X2S" | "RAM32X2S" => {
            // Two single-port RAMs of one address, `D` and `C`, `C` reading at
            // its own inputs and written at `D`'s, so both carry the address.
            for i in 0..words {
                pin(format!("waddr{i}"), input('D', i));
                pin(format!("waddr{i}"), input('C', i));
            }
            for (bit, lut) in ['D', 'C'].into_iter().enumerate() {
                match half {
                    Half::Upper => pin(format!("din{bit}"), format!("{w}_{lut}X")),
                    _ => pin(format!("din{bit}"), di1(w, lut)),
                }
                if half == Half::Whole && lut == 'C' {
                    mode.push(format!("{prefix}.CLUT.DI1MUX.CI"));
                }
                pin(format!("spo{bit}"), out6(lut));
                for n in 0..(1 << words) {
                    init.push((format!("INIT_0{bit}"), n, vec![table(lut, at(n))]));
                }
            }
        }
        "RAM128X1S" => {
            for i in 0..6 {
                pin(format!("waddr{i}"), input('D', i));
                pin(format!("waddr{i}"), input('C', i));
            }
            pin("waddr6".to_owned(), format!("{w}_CX"));
            pin("din".to_owned(), di1(w, 'D'));
            pin("spo".to_owned(), sites::wide_mux_wire(w, "F7B"));
            mode.push(format!("{prefix}.WA7USED"));
            for n in 0..128u32 {
                let lut = if n < 64 { 'D' } else { 'C' };
                init.push(("INIT".to_owned(), n, vec![table(lut, n % 64)]));
            }
        }
        "RAM128X1D" => {
            for i in 0..6 {
                pin(format!("waddr{i}"), input('D', i));
                pin(format!("waddr{i}"), input('C', i));
                pin(format!("raddr{i}"), input('B', i));
                pin(format!("raddr{i}"), input('A', i));
            }
            pin("waddr6".to_owned(), format!("{w}_CX"));
            pin("raddr6".to_owned(), format!("{w}_AX"));
            pin("din".to_owned(), di1(w, 'D'));
            pin("spo".to_owned(), sites::wide_mux_wire(w, "F7B"));
            pin("dout".to_owned(), sites::wide_mux_wire(w, "F7A"));
            mode.push(format!("{prefix}.WA7USED"));
            for n in 0..128u32 {
                let (write, read) = if n < 64 { ('D', 'B') } else { ('C', 'A') };
                init.push((
                    "INIT".to_owned(),
                    n,
                    vec![table(write, n % 64), table(read, n % 64)],
                ));
            }
        }
        "RAM256X1S" => {
            for i in 0..6 {
                for lut in ['D', 'C', 'B', 'A'] {
                    pin(format!("waddr{i}"), input(lut, i));
                }
            }
            pin("waddr6".to_owned(), format!("{w}_AX"));
            pin("waddr6".to_owned(), format!("{w}_CX"));
            pin("waddr7".to_owned(), format!("{w}_BX"));
            pin("din".to_owned(), di1(w, 'D'));
            pin("spo".to_owned(), sites::wide_mux_wire(w, "F8"));
            mode.push(format!("{prefix}.WA7USED"));
            mode.push(format!("{prefix}.WA8USED"));
            for n in 0..256u32 {
                let lut = ['D', 'C', 'B', 'A'][(n / 64) as usize];
                init.push(("INIT".to_owned(), n, vec![table(lut, n % 64)]));
            }
        }
        "RAM32M" | "RAM64M" => {
            let small = primitive == "RAM32M";
            for lut in luts.chars() {
                let port = lut.to_ascii_lowercase();
                for i in 0..words {
                    pin(format!("addr{port}{i}"), input(lut, i));
                }
                if lut != 'D' {
                    mode.push(format!("{prefix}.{lut}LUT.DI1MUX.{lut}I"));
                }
                let param = format!("INIT_{lut}");
                if small {
                    pin(format!("di{port}0"), di1(w, lut));
                    pin(format!("di{port}1"), format!("{w}_{lut}X"));
                    pin(format!("do{port}0"), out5(lut));
                    pin(format!("do{port}1"), out6(lut));
                    for k in 0..32u32 {
                        init.push((param.clone(), 2 * k, vec![table(lut, k)]));
                        init.push((param.clone(), 2 * k + 1, vec![table(lut, 32 + k)]));
                    }
                } else {
                    pin(format!("di{port}"), di1(w, lut));
                    pin(format!("do{port}"), out6(lut));
                    for n in 0..64u32 {
                        init.push((param.clone(), n, vec![table(lut, n)]));
                    }
                }
            }
        }
        _ => unreachable!("every primitive of SHAPES is built"),
    }
    pin("we".to_owned(), format!("{w}_CE"));
    pin("wclk".to_owned(), format!("{w}_CLK"));
    let mut inverted = mode.clone();
    inverted.push(format!("{prefix}.CLKINV"));
    LutRamSite {
        name: String::new(),
        kind: "",
        pins,
        blocks: Vec::new(),
        cells: vec![
            (primitive.to_owned(), mode),
            (format!("{primitive}_1"), inverted),
        ],
        init,
    }
}

/// What [`bels`] could and could not resolve, for the load's coverage
/// report.
#[derive(Default)]
pub(super) struct Resolved {
    /// Pins whose wire the tile type declares.
    pub pins: usize,
    /// Pins whose wire it does not.
    pub pins_missing: usize,
}

/// The distributed-RAM bels for one `SLICEM`, leaving out any the database
/// does not describe all of.
///
/// **All or nothing, per bel.** A RAM whose mode bit, any of whose `INIT`
/// features, or any of whose pin wires the tile type lacks is not declared
/// at all: a site the placer can fill and the bitstream cannot configure
/// would place, route, decode and hold nothing.
pub(super) fn bels(
    tile_type: &str,
    prefix: &str,
    features: &FeatureSet,
    wires: &HashSet<&str>,
    model: LutRamModel,
    resolved: &mut Resolved,
) -> Vec<BelDecl> {
    let mut out = Vec::new();
    for site in lutram_sites(tile_type, prefix, model) {
        let missing = site
            .pins
            .iter()
            .filter(|(_, wire)| !wires.contains(wire.as_str()))
            .count();
        resolved.pins += site.pins.len() - missing;
        resolved.pins_missing += missing;
        if missing > 0 {
            continue;
        }
        let Some(decl) = declare(site, features) else {
            continue;
        };
        out.push(decl);
    }
    out
}

/// A site's bel, or `None` when a feature it names is not in `features`.
fn declare(site: LutRamSite, features: &FeatureSet) -> Option<BelDecl> {
    let mut config = Vec::new();
    for (primitive, names) in &site.cells {
        let mut bits = Vec::new();
        for name in names {
            bits.extend(features.feature(name)?.ones.iter().copied());
        }
        if bits.is_empty() {
            return None;
        }
        config.push(ConfigEntry::Cell {
            primitive: primitive.clone(),
            bits,
        });
    }
    for (param, index, names) in &site.init {
        for name in names {
            for at in &features.feature(name)?.ones {
                config.push(ConfigEntry::Param {
                    name: param.clone(),
                    index: *index,
                    at: *at,
                });
            }
        }
    }
    let mut decl = BelDecl::new(site.name, site.kind);
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

    /// **`xc7.dev` puts each shape, and its `_1`, on the kind of site this
    /// module declares for it**, on two parts. Would catch a `site`
    /// clause missing or misspelt, which places a cell on a kind of site no
    /// bel has and fails the build, or on another shape's, which builds the
    /// wrong cell; would not catch a shape the device file does not name.
    #[test]
    fn the_device_file_puts_each_shape_on_its_own_kind_of_site() {
        let mut seen = 0;
        for part in ["xc7a35t-cpg236", "xc7a100t-ftg256"] {
            let device = crate::fpga::target(part).expect("the part");
            for (primitive, kind) in site_kinds() {
                for name in [primitive.to_owned(), format!("{primitive}_1")] {
                    if let Some(bel) = device.bels.iter().find(|b| b.name == name) {
                        assert_eq!(bel.site_kind(), kind, "{part} {name}");
                        seen += 1;
                    }
                }
            }
        }
        assert!(seen >= 38, "only {seen} bels");
    }

    fn site<'a>(all: &'a [LutRamSite], tail: &str) -> &'a LutRamSite {
        all.iter()
            .find(|s| s.name == format!("SLICEM_X0_{tail}"))
            .unwrap_or_else(|| panic!("no {tail}"))
    }

    fn wires_of<'a>(site: &'a LutRamSite, role: &str) -> Vec<&'a str> {
        site.pins
            .iter()
            .filter(|(r, _)| r == role)
            .map(|(_, w)| w.as_str())
            .collect()
    }

    /// The legacy model is the one `RAM64X1D` this module declared before,
    /// pin for pin, spelled out rather than computed. Would catch a change
    /// reaching a build that asked for the old one, which is the promise
    /// `--memories legacy` makes to a design measured on a board.
    #[test]
    fn the_legacy_model_is_the_ram64x1d_it_was() {
        let all = lutram_sites("CLBLM_L", "SLICEM_X0", LutRamModel::Legacy);
        assert_eq!(all.len(), 1);
        let pins: Vec<(&str, &str)> = all[0]
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
        assert_eq!(all[0].blocks, vec!["SLICEM_X0_DLUT", "SLICEM_X0_CLUT"]);
        assert_eq!(
            all[0].cells,
            vec![(
                "RAM64X1D".to_owned(),
                vec![
                    "SLICEM_X0.DLUT.RAM".to_owned(),
                    "SLICEM_X0.CLUT.RAM".to_owned()
                ]
            )]
        );
    }

    /// Vivado's pins, spelled out for the shapes whose wiring is least
    /// obvious, from `testdata/fpga/vivado/lutram/vivado.txt`'s `sitepin`
    /// lines. Would catch a data input on the wrong half's `DI`, the seventh
    /// address bit on the wrong bypass, an output off the wrong multiplexer,
    /// and the write enable back on `WE`. `tests/fpga_xray_lutram_vivado.rs`
    /// holds a whole build to the same answers.
    #[test]
    fn each_shape_is_wired_as_vivado_wired_it() {
        let all = lutram_sites("CLBLM_R", "SLICEM_X0", LutRamModel::Vivado);
        let m32 = site(&all, "RAM32M");
        assert_eq!(wires_of(m32, "dia0"), ["CLBLM_M_AI"]);
        assert_eq!(wires_of(m32, "dia1"), ["CLBLM_M_AX"]);
        assert_eq!(wires_of(m32, "did0"), ["CLBLM_M_DI"]);
        assert_eq!(wires_of(m32, "did1"), ["CLBLM_M_DX"]);
        assert_eq!(wires_of(m32, "doa0"), ["CLBLM_M_MUX_AO5"]);
        assert_eq!(wires_of(m32, "doa1"), ["CLBLM_M_A"]);
        assert_eq!(wires_of(m32, "addrd4"), ["CLBLM_M_D5"]);
        assert!(wires_of(m32, "addrd5").is_empty());
        assert_eq!(wires_of(m32, "we"), ["CLBLM_M_CE"]);
        let d32 = site(&all, "RAM32X1D");
        assert_eq!(wires_of(d32, "din"), ["CLBLM_M_DX", "CLBLM_M_CX"]);
        let d32_5 = site(&all, "RAM32X1D_LO");
        assert_eq!(wires_of(d32_5, "din"), ["CLBLM_M_DI"]);
        assert_eq!(wires_of(d32_5, "dout"), ["CLBLM_M_MUX_CO5"]);
        let d128 = site(&all, "RAM128X1D");
        assert_eq!(wires_of(d128, "waddr6"), ["CLBLM_M_CX"]);
        assert_eq!(wires_of(d128, "raddr6"), ["CLBLM_M_AX"]);
        assert_eq!(wires_of(d128, "raddr0"), ["CLBLM_M_B1", "CLBLM_M_A1"]);
        assert_eq!(wires_of(d128, "spo"), ["CLBLM_M_MUX_F7B"]);
        assert_eq!(wires_of(d128, "dout"), ["CLBLM_M_MUX_F7A"]);
        let s256 = site(&all, "RAM256X1S");
        assert_eq!(wires_of(s256, "waddr6"), ["CLBLM_M_AX", "CLBLM_M_CX"]);
        assert_eq!(wires_of(s256, "waddr7"), ["CLBLM_M_BX"]);
        assert_eq!(wires_of(s256, "spo"), ["CLBLM_M_MUX_F8"]);
    }

    /// Where a cell's contents go, at the edges of each part, spelled out
    /// from what Vivado's bitstreams hold for the same `INIT` (the module
    /// docs). Would catch a `RAM32M`'s two bits per word exchanged, a 32-word
    /// table in the wrong half, and a wide shape's quarters in the wrong order.
    #[test]
    fn contents_go_where_vivado_put_them() {
        let all = lutram_sites("CLBLM_L", "SLICEM_X0", LutRamModel::Vivado);
        let lookup = |tail: &str, param: &str, bit: u32| -> Vec<String> {
            site(&all, tail)
                .init
                .iter()
                .find(|(p, b, _)| p == param && *b == bit)
                .map(|(_, _, f)| f.clone())
                .unwrap_or_default()
        };
        assert_eq!(lookup("RAM32M", "INIT_A", 0), ["SLICEM_X0.ALUT.INIT[00]"]);
        assert_eq!(lookup("RAM32M", "INIT_A", 1), ["SLICEM_X0.ALUT.INIT[32]"]);
        assert_eq!(lookup("RAM32M", "INIT_C", 63), ["SLICEM_X0.CLUT.INIT[63]"]);
        assert_eq!(lookup("RAM32X1S", "INIT", 0), ["SLICEM_X0.DLUT.INIT[32]"]);
        assert_eq!(
            lookup("RAM32X2S", "INIT_01", 31),
            ["SLICEM_X0.CLUT.INIT[63]"]
        );
        assert_eq!(
            lookup("RAM32X1D_LO", "INIT", 31),
            ["SLICEM_X0.DLUT.INIT[31]", "SLICEM_X0.CLUT.INIT[31]"]
        );
        assert_eq!(lookup("RAM128X1S", "INIT", 64), ["SLICEM_X0.CLUT.INIT[00]"]);
        assert_eq!(
            lookup("RAM128X1D", "INIT", 127),
            ["SLICEM_X0.CLUT.INIT[63]", "SLICEM_X0.ALUT.INIT[63]"]
        );
        assert_eq!(
            lookup("RAM256X1S", "INIT", 128),
            ["SLICEM_X0.BLUT.INIT[00]"]
        );
        assert_eq!(
            lookup("RAM256X1S", "INIT", 255),
            ["SLICEM_X0.ALUT.INIT[63]"]
        );
        assert_eq!(lookup("RAM64M", "INIT_B", 5), ["SLICEM_X0.BLUT.INIT[05]"]);
    }

    /// Every shape blocks the others, and the two halves of one shape do
    /// not block each other. Would catch two shapes allowed into one slice,
    /// which would write one's contents into the other's tables.
    #[test]
    fn shapes_exclude_each_other_but_halves_do_not() {
        let all = lutram_sites("CLBLM_L", "SLICEM_X0", LutRamModel::Vivado);
        let o6 = site(&all, "RAM32X1D");
        assert!(!o6.blocks.contains(&"SLICEM_X0_RAM32X1D_LO".to_owned()));
        assert!(o6.blocks.contains(&"SLICEM_X0_RAM32M".to_owned()));
        assert!(o6.blocks.contains(&"SLICEM_X0_RAM32X1S".to_owned()));
        let two = site(&all, "RAM64X2S");
        assert_eq!(wires_of(two, "waddr0"), ["CLBLM_M_D1", "CLBLM_M_C1"]);
        assert_eq!(wires_of(two, "din1"), ["CLBLM_M_CI"]);
        assert_eq!(wires_of(site(&all, "RAM32X2S"), "din1"), ["CLBLM_M_CX"]);
        let m64 = site(&all, "RAM64M");
        for other in &all {
            if other.name != m64.name {
                assert!(m64.blocks.contains(&other.name), "{}", other.name);
            }
        }
        assert!(
            site(&all, "RAM256X1S")
                .blocks
                .contains(&"SLICEM_X0_F8MUX".to_owned())
        );
    }

    /// Only a `CLBLM`'s `SLICEM` is offered: a `SLICEL`, either slice of a
    /// `CLBLL`, and a non-CLB tile get nothing.
    #[test]
    fn only_a_slicem_can_be_a_ram() {
        for model in [LutRamModel::Vivado, LutRamModel::Legacy] {
            assert!(!lutram_sites("CLBLM_R", "SLICEM_X0", model).is_empty());
            assert!(lutram_sites("CLBLM_L", "SLICEL_X1", model).is_empty());
            assert!(lutram_sites("CLBLL_L", "SLICEL_X0", model).is_empty());
            assert!(lutram_sites("CLBLL_L", "SLICEM_X0", model).is_empty());
            assert!(lutram_sites("LIOB33", "IOB_Y0", model).is_empty());
        }
    }
}
