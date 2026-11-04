//! The clock buffers that are not global: `BUFHCE` and `BUFH` in a clock row,
//! `BUFR` and `BUFIO` beside a bank, and what Vivado writes for them.
//!
//! # Why a `BUFR` here may or may not be one
//!
//! Until this module a `BUFR` that did not divide, and a `BUFMRCE` in front of
//! it, were put on global buffers (`super::bufg`), and every other `BUFR` was
//! refused. That is what the ChipWhisperer-Husky's logic analyser builds with,
//! and what it does can only be judged on a board. So by default that one case
//! stays what it was: [`RegionalClocks::Auto`] gives a real site only to a
//! `BUFR` the global network could not stand in for, and to what drives it.
//! [`RegionalClocks::On`] gives every one a real site, which is what Vivado does.
//!
//! # What was asked, and what Vivado wrote
//!
//! `testdata/fpga/vivado/clocks/`, by `tools/vivado-parity/clocks.sh`, read back
//! by `--decode`. **Checked** against that, on the part at speed grade 1.
//! **Nothing here has been on a board.**
//!
//! | site | features |
//! |---|---|
//! | `BUFHCE_X<side>Y<n>`, `BUFHCE.BUFHCE_X<side>Y<n>.*` | `IN_USE` always; `ZINV_CE` unless the enable is tied low or inverted; `INIT_OUT`; `CE_TYPE.ASYNC` |
//! | `BUFR_Y<k>` | `IN_USE` always; `BUFR_DIVIDE.D<n>` from 1 to 8 and none for `BYPASS`. A tied or a routed `CE` or `CLR` sets nothing |
//! | `BUFIO_Y<k>` | two bits this database does not name, which [`derived_features`] does |
//! | `BUFMRCE_Y<n>` | as a `BUFHCE`, on bits of frame 28 or 29 none of which it names either: `IN_USE` at 187, `ZINV_CE` 188, `INIT_OUT` 189, `CE_TYPE.ASYNC` 191 |
//!
//! `BUFR_Y<k>` and `BUFIO_Y<k>` are numbered `k` up the tile, and their wires are
//! not: `k` 0, 1, 2, 3 are wires 2, 3, 0, 1, as the site list in
//! `tile_type_HCLK_IOI3.json` has it and as `bufr_loc0` to `3` measured, each held
//! to one site by a `LOC` (`WIRE_OF_SITE`).
//!
//! What goes with one, in tiles the buffer is not in:
//!
//! - `HCLK_CMT_CK_BUFHCLK<n>_USED` in the clock management tile of the row and
//!   side, on a `BUFHCE` in use, and `HCLK_CMT_CK_BUFRCLK<w>_USED` on a `BUFR`.
//!   Vivado set them on every design here. Neither has been shown to be needed.
//! - `CLK_HROW_CK_BUFRCLK_<side><w>_ACTIVE` in the row's spine, on a `BUFR`. Also
//!   every design here.
//! - A bit for each regional line of a horizontal clock tile, which no `segbits`
//!   file has: `ENABLE_REGIONAL.HCLK_CK_BUFRCLK<w>`, frames 0 and 1 at bits 23 and
//!   31 ([`derived_features`]), in the tile where a route leaves that line. It was
//!   `?` on every `BUFR` design until it was named.
//! - `HCLK_CMT_CCIO<n>_ACTIVE` in the row's clock management tile, on a `BUFR` or
//!   `BUFIO` on a pad, whose road does not go through that tile.
//!
//! A `BUFMR` is `BUFMRCE` with its enable high, and either site reaches a `BUFR` or
//! `BUFIO` of its own row and those on either side, by one way: through the row's
//! `PHASER_IN`, `CMT_PHASER_IN_C_RCLK2`, onto `HCLK_CMT_MUX_PHSR_PERFCLK<w>` by three
//! bits that are in that row's tile, and `HCLK_IOI_IO_PLL_CLK<w>_DMUX`. Each of
//! those, in each of three rows, was asked of Vivado by a `LOC` on each end.
//! Its input is a pad's `CCIO0`, on the first site only, which sets that line's
//! `HCLK_CMT_CCIO0_ACTIVE` and not the `_USED` a global buffer's road sets, or general interconnect
//! through the row's `CLK1`, on either; Vivado took the second for a pad on
//! `CCIO3` as well. These are what [`derived_features`] names.
//!
//! # What Vivado takes away
//!
//! A `BUFG` whose one load is a `BUFR` is gone in Vivado's netlist by the time it
//! routes, and its `BUFR` reads what the `BUFG` read (`bufr_bufg`). So it is here.
//!
//! # What Vivado refuses
//!
//! A `BUFIO` clocks IO logic and nothing else, and takes a pad or a `BUFMR` and
//! nothing else (it leaves them unrouted, `Route 35-4445`); its loads are in its
//! own bank (`Place 30-126`). A `BUFMR` drives a `BUFR`'s or a `BUFIO`'s input
//! only (`REQP-1936`). [`check_regional_wiring`] says so in this flow's words.

use std::collections::BTreeMap;
use std::fmt;

use super::Feature;
use super::XrayFabric;
use super::sites::{BelPin, ExtraBel};
use crate::fpga::arch::{ConfigBit, ConfigEntry, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::ir::{AttrValue, CellKind, Design, ModuleId};
use crate::logic::Bit;

/// The primitive a `BUFR` becomes when it takes a real site. The `.dev` file
/// declares it apart from `BUFR`, which it still builds on a global buffer.
pub(super) const REGIONAL_BUFR: &str = "BUFR_REGION";
/// The primitive a `BUFMRCE` becomes when it takes a real site.
pub(super) const REGIONAL_BUFMRCE: &str = "BUFMRCE_REGION";

/// Which `BUFR`s get a site of their own. See the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RegionalClocks {
    /// A `BUFR` that divides, is enabled or cleared by a signal or a constant
    /// other than the one that leaves it running, or is on a `BUFMR`, and the
    /// `BUFMRCE`s driving those. The rest stay on global buffers, as they were.
    #[default]
    Auto,
    /// Every `BUFR` and `BUFMRCE`, as Vivado does.
    On,
}

/// Which wire of a bank's clock tile each of its `BUFR_Y<k>` and `BUFIO_Y<k>` is
/// on, from the site list of `tile_type_HCLK_IOI3.json` and measured by a `LOC`.
const WIRE_OF_SITE: [u32; 4] = [2, 3, 0, 1];

/// Whether a kind of placed cell is a clock buffer that is not global: what
/// the rest of the flow asks of one is mostly what it asks of a `gb`.
#[must_use]
pub fn is_regional_kind(kind: &str) -> bool {
    matches!(kind, "bufh" | "bufr" | "bufio" | "bufmr")
}

/// The bels of a tile type that holds regional clock buffers.
pub(super) fn regional_bels(tile_type: &str) -> Vec<ExtraBel> {
    match tile_type {
        "CLK_HROW_BOT_R" | "CLK_HROW_TOP_R" => row_buffers(),
        "HCLK_IOI3" => bank_buffers(),
        "HCLK_CMT" | "HCLK_CMT_L" => multi_region_buffers(),
        _ => Vec::new(),
    }
}

/// A clock management tile's two `BUFMRCE`s. Project X-Ray names none of their
/// bits, and [`derived_features`] does.
fn multi_region_buffers() -> Vec<ExtraBel> {
    (0..2u32)
        .map(|n| {
            let pin = |role: &'static str, wire: String| BelPin { role, wire };
            let feature = |name: &str| format!("BUFMRCE.BUFMRCE_Y{n}.{name}");
            ExtraBel {
                name: format!("BUFMRCE_Y{n}"),
                kind: "bufmr",
                pins: vec![
                    pin("i", format!("HCLK_CMT_BUFMR_INP{n}")),
                    pin("o", format!("HCLK_CMT_BUFMRCE_O{n}")),
                    pin("ce", format!("HCLK_CMT_BUFMRCE_CEINP{n}")),
                ],
                cells: ROW_FEATURES
                    .iter()
                    .map(|tag| (*tag, vec![feature(tag.trim_start_matches('='))]))
                    .collect(),
            }
        })
        .collect()
}

/// A clock row's twenty-four `BUFHCE`s: the pins are the wires the pass-through
/// `sites::horizontal_clock_buffers` joins, and the enable the `always` pseudo-pip
/// `ppips_clk_hrow_*.db` gives it.
fn row_buffers() -> Vec<ExtraBel> {
    let mut out = Vec::new();
    for (side, x) in [("L", 0u32), ("R", 1u32)] {
        for n in 0..12u32 {
            let feature = |name: &str| format!("BUFHCE.BUFHCE_X{x}Y{n}.{name}");
            out.push(ExtraBel {
                name: format!("BUFHCE_X{x}Y{n}"),
                kind: "bufh",
                pins: vec![
                    BelPin {
                        role: "i",
                        wire: format!("CLK_HROW_CK_MUX_OUT_{side}{n}"),
                    },
                    BelPin {
                        role: "o",
                        wire: format!("CLK_HROW_CK_HCLK_OUT_{side}{n}"),
                    },
                    BelPin {
                        role: "ce",
                        wire: format!("CLK_HROW_BUFHCE_CE_{side}{n}"),
                    },
                ],
                cells: ROW_FEATURES
                    .iter()
                    .map(|tag| (*tag, vec![feature(tag.trim_start_matches('='))]))
                    .collect(),
            });
        }
    }
    out
}

/// What [`XrayFabric::configure_regional_buffers`] decides of a `BUFHCE`.
const ROW_FEATURES: [&str; 4] = ["=IN_USE", "=ZINV_CE", "=INIT_OUT", "=CE_TYPE.ASYNC"];

/// A bank's clock tile: four `BUFR`s and four `BUFIO`s.
fn bank_buffers() -> Vec<ExtraBel> {
    let mut out = Vec::new();
    for (k, wire) in WIRE_OF_SITE.iter().enumerate() {
        let pin = |role: &'static str, wire: String| BelPin { role, wire };
        out.push(ExtraBel {
            name: format!("BUFR_Y{k}"),
            kind: "bufr",
            pins: vec![
                pin("i", format!("HCLK_IOI_RCLK_BEFORE_DIV{wire}")),
                pin("o", format!("HCLK_IOI_RCLK_OUT{wire}")),
                pin("ce", format!("HCLK_IOI_BUFR{wire}_CE")),
                pin("clr", format!("HCLK_IOI_BUFR{wire}_CLR")),
            ],
            cells: Vec::new(),
        });
        out.push(ExtraBel {
            name: format!("BUFIO_Y{k}"),
            kind: "bufio",
            pins: vec![
                pin("i", format!("HCLK_IOI_IO_PLL_CLK{wire}")),
                pin("o", format!("HCLK_IOI_BUFIO_O{wire}")),
            ],
            cells: vec![("=IN_USE", vec![format!("BUFIO.BUFIO_Y{k}.IN_USE")])],
        });
    }
    out
}

/// The connections with no bit a tile type has that neither a `segbits` nor a
/// `ppips` file lists: the way a bank's clock multiplexer goes when no bit of its
/// asks otherwise. `tile_type_HCLK_IOI3.json` has the four pips, and Vivado's
/// route takes one from a pad, with no bit set, on every design here with a `BUFR`
/// or `BUFIO` on a pad.
///
/// And a `BUFMRCE`'s output's first hops, through the `PHASER_IN` of whichever row
/// it is driving: `CMT_PHASERREF_PHASERIN_C` takes its own row's, the row above's
/// and the row below's, and Vivado's route takes each with no bit set anywhere
/// that it could be the cause of (`bufmr_x2_r<n>`, `bufmr_x3_r<n>`).
pub(super) fn default_pips(tile_type: &str) -> Vec<(String, String)> {
    match tile_type {
        "HCLK_IOI3" => [("TOP0", 0u32), ("TOP1", 1), ("BOT0", 2), ("BOT1", 3)]
            .iter()
            .map(|(at, n)| {
                (
                    format!("HCLK_IOI_IO_PLL_CLK{n}_DMUX"),
                    format!("HCLK_IOI_I2IOCLK_{at}"),
                )
            })
            .collect(),
        // `HCLK_CMT` has these from `ppips_hclk_cmt.db`, and there is no such file
        // for the tile type on the right of the die.
        "HCLK_CMT_L" => vec![
            (
                "HCLK_CMT_BUFMR_PHASEREF0".to_owned(),
                "HCLK_CMT_BUFMRCE_O0".to_owned(),
            ),
            (
                "HCLK_CMT_BUFMR_PHASEREF1".to_owned(),
                "HCLK_CMT_BUFMRCE_O1".to_owned(),
            ),
            (
                "HCLK_CMT_BUFMRCE_CEINP0".to_owned(),
                "HCLK_CMT_BUFMR_CE0".to_owned(),
            ),
            (
                "HCLK_CMT_BUFMRCE_CEINP1".to_owned(),
                "HCLK_CMT_BUFMR_CE1".to_owned(),
            ),
        ],
        "CMT_TOP_L_UPPER_B" | "CMT_TOP_R_UPPER_B" => [
            "PHASERREF0",
            "PHASERREF1",
            "PHASERREF_ABOVE0",
            "PHASERREF_ABOVE1",
            "PHASERREF_BELOW0",
            "PHASERREF_BELOW1",
        ]
        .iter()
        .map(|from| {
            (
                "CMT_PHASERREF_PHASERIN_C".to_owned(),
                format!("CMT_PHASER_UP_{from}"),
            )
        })
        .collect(),
        _ => Vec::new(),
    }
}

/// Bits Vivado set for these buffers that no `segbits` file names, by the tile
/// type they are in. **Measured**: each `BUFIO` by a `LOC` on it, and each regional
/// line by a `LOC` on the `BUFR` driving it, on both sides of the die.
pub(super) fn derived_features(tile_type: &str) -> Vec<Feature> {
    let bit = |frame, at| ConfigBit::new(frame, at);
    match tile_type {
        "HCLK_IOI3" => [
            (0usize, [bit(36, 16), bit(36, 18)]),
            (1, [bit(37, 16), bit(37, 18)]),
            (2, [bit(36, 31), bit(37, 31)]),
            (3, [bit(36, 21), bit(37, 22)]),
        ]
        .into_iter()
        .map(|(k, ones)| Feature {
            name: format!("BUFIO.BUFIO_Y{k}.IN_USE"),
            ones: ones.to_vec(),
            zeros: Vec::new(),
        })
        .collect(),
        "HCLK_CMT" | "HCLK_CMT_L" => {
            let named = |name: String, ones: Vec<ConfigBit>| Feature {
                name,
                ones,
                zeros: Vec::new(),
            };
            let site = |n: u32, frame: u32, name: &str, at: u32| {
                named(format!("BUFMRCE.BUFMRCE_Y{n}.{name}"), vec![bit(frame, at)])
            };
            let mut out = Vec::new();
            for (n, frame) in [(0u32, 28u32), (1, 29)] {
                out.push(site(n, frame, "IN_USE", 187));
                out.push(site(n, frame, "ZINV_CE", 188));
                out.push(site(n, frame, "INIT_OUT", 189));
                out.push(site(n, frame, "CE_TYPE.ASYNC", 191));
            }
            // A `BUFMRCE`'s input: a pad's own, or the fabric's way in.
            out.push(named(
                "HCLK_CMT_BUFMR_INP0.HCLK_CMT_CCIO0".to_owned(),
                vec![bit(28, 194)],
            ));
            out.push(named(
                "HCLK_CMT_BUFMR_INP0.HCLK_CMT_MUX_CLKINT_1".to_owned(),
                vec![bit(26, 188), bit(27, 174), bit(27, 191), bit(29, 194)],
            ));
            out.push(named(
                "HCLK_CMT_BUFMR_INP1.HCLK_CMT_MUX_CLKINT_1".to_owned(),
                vec![bit(26, 187), bit(26, 190), bit(27, 174), bit(27, 186)],
            ));
            // And how it reaches a bank's buffers, on the `PHASER_IN`'s way out.
            for (perf, ones) in [
                (0u32, [bit(26, 178), bit(27, 179), bit(29, 131)]),
                (1, [bit(26, 179), bit(27, 178), bit(29, 131)]),
                (2, [bit(28, 178), bit(29, 131), bit(29, 179)]),
                (3, [bit(28, 179), bit(29, 131), bit(29, 178)]),
            ] {
                out.push(named(
                    format!("HCLK_CMT_MUX_PHSR_PERFCLK{perf}.HCLK_CMT_PHASERIN_RCLK2"),
                    ones.to_vec(),
                ));
            }
            out
        }
        "HCLK_L" | "HCLK_R" => [
            (0u32, bit(0, 23)),
            (1, bit(1, 23)),
            (2, bit(0, 31)),
            (3, bit(1, 31)),
        ]
        .into_iter()
        .map(|(wire, one)| Feature {
            name: format!("{LEAF_ENABLE}.HCLK_CK_BUFRCLK{wire}"),
            ones: vec![one],
            zeros: Vec::new(),
        })
        .collect(),
        _ => Vec::new(),
    }
}

/// The name of what enables a regional line in a horizontal clock tile. Not
/// `ENABLE_BUFFER`, since that is charged to every tile a line crosses
/// (`sites::wire_enable_features`), where Vivado set this one in the tile its
/// line leaves by alone: [`leaf_enable_bits`] charges it to that pip.
pub(super) const LEAF_ENABLE: &str = "ENABLE_REGIONAL";

/// The bits a pip from wire `from` costs for the regional line it comes off, in
/// a horizontal clock tile: its [`LEAF_ENABLE`]. Nothing else has any.
pub(super) fn leaf_enable_bits(set: &super::FeatureSet, to: &str, from: &str) -> Vec<ConfigBit> {
    if !to.starts_with("HCLK_LEAF_CLK") || !from.starts_with("HCLK_CK_BUFRCLK") {
        return Vec::new();
    }
    set.feature(&format!("{LEAF_ENABLE}.{from}"))
        .map_or_else(Vec::new, |f| f.ones.clone())
}

/// Why a regional clock buffer could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionalError {
    /// A parameter holds something this does not take.
    BadParameter {
        /// The instance.
        instance: String,
        /// The parameter.
        name: String,
        /// Its value.
        value: String,
    },
    /// The database has no feature this needs.
    MissingFeature {
        /// The feature.
        feature: String,
    },
    /// A bit fell outside its tile.
    Bitstream(BitstreamError),
}

impl fmt::Display for RegionalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegionalError::BadParameter {
                instance,
                name,
                value,
            } => write!(
                f,
                "`{instance}`: `{name}` is `{value}`, which this flow does not configure"
            ),
            RegionalError::MissingFeature { feature } => write!(
                f,
                "the chip database has no feature `{feature}`; the database is not the one this \
                 flow was written against"
            ),
            RegionalError::Bitstream(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RegionalError {}

impl From<BitstreamError> for RegionalError {
    fn from(e: BitstreamError) -> Self {
        RegionalError::Bitstream(e)
    }
}

/// Gives the buffers that need one a real site, by their primitive's name, and
/// says how many. See the module docs for which those are. It runs on the mapped
/// design, before the netlist.
pub fn regional_buffers(design: &mut Design, top: ModuleId, mode: RegionalClocks) -> usize {
    let Some(module) = design.modules.get_mut(top) else {
        return 0;
    };
    let text = |value: &AttrValue| {
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_int().map(|i| i.to_string()))
            .unwrap_or_default()
    };
    // The net each cell's output is, and the input a `BUFR` reads.
    let mut regional: Vec<crate::ir::CellId> = Vec::new();
    let mut drivers: BTreeMap<crate::ir::NetId, Vec<crate::ir::CellId>> = BTreeMap::new();
    let mut named_real: Vec<crate::ir::CellId> = Vec::new();
    for (id, cell) in module.cells.iter() {
        if let CellKind::Blackbox(name) = &cell.kind
            && matches!(name.as_str(), "BUFMRCE" | REGIONAL_BUFMRCE)
            && let Some((_, net)) = cell.outputs.first()
        {
            drivers.entry(*net).or_default().push(id);
            if name.as_str() == REGIONAL_BUFMRCE {
                // `BUFMR` is a library cell over this one.
                named_real.push(id);
            }
        }
    }
    let mut renamed = named_real;
    // What a `BUFIO` reads a `BUFMRCE` for has to be one.
    for (_, cell) in module.cells.iter() {
        if let CellKind::Blackbox(name) = &cell.kind
            && name.as_str() == "BUFIO"
            && let Some(net) = cell
                .input("I")
                .and_then(|e| module.exprs.get(e))
                .and_then(crate::ir::Expr::as_net)
        {
            for source in drivers.get(&net).into_iter().flatten() {
                if !renamed.contains(source) {
                    renamed.push(*source);
                }
            }
        }
    }
    for (id, cell) in module.cells.iter() {
        let CellKind::Blackbox(name) = &cell.kind else {
            continue;
        };
        if name.as_str() == "BUFMRCE" && mode == RegionalClocks::On {
            renamed.push(id);
        }
        if name.as_str() != "BUFR" {
            continue;
        }
        let divides = cell
            .params
            .get("BUFR_DIVIDE")
            .is_some_and(|v| !matches!(text(v).as_str(), "1" | "BYPASS"));
        let tied = |port: &str, wanted: u64| {
            cell.input(port)
                .and_then(|e| module.exprs.get(e))
                .and_then(|e| e.as_const())
                .is_some_and(|value| value.to_u64() == Some(wanted))
        };
        let plain = !divides && tied("CE", 1) && tied("CLR", 0);
        if mode == RegionalClocks::On || !plain {
            renamed.push(id);
            regional.push(id);
        }
    }
    // What drives a real `BUFR` from a `BUFMRCE` has to be real too, and so does
    // every other `BUFR` that `BUFMRCE` drives.
    let mut changed = true;
    while changed {
        changed = false;
        for (id, cell) in module.cells.iter() {
            let CellKind::Blackbox(name) = &cell.kind else {
                continue;
            };
            if name.as_str() != "BUFR" {
                continue;
            }
            let Some(net) = cell
                .input("I")
                .and_then(|e| module.exprs.get(e))
                .and_then(crate::ir::Expr::as_net)
            else {
                continue;
            };
            let Some(sources) = drivers.get(&net) else {
                continue;
            };
            let real_source = sources.iter().any(|s| renamed.contains(s));
            if regional.contains(&id) || real_source {
                for s in sources {
                    if !renamed.contains(s) {
                        renamed.push(*s);
                        changed = true;
                    }
                }
                if !renamed.contains(&id) {
                    renamed.push(id);
                    regional.push(id);
                    changed = true;
                }
            }
        }
    }
    for id in &renamed {
        let cell = &mut module.cells[*id];
        let name = match &cell.kind {
            CellKind::Blackbox(name) if name.as_str() == "BUFR" => REGIONAL_BUFR,
            _ => REGIONAL_BUFMRCE,
        };
        cell.kind = CellKind::Blackbox(name.into());
    }
    drop_buffers_in_front(module, &regional);
    renamed.len()
}

/// Takes away each `BUFG` whose one load is one of `bufrs`: see the module docs.
fn drop_buffers_in_front(module: &mut crate::ir::Module, bufrs: &[crate::ir::CellId]) {
    let net_of = |module: &crate::ir::Module, cell: crate::ir::CellId, port: &str| {
        module.cells[cell]
            .input(port)
            .and_then(|e| module.exprs.get(e))
            .and_then(crate::ir::Expr::as_net)
    };
    let readers = |module: &crate::ir::Module, net: crate::ir::NetId| -> usize {
        let cells: usize = module
            .cells
            .iter()
            .flat_map(|(_, cell)| cell.inputs.iter())
            .filter(|(_, e)| module.exprs.get(*e).and_then(crate::ir::Expr::as_net) == Some(net))
            .count();
        let elsewhere = module.assigns.iter().any(|assign| {
            module
                .exprs
                .get(assign.value)
                .and_then(crate::ir::Expr::as_net)
                == Some(net)
        }) || module.ports.iter().any(|port| port.net == net);
        cells + usize::from(elsewhere)
    };
    let mut gone = Vec::new();
    for bufr in bufrs {
        let Some(input) = net_of(module, *bufr, "I") else {
            continue;
        };
        let source = module.cells.iter().find_map(|(id, cell)| {
            let is_bufg = matches!(&cell.kind, CellKind::Blackbox(n) if n.as_str() == "BUFG");
            (is_bufg && cell.outputs.first().is_some_and(|(_, net)| *net == input)).then_some(id)
        });
        let Some(source) = source else { continue };
        if readers(module, input) != 1 {
            continue;
        }
        let Some(upstream) = module.cells[source].input("I") else {
            continue;
        };
        for (port, expr) in &mut module.cells[*bufr].inputs {
            if port.as_str() == "I" {
                *expr = upstream;
            }
        }
        gone.push(source);
    }
    module.cells.retain(|id, _| !gone.contains(&id));
}

/// What Vivado refuses of a regional clock buffer's wiring, as a sentence for a
/// diagnostic. It reads the netlist, so it runs before anything is placed.
///
/// # Errors
///
/// The first thing wrong, with the rule Vivado gave for it.
pub fn check_regional_wiring(netlist: &Netlist) -> Result<(), String> {
    for instance in &netlist.instances {
        let pin = |role: &str| {
            instance
                .pins
                .iter()
                .map(|p| &netlist.pins[*p])
                .find(|p| p.role == role)
        };
        let driver_of = |role: &str| {
            pin(role)
                .and_then(|p| p.signal)
                .and_then(|s| netlist.signals[s].driver)
                .map(|d| {
                    (
                        &netlist.instances[netlist.pins[d].instance],
                        &netlist.pins[d],
                    )
                })
        };
        let loads = || {
            pin("o")
                .and_then(|p| p.signal)
                .into_iter()
                .flat_map(|s| netlist.signals[s].sinks.iter())
                .map(|sink| {
                    (
                        &netlist.instances[netlist.pins[*sink].instance],
                        &netlist.pins[*sink],
                    )
                })
        };
        match instance.kind.as_str() {
            "bufio" => {
                match driver_of("i") {
                    Some((driver, _)) if matches!(driver.kind.as_str(), "io" | "bufmr") => {}
                    Some((driver, _)) => {
                        return Err(format!(
                            "`{}`: a `BUFIO` takes its clock from a clock-capable pad or a `BUFMR`, \
                             and this one is on `{}`, a `{}`; Vivado leaves that unrouted \
                             (Route 35-4445)",
                            instance.name, driver.name, driver.primitive
                        ));
                    }
                    None => {}
                }
                for (load, role) in loads() {
                    if !(matches!(load.kind.as_str(), "ddr_in" | "ddr_out")
                        && matches!(role.role.as_str(), "clk" | "clkb"))
                    {
                        return Err(format!(
                            "`{}`: a `BUFIO` clocks a bank's IO logic and nothing else, and this one \
                             reaches `{}`'s `{}`, a `{}`; Vivado leaves that unrouted (Route 35-4445)",
                            instance.name, load.name, role.port, load.primitive
                        ));
                    }
                }
            }
            "bufmr" => {
                for (load, role) in loads() {
                    if !(matches!(load.kind.as_str(), "bufr" | "bufio") && role.role == "i") {
                        return Err(format!(
                            "`{}`: a `BUFMR` drives only a `BUFR`'s or a `BUFIO`'s input, and this one \
                             reaches `{}`'s `{}`, a `{}` (REQP-1936)",
                            instance.name, load.name, role.port, load.primitive
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

impl XrayFabric {
    /// Configures every regional clock buffer the placement put on a site, and
    /// says how many bits that set. A pass of its own for the reason
    /// [`XrayFabric::configure_global_buffers`] is.
    ///
    /// # Errors
    ///
    /// [`RegionalError`]: a `BUFR_DIVIDE` or `CE_TYPE` that is not one of Vivado's,
    /// or a database without the features.
    #[allow(clippy::too_many_arguments)]
    pub fn configure_regional_buffers(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        routing: &crate::fpga::Routing,
        bits: &mut Bitstream,
    ) -> Result<usize, RegionalError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut count = 0usize;
        count += self.mark_pad_roads(graph, routing, bits)?;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if !is_regional_kind(&instance.kind) {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(bel) = self.arch.tile_types[site.tile_type].bel(&site.bel) else {
                continue;
            };
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let int = |name: &str| {
                params
                    .and_then(|p| p.get(name))
                    .and_then(AttrValue::as_int)
                    .unwrap_or(0)
            };
            let text = |name: &str, default: &str| {
                params.and_then(|p| p.get(name)).map_or_else(
                    || default.to_owned(),
                    |v| {
                        v.as_str()
                            .map(str::to_ascii_uppercase)
                            .or_else(|| v.as_int().map(|i| i.to_string()))
                            .unwrap_or_default()
                    },
                )
            };
            let bad = |name: &str, value: String| RegionalError::BadParameter {
                instance: instance.name.clone(),
                name: name.to_owned(),
                value,
            };
            let mut names: Vec<String> = vec!["IN_USE".to_owned()];
            match instance.kind.as_str() {
                "bufh" | "bufmr" => {
                    // A tie and an inverter meet at the bit: it is set when what
                    // the buffer sees of a tied enable is one, or the enable is a
                    // signal and not inverted.
                    let inverted = int("IS_CE_INVERTED") != 0;
                    let ce = instance
                        .pins
                        .iter()
                        .map(|p| &netlist.pins[*p])
                        .find(|p| p.role == "ce");
                    let enabled = match ce {
                        Some(p) if p.signal.is_some() => !inverted,
                        Some(p) => (p.constant != Some(Bit::Zero)) != inverted,
                        None => !inverted,
                    };
                    if enabled {
                        names.push("ZINV_CE".to_owned());
                    }
                    if int("INIT_OUT") != 0 {
                        names.push("INIT_OUT".to_owned());
                    }
                    match text("CE_TYPE", "SYNC").as_str() {
                        "SYNC" => {}
                        "ASYNC" => names.push("CE_TYPE.ASYNC".to_owned()),
                        other => return Err(bad("CE_TYPE", other.to_owned())),
                    }
                }
                "bufr" => match text("BUFR_DIVIDE", "BYPASS").as_str() {
                    "BYPASS" => {}
                    d @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8") => {
                        names.push(format!("BUFR_DIVIDE.D{d}"));
                    }
                    other => return Err(bad("BUFR_DIVIDE", other.to_owned())),
                },
                _ => {}
            }
            for name in &names {
                let tagged = format!("={name}");
                let entry = bel.config.iter().find_map(|entry| match entry {
                    ConfigEntry::Cell { primitive, bits }
                        if *primitive == tagged || primitive == name =>
                    {
                        Some(bits)
                    }
                    _ => None,
                });
                let Some(ones) = entry else {
                    return Err(RegionalError::MissingFeature {
                        feature: format!("{}.{name}", site.bel),
                    });
                };
                for bit in ones {
                    bits.set(site.tile, *bit)?;
                    count += 1;
                }
            }
            count += self.mark_regional_line(site, &instance.kind, bits)?;
        }
        Ok(count)
    }

    /// `HCLK_CMT_CCIO<n>_ACTIVE` for each pad that reaches a bank's clock tile by
    /// its road: see the module docs. The wire of that name is the road's own, a
    /// few bit-less hops on, and that is how which `n` is read off the graph.
    fn mark_pad_roads(
        &self,
        graph: &RoutingGraph,
        routing: &crate::fpga::Routing,
        bits: &mut Bitstream,
    ) -> Result<usize, RegionalError> {
        let mut count = 0usize;
        for route in routing.routes() {
            for id in &route.pips {
                let pip = graph.pip(*id);
                if !graph.wire(pip.to).name.starts_with("HCLK_IOI_IO_PLL_CLK")
                    || !graph.wire(pip.from).name.starts_with("HCLK_IOI_I2IOCLK")
                {
                    continue;
                }
                let mut seen = vec![pip.from];
                let mut next = 0;
                while next < seen.len() && seen.len() < 64 {
                    let node = seen[next];
                    next += 1;
                    for out in graph.outgoing(node) {
                        let hop = graph.pip(*out);
                        let wire = graph.wire(hop.to);
                        if !wire.name.contains("CCIO") || seen.contains(&hop.to) {
                            continue;
                        }
                        seen.push(hop.to);
                        if wire.name.starts_with("HCLK_CMT_CCIO") {
                            let name = format!("{}_ACTIVE", wire.name);
                            let cmt = self
                                .arch
                                .tile_index_at(hop.tile.0, hop.tile.1)
                                .map(|t| self.arch.tile_types[t].name.clone());
                            if let Some(ones) =
                                cmt.and_then(|t| self.wire_enables.get(&(t, name.clone())))
                            {
                                for bit in ones {
                                    bits.set(hop.tile, *bit)?;
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(count)
    }

    /// The enables Vivado sets for a regional buffer's line in tiles the buffer
    /// is not in: see the module docs. A tile the region did not load has nothing
    /// to set, as the far rebuffer of a global clock has not.
    fn mark_regional_line(
        &self,
        site: &crate::fpga::arch::ArchSite,
        kind: &str,
        bits: &mut Bitstream,
    ) -> Result<usize, RegionalError> {
        let (side_left, used) = match kind {
            "bufh" => {
                let Some((x, n)) = bufhce_numbers(&site.bel) else {
                    return Ok(0);
                };
                (x == 0, format!("HCLK_CMT_CK_BUFHCLK{n}_USED"))
            }
            "bufr" => {
                let Some(k) = site
                    .bel
                    .strip_prefix("BUFR_Y")
                    .and_then(|k| k.parse::<usize>().ok())
                else {
                    return Ok(0);
                };
                let wire = WIRE_OF_SITE.get(k).copied().unwrap_or(0);
                (
                    self.left_of_spine(site.tile),
                    format!("HCLK_CMT_CK_BUFRCLK{wire}_USED"),
                )
            }
            _ => return Ok(0),
        };
        let mut count = 0usize;
        let y = site.tile.1;
        // A row's clock management tile is `HCLK_CMT` on the left and
        // `HCLK_CMT_L` on the right, as `sites` has it.
        let cmt = if side_left { "HCLK_CMT" } else { "HCLK_CMT_L" };
        count += self.set_row_feature(cmt, y, &used, bits)?;
        if let ("bufr", Some(k)) = (
            kind,
            site.bel
                .strip_prefix("BUFR_Y")
                .and_then(|k| k.parse::<usize>().ok()),
        ) {
            let side = if side_left { "L" } else { "R" };
            let wire = WIRE_OF_SITE.get(k).copied().unwrap_or(0);
            let name = format!("CLK_HROW_CK_BUFRCLK_{side}{wire}_ACTIVE");
            for spine in ["CLK_HROW_BOT_R", "CLK_HROW_TOP_R"] {
                count += self.set_row_feature(spine, y, &name, bits)?;
            }
        }
        Ok(count)
    }

    /// Whether a tile is left of the clock spine.
    fn left_of_spine(&self, tile: (u32, u32)) -> bool {
        for x in 0..self.arch.width {
            if self
                .arch
                .tile_index_at(x, tile.1)
                .is_some_and(|t| self.arch.tile_types[t].name.starts_with("CLK_HROW"))
            {
                return tile.0 < x;
            }
        }
        true
    }

    /// Sets a one-component feature of the tile of a type in row `y`, if that
    /// tile is loaded and has it. Those bits are kept beside the wire's pips, so
    /// they are read off any pip touching the wire.
    fn set_row_feature(
        &self,
        tile_type: &str,
        y: u32,
        feature: &str,
        bits: &mut Bitstream,
    ) -> Result<usize, RegionalError> {
        let Some(ones) = self
            .wire_enables
            .get(&(tile_type.to_owned(), feature.to_owned()))
        else {
            return Ok(0);
        };
        let mut count = 0usize;
        for x in 0..self.arch.width {
            if self
                .arch
                .tile_index_at(x, y)
                .is_some_and(|t| self.arch.tile_types[t].name == tile_type)
            {
                for bit in ones {
                    bits.set((x, y), *bit)?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Each regional clock buffer's site, by the clock region it serves, for
    /// [`PlaceOptions::regional_sites`](crate::fpga::place::PlaceOptions): a bank's
    /// buffers serve the region their tile is in, and a clock row's its side's,
    /// which its tile, on the spine, is in none of.
    #[must_use]
    pub fn regional_sites(&self, graph: &RoutingGraph) -> BTreeMap<usize, u16> {
        let regions = self.clock_regions();
        let mut out = BTreeMap::new();
        if regions.is_empty() {
            return out;
        }
        let width = self.arch.width;
        for (index, site) in graph.sites.iter().enumerate() {
            if !is_regional_kind(&site.kind) {
                continue;
            }
            let (x, y) = site.tile;
            let own = regions[(y * width + x) as usize];
            let region = match bufhce_numbers(&site.bel) {
                // Its own tile is on the spine: the region is the neighbour's.
                Some((side, _)) if x > 0 => {
                    let beside = if side == 0 { x - 1 } else { x + 1 };
                    regions
                        .get((y * width + beside) as usize)
                        .copied()
                        .unwrap_or(crate::fpga::place::NO_CLOCK_REGION)
                }
                _ => own,
            };
            out.insert(index, region);
        }
        out
    }
}

/// `BUFHCE_X<side>Y<n>`'s two numbers.
fn bufhce_numbers(bel: &str) -> Option<(u32, u32)> {
    let rest = bel.strip_prefix("BUFHCE_X")?;
    let (x, y) = rest.split_once('Y')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `BUFR` site's wire, as the tile's site list has it: `k` 0 is wire 2.
    /// Would catch the table read the other way, which would put a `BUFR_DIVIDE`
    /// on one buffer and the clock through another. The measurement is the fixture.
    #[test]
    fn a_banks_sites_are_not_numbered_as_its_wires() {
        let bels = bank_buffers();
        let of = |name: &str| bels.iter().find(|b| b.name == name).unwrap();
        assert_eq!(of("BUFR_Y0").pins[0].wire, "HCLK_IOI_RCLK_BEFORE_DIV2");
        assert_eq!(of("BUFR_Y1").pins[1].wire, "HCLK_IOI_RCLK_OUT3");
        assert_eq!(of("BUFR_Y2").pins[2].wire, "HCLK_IOI_BUFR0_CE");
        assert_eq!(of("BUFIO_Y3").pins[1].wire, "HCLK_IOI_BUFIO_O1");
    }
}
