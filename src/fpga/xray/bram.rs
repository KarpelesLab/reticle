//! Block RAM on a 7-series fabric: a `RAMB18E1` placed on one half of a
//! `BRAM_L` or `BRAM_R` tile, configured, and given a value on every input
//! the design leaves alone.
//!
//! # What reaches the frames, and from where
//!
//! | Piece | Where it comes from | Checked or quoted |
//! |---|---|---|
//! | which bit is `INIT_xx[n]` | `segbits_bram_l.block_ram.db`, prjxray fuzzer `026-bram-data` | quoted: the fuzzer gave Vivado random 256-bit `INIT_xx` values (`val & (1 << bit)`) and solved for the bits. The generic `ConfigEntry::Param` path carries it; no code here touches it |
//! | which half is `RAMB18_Y0` | prjxray `segmaker.py`'s `name_bram18` (even site `Y`) | quoted, and consistent with the contents bits' word positions; see `sites::block_ram_half` |
//! | the mode features ([`mode_features`]) | nextpnr-xilinx `fasm.cc`, `write_bram_half` | quoted: the feature *names* are the database's, *which ones* a `RAMB18E1` turns on is nextpnr's choice, read from its source |
//! | which tile wire is which site pin ([`bel_pins`]) | the wire names of `ppips_bram_l.db` (`BRAM_FIFO18_ADDRARDADDR0`), and nextpnr-xilinx `pack.cc` for which cell port reaches which site pin (`WEA[0]` to `WEA0` and `WEA1`) | quoted |
//! | what an input nobody drives reads ([`TiePolicy`]) | `ppips_int_l.db`'s `default` lines: every `IMUX`, `BYP_ALT` and `FAN_ALT` reads `VCC_WIRE` unless a pip drives it | the database's statement; how it behaves on silicon is unverified |
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
//! `examples/basys3/ssd1306_console.v` did not build at any of nine
//! placement settings; `examples/basys3/selftest.v` built, then an
//! unrelated edit moved the placement and it did not, then another moved it
//! back and it did — and it builds at all nine today, which is what the
//! same lottery looks like from the lucky side. **A
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
use super::parse::{Ppip, PpipKind};
use crate::fpga::arch::{Arch, ConfigBit, NodeId, PipId, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, param_bit};
use crate::fpga::place::{Netlist, Placement};
use crate::fpga::route::Routing;
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// The primitive this module configures.
const PRIMITIVE: &str = "RAMB18E1";

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
/// features configure the two halves as one 36 kbit block, and nothing
/// places a `RAMB36E1`.
pub(super) fn is_two_part_site(head: &str) -> bool {
    matches!(head, "RAMB18_Y0" | "RAMB18_Y1")
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
pub(super) fn bel_pins(tile_type: &str, prefix: &str) -> Vec<(String, String)> {
    let Some(wires) = wire_prefix(tile_type, prefix) else {
        return Vec::new();
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
            pin(format!("{TIE_ONLY}{p}dinp{i}"), format!("{}{i}", side.dinp));
            pin(
                format!("{TIE_ONLY}{p}addrtie{i}"),
                format!("{}{i}", side.addr_tie),
            );
        }
        pin(format!("{TIE_ONLY}{p}regce"), side.regce.to_owned());
        pin(format!("{TIE_ONLY}{p}rstreg"), side.rstreg.to_owned());
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
/// block landed. `examples/basys3/ssd1306_console.v` failed at **all nine**
/// placement settings tried on `097a1d4`, every one of them on `p0_addr3`;
/// `examples/basys3/selftest.v` is the one `docs/fpga-xray.md` records
/// building, then not, then building again as unrelated edits moved its
/// placement — and it happens to build at all nine settings on `097a1d4`
/// today, which is what a lottery looks like from the lucky side. A bit
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
    if kind.starts_with("addrtie") {
        TiePolicy::One
    } else if kind == "clk" || kind == "regce" {
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
        TiePolicy::Idle
    } else {
        TiePolicy::Zero
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

/// The mode features a `RAMB18E1` with these parameters turns on, without
/// the site prefix, in the order nextpnr-xilinx's `write_bram_half`
/// writes them.
///
/// | Feature | When |
/// |---|---|
/// | `IN_USE` | always |
/// | `READ_WIDTH_A_<w>` and the three like it | the parameter is not 0; `w` is 1, 2, 4, 9 or 18 |
/// | `DOA_REG`, `DOB_REG` | the parameter is 1 |
/// | `ZINV_<pin>` | `IS_<pin>_INVERTED` is not 1 |
/// | `WRITE_MODE_A_READ_FIRST` / `_NO_CHANGE` | that mode; `WRITE_FIRST` is no bit |
/// | `ZINIT_A[i]`, `ZINIT_B[i]`, `ZSRVAL_A[i]`, `ZSRVAL_B[i]` | bit `i` of `INIT_A` (...) is **0**; nextpnr writes all eighteen of each, because it never sets those parameters |
///
/// The contents are not here: they are ordinary indexed features and the
/// generic path carries them.
///
/// # Errors
///
/// A message for a mode this does not describe: `RAM_MODE` other than
/// `TDP` (the 36-bit simple-dual-port mode needs `SDP_*` features this
/// has not been given), a width that is not 0, 1, 2, 4, 9 or 18, or a
/// write mode that is not one of the three.
pub fn mode_features(params: &dyn Fn(&str) -> Option<AttrValue>) -> Result<Vec<String>, String> {
    let mut out = vec!["IN_USE".to_owned()];
    if let Some(mode) = params("RAM_MODE") {
        let AttrValue::String(mode) = &mode else {
            return Err("`RAM_MODE` is not a string".to_owned());
        };
        if mode != "TDP" {
            return Err(format!(
                "`RAM_MODE` is \"{mode}\"; only \"TDP\" is described for this fabric"
            ));
        }
    }
    for name in [
        "READ_WIDTH_A",
        "READ_WIDTH_B",
        "WRITE_WIDTH_A",
        "WRITE_WIDTH_B",
    ] {
        let width = params(name).and_then(|v| v.as_int()).unwrap_or(0);
        match width {
            0 => {}
            1 | 2 | 4 | 9 | 18 => out.push(format!("{name}_{width}")),
            other => {
                return Err(format!(
                    "`{name}` is {other}; the widths described are 1, 2, 4, 9 and 18"
                ));
            }
        }
    }
    for name in ["DOA_REG", "DOB_REG"] {
        if params(name).and_then(|v| v.as_int()).unwrap_or(0) == 1 {
            out.push(name.to_owned());
        }
    }
    for pin in INVERTIBLE {
        let inverted = params(&format!("IS_{pin}_INVERTED"))
            .and_then(|v| v.as_int())
            .unwrap_or(0);
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
            "WRITE_FIRST" => {}
            "READ_FIRST" | "NO_CHANGE" => out.push(format!("{name}_{mode}")),
            other => return Err(format!("`{name}` is \"{other}\"")),
        }
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
                out.push(format!("{feature}[{i}]"));
            }
        }
    }
    Ok(out)
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
}

impl BlockRamTables {
    /// Gathers the tables from every tile type's features and pseudo-pips.
    pub(super) fn new(
        features: &HashMap<String, super::FeatureSet>,
        fixed: &HashMap<String, Vec<Ppip>>,
    ) -> BlockRamTables {
        let mut out = BlockRamTables::default();
        for (tile_type, set) in features {
            if wire_prefix(tile_type, "RAMB18_Y0").is_none() {
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

/// Why a block RAM could not be configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRamError {
    /// The instance.
    pub instance: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for BlockRamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block RAM `{}`: {}", self.instance, self.message)
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

/// Ties one input pin's node to `value`.
///
/// A zero: the first node of the pin's [`fixed_chain`] that a programmable
/// pip reaches from a node which itself is reachable by a programmable pip
/// from a `GND_WIRE` — in an interconnect tile, `GFAN0` or `GFAN1` — and
/// whose source is not carrying a routed signal or a one. Both pips are
/// switched on (the `GND_WIRE` hop once per tile). A one: some node of the
/// chain must be a wire its tile type's `ppips` file says defaults to
/// `VCC_WIRE`, and no node of the chain may be driven by anything.
fn tie(
    arch: &Arch,
    graph: &RoutingGraph,
    vcc_defaults: &BTreeMap<String, BTreeSet<String>>,
    used: &HashSet<NodeId>,
    ties: &mut Ties,
    pin: NodeId,
    value: Bit,
) -> Result<(), String> {
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
        let default = chain.iter().any(|node| {
            vcc_defaults
                .get(tile_type_of(arch, graph, *node))
                .is_some_and(|wires| wires.contains(&graph.wire(*node).name))
        });
        if !default {
            return Err(format!(
                "nothing behind `{}` defaults to VCC_WIRE, and a one has no other source here",
                name(pin)
            ));
        }
        ties.one.extend(chain);
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

impl XrayFabric {
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
        let mut wants: Vec<(TiePolicy, String, String, NodeId)> = Vec::new();

        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.primitive != PRIMITIVE {
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
            for feature in mode_features(&params).map_err(fail)? {
                let full = format!("{}.{feature}", site.bel);
                let Some(ones) = table.get(&full) else {
                    return Err(fail(format!("the database has no feature `{full}`")));
                };
                for bit in ones {
                    bits.set(site.tile, *bit).map_err(|e| fail(e.to_string()))?;
                }
                report.features += 1;
            }

            // Every input pin of the bel: routed, a constant, or unconnected.
            let mut given: HashMap<&str, Option<Bit>> = HashMap::new();
            for pin in &instance.pins {
                let pin = &netlist.pins[*pin];
                if pin.output {
                    continue;
                }
                let value = match (pin.signal, pin.constant) {
                    (Some(signal), _) if routing.route(signal).is_some() => None,
                    (Some(_), _) => {
                        return Err(fail(format!(
                            "`{}` carries a signal the router did not route",
                            pin.role
                        )));
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
                .filter(|pin| pin.signal.is_some_and(|s| routing.route(s).is_some()))
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
            for (role, node) in &site.pins {
                if is_output(role) {
                    continue;
                }
                // An address bit below the width mode's own range, or a data
                // bit nothing stores or nothing reads, is one that does not matter,
                // whether the netlist gives it a constant or nothing at all,
                // so it must not be able to fail a build: see
                // [`unused_address_bit`] and [`unused_data_bit`].
                let idle =
                    unused_address_bit(role, &params) || unused_data_bit(role, &params, &read);
                let want = match given.get(role.as_str()) {
                    Some(None) => continue,
                    Some(Some(Bit::Zero)) if idle => TiePolicy::Idle,
                    Some(Some(Bit::Zero)) => TiePolicy::Zero,
                    Some(Some(Bit::One)) => TiePolicy::One,
                    Some(Some(other)) => {
                        return Err(fail(format!("`{role}` is the constant `{other:?}`")));
                    }
                    None if idle => TiePolicy::Idle,
                    None => match tie_policy(role) {
                        // A port whose enable is tied low reads and writes
                        // nothing, so its address is as idle as data.
                        TiePolicy::Zero if !role.ends_with("_en") && !enabled(role) => {
                            TiePolicy::Idle
                        }
                        other => other,
                    },
                };
                if want != TiePolicy::Leave {
                    wants.push((want, instance.name.clone(), role.clone(), *node));
                }
            }
            report.rams += 1;
        }

        // The ties that matter first, so that a fan wire the router left
        // free goes to a write enable before it goes to a data bit nobody
        // writes.
        wants.sort_by_key(|(want, _, _, _)| *want == TiePolicy::Idle);
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
            let tied = tie(&self.arch, graph, vcc, &used, &mut ties, node, value);
            match (tied, want) {
                (Ok(()), TiePolicy::One) => report.ones += 1,
                (Ok(()), _) => report.zeros += 1,
                (Err(_), TiePolicy::Idle) => report.idle_left += 1,
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
        for role in ["p0_din2", "tie_p1_dinp1", "tie_p1_we4", "tie_p1_we7"] {
            assert_eq!(tie_policy(role), TiePolicy::Idle, "{role}");
        }
        assert_eq!(tie_policy("tie_p0_addrtie1"), TiePolicy::One);
        assert_eq!(tie_policy("p0_clk"), TiePolicy::Leave);
        assert_eq!(tie_policy("tie_p1_regce"), TiePolicy::Leave);
        assert!(is_output("p0_dout7"));
        assert!(!is_output("p0_din7"));
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
        assert!(!unused_data_bit("tie_p0_dinp1", &mixed, &all_read));
        // 9 bits stores `DI[7:0]` and `DIP[0]`; the read width is 18 and
        // does not decide.
        assert!(!unused_data_bit("p1_din7", &mixed, &all_read));
        for bit in 8..16 {
            assert!(
                unused_data_bit(&format!("p1_din{bit}"), &mixed, &all_read),
                "{bit}"
            );
        }
        assert!(!unused_data_bit("tie_p1_dinp0", &mixed, &all_read));
        assert!(unused_data_bit("tie_p1_dinp1", &mixed, &all_read));
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
    /// inverted), or a `ZINV` missing; would not catch nextpnr itself being
    /// wrong about which features Vivado sets.
    #[test]
    fn a_sixteen_bit_block_turns_on_what_nextpnr_writes() {
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

        let inverted = [
            ("IS_CLKARDCLK_INVERTED", AttrValue::Int(1)),
            ("INIT_A", AttrValue::Int(1)),
            ("WRITE_MODE_A", AttrValue::String("READ_FIRST".to_owned())),
        ];
        let features = mode_features(&params(&inverted)).unwrap();
        assert!(!features.iter().any(|f| f == "ZINV_CLKARDCLK"));
        assert!(!features.iter().any(|f| f == "ZINIT_A[0]"));
        assert!(features.iter().any(|f| f == "ZINIT_A[1]"));
        assert!(features.iter().any(|f| f == "WRITE_MODE_A_READ_FIRST"));
        assert!(!features.iter().any(|f| f.starts_with("READ_WIDTH")));

        let sdp = [("RAM_MODE", AttrValue::String("SDP".to_owned()))];
        assert!(mode_features(&params(&sdp)).unwrap_err().contains("TDP"));
        let wide = [("READ_WIDTH_A", AttrValue::Int(36))];
        assert!(mode_features(&params(&wide)).is_err());
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
