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
        "no free GND_WIRE path reaches `{}` within two pips",
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
                let want = match given.get(role.as_str()) {
                    Some(None) => continue,
                    Some(Some(Bit::Zero)) => TiePolicy::Zero,
                    Some(Some(Bit::One)) => TiePolicy::One,
                    Some(Some(other)) => {
                        return Err(fail(format!("`{role}` is the constant `{other:?}`")));
                    }
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
            "tie_p1_we5",
            "p0_rst",
            "tie_p0_rstreg",
            "p1_en",
            "p1_addr9",
        ] {
            assert_eq!(tie_policy(role), TiePolicy::Zero, "{role}");
        }
        for role in ["p0_din2", "tie_p1_dinp1"] {
            assert_eq!(tie_policy(role), TiePolicy::Idle, "{role}");
        }
        assert_eq!(tie_policy("tie_p0_addrtie1"), TiePolicy::One);
        assert_eq!(tie_policy("p0_clk"), TiePolicy::Leave);
        assert_eq!(tie_policy("tie_p1_regce"), TiePolicy::Leave);
        assert!(is_output("p0_dout7"));
        assert!(!is_output("p0_din7"));
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
