//! Block RAM on a 7-series fabric: a `RAMB18E1` placed on one half of a
//! `BRAM_L` or `BRAM_R` tile, or a `RAMB36E1` on both, configured, and given
//! a value on every input the design leaves alone.
//!
//! # What reaches the frames, and from where
//!
//! | Piece | Where it comes from | Checked or quoted |
//! |---|---|---|
//! | which bit is `INIT_xx[n]` | `segbits_bram_l.block_ram.db`, prjxray fuzzer `026-bram-data` | quoted: the fuzzer gave Vivado random 256-bit `INIT_xx` values (`val & (1 << bit)`) and solved for the bits. The generic `ConfigEntry::Param` path carries it; no code here touches it. **Checked** since against Vivado's bitstreams of random contents, every bit, in `tests/fpga_xray_bram_vivado.rs`, and a `RAMB36E1`'s ([`ramb36_param_of`]) too |
//! | which half is `RAMB18_Y0` | prjxray `segmaker.py`'s `name_bram18` (even site `Y`) | quoted, and consistent with the contents bits' word positions; see `sites::block_ram_half` |
//! | the mode features ([`mode_features_in`], [`ramb36_features`]) | the legacy model: nextpnr-xilinx `fasm.cc`, `write_bram_half`, quoted. The Vivado model: Vivado's bitstreams of a cell named with each parameter | **checked**, `testdata/fpga/vivado/bram/`; nextpnr's order of `ZINIT` was not Vivado's, nor its `ZINV_REGCLK` |
//! | which tile wire is which site pin ([`bel_pins_in`], [`ramb36_pins`], [`write_enable_lanes`]) | the wire names of `ppips_bram_l.db` (`BRAM_FIFO18_ADDRARDADDR0`); for the legacy model nextpnr-xilinx `pack.cc`, quoted, and for the Vivado model Vivado's own report of the site pins each cell pin reaches | **checked** for the Vivado model, net by net |
//! | what an input nobody drives reads ([`TiePolicy`]) | `ppips_int_l.db`'s `default` lines: every `IMUX`, `BYP_ALT` and `FAN_ALT` reads `VCC_WIRE` unless a pip drives it | the database's statement; how it behaves on silicon is unverified. Vivado relies on it: an input it gives `<const1>` has no pip |
//!
//! Nothing here has run on a part.
//!
//! # Why every input gets a value
//!
//! An interconnect input nothing drives is **not** zero on this family: the
//! database says each `IMUX` defaults to `VCC_WIRE`. A `RAMB18E1` whose
//! design connects only a read port would therefore come up with `WEA`
//! high and `DIADI` all ones on its read port, and with `ENBWREN` and
//! `WEBWE` high on the port nobody connected — writing `0xFFFF` into the
//! memory on every clock edge. Mapping cannot see that, routing cannot see
//! it, and every bit of the result would decode. So every input the
//! design does not drive is tied to the value [`TiePolicy`] gives it,
//! after routing, in the tiles around the block: a zero is
//! `GND_WIRE -> GFAN<n> -> <the input's IMUX or CTRL>`, two pips in the
//! interconnect tile the input enters from, which is how prjxray's
//! database says a zero is made (`INT_L.GFAN0.GND_WIRE`); a one is the
//! input left alone, which is only accepted once the walk back from the
//! pin has reached a wire the database says defaults to `VCC_WIRE` and
//! nothing on the way is driven.
//!
//! # An address bit the width mode does not read cannot fail a build
//!
//! It used to, and that was a defect of a different shape from the one
//! above. A `RAMB18E1` in 18-bit mode takes its word address on
//! `ADDRARDADDR[13:4]` and `src/fpga/devices/xc7.dev` pads the four bits
//! below with zeros, so four address pins per port arrived here as
//! **netlist constants** — and this pass went looking for a ground path
//! for each of them.
//!
//! There is exactly one such path and it is contended. `segbits_int_l.db`
//! has two `GND_WIRE` pips in the whole tile, `INT_L.GFAN0.GND_WIRE` and
//! `INT_L.GFAN1.GND_WIRE`; every one of the 48 `IMUX_L<n>` has a pip from
//! one of those two fans and from exactly one of them, so a tile offers
//! **two** ground sources for 48 inputs — and `GFAN0` and `GFAN1` are
//! ordinary routing wires the router is free to spend on a signal. So
//! whether the hunt found a path depended on what the router had done
//! around that block, which depended on where the placer had put it.
//! `examples/basys3/ssd1306_console.v` did not build (a record of "nine
//! placement settings" was one placement: the 7-series writer ignored
//! `--place-*` until `3c91260`); `examples/basys3/selftest.v` built, then an
//! unrelated edit moved the placement and it did not, then another moved it
//! back and it did — and it builds today, which is what the same lottery
//! looks like from the lucky side. **A
//! flow that is meant to be deterministic should not succeed or fail at
//! tying an address bit to ground depending on where a block landed**, and
//! that non-determinism was the defect, more than any one failure.
//!
//! **The remedy is not a wider search and it is not a driver either.** The
//! search is not too narrow — the path exists for every input, and what
//! fails is contention — and a driver would be a lookup table and a route
//! for a bit the block does not read. The fix is to stop treating such a
//! bit as something that can fail: [`unused_address_bit`] works out from
//! the cell's own width parameters which address bits the mode leaves
//! unread, and gives them [`TiePolicy::Idle`], so they are tied to zero
//! when a fan is free and left at the interconnect's default when none is.
//!
//! A family that *does* read those bits is the opposite case and is not
//! handled here: an ECP5 `DP16KD` in 18-bit mode needs `AD[3:0] = 0011`,
//! `src/fpga/devices/ecp5.dev` says so with `pad 4'b0011`, and
//! [`techcells::drive_constant_data`](crate::fpga::techcells) gives those
//! bits a real driver before placement. The rule there is the device
//! file's statement and not a guess about what a block ignores.
//!
//! What is left for the hunt is the case neither covers: a pin the design
//! **does not connect at all**, which has no net to route and still needs
//! a value. Those are far fewer — one two-block design ties 27 and leaves
//! 5 idle — and when it fails for one of them it now says what is
//! contended and what to do about it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;

use super::XrayFabric;
use super::lutram::LutRamModel;
use super::parse::{Ppip, PpipKind};
use crate::fpga::arch::{Arch, ConfigBit, NodeId, PipId, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, param_bit};
use crate::fpga::place::{Netlist, Placement};
use crate::fpga::route::Routing;
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// Whether this module configures a cell of `primitive`: a `RAMB18E1` on
/// one half of a tile, or a `RAMB36E1` on both, which only
/// [`LutRamModel::Vivado`] has a bel for ([`ramb36_pins`]).
fn is_block_ram(primitive: &str) -> bool {
    matches!(primitive, "RAMB18E1" | "RAMB36E1")
}

/// The wire prefix of one half of a block RAM tile: `RAMB18_Y0` is the
/// lower, `FIFO18E1`-typed site and its wires are `BRAM_FIFO18_*`;
/// `RAMB18_Y1` is the upper and its wires are `BRAM_RAMB18_*`. The
/// pairing is by the site *type* `tilegrid.json` gives each half
/// (`RAMB18_X0Y0` is `FIFO18E1`, `RAMB18_X0Y1` is `RAMB18E1`).
fn wire_prefix(tile_type: &str, prefix: &str) -> Option<&'static str> {
    if tile_type != "BRAM_L" && tile_type != "BRAM_R" {
        return None;
    }
    match prefix {
        "RAMB18_Y0" => Some("BRAM_FIFO18"),
        "RAMB18_Y1" => Some("BRAM_RAMB18"),
        _ => None,
    }
}

/// Whether a feature prefix is a site even though every feature under it
/// has two components.
///
/// The loader's rule for telling a site from a wire is that a site heads a
/// feature of three components or more (`SLICEL_X0.AFF.ZINI`). A block RAM
/// half breaks it: `segbits_bram_l.db` and its `.block_ram.db` spell every
/// `RAMB18_Y0` feature in two (`RAMB18_Y0.IN_USE`, `RAMB18_Y0.INIT_00[000]`),
/// so before this the two halves were read as wires, their 37 000
/// features as pips into nothing, and a `BRAM_L` tile got no bel at all —
/// which is the precise reason "a design with a `RAMB18E1` will not
/// route" in `docs/fpga-xray.md`. Named rather than inferred, because a
/// looser rule would misread interconnect wires.
///
/// `RAMB36` is the same shape and is deliberately not here: its seven
/// features configure the two halves as one 36 kbit block and are read by
/// name ([`ramb36_features`]), and the Vivado model's `RAMB36E1` bel is built
/// apart (`parse::bels_of`). Making it a site would change every legacy
/// fabric's graph, which a legacy build must not see; its features are left
/// out of the arcs instead (`NOT_ARCS`).
///
/// The XADC is here too, though not a block RAM: its features are this
/// flow's own (`xadc::derived_features`, `XADC.INIT_41[13]`), named in the
/// same two-part spelling so that the generic reader turns each register
/// bit into a parameter bit of the bel.
pub(super) fn is_two_part_site(head: &str) -> bool {
    matches!(head, "RAMB18_Y0" | "RAMB18_Y1" | "XADC")
}

/// The two ports of a `RAMB18E1` as `xc7.dev` declares them, in its
/// order: port 0 is `A` (`CLKARDCLK`, `ADDRARDADDR`, ...) and port 1 is
/// `B`. The netlist names a pin `p<port>_<role><bit>`, so these names are
/// tied to that file's `port` lines; `tests/fpga_xray_bram.rs` checks
/// every role this produces is one the device's netlist can ask for.
struct Side {
    clk: &'static str,
    en: &'static str,
    rst: &'static str,
    addr: &'static str,
    din: &'static str,
    dout: &'static str,
    dinp: &'static str,
    doutp: &'static str,
    regce: &'static str,
    rstreg: &'static str,
    addr_tie: &'static str,
}

const SIDES: [Side; 2] = [
    Side {
        clk: "CLKARDCLK",
        en: "ENARDEN",
        rst: "RSTRAMARSTRAM",
        addr: "ADDRARDADDR",
        din: "DIADI",
        dout: "DOADO",
        dinp: "DIPADIP",
        doutp: "DOPADOP",
        regce: "REGCEAREGCE",
        rstreg: "RSTREGARSTREG",
        addr_tie: "ADDRATIEHIGH",
    },
    Side {
        clk: "CLKBWRCLK",
        en: "ENBWREN",
        rst: "RSTRAMB",
        addr: "ADDRBWRADDR",
        din: "DIBDI",
        dout: "DOBDO",
        dinp: "DIPBDIP",
        doutp: "DOPBDOP",
        regce: "REGCEB",
        rstreg: "RSTREGB",
        addr_tie: "ADDRBTIEHIGH",
    },
];

/// Roles that no cell port maps to and that exist only so the input has
/// a node to tie: they start with this, which no `xc7.dev` role does.
const TIE_ONLY: &str = "tie_";

/// The pins of the `RAMB18E1` bel `prefix` in a tile of type `tile_type`,
/// as `(role, tile wire)`; empty for anything that is not a block RAM
/// half.
///
/// # The site pins
///
/// A tile wire `BRAM_FIFO18_<PIN>` is the site pin `<PIN>` of the lower
/// half, which is the convention every site-pin wire of the database
/// follows (`CLBLL_L_A1` is a slice's `A1`). The port-to-pin map is
/// nextpnr-xilinx's (`pack.cc`, `XC7Packer::pack_bram`): a port bit
/// `ADDRARDADDR[i]` is the site pin `ADDRARDADDR<i>`, and the two-bit
/// `WEA` is four site pins, `WEA[0]` on `WEA0` and `WEA1` and `WEA[1]` on
/// `WEA2` and `WEA3`. `WEBWE[3:0]` reach `WEBWE0..3`; the upper four are
/// tie-only, tied low as nextpnr ties them outside its 36-bit mode.
///
/// Note that `ADDRARDADDR0` of the lower half is fed from the tile wire
/// `BRAM_ADDRARDADDRL1`, not `...L0` — the tile wires count the 36 kbit
/// block's address bits, of which the half's bit 0 is the block's bit 1.
/// That is `ppips_bram_l.db`'s statement and nothing here depends on it:
/// the pin is named, the wiring behind it is the database's.
#[cfg(test)]
pub(super) fn bel_pins(tile_type: &str, prefix: &str) -> Vec<(String, String)> {
    bel_pins_in(tile_type, prefix, LutRamModel::Legacy)
}

/// [`bel_pins`] for a fabric whose block RAMs are `model`'s.
///
/// [`LutRamModel::Vivado`] names every write-enable site pin on its own,
/// `p0_we0` to `p0_we3` for `WEA0` to `WEA3` and `p1_we0` to `p1_we7` for
/// `WEBWE0` to `WEBWE7`, because which cell bit reaches which of them
/// depends on the cell's widths: [`write_enable_lanes`] says how, as Vivado
/// wired them, and [`XrayFabric::spread_block_ram_pins`] makes the
/// netlist say the same. It also names the output register's clock enable
/// and reset as the cell's own pins, `p0_regce` and `p0_rstreg`, which a
/// cell named with `DOA_REG` drives; the legacy names are tie-only.
pub(super) fn bel_pins_in(
    tile_type: &str,
    prefix: &str,
    model: LutRamModel,
) -> Vec<(String, String)> {
    let Some(wires) = wire_prefix(tile_type, prefix) else {
        return Vec::new();
    };
    let own = if model == LutRamModel::Vivado {
        ""
    } else {
        TIE_ONLY
    };
    let mut out = Vec::new();
    let mut pin = |role: String, site_pin: String| out.push((role, format!("{wires}_{site_pin}")));
    for (port, side) in SIDES.iter().enumerate() {
        let p = format!("p{port}_");
        pin(format!("{p}clk"), side.clk.to_owned());
        pin(format!("{p}en"), side.en.to_owned());
        pin(format!("{p}rst"), side.rst.to_owned());
        for i in 0..14 {
            pin(format!("{p}addr{i}"), format!("{}{i}", side.addr));
        }
        for i in 0..16 {
            pin(format!("{p}din{i}"), format!("{}{i}", side.din));
            pin(format!("{p}dout{i}"), format!("{}{i}", side.dout));
        }
        for i in 0..2 {
            pin(format!("{p}dinp{i}"), format!("{}{i}", side.dinp));
            pin(format!("{p}doutp{i}"), format!("{}{i}", side.doutp));
            pin(
                format!("{TIE_ONLY}{p}addrtie{i}"),
                format!("{}{i}", side.addr_tie),
            );
        }
        pin(format!("{own}{p}regce"), side.regce.to_owned());
        pin(format!("{own}{p}rstreg"), side.rstreg.to_owned());
    }
    if model == LutRamModel::Vivado {
        // The output registers' clocks, which a cell has no port for:
        // Vivado routes the port's clock to them with `DO<x>_REG` and a one
        // without (`b18_doreg`'s `CLKARDCLK` reaches `RDCLK` and `RDRCLK`).
        // See [`XrayFabric::spread_block_ram_pins`].
        pin("p0_regclk".to_owned(), "REGCLKARDRCLK".to_owned());
        pin("p1_regclk".to_owned(), "REGCLKB".to_owned());
        for i in 0..4 {
            pin(format!("p0_we{i}"), format!("WEA{i}"));
        }
        for i in 0..8 {
            pin(format!("p1_we{i}"), format!("WEBWE{i}"));
        }
        return out;
    }
    pin("p0_we0".to_owned(), "WEA0".to_owned());
    pin("p0_we0".to_owned(), "WEA1".to_owned());
    pin("p0_we1".to_owned(), "WEA2".to_owned());
    pin("p0_we1".to_owned(), "WEA3".to_owned());
    for i in 0..4 {
        pin(format!("p1_we{i}"), format!("WEBWE{i}"));
        pin(
            format!("{TIE_ONLY}p1_we{}", i + 4),
            format!("WEBWE{}", i + 4),
        );
    }
    out
}

/// The output roles: everything else a block RAM bel has is an input.
fn is_output(role: &str) -> bool {
    role.contains("_dout")
        || role.ends_with("_dbiterr")
        || role.ends_with("_sbiterr")
        || role.contains("_eccparity")
        || role.contains("_rdaddrecc")
}

/// The pins of the `RAMB36E1` bel of a `BRAM_L` or `BRAM_R` tile, as
/// `(role, tile wire)`, or `None` for another tile type.
///
/// **Vivado's**, read off its own cells (`testdata/fpga/vivado/bram/`'s
/// `b36_*`, `get_site_pins -of` each cell pin) and the site pins' wires off
/// the tile type's site description, which names each `BRAM_FIFO36_<pin>`:
///
/// - A word's even bits are the lower half's and its odd bits the upper's:
///   `DIADI[2k]` is `DIADIL<k>` and `DIADI[2k+1]` is `DIADIU<k>`, and so are
///   `DOADO`, the parity bits (`DIPADIP[2k]` is `DIPADIPL<k>`) and port B's.
/// - An address bit reaches both halves, `ADDRARDADDR[i]` on
///   `ADDRARDADDRL<i>` and `ADDRARDADDRU<i>`, except bit 15, which only the
///   lower half has and which only a cascade reads: Vivado ties it high.
/// - A clock, an enable, a reset and a clock enable reach both halves
///   (`CLKARDCLKL`, `CLKARDCLKU`), and so does each output register's
///   clock (`REGCLKARDRCLKL`, `REGCLKARDRCLKU`), which the cell has no port
///   for: [`XrayFabric::spread_block_ram_pins`].
/// - Each half has its own write enables, `p0_we0` to `p0_we3` the lower
///   half's `WEAL0` to `WEAL3` and `p0_we4` to `p0_we7` the upper's, and
///   `p1_we0` to `p1_we15` likewise over `WEBWEL0` to `WEBWEU7`;
///   [`write_enable_lanes`] says which cell bit each carries.
///
/// The tile wires behind them are the halves' own: `BRAM_FIFO36_DIADIL0`
/// and `BRAM_FIFO18_DIADI0` both hang off `BRAM_IMUX16_1`, in the
/// database's `tile_type_BRAM_L.json`. The cascade and the error-correction
/// pins are not here: nothing here builds either.
pub(super) fn ramb36_pins(tile_type: &str) -> Option<Vec<(String, String)>> {
    if tile_type != "BRAM_L" && tile_type != "BRAM_R" {
        return None;
    }
    let mut out = Vec::new();
    let mut pin = |role: String, site_pin: String| {
        out.push((role, format!("BRAM_FIFO36_{site_pin}")));
    };
    let both = |name: &str| [format!("{name}L"), format!("{name}U")];
    for (port, side) in SIDES.iter().enumerate() {
        let p = format!("p{port}_");
        for (role, name) in [
            ("clk", side.clk),
            ("en", side.en),
            ("regce", side.regce),
            ("rstreg", side.rstreg),
        ] {
            for site_pin in both(name) {
                pin(format!("{p}{role}"), site_pin);
            }
        }
        // `RSTRAMARSTRAM`'s lower pin is spelled `...LRST`.
        let (low, high) = match port {
            0 => ("RSTRAMARSTRAMLRST", "RSTRAMARSTRAMU"),
            _ => ("RSTRAMBL", "RSTRAMBU"),
        };
        let regclk = if port == 0 {
            "REGCLKARDRCLK"
        } else {
            "REGCLKB"
        };
        for site_pin in both(regclk) {
            pin(format!("{p}regclk"), site_pin);
        }
        pin(format!("{p}rst"), low.to_owned());
        pin(format!("{p}rst"), high.to_owned());
        for i in 0..16 {
            pin(format!("{p}addr{i}"), format!("{}L{i}", side.addr));
            if i < 15 {
                pin(format!("{p}addr{i}"), format!("{}U{i}", side.addr));
            }
        }
        for j in 0..32 {
            let half = if j % 2 == 0 { 'L' } else { 'U' };
            pin(format!("{p}din{j}"), format!("{}{half}{}", side.din, j / 2));
            pin(
                format!("{p}dout{j}"),
                format!("{}{half}{}", side.dout, j / 2),
            );
        }
        for j in 0..4 {
            let half = if j % 2 == 0 { 'L' } else { 'U' };
            pin(
                format!("{p}dinp{j}"),
                format!("{}{half}{}", side.dinp, j / 2),
            );
            pin(
                format!("{p}doutp{j}"),
                format!("{}{half}{}", side.doutp, j / 2),
            );
        }
    }
    for i in 0..8 {
        let half = if i < 4 { 'L' } else { 'U' };
        pin(format!("p0_we{i}"), format!("WEA{half}{}", i % 4));
    }
    for i in 0..16 {
        let half = if i < 8 { 'L' } else { 'U' };
        pin(format!("p1_we{i}"), format!("WEBWE{half}{}", i % 8));
    }
    // The error correction's pins, on the site's own (Vivado's
    // `ramb36e1_probe`: `RDADDRECC[i]` on `RDCOUNT<i>`). Nothing here turns
    // the correction on, and these are pins a design may connect anyway.
    for name in ["DBITERR", "SBITERR", "INJECTDBITERR", "INJECTSBITERR"] {
        pin(format!("p0_{}", name.to_lowercase()), name.to_owned());
    }
    for i in 0..8 {
        pin(format!("p0_eccparity{i}"), format!("ECCPARITY{i}"));
    }
    for i in 0..9 {
        pin(format!("p0_rdaddrecc{i}"), format!("RDCOUNT{i}"));
    }
    Some(out)
}

/// Which bit of a block RAM cell's write enable each of its site's write
/// enable pins carries, in the order of the Vivado model's roles
/// (`p0_we0`, `p0_we1`, ...): `Some(k)` for the cell's bit `k`, `None` for
/// a pin held low. `port` is 0 for `WEA` and 1 for `WEBWE`.
///
/// **Vivado's**, read off its cells in `testdata/fpga/vivado/bram/`
/// (`get_site_pins -of` each `WEA[k]`), for every width it was given:
///
/// | cell, port | width written | the site's pins |
/// |---|---|---|
/// | `RAMB18E1`, either, `TDP` | 18 | `WE*0`, `WE*1` bit 0; `WE*2`, `WE*3` bit 1 |
/// | `RAMB18E1`, either, `TDP` | 9 or less | all four bit 0 |
/// | `RAMB18E1`, B | any but `SDP` 36 | `WEBWE4` to `WEBWE7` low |
/// | `RAMB18E1`, B, `SDP` | 36 | `WEBWE<2k>`, `WEBWE<2k+1>` bit `k` |
/// | `RAMB18E1`, A, `SDP` | — | all low |
/// | `RAMB36E1`, either | 36 | `WE*L<k>`, `WE*U<k>` bit `k` |
/// | `RAMB36E1`, either | 18 | `L0`, `L2`, `U0`, `U2` bit 0; the odd ones bit 1 |
/// | `RAMB36E1`, either | 9 or less | all eight bit 0 |
/// | `RAMB36E1`, B | any but `SDP` 72 | `WEBWEL4` to `L7`, `U4` to `U7` low |
/// | `RAMB36E1`, B, `SDP` | 72 | `WEBWEL<k>`, `WEBWEU<k>` bit `k` |
///
/// The pins held low are the ones `docs/fpga-vivado-parity.md` found
/// Vivado grounding (`GNDTerm`) where Reticle had routed a signal through
/// them: in `TDP` nothing reads `WEBWE[7:4]`, on UG473's word, **quoted**,
/// and Vivado grounds them anyway.
pub fn write_enable_lanes(
    primitive: &str,
    port: usize,
    params: &dyn Fn(&str) -> Option<AttrValue>,
) -> Vec<Option<u32>> {
    let sdp = matches!(params("RAM_MODE"), Some(AttrValue::String(m)) if m == "SDP");
    let suffix = if port == 0 { 'A' } else { 'B' };
    let width = params(&format!("WRITE_WIDTH_{suffix}"))
        .and_then(|v| v.as_int())
        .unwrap_or(0);
    let wide36 = primitive == "RAMB36E1";
    let pins = match (wide36, port) {
        (false, 0) => 4,
        (false, _) => 8,
        (true, 0) => 8,
        (true, _) => 16,
    };
    if sdp && port == 0 {
        return vec![None; pins];
    }
    let mut out = vec![None; pins];
    if wide36 {
        // The lower half's lanes, then the upper's alike.
        let half = pins / 2;
        let (lower, upper) = out.split_at_mut(half);
        let bytes = match width {
            36 => 4,
            18 => 2,
            _ => 1,
        };
        let used = if sdp && width == 72 { half } else { 4 };
        for (i, (low, high)) in lower
            .iter_mut()
            .zip(upper.iter_mut())
            .take(used)
            .enumerate()
        {
            let bit = if sdp && width == 72 { i } else { i % bytes };
            *low = Some(u32::try_from(bit).unwrap_or(0));
            *high = *low;
        }
        return out;
    }
    if sdp && width == 36 {
        for (i, lane) in out.iter_mut().enumerate() {
            *lane = Some(u32::try_from(i / 2).unwrap_or(0));
        }
        return out;
    }
    let bytes = if width == 18 { 2 } else { 1 };
    for (i, lane) in out.iter_mut().take(4).enumerate() {
        *lane = Some(u32::try_from(i * bytes / 4).unwrap_or(0));
    }
    out
}

/// What an input the netlist leaves unconnected is made to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TiePolicy {
    /// Tied to zero: a write enable, a reset, an enable (a port nobody
    /// connected is a port that is off), or an address bit.
    Zero,
    /// An input nothing reads: tied to zero when a ground path is free
    /// after the [`TiePolicy::Zero`] ties have had theirs, and otherwise
    /// left to whatever the fabric gives it. A data or parity input (its
    /// value is stored only under a write enable, and an unconnected write
    /// enable is tied to zero), and every input but the enable of a port
    /// whose enable is tied low.
    ///
    /// The first two-block design this was tried on found both reasons it
    /// is needed: a data input whose interconnect tile's two ground fans
    /// the router had already used, and the idle port's `WEBWE7`, whose
    /// wire `BYP_ALT6` the router had taken as a hop for another signal —
    /// so that pin *carries* that signal. On an idle port that is
    /// harmless; on a port in use it is refused.
    Idle,
    /// Left at the interconnect's default, which the database says is
    /// `VCC_WIRE`; checked rather than assumed. The `ADDR?TIEHIGH` pins,
    /// which nextpnr ties to its `VCC` net.
    One,
    /// Not tied at all: a clock nothing drives, and `REGCE`, which does
    /// nothing while `DOA_REG` and `DOB_REG` are 0.
    Leave,
    /// A clock pin tied to zero as Vivado ties a configuration primitive's
    /// (`super::config`): a `CLK_L` wire has no ground fan two hops back, so
    /// over a bounce, `GND_WIRE`, `GFAN<n>`, `FAN_ALT<k>`, `FAN_BOUNCE<k>`,
    /// when [`TiePolicy::Zero`]'s search finds nothing, and left as
    /// [`TiePolicy::Idle`] leaves an input when neither does.
    ClockZero,
}

/// Whether `role` is an address bit the block's **width mode does not
/// read**, which makes it idle whatever the netlist put on it.
///
/// A `RAMB18E1` addresses its words through the top of `ADDRARDADDR` and
/// ignores the bits below, and how many it ignores is the width mode: a
/// port 18 bits wide uses `[13:4]`, 9 bits `[13:3]`, 4 bits `[13:2]`, 2
/// bits `[13:1]` and 1 bit all fourteen. The Xilinx library says so, and
/// `src/fpga/devices/xc7.dev` says the same thing from the other side with
/// its `addr N` clause, which is what makes the mapper pad those bits with
/// zeros in the first place.
///
/// **This is what stopped a contended ground fan failing a build.** Those
/// padding bits used to arrive here as netlist constants with
/// [`TiePolicy::Zero`], which hunts a free `GND_WIRE -> GFAN<n>` path; an
/// interconnect tile has two such fans for 48 inputs and the router
/// competes for both, so whether the hunt found one depended on where the
/// block landed. `examples/basys3/ssd1306_console.v` failed on `097a1d4`
/// on `p0_addr3` — recorded at the time as "all nine placement settings",
/// which were one placement, since the 7-series writer ignored `--place-*`
/// until `3c91260`;
/// `examples/basys3/selftest.v` is the one `docs/fpga-xray.md` records
/// building, then not, then building again as unrelated edits moved its
/// placement — and it happens to build at the default placement on
/// `097a1d4` today, which is what a lottery looks like from the lucky side.
/// A bit
/// the block does not read has no business failing anything, so it is
/// [`TiePolicy::Idle`] now: tied to zero when a fan is free and left to
/// the interconnect's own default when none is.
///
/// A family that *does* read those bits is the opposite case and is not
/// handled here at all: an ECP5 `DP16KD` in 18-bit mode needs
/// `AD[3:0] = 0011`, `src/fpga/devices/ecp5.dev` says so with `pad
/// 4'b0011`, and `techcells::drive_constant_data` gives them a real
/// driver before placement.
///
/// The widths are read off the cell's own parameters, and a port whose
/// width is missing or 0 is a port in use at the widest mode — which is
/// the conservative answer, because it claims the fewest unread bits.
fn unused_address_bit(role: &str, params: &dyn Fn(&str) -> Option<AttrValue>) -> bool {
    let Some((port, bit)) = role.split_once("_addr") else {
        return false;
    };
    let Ok(bit) = bit.parse::<u32>() else {
        return false;
    };
    let suffix = match port {
        "p0" => 'A',
        "p1" => 'B',
        _ => return false,
    };
    // The narrowest width either direction of this port is used at decides
    // how many low bits it reads: a port read at 18 bits and written at 9
    // reads `AD[13:3]`.
    let low = ["READ_WIDTH_", "WRITE_WIDTH_"]
        .into_iter()
        .filter_map(|name| params(&format!("{name}{suffix}")).and_then(|v| v.as_int()))
        .filter(|width| *width > 0)
        .map(address_low_bit)
        .min()
        .unwrap_or(0);
    bit < low
}

/// Whether `role` is a data input whose value nothing will ever see, which
/// makes it idle whatever the netlist put on it — the data-side twin of
/// [`unused_address_bit`]. Two cases, both read off the cell:
///
/// - **Above the port's write width.** A port written 9 bits wide stores
///   `DI[7:0]` and `DIP[0]`, 18 bits `DI[15:0]` and `DIP[1:0]`, and 4, 2
///   and 1 bits store that many of `DI` and no parity.
/// - **Stored but never read.** When all four widths are the same, bit `i`
///   written on either port comes back as `DO[i]` on either port and
///   nowhere else, so if neither port's `dout<i>` is routed (`read` holds
///   the `i` that are), what `din<i>` stores is never seen. This is the
///   case that failed: the mapper builds a 1024x8 memory in the 16-bit
///   mode with `DIBDI = {8'b0, data}`, and bits 8 to 15 are written and
///   never read.
///
/// Both are UG473's tables, **quoted**: nothing here has measured what a
/// one on an unread `DIBDI13` does — but whatever it does lands in a bit
/// no routed wire reads.
///
/// It is needed for the same reason the address rule was. Those bits
/// arrive as netlist zeros, a netlist zero was [`TiePolicy::Zero`], and a
/// zero tie hunts one of a tile's two ground fans — so whether
/// `examples/basys3/ssd1306_console.v` built depended on where its block
/// landed. It built at the default placement and failed at
/// `--place-effort 3` on `p1_din13`, which nobody saw until `--place-*`
/// reached the 7-series placer at all.
///
/// Mixed widths and a width that is missing, 0 or not one of the five
/// claim nothing beyond the first case, which is the conservative answer.
fn unused_data_bit(
    role: &str,
    params: &dyn Fn(&str) -> Option<AttrValue>,
    read: &HashSet<u32>,
) -> bool {
    let bare = role.strip_prefix(TIE_ONLY).unwrap_or(role);
    let Some((port, bit)) = bare.split_once("_din") else {
        return false;
    };
    let suffix = match port {
        "p0" => 'A',
        "p1" => 'B',
        _ => return false,
    };
    let width = |name: &str| params(name).and_then(|v| v.as_int());
    let (data, parity) = match width(&format!("WRITE_WIDTH_{suffix}")) {
        Some(18) => (16, 2),
        Some(9) => (8, 1),
        Some(4) => (4, 0),
        Some(2) => (2, 0),
        Some(1) => (1, 0),
        _ => return false,
    };
    if let Some(bit) = bit.strip_prefix('p') {
        return bit.parse::<u32>().is_ok_and(|bit| bit >= parity);
    }
    let Ok(bit) = bit.parse::<u32>() else {
        return false;
    };
    if bit >= data {
        return true;
    }
    let all = [
        "READ_WIDTH_A",
        "WRITE_WIDTH_A",
        "READ_WIDTH_B",
        "WRITE_WIDTH_B",
    ]
    .map(width);
    all.iter().all(|w| *w == all[0]) && !read.contains(&bit)
}

/// The lowest `ADDRARDADDR` bit a port of this width uses: 18 bits take
/// `[13:4]` and one bit takes all fourteen. A width
/// [`mode_features`] does not describe answers 0, which claims nothing.
fn address_low_bit(width: i64) -> u32 {
    match width {
        18 => 4,
        9 => 3,
        4 => 2,
        2 => 1,
        _ => 0,
    }
}

/// The policy for one input role of a block RAM bel (`p0_we0`, `tie_p1_regce`).
pub fn tie_policy(role: &str) -> TiePolicy {
    let bare = role.strip_prefix(TIE_ONLY).unwrap_or(role);
    let kind = bare.split_once('_').map_or(bare, |(_, k)| k);
    if kind.starts_with("addrtie") || kind == "addr15" || kind == "regclk" {
        // `ADDRARDADDR[15]` exists only on a `RAMB36E1` and only a cascade
        // reads it; Vivado ties it high, and refuses a block whose bit 15
        // is low outside a cascade. An output register's clock pin, which
        // only the Vivado model has, is a one when nothing clocks it, as
        // Vivado makes it: see `XrayFabric::spread_block_ram_pins`, which
        // gives a netlist the pin, and which a flow that skips it does
        // without.
        TiePolicy::One
    } else if kind == "clk" || kind == "regce" || kind.starts_with("inject") {
        // A `RAMB36E1`'s error injection, which only the correction reads:
        // Vivado leaves both pins unconnected on a block it infers
        // (`ib_1024x36`), and grounds them only when a design ties them.
        TiePolicy::Leave
    } else if kind.starts_with("din") {
        TiePolicy::Idle
    } else if role.starts_with(TIE_ONLY) && kind.starts_with("we") {
        // `WEBWE[7:4]`, the only tie-only write enables there are: a
        // `RAMB18E1` uses them for the upper bytes of its 36-bit
        // simple-dual-port mode and **not** in `TDP`, which is the only
        // mode `mode_features` describes and so the only one this flow
        // writes. nextpnr ties them low anyway, and so does this when a
        // ground fan is free; what `Idle` adds is that it will not fail a
        // build when one is not.
        //
        // That matters because they are the pins the router is most likely
        // to have walked through: it was `tie_p1_we4` on
        // `examples/basys3/ssd1306_console.v`, whose `FAN_ALT1` carried
        // another signal, so the pin could not be tied at all — and a pin
        // the mode does not read has no business failing a build. This is
        // the Xilinx library's statement (UG473: in TDP, `WEBWE[3:0]` is
        // used and `WEBWE[7:4]` is not) and nextpnr's own, **quoted**: no
        // instrument here has measured what a one on one of them does.
        //
        // That is the legacy model. The Vivado model has no tie-only write
        // enable: `WEBWE4` to `WEBWE7` are lanes of their own
        // ([`write_enable_lanes`]), held low in `TDP` as Vivado holds them,
        // with their wires closed to the router
        // ([`XrayFabric::block_ram_ground_fans`]), since a signal walked
        // through one is a signal on the pin, which Vivado's router refused
        // in Reticle's routes of `bram_fifo` and `lutram32`.
        TiePolicy::Idle
    } else {
        TiePolicy::Zero
    }
}

/// One input to make read something: the policy, the instance, the pin's
/// role, and the pin's node.
pub(super) type Want = (TiePolicy, String, String, NodeId);

/// An `IDDR`'s set or reset, which reads one unrouted — and an `ILOGIC`
/// has no feature that ignores it, where an `OLOGIC`'s `SRUSED` does — is
/// tied to zero here with the block RAMs' inputs, over the same ground
/// fans. NewAE's image does it (`INT_L.CTRL_L1.GFAN0`, `INT_L.GFAN0.GND_WIRE`
/// beside each of the Husky Plus's ADC inputs); left unrouted, every
/// `IDDR` of this flow was held set, and the ADC read `0xfff` whatever it
/// sent.
fn io_logic_wants(
    instance: &crate::fpga::place::Instance,
    site: &crate::fpga::arch::ArchSite,
    netlist: &Netlist,
    is_routed: &dyn Fn(usize) -> bool,
    wants: &mut Vec<Want>,
) {
    if matches!(
        instance.primitive.as_str(),
        "SRLC32E" | "SRL16E" | "SRLC16E"
    ) {
        // A shift register's address at a fixed value, and its enable tied
        // high: see `super::srl`. A one is left to read what it reads
        // undriven, with the wire claimed so that nothing else drives it, and a
        // zero comes over a ground fan, both as Vivado has them.
        for p in &instance.pins {
            let pin = &netlist.pins[*p];
            let want = match pin.constant {
                _ if pin.output || pin.signal.is_some() => continue,
                Some(Bit::One) => TiePolicy::One,
                Some(Bit::Zero) => TiePolicy::Zero,
                _ => continue,
            };
            if let Some(node) = site.pin(&pin.role) {
                wants.push((want, instance.name.clone(), pin.role.clone(), node));
            }
        }
        return;
    }
    if instance.primitive == super::regional::REGIONAL_BUFR {
        // Its `CLR` and `CE` have no inverter either, and an unrouted one reads
        // one: left so, every `BUFR` would be held clear. Vivado grounds a `CLR` it
        // leaves low and a `CE` tied low (`INT_L.BYP_ALT4.GFAN0`, `GFAN0.GND_WIRE`),
        // and leaves a tie high alone.
        for role in ["clr", "ce"] {
            let pin = instance
                .pins
                .iter()
                .map(|p| &netlist.pins[*p])
                .find(|p| p.role == role);
            let low = match pin {
                Some(p) => p.signal.is_none() && p.constant == Some(Bit::Zero),
                None => role == "clr",
            };
            if low && let Some(node) = site.pin(role) {
                wants.push((
                    TiePolicy::Zero,
                    instance.name.clone(),
                    role.to_owned(),
                    node,
                ));
            }
        }
        return;
    }
    let input = matches!(
        instance.primitive.as_str(),
        "IDDR" | "IDDR_2CLK" | "ISERDESE2"
    );
    let serialiser = matches!(instance.primitive.as_str(), "ISERDESE2" | "OSERDESE2");
    if !input && !serialiser && instance.primitive != "ODDR" {
        return;
    }
    let pins = |role: &str| {
        instance
            .pins
            .iter()
            .map(|p| &netlist.pins[*p])
            .filter(move |pin| pin.role == role)
            .collect::<Vec<_>>()
    };
    let routed = |role: &str| {
        pins(role)
            .iter()
            .any(|pin| pin.signal.is_some_and(is_routed))
    };
    let tied_one = |role: &str| {
        pins(role)
            .iter()
            .any(|pin| pin.signal.is_none() && pin.constant == Some(Bit::One))
    };
    let tied_zero = |role: &str| {
        let here = pins(role);
        !here.is_empty()
            && here
                .iter()
                .all(|pin| pin.signal.is_none() && pin.constant != Some(Bit::One))
    };
    let mut tie = |role: &str| {
        if let Some(node) = site.pin(role) {
            wants.push((
                TiePolicy::Zero,
                instance.name.clone(),
                role.to_owned(),
                node,
            ));
        }
    };
    // A set or reset tied high is left unrouted, which reads a one.
    if (input || serialiser) && !routed("sr") && !tied_one("sr") {
        tie("sr");
    }
    // A clock enable tied low, which Vivado grounds the same way
    // (`iddr_sweep`, `oddr_sweep`: the same site bits as one tied high).
    if tied_zero("ce") {
        tie("ce");
    }
    if !serialiser {
        return;
    }
    // A serialiser's other controls, as Vivado has them (`iserdes_pins`,
    // `oserdes_pins`, `oserdes_tristate`): each tied low is grounded, and each
    // unconnected is left to read a one, but for `BITSLIP` and an
    // `OSERDESE2`'s `RST` (above, with an `ISERDESE2`'s), which are grounded
    // unconnected too. A data or tristate input is its pin's inverter instead
    // (`iologic::oserdes_features`).
    for role in ["ce2", "dynclksel", "dynclkdivsel", "tce"] {
        if tied_zero(role) {
            tie(role);
        }
    }
    if input && !routed("bitslip") && !tied_one("bitslip") {
        tie("bitslip");
    }
}

/// The inverting pins of a `RAMB18E1` half, each of which has a
/// `ZINV_<pin>` feature: set means **not** inverted. nextpnr-xilinx sets it
/// for every one of them unless the cell's `IS_<pin>_INVERTED` is 1.
const INVERTIBLE: [&str; 10] = [
    "CLKARDCLK",
    "CLKBWRCLK",
    "ENARDEN",
    "ENBWREN",
    "REGCLKARDRCLK",
    "REGCLKB",
    "RSTRAMARSTRAM",
    "RSTRAMB",
    "RSTREGARSTREG",
    "RSTREGB",
];

/// The pins of a block RAM's site with an inverter that the Vivado model
/// holds low through it, and the roles of the netlist that reach them:
/// `p0_rst` is `RSTRAMARSTRAM`, `p1_en` is `ENBWREN`.
const INVERTED_LOW: [(&str, &str); 8] = [
    ("p0_rst", "RSTRAMARSTRAM"),
    ("p1_rst", "RSTRAMB"),
    ("p0_rstreg", "RSTREGARSTREG"),
    ("p1_rstreg", "RSTREGB"),
    ("p0_en", "ENARDEN"),
    ("p1_en", "ENBWREN"),
    ("p0_clk", "CLKARDCLK"),
    ("p1_clk", "CLKBWRCLK"),
];

/// The resets, enables and clocks of one block RAM that the Vivado model
/// holds low through their inverter, by site pin name: each one the netlist
/// leaves unconnected or ties to zero, unless the cell inverts it already.
/// None in the legacy model, which grounds them.
///
/// **Vivado's**: on every inferred block of `testdata/fpga/vivado/bram/`
/// the four resets are on its `<const0>` net, and its bitstream has them on
/// `CTRL0`, from `BYP_BOUNCE4`, which nothing drives and which reads
/// `VCC_WIRE`, with all four `ZINV_RST*` clear: a one, inverted. So does
/// this ([`tie`] makes a one through a pip when nothing behind the pin
/// defaults to one), which spends no ground fan on a reset. A port nothing
/// clocks or enables is the same to Vivado: given Reticle's netlist of
/// `ib_1024x32_bytes`, whose port B neither, it clears `ZINV_CLKBWRCLK` and
/// `ZINV_ENBWREN` and gives both pins a one (`CLK0.FAN_BOUNCE5`). What an
/// inverted one on a part reads is UG473's word and `ZINV`'s name, quoted.
fn held_by_inversion(
    model: LutRamModel,
    params: &dyn Fn(&str) -> Option<AttrValue>,
    instance: &crate::fpga::place::Instance,
    netlist: &Netlist,
) -> Vec<&'static str> {
    if model != LutRamModel::Vivado {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (role, pin) in INVERTED_LOW {
        let driven = instance.pins.iter().map(|p| &netlist.pins[*p]).any(|p| {
            p.role == role && (p.signal.is_some() || p.constant.is_some_and(|b| b != Bit::Zero))
        });
        let inverted = params(&format!("IS_{pin}_INVERTED"))
            .and_then(|v| v.as_int())
            .unwrap_or(0)
            == 1;
        if !driven && !inverted {
            out.push(pin);
        }
    }
    out
}

/// The mode features a `RAMB18E1` with these parameters turns on, without
/// the site prefix, as the legacy model writes them: [`mode_features_in`]
/// with [`LutRamModel::Legacy`].
///
/// # Errors
///
/// As [`mode_features_in`].
pub fn mode_features(params: &dyn Fn(&str) -> Option<AttrValue>) -> Result<Vec<String>, String> {
    mode_features_in(params, LutRamModel::Legacy, &[])
}

/// The mode features a `RAMB18E1` with these parameters turns on, without
/// the site prefix.
///
/// | Feature | When |
/// |---|---|
/// | `IN_USE` | always |
/// | `READ_WIDTH_A_<w>` and the three like it | the parameter is not 0; `w` is 1, 2, 4, 9 or 18 |
/// | `SDP_READ_WIDTH_36`, `SDP_WRITE_WIDTH_36` | `RAM_MODE` is `SDP` and the width 36: with `READ_WIDTH_B_18` for a read, `WRITE_WIDTH_A_18` and `WRITE_WIDTH_B_18` for a write |
/// | `DOA_REG`, `DOB_REG` | the parameter is 1 |
/// | `ZINV_<pin>` | `IS_<pin>_INVERTED` is not 1 and `held` does not name the pin; `REGCLK<x>` only beside its output register |
/// | `WRITE_MODE_A_READ_FIRST` / `_NO_CHANGE` | that mode; `WRITE_FIRST` is no bit, but in `SDP` is `NO_CHANGE` |
/// | `RSTREG_PRIORITY_<x>_REGCE` | that priority |
/// | `RDADDR_COLLISION_HWCONFIG_PERFORMANCE` | that |
/// | `ZINIT_A[17 - i]`, `ZINIT_B`, `ZSRVAL_A`, `ZSRVAL_B` | bit `i` of `INIT_A` (...) is **0** |
///
/// **Vivado's answers** (`testdata/fpga/vivado/bram/`, decoded from its
/// bitstreams of a cell named with each parameter): this was nextpnr-xilinx's
/// `write_bram_half`, quoted, and three things of it were not what Vivado
/// writes. `ZINIT_A[j]` is bit `17 - j` of `INIT_A`, the register's top bit
/// first, not bit `j`: invisible until a value is not all zeros, which no
/// inferred memory has. `ZINV_REGCLK<x>` is set only with its port's output
/// register. And `SDP` exists. The legacy model keeps nextpnr's `REGCLK` rule,
/// since it changes bits of every block RAM; the other two never did.
///
/// `held` names the resets the Vivado model holds low **through their
/// inverter**: a `VCC` on the pin and `ZINV` clear, as Vivado does for a
/// reset the design ties low. See `held_by_inversion`.
///
/// The contents are not here: they are ordinary indexed features and the
/// generic path carries them.
///
/// # Errors
///
/// A message for a mode this does not describe: `RAM_MODE` other than `TDP`
/// or `SDP`, a width that is not 0, 1, 2, 4, 9, 18 or (in `SDP`) 36, or a
/// write mode that is not one of the three.
pub fn mode_features_in(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    model: LutRamModel,
    held: &[&str],
) -> Result<Vec<String>, String> {
    let mut out = vec!["IN_USE".to_owned()];
    let sdp = match params("RAM_MODE") {
        None => false,
        Some(AttrValue::String(mode)) if mode == "TDP" => false,
        Some(AttrValue::String(mode)) if mode == "SDP" => true,
        Some(AttrValue::String(mode)) => {
            return Err(format!(
                "`RAM_MODE` is \"{mode}\"; \"TDP\" and \"SDP\" are described for this fabric"
            ));
        }
        Some(_) => return Err("`RAM_MODE` is not a string".to_owned()),
    };
    let width = |name: &str| params(name).and_then(|v| v.as_int()).unwrap_or(0);
    for name in [
        "READ_WIDTH_A",
        "READ_WIDTH_B",
        "WRITE_WIDTH_A",
        "WRITE_WIDTH_B",
    ] {
        match width(name) {
            0 => {}
            1 | 2 | 4 | 9 | 18 => out.push(format!("{name}_{}", width(name))),
            36 if sdp && name == "READ_WIDTH_A" => {
                out.push("SDP_READ_WIDTH_36".to_owned());
                out.push("READ_WIDTH_B_18".to_owned());
            }
            36 if sdp && name == "WRITE_WIDTH_B" => {
                out.push("SDP_WRITE_WIDTH_36".to_owned());
                out.push("WRITE_WIDTH_A_18".to_owned());
                out.push("WRITE_WIDTH_B_18".to_owned());
            }
            other => {
                return Err(format!(
                    "`{name}` is {other}; the widths described are 1, 2, 4, 9, 18 and, read on \
                     port A or written on port B in SDP, 36"
                ));
            }
        }
    }
    let mut registered = [false; 2];
    for (at, name) in ["DOA_REG", "DOB_REG"].into_iter().enumerate() {
        if params(name).and_then(|v| v.as_int()).unwrap_or(0) == 1 {
            out.push(name.to_owned());
            registered[at] = true;
        }
    }
    for pin in INVERTIBLE {
        let inverted = params(&format!("IS_{pin}_INVERTED"))
            .and_then(|v| v.as_int())
            .unwrap_or(0);
        let register = match pin {
            "REGCLKARDRCLK" => Some(registered[0]),
            "REGCLKB" => Some(registered[1]),
            _ => None,
        };
        if (model == LutRamModel::Vivado && register == Some(false)) || held.contains(&pin) {
            continue;
        }
        if inverted != 1 {
            out.push(format!("ZINV_{pin}"));
        }
    }
    for name in ["WRITE_MODE_A", "WRITE_MODE_B"] {
        let mode = match params(name) {
            None => "WRITE_FIRST".to_owned(),
            Some(AttrValue::String(s)) => s,
            Some(_) => return Err(format!("`{name}` is not a string")),
        };
        match mode.as_str() {
            "WRITE_FIRST" if sdp => out.push(format!("{name}_NO_CHANGE")),
            "WRITE_FIRST" => {}
            "READ_FIRST" | "NO_CHANGE" => out.push(format!("{name}_{mode}")),
            other => return Err(format!("`{name}` is \"{other}\"")),
        }
    }
    for (name, feature) in [
        ("RSTREG_PRIORITY_A", "RSTREG_PRIORITY_A_REGCE"),
        ("RSTREG_PRIORITY_B", "RSTREG_PRIORITY_B_REGCE"),
    ] {
        match params(name) {
            Some(AttrValue::String(s)) if s == "REGCE" => out.push(feature.to_owned()),
            Some(AttrValue::String(s)) if s == "RSTREG" => {}
            None => {}
            Some(other) => return Err(format!("`{name}` is {other:?}")),
        }
    }
    match params("RDADDR_COLLISION_HWCONFIG") {
        Some(AttrValue::String(s)) if s == "PERFORMANCE" => {
            out.push("RDADDR_COLLISION_HWCONFIG_PERFORMANCE".to_owned());
        }
        Some(AttrValue::String(s)) if s == "DELAYED_WRITE" => {}
        None => {}
        Some(other) => return Err(format!("`RDADDR_COLLISION_HWCONFIG` is {other:?}")),
    }
    for (feature, param) in [
        ("ZINIT_A", "INIT_A"),
        ("ZINIT_B", "INIT_B"),
        ("ZSRVAL_A", "SRVAL_A"),
        ("ZSRVAL_B", "SRVAL_B"),
    ] {
        let value = params(param);
        for i in 0..18u32 {
            if !param_bit(value.as_ref(), i) {
                out.push(format!("{feature}[{}]", 17 - i));
            }
        }
    }
    Ok(out)
}

/// The features a `RAMB36E1` with these parameters turns on, each with its
/// prefix: `RAMB18_Y0.` and `RAMB18_Y1.` for the two halves it is, and
/// `RAMB36.` for its own.
///
/// **Vivado's answers**, decoded from its bitstreams of a cell named with
/// each parameter (`testdata/fpga/vivado/bram/`):
///
/// - Each half is a `RAMB18E1` half as wide: a port 36 wide is 18 on each, 18
///   is 9, 9 is 4, 4 is 2, and 2 and 1 are 1. A port 9 or 1 wide also sets
///   `RAMB36.BRAM36_<width>_1`.
/// - In `SDP`, 72 wide, each half is 36 wide in `SDP` with all four of its
///   widths 18.
/// - The rest is each half's, set alike on both: registers, inversions, write
///   modes, priorities.
/// - A word's even bits are the lower half's and its odd bits the upper's:
///   bit `2k` of `INIT_A` (36 bits, parity on top) is bit `k` of the lower
///   half's (18 bits), and bit `2k + 1` the upper half's. The contents are
///   the same: bit `2k` of the flat contents, `INIT_00` bit 0 first, is the
///   lower half's bit `k` (`ramb36_param_of`). Checked on `b36_srval`'s
///   four values and on `b36_w36`'s random contents, every bit.
///
/// `held` is as [`mode_features_in`]'s, for both halves.
///
/// # Errors
///
/// As [`mode_features_in`], and for a width a `RAMB36E1` does not have.
pub fn ramb36_features(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    model: LutRamModel,
    held: &[&str],
) -> Result<Vec<String>, String> {
    let sdp = matches!(params("RAM_MODE"), Some(AttrValue::String(m)) if m == "SDP");
    let mut own = Vec::new();
    let mut half: BTreeMap<String, AttrValue> = BTreeMap::new();
    for name in [
        "READ_WIDTH_A",
        "READ_WIDTH_B",
        "WRITE_WIDTH_A",
        "WRITE_WIDTH_B",
    ] {
        let width = params(name).and_then(|v| v.as_int()).unwrap_or(0);
        let narrow = match width {
            0 => 0,
            72 if sdp && (name == "READ_WIDTH_A" || name == "WRITE_WIDTH_B") => 36,
            36 => 18,
            18 => 9,
            9 => 4,
            4 => 2,
            2 | 1 => 1,
            other => return Err(format!("`{name}` is {other} for a RAMB36E1")),
        };
        if matches!(width, 9 | 1) {
            own.push(format!("RAMB36.BRAM36_{name}_1"));
        }
        if narrow != 0 {
            half.insert(name.to_owned(), AttrValue::Int(narrow));
        }
    }
    // A word's bits alternate between the halves.
    let split = |name: &str, lower: bool| -> Option<AttrValue> {
        let value = params(name)?;
        let mut bits = Vec::with_capacity(18);
        for k in 0..18u32 {
            let at = if lower { 2 * k } else { 2 * k + 1 };
            bits.push(if param_bit(Some(&value), at) {
                crate::logic::Bit::One
            } else {
                crate::logic::Bit::Zero
            });
        }
        Some(AttrValue::Const(crate::ir::Const::from_bits(&bits)))
    };
    let mut out = own;
    for (prefix, lower) in [("RAMB18_Y0", true), ("RAMB18_Y1", false)] {
        let one = |name: &str| -> Option<AttrValue> {
            if let Some(v) = half.get(name) {
                return Some(v.clone());
            }
            if matches!(name, "INIT_A" | "INIT_B" | "SRVAL_A" | "SRVAL_B") {
                return split(name, lower);
            }
            if name.starts_with("READ_WIDTH") || name.starts_with("WRITE_WIDTH") {
                return None;
            }
            params(name)
        };
        for feature in mode_features_in(&one, model, held)? {
            out.push(format!("{prefix}.{feature}"));
        }
        // Read 72 wide, each half is read 36 wide in `SDP`, and Vivado sets
        // its port A's 18 beside it, which a `RAMB18E1` read 36 wide does
        // not have (`b36_sdp72` against `b18_sdp36`).
        if sdp && params("READ_WIDTH_A").and_then(|v| v.as_int()) == Some(72) {
            out.push(format!("{prefix}.READ_WIDTH_A_18"));
        }
    }
    Ok(out)
}

/// The bel name of a tile's `RAMB36E1`, and the kind of site it is.
pub(super) const RAMB36_BEL: &str = "RAMB36";
/// See [`RAMB36_BEL`]: not `bram`, so that a `RAMB18E1` is never put on it.
pub(super) const RAMB36_KIND: &str = "bram36";

/// The `RAMB36E1` parameter bit a half's contents bit is: bit `k` of a
/// half's flat contents is bit `2k` of the whole's for the lower half and
/// `2k + 1` for the upper (see [`ramb36_features`]). `half` is the feature prefix (`RAMB18_Y1`), `name`
/// and `index` the half's parameter bit (`INIT_3F`, 255). `None` for
/// anything that is not contents.
pub(super) fn ramb36_param_of(half: &str, name: &str, index: u32) -> Option<(String, u32)> {
    let odd = match half {
        "RAMB18_Y0" => 0,
        "RAMB18_Y1" => 1,
        _ => return None,
    };
    let (kind, row) = if let Some(row) = name.strip_prefix("INITP_") {
        ("INITP_", row)
    } else {
        ("INIT_", name.strip_prefix("INIT_")?)
    };
    if row.len() != 2 {
        return None;
    }
    let row = u32::from_str_radix(row, 16).ok()?;
    let flat = 2 * (row * 256 + index) + odd;
    Some((format!("{kind}{:02X}", flat / 256), flat % 256))
}

/// What [`XrayFabric::configure_block_rams`] needs from the database,
/// read once at load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockRamTables {
    /// Tile type to feature name (`RAMB18_Y0.IN_USE`) to the bits that
    /// must be one, for every block RAM feature that is not contents.
    pub features: BTreeMap<String, BTreeMap<String, Vec<ConfigBit>>>,
    /// Tile type to the wires its `ppips` file says read `VCC_WIRE` when
    /// nothing drives them.
    pub vcc_defaults: BTreeMap<String, BTreeSet<String>>,
    /// Whose block RAMs: Vivado's features and ties, or the legacy ones.
    /// See [`mode_features_in`] and [`XrayFabric::configure_block_rams`].
    pub model: LutRamModel,
}

impl BlockRamTables {
    /// Gathers the tables from every tile type's features and pseudo-pips.
    pub(super) fn new(
        features: &HashMap<String, super::FeatureSet>,
        fixed: &HashMap<String, Vec<Ppip>>,
    ) -> BlockRamTables {
        let mut out = BlockRamTables::default();
        for (tile_type, set) in features {
            // A `DSP48E1`'s features are looked up the same way.
            if wire_prefix(tile_type, "RAMB18_Y0").is_none() && !super::dsp::is_dsp_tile(tile_type)
            {
                continue;
            }
            let mut map = BTreeMap::new();
            for feature in set.features() {
                let contents = feature.name.contains(".INIT_") || feature.name.contains(".INITP_");
                if contents {
                    continue;
                }
                map.insert(feature.name.clone(), feature.ones.clone());
            }
            out.features.insert(tile_type.clone(), map);
        }
        for (tile_type, ppips) in fixed {
            let wires: BTreeSet<String> = ppips
                .iter()
                .filter(|p| p.kind == PpipKind::Default && p.from == "VCC_WIRE")
                .map(|p| p.to.clone())
                .collect();
            if !wires.is_empty() {
                out.vcc_defaults.insert(tile_type.clone(), wires);
            }
        }
        out
    }
}

/// What configuring the block RAMs did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockRamReport {
    /// `RAMB18E1` instances configured.
    pub rams: usize,
    /// Mode features turned on over all of them.
    pub features: usize,
    /// Inputs tied to zero through `GND_WIRE`.
    pub zeros: usize,
    /// Inputs checked to read the interconnect's `VCC_WIRE` default.
    pub ones: usize,
    /// Inputs nothing reads that were left untied, because no ground
    /// path was free or a routed signal passes their wire; see
    /// [`TiePolicy::Idle`].
    pub idle_left: usize,
    /// The pips the zero ties switched on, which a decoding of the
    /// bitstream finds beside the router's.
    pub tie_pips: Vec<PipId>,
    /// `DSP48E1` instances configured, whose constants go through the same
    /// ties; see `super::dsp`.
    pub dsps: usize,
    /// Features turned on over all of those.
    pub dsp_features: usize,
}

impl BlockRamReport {
    /// One line, or nothing when the design has no block RAM.
    pub fn to_text(&self) -> String {
        if self.rams == 0 {
            return String::new();
        }
        format!(
            "note: {} block RAM(s) configured with {} mode feature(s); {} input(s) tied to zero \
             over {} pip(s), {} left at the interconnect's VCC default, {} idle input(s) \
             left untied\n",
            self.rams,
            self.features,
            self.zeros,
            self.tie_pips.len(),
            self.ones,
            self.idle_left,
        )
    }
}

/// Whether a pip of a `BRAM_L` / `BRAM_R` tile reads one of the address
/// cascade inputs (`BRAM_CASCINBOT_*`, `BRAM_CASCINTOP_*`), which the
/// loader leaves out of the graph.
///
/// # Why
///
/// The cascade carries a 36 kbit block's address from one tile to the
/// next, for the cascaded 64 kbit mode. To a router it is also just a
/// wire, and the first build of `examples/basys3/bram_rom.v` used it as
/// one: four of the eight address bits travelled up through the address
/// buses of the four *unused* block RAMs below the one placed, switching
/// on `ADDRARDADDRU<n> <- CASCINBOT` in each and needing nextpnr's
/// `CASCOUT_ARD_ACTIVE` in every tile it left. Every bit of that decoded
/// and the arcs matched — it was a legal reading of the database — but it
/// is a path neither Vivado nor nextpnr takes for a lone block, it runs
/// through four sites this flow does not otherwise configure, and nothing
/// says the cascade's output is driven when its block is not in a
/// cascaded mode. Nothing here places a cascaded pair, so the path is
/// removed rather than taken on trust.
pub(super) fn is_cascade_input(tile_type: &str, from: &str) -> bool {
    wire_prefix(tile_type, "RAMB18_Y0").is_some()
        && (from.starts_with("BRAM_CASCINBOT_") || from.starts_with("BRAM_CASCINTOP_"))
}

/// Why a block RAM or a `DSP48E1` could not be configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRamError {
    /// The instance.
    pub instance: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for BlockRamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cell `{}`: {}", self.instance, self.message)
    }
}

impl Error for BlockRamError {}

/// The constant drivers built so far, shared between the inputs that
/// want the same value.
#[derive(Default)]
struct Ties {
    /// Nodes held at zero by a tie.
    zero: HashSet<NodeId>,
    /// Nodes left at the `VCC_WIRE` default.
    one: HashSet<NodeId>,
    /// The pips switched on, in order.
    pips: Vec<PipId>,
}

/// The tile type a node belongs to.
fn tile_type_of<'a>(arch: &'a Arch, graph: &RoutingGraph, node: NodeId) -> &'a str {
    let (x, y) = graph.wire(node).tile;
    arch.tile_index_at(x, y)
        .map_or("", |index| arch.tile_types[index].name.as_str())
}

/// The nodes a pin's node is fed from through connections that cost no
/// bits — its own wire, the fixed site-pin hop, the tile joins — nearest
/// first.
fn fixed_chain(graph: &RoutingGraph, pin: NodeId) -> Vec<NodeId> {
    let mut seen: HashSet<NodeId> = HashSet::from([pin]);
    let mut order = vec![pin];
    let mut next = 0;
    while next < order.len() {
        let node = order[next];
        next += 1;
        for pip in graph.incoming(node) {
            if !graph.pip_bits(*pip).is_empty() {
                continue;
            }
            let from = graph.pip(*pip).from;
            if seen.insert(from) {
                order.push(from);
            }
        }
    }
    order
}

/// Whether `node` is a wire its tile type's `ppips` file says defaults to
/// `VCC_WIRE`.
fn reads_vcc(
    arch: &Arch,
    graph: &RoutingGraph,
    vcc_defaults: &BTreeMap<String, BTreeSet<String>>,
    node: NodeId,
) -> bool {
    vcc_defaults
        .get(tile_type_of(arch, graph, node))
        .is_some_and(|wires| wires.contains(&graph.wire(node).name))
}

/// A one for a pin whose own wires default to nothing, as Vivado makes one
/// for a block RAM's reset: the first programmable pip into `chain` from a
/// bounce wire whose [`fixed_chain`] reaches a wire that defaults to
/// `VCC_WIRE` and carries nothing (`CTRL0.BYP_BOUNCE4`, `BYP_BOUNCE4` being
/// `BYP_ALT4`'s with no switch between). The pip and that chain, or `None`.
fn vcc_source(
    arch: &Arch,
    graph: &RoutingGraph,
    vcc_defaults: &BTreeMap<String, BTreeSet<String>>,
    used: &HashSet<NodeId>,
    ties: &Ties,
    chain: &[NodeId],
) -> Option<(PipId, Vec<NodeId>)> {
    for node in chain {
        for pip in graph.incoming(*node) {
            if graph.pip_bits(*pip).is_empty() {
                continue;
            }
            let from = graph.pip(*pip).from;
            if !graph.wire(from).name.contains("_BOUNCE") {
                continue;
            }
            let behind = fixed_chain(graph, from);
            if behind
                .iter()
                .any(|n| used.contains(n) || ties.zero.contains(n))
            {
                continue;
            }
            if behind
                .iter()
                .any(|n| reads_vcc(arch, graph, vcc_defaults, *n))
            {
                return Some((*pip, behind));
            }
        }
    }
    None
}

/// Ties a clock pin's node to zero over a bounce: the first node of its
/// [`fixed_chain`] that a programmable pip reaches from a `*_BOUNCE*` wire, whose
/// own fixed chain a programmable pip reaches from a ground fan. Three pips and
/// the fan's `GND_WIRE` hop, which is the road Vivado takes to a `STARTUPE2`'s
/// clock (`INT_L.CLK_L1.FAN_BOUNCE5`, `FAN_ALT5.GFAN1`, `GFAN1.GND_WIRE`, in
/// `testdata/fpga/vivado/config/`).
fn tie_over_bounce(
    graph: &RoutingGraph,
    used: &HashSet<NodeId>,
    ties: &mut Ties,
    pin: NodeId,
) -> Result<(), String> {
    let chain = fixed_chain(graph, pin);
    let free = |ties: &Ties, node: &NodeId| !used.contains(node) && !ties.one.contains(node);
    if !chain.iter().all(|n| free(ties, n)) {
        return Err("the pin's wire is in use".to_owned());
    }
    for node in &chain {
        for into in graph.incoming(*node) {
            if graph.pip_bits(*into).is_empty() {
                continue;
            }
            let bounce = graph.pip(*into).from;
            if !graph.wire(bounce).name.contains("_BOUNCE") {
                continue;
            }
            let behind = fixed_chain(graph, bounce);
            if !behind.iter().all(|n| free(ties, n)) {
                continue;
            }
            for alt in &behind {
                for from_fan in graph.incoming(*alt) {
                    if graph.pip_bits(*from_fan).is_empty() {
                        continue;
                    }
                    let fan = graph.pip(*from_fan).from;
                    if !free(ties, &fan) {
                        continue;
                    }
                    let ground = graph.incoming(fan).iter().copied().find(|p| {
                        !graph.pip_bits(*p).is_empty()
                            && graph.wire(graph.pip(*p).from).name == "GND_WIRE"
                    });
                    let Some(ground) = ground else { continue };
                    if !ties.zero.contains(&fan) {
                        ties.pips.push(ground);
                        ties.zero.insert(fan);
                        ties.zero.insert(graph.pip(ground).from);
                    }
                    ties.pips.push(*from_fan);
                    ties.pips.push(*into);
                    ties.zero.extend(behind);
                    ties.zero.extend(chain);
                    return Ok(());
                }
            }
        }
    }
    Err("no bounce reaches a free ground fan".to_owned())
}

/// Ties one input pin's node to `value`.
///
/// A zero: the first node of the pin's [`fixed_chain`] that a programmable
/// pip reaches from a node which itself is reachable by a programmable pip
/// from a `GND_WIRE` — in an interconnect tile, `GFAN0` or `GFAN1` — and
/// whose source is not carrying a routed signal or a one. Both pips are
/// switched on (the `GND_WIRE` hop once per tile). A one: some node of the
/// chain must be a wire its tile type's `ppips` file says defaults to
/// `VCC_WIRE`, and no node of the chain may be driven by anything; or,
/// when none is, one pip from such a wire ([`vcc_source`]).
fn tie(
    arch: &Arch,
    graph: &RoutingGraph,
    vcc_defaults: &BTreeMap<String, BTreeSet<String>>,
    used: &HashSet<NodeId>,
    ties: &mut Ties,
    pin: NodeId,
    value: Bit,
) -> Result<(), String> {
    // A `DSP48E1`'s pin with a mux of its own takes either constant right at
    // the pin, with nothing behind it involved. See `super::dsp`.
    let local = if value == Bit::One {
        "DSP_VCC_"
    } else {
        "DSP_GND_"
    };
    if let Some(pip) = graph
        .incoming(pin)
        .iter()
        .copied()
        .find(|p| graph.wire(graph.pip(*p).from).name.starts_with(local))
    {
        ties.pips.push(pip);
        return Ok(());
    }
    let chain = fixed_chain(graph, pin);
    let name = |node: NodeId| graph.wire(node).full_name();
    for node in &chain {
        if used.contains(node) {
            return Err(format!(
                "`{}` is behind the pin and carries a routed signal",
                name(*node)
            ));
        }
        let other = if value == Bit::One {
            &ties.zero
        } else {
            &ties.one
        };
        if other.contains(node) {
            return Err(format!(
                "`{}` is behind the pin and is already tied the other way",
                name(*node)
            ));
        }
    }
    if value == Bit::One {
        let default = chain
            .iter()
            .any(|node| reads_vcc(arch, graph, vcc_defaults, *node));
        if default {
            ties.one.extend(chain);
            return Ok(());
        }
        // A `CTRL` wire has no default of its own, and Vivado gives one a hop
        // from a bounce wire that does (`CTRL_L0.BYP_BOUNCE4`, in
        // `dsp::sweep.txt`'s `tie_one`, and every block RAM's reset held
        // through its inverter); a block RAM's register clock likewise
        // (`CLK0.FAN_BOUNCE5`). See [`vcc_source`].
        let Some((pip, behind)) = vcc_source(arch, graph, vcc_defaults, used, ties, &chain) else {
            return Err(format!(
                "nothing behind `{}` defaults to VCC_WIRE, and no pip reaches it from a wire \
                 that does and that nothing drives",
                name(pin)
            ));
        };
        ties.pips.push(pip);
        ties.one.extend(chain);
        ties.one.extend(behind);
        return Ok(());
    }
    if chain.iter().any(|node| ties.zero.contains(node)) {
        ties.zero.extend(chain);
        return Ok(());
    }
    for node in &chain {
        for pip in graph.incoming(*node) {
            if graph.pip_bits(*pip).is_empty() {
                continue;
            }
            let fan = graph.pip(*pip).from;
            if used.contains(&fan) || ties.one.contains(&fan) {
                continue;
            }
            let ground = graph.incoming(fan).iter().copied().find(|p| {
                !graph.pip_bits(*p).is_empty() && graph.wire(graph.pip(*p).from).name == "GND_WIRE"
            });
            let Some(ground) = ground else { continue };
            if !ties.zero.contains(&fan) {
                ties.pips.push(ground);
                ties.zero.insert(fan);
                ties.zero.insert(graph.pip(ground).from);
            }
            ties.pips.push(*pip);
            ties.zero.extend(chain);
            return Ok(());
        }
    }
    Err(format!(
        "no free GND_WIRE path reaches `{}` within two pips. An interconnect tile has exactly \
         two ground fans — `GFAN0` serves half its `IMUX`es and `GFAN1` the other half — and the \
         router competes for both, so this says the fan this pin's `IMUX` is reachable from \
         carries a signal. An **address** bit never asks this any more — one below the width \
         mode's own range is a bit the block does not read and gets `TiePolicy::Idle`, and one \
         inside it is a bit the design drives — so this is a pin nothing connects at all, and \
         the ways out are to connect it or to leave the port it belongs to entirely unused, \
         which also makes it idle and lets it go untied",
        name(pin)
    ))
}

/// A `RAMB36E1` port written narrower than 18 bits, its data inputs as
/// Vivado wires them (`b36_w1`, `b36_w9`, `b36_w4`): written one bit wide,
/// `DI[0]` reaches both halves' first data pin and `DI[1]` nothing; nine
/// wide, `DIP[0]` both halves' first parity pin and `DIP[1]` nothing; four,
/// two or one wide, neither `DIP[0]` nor `DIP[1]` anything (`DIP[3:2]` stay
/// on their pins, which those modes do not read either). A half holds every other word
/// there, so the bit must reach the half the address picks, which is either.
/// [`ramb36_pins`] alone would have put a one-bit port's upper half on
/// `DI[1]`, which the cell does not read in that mode, and none of this
/// flow's checks could see it: every bit decodes, and the netlist's
/// simulation never sees pins.
fn narrow_ramb36_inputs(netlist: &mut Netlist, index: usize, port: usize, width: i64) {
    let p = format!("p{port}_");
    let pins: Vec<usize> = netlist.instances[index]
        .pins
        .iter()
        .copied()
        .filter(|pin| {
            let role = &netlist.pins[*pin].role;
            role.starts_with(&format!("{p}din"))
        })
        .collect();
    for pin in pins {
        let role = netlist.pins[pin].role.clone();
        let reaches = ramb36_input_roles(width, &role);
        if reaches.len() == 1 && reaches[0] == role {
            continue;
        }
        let source = netlist.pins[pin].clone();
        if let Some(signal) = netlist.pins[pin].signal.take() {
            netlist.signals[signal].sinks.retain(|p| *p != pin);
        }
        netlist.pins[pin].constant = None;
        netlist.pins[pin].role = format!("cell_{role}");
        for to in reaches {
            let mut copy = source.clone();
            copy.role = to;
            netlist.pins.push(copy);
            let at = netlist.pins.len() - 1;
            netlist.instances[index].pins.push(at);
            if let Some(signal) = netlist.pins[at].signal {
                netlist.signals[signal].sinks.push(at);
            }
        }
    }
}

/// The site roles a `RAMB36E1`'s data input role reaches when its port is
/// written `width` bits wide: see `narrow_ramb36_inputs`. `p0_din0` of a
/// one-bit port is `["p0_din0", "p0_din1"]`, its `p0_din1` is `[]`, and any
/// role this does not change is itself alone.
#[must_use]
pub fn ramb36_input_roles(width: i64, role: &str) -> Vec<String> {
    let Some((port, rest)) = role.split_once('_') else {
        return vec![role.to_owned()];
    };
    let (parity, bit) = match rest.strip_prefix("dinp") {
        Some(bit) => (true, bit),
        None => match rest.strip_prefix("din") {
            Some(bit) => (false, bit),
            None => return vec![role.to_owned()],
        },
    };
    let Ok(bit) = bit.parse::<u32>() else {
        return vec![role.to_owned()];
    };
    let both = |kind: &str| vec![format!("{port}_{kind}0"), format!("{port}_{kind}1")];
    match (width, parity, bit) {
        (1, false, 0) => both("din"),
        (1, false, 1) => Vec::new(),
        (9, true, 0) => both("dinp"),
        (9, true, 1) => Vec::new(),
        (1 | 2 | 4, true, 0 | 1) => Vec::new(),
        _ => vec![role.to_owned()],
    }
}

impl XrayFabric {
    /// Gives every block RAM's write-enable site pins the cell bits Vivado
    /// gives them, and its output registers' clocks theirs, in the Vivado
    /// model; nothing in the legacy one. Returns the number of cells it
    /// rewired.
    ///
    /// **The register clocks.** A site has a clock pin for each port's
    /// output register, `REGCLKARDRCLK` and `REGCLKB`, and the cell has no
    /// port for them. Vivado routes the port's own clock to it when
    /// `DO<x>_REG` is 1 (`b18_doreg`: `CLKARDCLK` reaches the site's
    /// `RDCLK` and `RDRCLK`), and otherwise a one with `ZINV_REGCLK<x>`
    /// clear. So each port gets a pin `p<port>_regclk`: on the clock's
    /// signal, or the constant one. Before this, a registered output in a
    /// bitstream of this flow had every feature Vivado's has and no clock:
    /// a fault every bit decoding, every arc matching and the netlist's
    /// simulation all pass, which Vivado's router found, asked to finish
    /// Reticle's routes (`VCCNet ... REGCLKB` unroutable).
    ///
    /// A cell's `WEA[k]` is one netlist pin, `p0_we<k>`, but the site has
    /// more write-enable pins than the cell has bits, and which bit each
    /// one carries depends on the widths: [`write_enable_lanes`] has
    /// Vivado's answer. So the cell's pins become the site's, one per pin
    /// `p0_we<i>` of `bel_pins_in` and `ramb36_pins`, each carrying
    /// what the cell's bit for that lane carried, and a lane Vivado holds
    /// low the constant zero, which the tie pass grounds. A cell bit no
    /// lane carries (`WEA[1]` of a 9-bit port, which Vivado leaves
    /// unconnected) keeps its pin with no signal and a role no site has.
    ///
    /// Run on the netlist before placement, so that the routes, the ties
    /// and [`XrayFabric::block_ram_ground_fans`] all see the site's pins.
    pub fn spread_block_ram_pins(
        &self,
        design: &Design,
        module: ModuleId,
        netlist: &mut Netlist,
    ) -> usize {
        if self.block_ram.model != LutRamModel::Vivado {
            return 0;
        }
        let Some(m) = design.modules.get(module) else {
            return 0;
        };
        let mut rewired = 0;
        for index in 0..netlist.instances.len() {
            let primitive = netlist.instances[index].primitive.clone();
            if !is_block_ram(&primitive) {
                continue;
            }
            let cell = m.cells.get(netlist.instances[index].cell);
            let params = |name: &str| cell.and_then(|c| c.params.get(name)).cloned();
            for port in 0..2usize {
                let registered = params(["DOA_REG", "DOB_REG"][port])
                    .and_then(|v| v.as_int())
                    .unwrap_or(0)
                    == 1;
                let clock = netlist.instances[index]
                    .pins
                    .iter()
                    .map(|p| &netlist.pins[*p])
                    .find(|p| p.role == format!("p{port}_clk"))
                    .and_then(|p| p.signal);
                let (signal, constant) = match clock {
                    Some(signal) if registered => (Some(signal), None),
                    _ => (None, Some(Bit::One)),
                };
                netlist.pins.push(crate::fpga::place::NetPin {
                    instance: index,
                    port: format!("REGCLK{}", if port == 0 { "ARDRCLK" } else { "B" }),
                    bit: 0,
                    role: format!("p{port}_regclk"),
                    output: false,
                    signal,
                    constant,
                });
                let pin = netlist.pins.len() - 1;
                netlist.instances[index].pins.push(pin);
                if let Some(signal) = signal {
                    netlist.signals[signal].sinks.push(pin);
                }
                let prefix = format!("p{port}_we");
                let old: Vec<usize> = netlist.instances[index]
                    .pins
                    .iter()
                    .copied()
                    .filter(|pin| {
                        netlist.pins[*pin]
                            .role
                            .strip_prefix(&prefix)
                            .is_some_and(|k| k.parse::<u32>().is_ok())
                    })
                    .collect();
                let bit_of = |pin: usize| -> u32 {
                    netlist.pins[pin].role[prefix.len()..]
                        .parse()
                        .unwrap_or(u32::MAX)
                };
                let sources: BTreeMap<u32, (Option<usize>, Option<Bit>, String)> = old
                    .iter()
                    .map(|pin| {
                        let p = &netlist.pins[*pin];
                        (bit_of(*pin), (p.signal, p.constant, p.port.clone()))
                    })
                    .collect();
                let port_name = sources.values().next().map_or_else(
                    || if port == 0 { "WEA" } else { "WEBWE" }.to_owned(),
                    |s| s.2.clone(),
                );
                // Off the signals they were on: each pin is given its
                // lane's source below, or nothing.
                for pin in &old {
                    if let Some(signal) = netlist.pins[*pin].signal.take() {
                        netlist.signals[signal].sinks.retain(|p| p != pin);
                    }
                    netlist.pins[*pin].constant = None;
                    let bit = netlist.pins[*pin].bit;
                    netlist.pins[*pin].role = format!("cell_{prefix}{bit}");
                }
                let mut spare = old.into_iter();
                for (lane, bit) in write_enable_lanes(&primitive, port, &params)
                    .into_iter()
                    .enumerate()
                {
                    let (signal, constant) = match bit {
                        Some(bit) => match sources.get(&bit) {
                            Some((signal, constant, _)) => (*signal, *constant),
                            // The cell does not connect that bit: the lane
                            // has no pin, and the tie pass decides.
                            None => continue,
                        },
                        None => (None, Some(Bit::Zero)),
                    };
                    let pin = match spare.next() {
                        Some(pin) => pin,
                        None => {
                            netlist.pins.push(crate::fpga::place::NetPin {
                                instance: index,
                                port: port_name.clone(),
                                bit: u32::try_from(lane).unwrap_or(0),
                                role: String::new(),
                                output: false,
                                signal: None,
                                constant: None,
                            });
                            let pin = netlist.pins.len() - 1;
                            netlist.instances[index].pins.push(pin);
                            pin
                        }
                    };
                    let p = &mut netlist.pins[pin];
                    p.role = format!("{prefix}{lane}");
                    p.port.clone_from(&port_name);
                    p.bit = bit.unwrap_or(u32::try_from(lane).unwrap_or(0));
                    p.signal = signal;
                    p.constant = constant;
                    if let Some(signal) = signal {
                        netlist.signals[signal].sinks.push(pin);
                    }
                }
                if primitive == "RAMB36E1" {
                    let width = params(["WRITE_WIDTH_A", "WRITE_WIDTH_B"][port])
                        .and_then(|v| v.as_int())
                        .unwrap_or(0);
                    narrow_ramb36_inputs(netlist, index, port, width);
                }
            }
            rewired += 1;
        }
        rewired
    }

    /// What every input pin of one placed `RAMB18E1` is to be made to
    /// read, appended to `wants`: the pins a signal reaches are left
    /// alone, every other one gets a [`TiePolicy`]. `is_routed` says which
    /// signals reach their pins; before routing it is "all of them", which
    /// is what [`XrayFabric::block_ram_ground_fans`] asks.
    #[allow(clippy::too_many_arguments, reason = "one instance's whole context")]
    fn instance_wants(
        model: LutRamModel,
        params: &dyn Fn(&str) -> Option<AttrValue>,
        instance: &crate::fpga::place::Instance,
        site: &crate::fpga::arch::ArchSite,
        netlist: &Netlist,
        is_routed: &dyn Fn(usize) -> bool,
        wants: &mut Vec<Want>,
    ) -> Result<(), String> {
        // Every input pin of the bel: routed, a constant, or unconnected.
        let mut given: HashMap<&str, Option<Bit>> = HashMap::new();
        for pin in &instance.pins {
            let pin = &netlist.pins[*pin];
            if pin.output {
                continue;
            }
            let value = match (pin.signal, pin.constant) {
                (Some(signal), _) if is_routed(signal) => None,
                (Some(_), _) => {
                    return Err(format!(
                        "`{}` carries a signal the router did not route",
                        pin.role
                    ));
                }
                (None, Some(bit)) => Some(bit),
                (None, None) => continue,
            };
            given.insert(pin.role.as_str(), value);
        }
        // The data bits some port's output is routed from: a `din<i>`
        // outside this set stores a value nobody reads.
        let read: HashSet<u32> = instance
            .pins
            .iter()
            .map(|pin| &netlist.pins[*pin])
            .filter(|pin| pin.output)
            .filter(|pin| pin.signal.is_some_and(is_routed))
            .filter_map(|pin| pin.role.split_once("_dout"))
            .filter_map(|(_, bit)| bit.parse::<u32>().ok())
            .collect();
        // Whether the port a role belongs to is ever enabled: its
        // enable is routed or tied to one.
        let enabled = |role: &str| -> bool {
            let port = role.strip_prefix(TIE_ONLY).unwrap_or(role);
            let Some((port, _)) = port.split_once('_') else {
                return true;
            };
            !matches!(
                given.get(format!("{port}_en").as_str()),
                None | Some(Some(Bit::Zero))
            )
        };
        let held = held_by_inversion(model, params, instance, netlist);
        for (role, node) in &site.pins {
            if is_output(role) {
                continue;
            }
            if INVERTED_LOW
                .iter()
                .any(|(r, pin)| r == role && held.contains(pin))
            {
                wants.push((TiePolicy::One, instance.name.clone(), role.clone(), *node));
                continue;
            }
            // An address bit below the width mode's own range, or a data
            // bit nothing stores or nothing reads, is one that does not matter,
            // whether the netlist gives it a constant or nothing at all,
            // so it must not be able to fail a build: see
            // [`unused_address_bit`] and [`unused_data_bit`].
            let idle = unused_address_bit(role, &params) || unused_data_bit(role, &params, &read);
            // Vivado ties such a bit to its `VCC`, which is the
            // interconnect's own default: no pip, and no ground fan spent
            // (`testdata/fpga/vivado/bram/ib_1024x18`: `ADDRARDADDR[3:0]`
            // and the read port's `DIBDI` are on `<const1>`).
            if idle
                && model == LutRamModel::Vivado
                && !matches!(given.get(role.as_str()), Some(None))
            {
                continue;
            }
            let want = match given.get(role.as_str()) {
                Some(None) => continue,
                Some(Some(Bit::Zero)) if idle => TiePolicy::Idle,
                Some(Some(Bit::Zero)) => TiePolicy::Zero,
                Some(Some(Bit::One)) => TiePolicy::One,
                Some(Some(other)) => {
                    return Err(format!("`{role}` is the constant `{other:?}`"));
                }
                None if idle => TiePolicy::Idle,
                None => match tie_policy(role) {
                    // A port whose enable is tied low reads and writes
                    // nothing, so its address is as idle as data.
                    TiePolicy::Zero if !role.ends_with("_en") && !enabled(role) => TiePolicy::Idle,
                    // Vivado grounds an output register's clock enable that
                    // nothing drives (`ib_1024x18`: `REGCEAREGCE` and
                    // `REGCEB` on its `<const0>`).
                    TiePolicy::Leave
                        if model == LutRamModel::Vivado && role.ends_with("_regce") =>
                    {
                        TiePolicy::Zero
                    }
                    other => other,
                },
            };
            if want == TiePolicy::Idle && model == LutRamModel::Vivado {
                continue;
            }
            if want != TiePolicy::Leave {
                wants.push((want, instance.name.clone(), role.clone(), *node));
            }
        }
        Ok(())
    }

    /// The ground fans the block RAMs' zero ties will need, for the router
    /// to leave alone.
    ///
    /// A zero on a block RAM's input is `GND_WIRE -> GFAN<n> -> the input`
    /// in the interconnect tile the input enters from, and a tile has two
    /// such fans. They are ordinary wires to the router, which spends them
    /// on signals as hops, and the tie, made after routing, then finds both
    /// taken: a block RAM's reset arrives on a `CTRL` wire, which has no
    /// default of one to fall back on either. The ChipWhisperer-Husky's
    /// FIFOs failed exactly so. Asked before routing, with every signal
    /// taken to reach its pins, this names every fan that could serve a pin
    /// [`TiePolicy::Zero`] will tie, so that none of them carries a signal.
    /// A fan serves any number of zero ties at once, so reserving it costs
    /// the tie nothing.
    ///
    /// # Errors
    ///
    /// As [`XrayFabric::configure_block_rams`], for an instance it would
    /// refuse anyway.
    pub fn block_ram_ground_fans(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
    ) -> Result<Vec<NodeId>, BlockRamError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(Vec::new());
        };
        let mut wants: Vec<Want> = Vec::new();
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.primitive == super::dsp::DSP_PRIMITIVE {
                if let Some(site) = placement.site_of(index) {
                    let cell = m.cells.get(instance.cell);
                    let params = |name: &str| cell.and_then(|c| c.params.get(name)).cloned();
                    super::dsp::wants(
                        &params,
                        instance,
                        &graph.sites[site],
                        netlist,
                        &|_| true,
                        &mut wants,
                    )
                    .map_err(|message| BlockRamError {
                        instance: instance.name.clone(),
                        message,
                    })?;
                }
                continue;
            }
            if !is_block_ram(&instance.primitive) {
                if let Some(site) = placement.site_of(index) {
                    io_logic_wants(instance, &graph.sites[site], netlist, &|_| true, &mut wants);
                    super::config::wants(instance, &graph.sites[site], netlist, &mut wants);
                }
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let cell = m.cells.get(instance.cell);
            let params = |name: &str| cell.and_then(|c| c.params.get(name)).cloned();
            let model = self.block_ram.model;
            Self::instance_wants(
                model,
                &params,
                instance,
                site,
                netlist,
                &|_| true,
                &mut wants,
            )
            .map_err(|message| BlockRamError {
                instance: instance.name.clone(),
                message,
            })?;
        }
        let mut fans: Vec<NodeId> = Vec::new();
        // The Vivado model closes a block RAM's tied wires to the router; a
        // DSP's and an IO register's are as they were.
        let ours: HashSet<&str> = netlist
            .instances
            .iter()
            .filter(|i| is_block_ram(&i.primitive))
            .map(|i| i.name.as_str())
            .collect();
        for (want, instance, role, node) in &wants {
            let vivado =
                self.block_ram.model == LutRamModel::Vivado && ours.contains(instance.as_str());
            // A shift register's enable reads one because nothing drives its
            // wire, and that wire leads to a bounce the router likes: closed
            // to every signal, as a ground fan is.
            if *want == TiePolicy::One && role == super::srl::CONSTANT_ENABLE {
                fans.extend(fixed_chain(graph, *node));
                continue;
            }
            // A one's wires, and the pip's source a one through a pip will
            // take, closed to the router as a zero's are: a signal through
            // either would be on the pin.
            if *want == TiePolicy::One && vivado {
                let chain = fixed_chain(graph, *node);
                let vcc = &self.block_ram.vcc_defaults;
                let own = chain.iter().any(|n| reads_vcc(&self.arch, graph, vcc, *n));
                let behind = if own {
                    Vec::new()
                } else {
                    vcc_source(
                        &self.arch,
                        graph,
                        vcc,
                        &HashSet::new(),
                        &Ties::default(),
                        &chain,
                    )
                    .map(|(_, behind)| behind)
                    .unwrap_or_default()
                };
                for n in chain.into_iter().chain(behind) {
                    if !fans.contains(&n) {
                        fans.push(n);
                    }
                }
                continue;
            }
            if *want != TiePolicy::Zero {
                continue;
            }
            // The wires a tie will hold, closed to the router too in the
            // Vivado model: a signal the router walked through one of
            // them reaches the pin with no switch in its way, which is how
            // Reticle's signals reached `WEBWE[7:4]` where Vivado grounds
            // them (`docs/fpga-vivado-parity.md`). The legacy model keeps
            // its routes.
            if vivado {
                for behind in fixed_chain(graph, *node) {
                    if !fans.contains(&behind) {
                        fans.push(behind);
                    }
                }
            }
            for behind in fixed_chain(graph, *node) {
                for pip in graph.incoming(behind) {
                    if graph.pip_bits(*pip).is_empty() {
                        continue;
                    }
                    let fan = graph.pip(*pip).from;
                    let grounded = graph.incoming(fan).iter().any(|p| {
                        !graph.pip_bits(*p).is_empty()
                            && graph.wire(graph.pip(*p).from).name == "GND_WIRE"
                    });
                    if grounded && !fans.contains(&fan) {
                        fans.push(fan);
                    }
                }
            }
        }
        Ok(fans)
    }

    /// Configures every placed `RAMB18E1`: its mode features and a value
    /// on every input the
    /// design does not drive. Run after routing and after
    /// [`bitstream::generate`](crate::fpga::bitstream::generate), which has
    /// already written the contents.
    ///
    /// See the module documentation for what each part rests on.
    ///
    /// # Errors
    ///
    /// [`BlockRamError`] for a mode [`mode_features`] does not describe, a
    /// feature the database has not got, an input that cannot be given
    /// the value it needs, or a bit that falls outside its tile.
    #[allow(clippy::too_many_arguments)]
    pub fn configure_block_rams(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        routing: &Routing,
        bits: &mut Bitstream,
    ) -> Result<BlockRamReport, BlockRamError> {
        let mut report = BlockRamReport::default();
        let Some(m) = design.modules.get(module) else {
            return Ok(report);
        };
        let used: HashSet<NodeId> = routing
            .routes()
            .flat_map(|route| route.nodes.iter().copied())
            .collect();
        let mut ties = Ties::default();
        let mut wants: Vec<Want> = Vec::new();

        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.primitive == super::dsp::DSP_PRIMITIVE {
                let fail = |message: String| BlockRamError {
                    instance: instance.name.clone(),
                    message,
                };
                let Some(site) = placement.site_of(index) else {
                    return Err(fail("it was not placed".to_owned()));
                };
                let site = &graph.sites[site];
                let cell = m.cells.get(instance.cell);
                let params = |name: &str| cell.and_then(|c| c.params.get(name)).cloned();
                let routed = |signal: usize| routing.route(signal).is_some();
                report.dsp_features +=
                    self.configure_dsp(&params, instance, site, netlist, &routed, bits)?;
                report.dsps += 1;
                super::dsp::wants(&params, instance, site, netlist, &routed, &mut wants)
                    .map_err(fail)?;
                continue;
            }
            if !is_block_ram(&instance.primitive) {
                if let Some(site) = placement.site_of(index) {
                    io_logic_wants(
                        instance,
                        &graph.sites[site],
                        netlist,
                        &|signal| routing.route(signal).is_some(),
                        &mut wants,
                    );
                    // A configuration primitive's open and tied inputs, as
                    // Vivado ties them: see `super::config`.
                    super::config::wants(instance, &graph.sites[site], netlist, &mut wants);
                }
                continue;
            }
            let fail = |message: String| BlockRamError {
                instance: instance.name.clone(),
                message,
            };
            let Some(site) = placement.site_of(index) else {
                return Err(fail("it was not placed".to_owned()));
            };
            let site = &graph.sites[site];
            let tile_type = self.arch.tile_types[site.tile_type].name.as_str();
            let Some(table) = self.block_ram.features.get(tile_type) else {
                return Err(fail(format!(
                    "it sits in a `{tile_type}` tile, which has no block RAM features"
                )));
            };

            let cell = m.cells.get(instance.cell);
            let params = |name: &str| cell.and_then(|c| c.params.get(name)).cloned();
            let model = self.block_ram.model;
            let held = held_by_inversion(model, &params, instance, netlist);
            let mut features = if instance.primitive == "RAMB36E1" {
                ramb36_features(&params, model, &held).map_err(fail)?
            } else {
                mode_features_in(&params, model, &held)
                    .map_err(fail)?
                    .into_iter()
                    .map(|feature| format!("{}.{feature}", site.bel))
                    .collect()
            };
            // The tile's FIFO flag offsets, which Vivado writes as zero (all
            // thirteen `Z` bits set) beside every block RAM it builds on the
            // lower half, the one a `FIFO18E1` would take, or on both
            // (`testdata/fpga/vivado/bram/`, every design), and not beside
            // one on the upper half alone (`ib_1024x18` on Reticle's
            // placement, `docs/fpga-vivado-parity.md`). None of them is a
            // FIFO and UG473 gives a RAM no use for the offsets, so this is
            // Vivado's habit followed, and harmless on that word.
            // Read 36 wide in `SDP` on the upper half, Vivado sets port A's
            // 18 beside it, which it does not on the lower (`b18_sdp36` on
            // its own placement, `RAMB18_Y0`, and on Reticle's, `RAMB18_Y1`),
            // and does on both halves of a `RAMB36E1` read 72 wide.
            let sdp = matches!(params("RAM_MODE"), Some(AttrValue::String(m)) if m == "SDP");
            let read36 = params("READ_WIDTH_A").and_then(|v| v.as_int()) == Some(36);
            if model == LutRamModel::Vivado && sdp && read36 && site.bel == "RAMB18_Y1" {
                features.push("RAMB18_Y1.READ_WIDTH_A_18".to_owned());
            }
            if model == LutRamModel::Vivado && site.bel != "RAMB18_Y1" {
                for name in ["ZALMOST_EMPTY_OFFSET", "ZALMOST_FULL_OFFSET"] {
                    features.extend((0..13).map(|i| format!("{name}[{i}]")));
                }
            }
            for full in features {
                let Some(ones) = table.get(&full) else {
                    return Err(fail(format!("the database has no feature `{full}`")));
                };
                for bit in ones {
                    bits.set(site.tile, *bit).map_err(|e| fail(e.to_string()))?;
                }
                report.features += 1;
            }

            Self::instance_wants(
                self.block_ram.model,
                &params,
                instance,
                site,
                netlist,
                &|signal| routing.route(signal).is_some(),
                &mut wants,
            )
            .map_err(fail)?;
            report.rams += 1;
        }

        // The ties that matter first, so that a fan wire the router left
        // free goes to a write enable before it goes to a data bit nobody
        // writes.
        wants.sort_by_key(|(want, ..)| matches!(want, TiePolicy::Idle | TiePolicy::ClockZero));
        for (want, instance, role, node) in wants {
            let fail = |message: String| BlockRamError {
                instance: instance.clone(),
                message,
            };
            let vcc = &self.block_ram.vcc_defaults;
            let value = if want == TiePolicy::One {
                Bit::One
            } else {
                Bit::Zero
            };
            let mut tied = tie(&self.arch, graph, vcc, &used, &mut ties, node, value);
            if want == TiePolicy::ClockZero && tied.is_err() {
                tied = tie_over_bounce(graph, &used, &mut ties, node);
            }
            match (tied, want) {
                (Ok(()), TiePolicy::One) => report.ones += 1,
                (Ok(()), _) => report.zeros += 1,
                (Err(_), TiePolicy::Idle | TiePolicy::ClockZero) => report.idle_left += 1,
                (Err(message), _) => {
                    return Err(fail(format!("tying `{role}` to {value:?}: {message}")));
                }
            }
        }

        for pip in &ties.pips {
            let tile = graph.pip(*pip).tile;
            for bit in graph.pip_bits(*pip) {
                bits.set(tile, *bit).map_err(|e| BlockRamError {
                    instance: graph.wire(graph.pip(*pip).to).full_name(),
                    message: e.to_string(),
                })?;
            }
        }
        report.tie_pips = ties.pips;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::arch::{PipDecl, TileType, WireDecl, WireRef};

    #[test]
    fn the_lower_half_is_the_fifo18_wires_and_the_ports_follow_nextpnr() {
        let pins = bel_pins("BRAM_L", "RAMB18_Y0");
        let wire = |role: &str| -> Vec<&str> {
            pins.iter()
                .filter(|(r, _)| r == role)
                .map(|(_, w)| w.as_str())
                .collect()
        };
        assert_eq!(wire("p0_addr0"), ["BRAM_FIFO18_ADDRARDADDR0"]);
        assert_eq!(wire("p0_addr13"), ["BRAM_FIFO18_ADDRARDADDR13"]);
        assert_eq!(wire("p0_dout15"), ["BRAM_FIFO18_DOADO15"]);
        assert_eq!(wire("p1_din3"), ["BRAM_FIFO18_DIBDI3"]);
        assert_eq!(wire("p0_clk"), ["BRAM_FIFO18_CLKARDCLK"]);
        assert_eq!(wire("p1_en"), ["BRAM_FIFO18_ENBWREN"]);
        assert_eq!(wire("p0_rst"), ["BRAM_FIFO18_RSTRAMARSTRAM"]);
        assert_eq!(wire("p0_we0"), ["BRAM_FIFO18_WEA0", "BRAM_FIFO18_WEA1"]);
        assert_eq!(wire("p0_we1"), ["BRAM_FIFO18_WEA2", "BRAM_FIFO18_WEA3"]);
        assert_eq!(wire("p1_we3"), ["BRAM_FIFO18_WEBWE3"]);
        assert_eq!(wire("tie_p1_we7"), ["BRAM_FIFO18_WEBWE7"]);
        assert_eq!(
            bel_pins("BRAM_R", "RAMB18_Y1")
                .iter()
                .find(|(r, _)| r == "p1_addr2")
                .unwrap()
                .1,
            "BRAM_RAMB18_ADDRBWRADDR2"
        );
        assert!(bel_pins("CLBLL_L", "RAMB18_Y0").is_empty());
        assert!(bel_pins("BRAM_L", "RAMB36").is_empty());
    }

    /// Would catch a policy that left a write enable or the idle port's
    /// enable at the fabric's one; would not catch a wrong *wire* behind
    /// a role, which the test above and the real database do.
    #[test]
    fn what_an_unconnected_input_is_made_to_read() {
        for role in [
            "p0_we0",
            "p1_we3",
            "p0_rst",
            "tie_p0_rstreg",
            "p1_en",
            "p1_addr9",
        ] {
            assert_eq!(tie_policy(role), TiePolicy::Zero, "{role}");
        }
        // A data bit, a parity bit — and `WEBWE[7:4]`, the only tie-only
        // write enables there are, which `TDP` does not use. A write
        // enable the netlist *does* map a port onto stays `Zero`, which is
        // the pair of assertions that matter here.
        for role in ["p0_din2", "p1_dinp1", "tie_p1_we4", "tie_p1_we7"] {
            assert_eq!(tie_policy(role), TiePolicy::Idle, "{role}");
        }
        assert_eq!(tie_policy("tie_p0_addrtie1"), TiePolicy::One);
        assert_eq!(tie_policy("p0_clk"), TiePolicy::Leave);
        assert_eq!(tie_policy("tie_p1_regce"), TiePolicy::Leave);
        assert!(is_output("p0_dout7"));
        assert!(!is_output("p0_din7"));
        // The parity pins are roles of their own since the 18- and 9-bit
        // modes carry data on them.
        assert!(is_output("p1_doutp1"));
        assert!(!is_output("p1_dinp1"));
    }

    /// Which address bits a width mode leaves unread, which is what stops
    /// a contended ground fan failing a build over one.
    ///
    /// What this would catch: a width whose low bit count is wrong, and a
    /// role name parsed as the wrong port. What it would **not** catch:
    /// that the Xilinx library really does ignore those bits — that is
    /// quoted, and `src/fpga/devices/xc7.dev`'s `addr N` clauses quote the
    /// same thing from the other side.
    #[test]
    fn an_address_bit_below_the_width_modes_range_is_unread() {
        let wide = params(&[
            ("READ_WIDTH_A", AttrValue::Int(18)),
            ("WRITE_WIDTH_A", AttrValue::Int(18)),
            ("READ_WIDTH_B", AttrValue::Int(9)),
            ("WRITE_WIDTH_B", AttrValue::Int(9)),
        ]);
        // 18 bits reads `ADDRARDADDR[13:4]`, so 0..3 are unread and 4 is
        // the first bit that matters.
        for bit in 0..4 {
            assert!(unused_address_bit(&format!("p0_addr{bit}"), &wide), "{bit}");
        }
        for bit in 4..14 {
            assert!(
                !unused_address_bit(&format!("p0_addr{bit}"), &wide),
                "{bit}"
            );
        }
        // 9 bits reads one bit further down.
        assert!(unused_address_bit("p1_addr2", &wide));
        assert!(!unused_address_bit("p1_addr3", &wide));
        // Nothing but an address bit, and nothing but the two ports.
        for role in ["p0_we0", "p0_din0", "p0_clk", "tie_p0_addrtie1", "p2_addr0"] {
            assert!(!unused_address_bit(role, &wide), "{role}");
        }
        // A port read at 18 bits and written at 1 reads all fourteen: the
        // narrowest use of either direction decides, which claims the
        // fewest unread bits and is the conservative answer.
        let mixed = params(&[
            ("READ_WIDTH_A", AttrValue::Int(18)),
            ("WRITE_WIDTH_A", AttrValue::Int(1)),
        ]);
        assert!(!unused_address_bit("p0_addr0", &mixed));
        // A port with no width at all is a port this says nothing about.
        let none = params(&[]);
        assert!(!unused_address_bit("p0_addr0", &none));
    }

    /// What this would catch: a write width with the wrong data or parity
    /// count, a parity role parsed as a data bit or the other way round,
    /// the read width deciding instead of the write width, and an unread
    /// bit called idle when the widths differ. What it would **not** catch:
    /// that the block really ignores those bits — that is UG473, quoted —
    /// or a `read` set built wrongly from the netlist, which
    /// `ssd1306_console.v` building at `--place-effort 3` is the check on.
    #[test]
    fn a_data_bit_nothing_stores_or_reads_is_idle() {
        let all_read: HashSet<u32> = (0..16).collect();
        let mixed_list = [
            ("WRITE_WIDTH_A", AttrValue::Int(18)),
            ("READ_WIDTH_B", AttrValue::Int(18)),
            ("WRITE_WIDTH_B", AttrValue::Int(9)),
        ];
        let mixed = params(&mixed_list);
        // 18 bits stores all sixteen data bits and both parity bits.
        for bit in 0..16 {
            assert!(
                !unused_data_bit(&format!("p0_din{bit}"), &mixed, &all_read),
                "{bit}"
            );
        }
        assert!(!unused_data_bit("p0_dinp1", &mixed, &all_read));
        // 9 bits stores `DI[7:0]` and `DIP[0]`; the read width is 18 and
        // does not decide.
        assert!(!unused_data_bit("p1_din7", &mixed, &all_read));
        for bit in 8..16 {
            assert!(
                unused_data_bit(&format!("p1_din{bit}"), &mixed, &all_read),
                "{bit}"
            );
        }
        assert!(!unused_data_bit("p1_dinp0", &mixed, &all_read));
        assert!(unused_data_bit("p1_dinp1", &mixed, &all_read));
        // Mixed widths: an unread bit inside the write width is not
        // claimed, because bit `i` in is not bit `i` out.
        assert!(!unused_data_bit("p0_din13", &mixed, &HashSet::new()));
        // Nothing but a data bit, and nothing but the two ports.
        for role in ["p1_addr0", "p1_we0", "tie_p1_we4", "p1_clk", "p2_din9"] {
            assert!(!unused_data_bit(role, &mixed, &all_read), "{role}");
        }
        // A port with no write width is a port this says nothing about.
        assert!(!unused_data_bit("p0_din15", &params(&[]), &HashSet::new()));

        // The shape that failed a build: a 1024x8 memory in the 16-bit
        // mode, written on port B with `{8'b0, data}` and read on port A's
        // low eight bits. `p1_din13` is stored and never read.
        let sixteen_list = [
            ("READ_WIDTH_A", AttrValue::Int(18)),
            ("WRITE_WIDTH_A", AttrValue::Int(18)),
            ("READ_WIDTH_B", AttrValue::Int(18)),
            ("WRITE_WIDTH_B", AttrValue::Int(18)),
        ];
        let sixteen = params(&sixteen_list);
        let low_byte: HashSet<u32> = (0..8).collect();
        assert!(unused_data_bit("p1_din13", &sixteen, &low_byte));
        assert!(unused_data_bit("p0_din8", &sixteen, &low_byte));
        assert!(!unused_data_bit("p1_din7", &sixteen, &low_byte));
        assert!(!unused_data_bit("p1_din13", &sixteen, &all_read));
    }

    fn params<'a>(list: &'a [(&'a str, AttrValue)]) -> impl Fn(&str) -> Option<AttrValue> + 'a {
        move |name| {
            list.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        }
    }

    /// The 16-bit read-only shape the mapper writes. Would catch a width
    /// written for the wrong port, `ZINIT` written the right way up (it is
    /// inverted) or in nextpnr's order (it is reversed), or a `ZINV`
    /// missing; would not catch a feature Vivado sets that nothing here
    /// knows of, which `tests/fpga_xray_bram_vivado.rs` holds against its
    /// bitstreams.
    #[test]
    fn a_sixteen_bit_block_turns_on_what_vivado_writes() {
        let list = [
            ("READ_WIDTH_A", AttrValue::Int(18)),
            ("WRITE_WIDTH_A", AttrValue::Int(18)),
            ("READ_WIDTH_B", AttrValue::Int(18)),
            ("WRITE_WIDTH_B", AttrValue::Int(18)),
            ("RAM_MODE", AttrValue::String("TDP".to_owned())),
            ("DOA_REG", AttrValue::Int(0)),
        ];
        let features = mode_features(&params(&list)).unwrap();
        for name in [
            "IN_USE",
            "READ_WIDTH_A_18",
            "WRITE_WIDTH_B_18",
            "ZINV_CLKARDCLK",
            "ZINV_RSTREGB",
            "ZINIT_A[0]",
            "ZINIT_B[17]",
            "ZSRVAL_A[5]",
        ] {
            assert!(features.iter().any(|f| f == name), "{name} missing");
        }
        assert!(!features.iter().any(|f| f == "DOA_REG"));
        assert!(!features.iter().any(|f| f.starts_with("WRITE_MODE")));
        assert_eq!(features.len(), 1 + 4 + 10 + 4 * 18);

        // `INIT_A` bit 0 is `ZINIT_A[17]`: the register's top bit first, as
        // Vivado wrote `b18_srval`'s values.
        let inverted = [
            ("IS_CLKARDCLK_INVERTED", AttrValue::Int(1)),
            ("INIT_A", AttrValue::Int(1)),
            ("WRITE_MODE_A", AttrValue::String("READ_FIRST".to_owned())),
        ];
        let features = mode_features(&params(&inverted)).unwrap();
        assert!(!features.iter().any(|f| f == "ZINV_CLKARDCLK"));
        assert!(!features.iter().any(|f| f == "ZINIT_A[17]"));
        assert!(features.iter().any(|f| f == "ZINIT_A[0]"));
        assert!(features.iter().any(|f| f == "ZINIT_A[16]"));
        assert!(features.iter().any(|f| f == "WRITE_MODE_A_READ_FIRST"));
        assert!(!features.iter().any(|f| f.starts_with("READ_WIDTH")));

        // The Vivado model: an output register's clock inverter only beside
        // the register, which the legacy model sets always.
        let vivado = mode_features_in(&params(&list), LutRamModel::Vivado, &[]).unwrap();
        assert!(!vivado.iter().any(|f| f.starts_with("ZINV_REGCLK")));
        assert_eq!(vivado.len(), 1 + 4 + 8 + 4 * 18);
        let registered = [("DOB_REG", AttrValue::Int(1))];
        let vivado = mode_features_in(&params(&registered), LutRamModel::Vivado, &[]).unwrap();
        assert!(vivado.iter().any(|f| f == "ZINV_REGCLKB"));
        assert!(!vivado.iter().any(|f| f == "ZINV_REGCLKARDRCLK"));
        // A reset held through its inverter has no `ZINV`.
        let held = mode_features_in(&params(&list), LutRamModel::Vivado, &["RSTRAMB"]).unwrap();
        assert!(!held.iter().any(|f| f == "ZINV_RSTRAMB"));
        assert!(held.iter().any(|f| f == "ZINV_RSTRAMARSTRAM"));

        // `SDP` 36 wide, as Vivado wrote `b18_sdp36`, with `WRITE_FIRST`
        // its `NO_CHANGE`.
        let sdp = [
            ("RAM_MODE", AttrValue::String("SDP".to_owned())),
            ("READ_WIDTH_A", AttrValue::Int(36)),
            ("WRITE_WIDTH_B", AttrValue::Int(36)),
        ];
        let features = mode_features(&params(&sdp)).unwrap();
        for name in [
            "SDP_READ_WIDTH_36",
            "SDP_WRITE_WIDTH_36",
            "READ_WIDTH_B_18",
            "WRITE_WIDTH_A_18",
            "WRITE_WIDTH_B_18",
            "WRITE_MODE_A_NO_CHANGE",
            "WRITE_MODE_B_NO_CHANGE",
        ] {
            assert!(features.iter().any(|f| f == name), "{name} missing");
        }
        assert!(!features.iter().any(|f| f == "READ_WIDTH_A_18"));
        let ecc = [("RAM_MODE", AttrValue::String("ECC".to_owned()))];
        assert!(mode_features(&params(&ecc)).unwrap_err().contains("SDP"));
        let wide = [("READ_WIDTH_A", AttrValue::Int(36))];
        assert!(mode_features(&params(&wide)).is_err());
    }

    /// A `RAMB36E1`'s features are its halves', each half as wide, a word's
    /// even bits the lower's: what Vivado wrote for `b36_w9` and
    /// `b36_srval`. Would catch a width halved wrongly, `BRAM36_*_1`
    /// missing, an initial value split by halves rather than by bits; the
    /// bitstreams themselves are `tests/fpga_xray_bram_vivado.rs`'s.
    #[test]
    fn a_36_kbit_block_is_its_halves() {
        let nine = [
            ("READ_WIDTH_A", AttrValue::Int(9)),
            ("WRITE_WIDTH_A", AttrValue::Int(9)),
            ("READ_WIDTH_B", AttrValue::Int(36)),
            ("WRITE_WIDTH_B", AttrValue::Int(36)),
            // Bit 0 to the lower half's bit 0, bit 1 to the upper's.
            ("INIT_A", AttrValue::Int(0b10)),
        ];
        let features = ramb36_features(&params(&nine), LutRamModel::Vivado, &[]).unwrap();
        for name in [
            "RAMB36.BRAM36_READ_WIDTH_A_1",
            "RAMB36.BRAM36_WRITE_WIDTH_A_1",
            "RAMB18_Y0.READ_WIDTH_A_4",
            "RAMB18_Y1.WRITE_WIDTH_A_4",
            "RAMB18_Y0.READ_WIDTH_B_18",
            "RAMB18_Y0.ZINIT_A[17]",
        ] {
            assert!(features.iter().any(|f| f == name), "{name} missing");
        }
        assert!(!features.iter().any(|f| f == "RAMB36.BRAM36_READ_WIDTH_B_1"));
        assert!(!features.iter().any(|f| f == "RAMB18_Y1.ZINIT_A[17]"));
        let sdp = [
            ("RAM_MODE", AttrValue::String("SDP".to_owned())),
            ("READ_WIDTH_A", AttrValue::Int(72)),
            ("WRITE_WIDTH_B", AttrValue::Int(72)),
        ];
        let features = ramb36_features(&params(&sdp), LutRamModel::Vivado, &[]).unwrap();
        for half in ["RAMB18_Y0", "RAMB18_Y1"] {
            for name in ["SDP_READ_WIDTH_36", "SDP_WRITE_WIDTH_36", "READ_WIDTH_A_18"] {
                let name = format!("{half}.{name}");
                assert!(features.contains(&name), "{name} missing");
            }
        }
        // The contents: bit `k` of a half is bit `2k` or `2k + 1` of the
        // whole.
        let at = |half: &str, name: &str, index: u32| ramb36_param_of(half, name, index);
        assert_eq!(
            at("RAMB18_Y0", "INIT_00", 0),
            Some(("INIT_00".to_owned(), 0))
        );
        assert_eq!(
            at("RAMB18_Y1", "INIT_00", 0),
            Some(("INIT_00".to_owned(), 1))
        );
        assert_eq!(
            at("RAMB18_Y0", "INIT_00", 128),
            Some(("INIT_01".to_owned(), 0))
        );
        assert_eq!(
            at("RAMB18_Y1", "INIT_3F", 255),
            Some(("INIT_7F".to_owned(), 255))
        );
        assert_eq!(
            at("RAMB18_Y1", "INITP_07", 255),
            Some(("INITP_0F".to_owned(), 255))
        );
        assert_eq!(at("RAMB18_Y0", "INIT_A", 0), None);
    }

    /// Which cell bit each write-enable pin carries, as Vivado wired them.
    /// Would catch a lane order changed or a held-low lane driven; would not
    /// catch Vivado's report being wrong about its own sites.
    #[test]
    fn a_write_enable_reaches_the_pins_vivado_gave_it() {
        let widths = |a: i64, b: i64, mode: &str| {
            vec![
                ("WRITE_WIDTH_A", AttrValue::Int(a)),
                ("WRITE_WIDTH_B", AttrValue::Int(b)),
                ("RAM_MODE", AttrValue::String(mode.to_owned())),
            ]
        };
        let lanes = |primitive: &str, port: usize, list: &[(&str, AttrValue)]| {
            write_enable_lanes(primitive, port, &params(list))
        };
        let (s, n) = (Some, None::<u32>);
        let tdp18 = widths(18, 18, "TDP");
        assert_eq!(lanes("RAMB18E1", 0, &tdp18), [s(0), s(0), s(1), s(1)]);
        assert_eq!(
            lanes("RAMB18E1", 1, &tdp18),
            [s(0), s(0), s(1), s(1), n, n, n, n]
        );
        let tdp9 = widths(9, 9, "TDP");
        assert_eq!(lanes("RAMB18E1", 0, &tdp9), [s(0); 4]);
        let sdp = widths(0, 36, "SDP");
        assert_eq!(lanes("RAMB18E1", 0, &sdp), [n; 4]);
        assert_eq!(
            lanes("RAMB18E1", 1, &sdp),
            [s(0), s(0), s(1), s(1), s(2), s(2), s(3), s(3)]
        );
        let b36 = widths(18, 36, "TDP");
        assert_eq!(
            lanes("RAMB36E1", 0, &b36),
            [s(0), s(1), s(0), s(1), s(0), s(1), s(0), s(1)]
        );
        let b = lanes("RAMB36E1", 1, &b36);
        assert_eq!(b[..4], [s(0), s(1), s(2), s(3)]);
        assert_eq!(b[4..8], [n; 4]);
        assert_eq!(b[8..12], [s(0), s(1), s(2), s(3)]);
        let sdp72 = widths(0, 72, "SDP");
        let b = lanes("RAMB36E1", 1, &sdp72);
        assert_eq!(b[7], s(7));
        assert_eq!(b[15], s(7));
    }

    /// One tile shaped like an interconnect tile beside a block RAM pin:
    /// `GND_WIRE -> GFAN0 -> IMUX0 -> PIN` (the last hop free), plus a
    /// second pin behind a `VCC_WIRE` default and a third behind a wire
    /// with no default and no ground path.
    fn tiny() -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "xc7", 1, 1);
        let mut t = TileType::new("INT_L", "INT_L", 1, 8);
        for w in [
            "GND_WIRE", "GFAN0", "IMUX0", "PIN", "IMUX1", "PIN1", "CTRL9", "PIN2",
        ] {
            t.wires.push(WireDecl {
                name: w.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        let pip = |from: &str, to: &str, bits: Vec<ConfigBit>| PipDecl {
            from: WireRef::local(from),
            to: WireRef::local(to),
            bits,
        };
        t.pips
            .push(pip("GND_WIRE", "GFAN0", vec![ConfigBit::new(0, 1)]));
        t.pips
            .push(pip("GFAN0", "IMUX0", vec![ConfigBit::new(0, 2)]));
        t.pips.push(pip("IMUX0", "PIN", vec![]));
        t.pips.push(pip("IMUX1", "PIN1", vec![]));
        t.pips.push(pip("CTRL9", "PIN2", vec![]));
        arch.tile_types.push(t);
        arch.set_tile(0, 0, 0);
        let graph = arch.build_graph();
        (arch, graph)
    }

    fn node(graph: &RoutingGraph, name: &str) -> NodeId {
        let index = graph.nodes.iter().position(|w| w.name == name).unwrap();
        NodeId::try_from(index).unwrap()
    }

    /// Would catch a tie that sets no ground hop, takes a fan wire a
    /// signal uses, or accepts a one with no default behind it; would not
    /// catch a database whose `default` lines are wrong about silicon.
    #[test]
    fn a_zero_is_made_from_ground_and_a_one_is_a_checked_default() {
        let (arch, graph) = tiny();
        let vcc: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("INT_L".to_owned(), BTreeSet::from(["IMUX1".to_owned()]))]);
        let mut ties = Ties::default();
        let used = HashSet::new();
        tie(
            &arch,
            &graph,
            &vcc,
            &used,
            &mut ties,
            node(&graph, "PIN"),
            Bit::Zero,
        )
        .unwrap();
        assert_eq!(ties.pips.len(), 2, "ground hop and fan hop");
        tie(
            &arch,
            &graph,
            &vcc,
            &used,
            &mut ties,
            node(&graph, "PIN1"),
            Bit::One,
        )
        .unwrap();
        assert_eq!(ties.pips.len(), 2, "a one costs no pip");
        let err = tie(
            &arch,
            &graph,
            &vcc,
            &used,
            &mut ties,
            node(&graph, "PIN2"),
            Bit::One,
        )
        .unwrap_err();
        assert!(err.contains("defaults to VCC_WIRE"), "{err}");
        let err = tie(
            &arch,
            &graph,
            &vcc,
            &used,
            &mut ties,
            node(&graph, "PIN2"),
            Bit::Zero,
        )
        .unwrap_err();
        assert!(err.contains("GND_WIRE"), "{err}");

        // A fan wire a routed signal holds is not taken.
        let mut fresh = Ties::default();
        let busy = HashSet::from([node(&graph, "GFAN0")]);
        assert!(
            tie(
                &arch,
                &graph,
                &vcc,
                &busy,
                &mut fresh,
                node(&graph, "PIN"),
                Bit::Zero
            )
            .is_err()
        );
    }
}
