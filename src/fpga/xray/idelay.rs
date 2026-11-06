//! The input delay beside a pad and the controller that calibrates it: an
//! `IDELAYE2` on its `IDELAYE2` site, an `IDELAYCTRL` in the clock row of its
//! bank.
//!
//! # Where they sit, and how they are reached
//!
//! Every IO logic tile has, per pad, an `IDELAYE2` between the pad and the
//! `ILOGICE3`: its `IDATAIN` is `<side>_IDELAY<n>_IDATAIN`, which
//! `ppips_lioi3.db` reaches only from the pad (`LIOI_IDELAY0_IDATAIN ←
//! LIOI_I0`), so the placer puts a delay of a pad beside it
//! (`place::follow_fixed_ends`), and its `DATAOUT` is
//! `<side>_ILOGIC<n>_DDLY`, the `ILOGIC`'s second data input. From there the
//! `ILOGIC` takes it to the fabric (`IDELMUXE3.P0`, on its `O`) or to its own
//! registers (`IFFDELMUXE3.P0`), both declared here as hops that cost those
//! bits ([`pass_throughs`]). The controls arrive from the interconnect by
//! `always` pseudo-pips, half 0's from the interconnect's `_1` wires, and the
//! clock `C` from the interconnect's `CLK1`.
//!
//! A delay of the fabric (`DELAY_SRC("DATAIN")`) has no pad: its `DATAIN` is
//! `IOI_IMUX25`, and it may sit on any delay whose `ILOGIC` and pad are free.
//!
//! The controller has no bits of its own: Vivado's bitstream of one is its
//! reference clock's pip in the clock row (`HCLK_IOI_IDELAYCTRL_REFCLK`), and
//! its reset's and ready's routes, which reach it through the IO logic tiles
//! below and above it (`IOI_IDELAYCTRL_RST ← IOI_IMUX24_0`,
//! `IOI_IDELAYCTRL_RDY → IOI_LOGIC_OUTS22_1`).
//!
//! # The bits, and where each came from
//!
//! **Vivado 2026.1, every setting separately**, since 2026-11-05:
//! `tools/vivado-parity/gen_idelay.py` puts one setting beside each pad of a
//! bank, and `tests/fpga_xray_idelay.rs` holds every `IDELAY` and `ILOGIC`
//! feature, and which control pins are grounded, to
//! `testdata/fpga/vivado/idelay/vivado.txt`.
//!
//! | Setting | Features (`IDELAY_Y<n>.`) |
//! |---|---|
//! | always | `IN_USE`, and `DELAY_SRC_IDATAIN` or `DELAY_SRC_DATAIN` |
//! | `IDELAY_TYPE` | `FIXED` nothing, `VARIABLE` `IDELAY_TYPE_VARIABLE`, **`VAR_LOAD` and `VAR_LOAD_PIPE` the same**, `IDELAY_TYPE_VAR_LOAD` |
//! | `IDELAY_VALUE` | `IDELAY_VALUE[i]` for each bit set and `ZIDELAY_VALUE[i]` for each clear, **under `FIXED` and `VARIABLE` only**: a loaded delay writes every `ZIDELAY_VALUE`, whatever its value |
//! | `HIGH_PERFORMANCE_MODE("TRUE")`, `CINVCTRL_SEL("TRUE")`, `PIPE_SEL("TRUE")` | a bit each |
//! | `IS_DATAIN_INVERTED`, `IS_IDATAIN_INVERTED` | a bit each; **the first whatever the source, the second from the pad only**: of the fabric it writes nothing |
//! | `IS_C_INVERTED` | a bit the database does not name, [`derived_features`]'s; measured with a variable delay |
//! | `SIGNAL_PATTERN`, `REFCLK_FREQUENCY` | **nothing**, at any value tried; 300 MHz at speed grade 1 too, with no word |
//! | to the fabric | `ILOGIC_Y<n>.IDELMUXE3.P0`, and no `ZINV_D` |
//! | to an `IDDR` | `ILOGIC_Y<n>.IFFDELMUXE3.P0`, beside the `IDDR`'s own features, `ZINV_D` among them |
//!
//! Unconnected, `CE`, `INC`, `LD`, `LDPIPEEN`, `REGRST` and `CINVCTRL` are
//! grounded; tied low, so is each, and a bit of `CNTVALUEIN`; tied high, or a
//! `CNTVALUEIN` or `C` left open, nothing is routed, and the pin reads one.
//!
//! Refused, as Vivado refuses them: a delay with no controller (`PLIDC-10`),
//! two controllers with no group between them (`PLIDC-7`), a source
//! unconnected (`REQP-83`, `REQP-84`), and a delay into an `ISERDESE2`'s
//! `DDLY` that the fabric reads as well, unless the serialiser's `IOBDELAY` is
//! `BOTH` (`PDIL-1`; `NONE`, `IBUF` and `IFD` each asked). Stricter than
//! Vivado: an `IDELAY_VALUE` past 31, which it warns of and takes as 0.
//!
//! # Into a serialiser
//!
//! A delay of a pad on an `ISERDESE2`'s `DDLY` (`ddly_*`, since 2026-11-06)
//! writes nothing the two do not write alone: the delay's features, and the
//! serialiser's, `IOBDELAY`'s muxes among them, in every interface, rate and
//! width asked, a master's with its slave, with the pad on `D` or not, and
//! with `IOBDELAY("NONE")`, which Vivado builds although the registers then
//! read the pad. The delay's grounded controls are closed to the router
//! before it routes, as a block RAM's ties are: beside a serialiser it took
//! `BYP_ALT6`, behind `CINVCTRL`, as a hop, and the tie Vivado makes there
//! could not be made.
//!
//! # The controller, and how many
//!
//! Vivado puts a controller in every clock region a delay of its group is in,
//! replicating the one instantiated (`ctl_REPLICATED_0`, `Place 30-1907`) and
//! taking `RDY` as the AND of theirs (`ctl_TOP_AND`), and takes away a
//! controller no delay needs, whose `RDY` is then a one. A delay of a group
//! no controller is in is served by the ungrouped one (`group_mismatch`).
//! [`io_delay_controllers`] does the same before placing, from the pads'
//! regions, and [`XrayFabric::place_io_delay_controllers`] puts each where it
//! serves after. **Where this is not Vivado:** a delay of the fabric, which
//! Vivado put in the top left corner of the die (`IDELAY_X0Y199`) and gave a
//! controller of its own, is moved here into a region that has one.

use std::collections::{BTreeMap, BTreeSet};

use super::XrayFabric;
use super::sites::{BelPin, ExtraBel, PassThrough};
use crate::fpga::arch::{ConfigEntry, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// The delay's primitive.
pub(super) const DELAY: &str = "IDELAYE2";
/// The controller's.
pub(super) const CONTROLLER: &str = "IDELAYCTRL";

/// The site kinds, in [`BelRole`](crate::fpga::BelRole)'s vocabulary.
pub(super) const DELAY_KIND: &str = "iodelay";
/// See [`DELAY_KIND`].
pub(super) const CONTROLLER_KIND: &str = "idelayctrl";

/// The `IDELAY` features [`XrayFabric::configure_io_delays`] may set, tagged
/// with a leading `=` as `super::iologic` tags its own.
const FEATURES: [&str; 21] = [
    "=IN_USE",
    "=IS_C_INVERTED",
    "=DELAY_SRC_IDATAIN",
    "=DELAY_SRC_DATAIN",
    "=IDELAY_TYPE_VARIABLE",
    "=IDELAY_TYPE_VAR_LOAD",
    "=HIGH_PERFORMANCE_MODE",
    "=CINVCTRL_SEL",
    "=PIPE_SEL",
    "=IS_IDATAIN_INVERTED",
    "=IS_DATAIN_INVERTED",
    "=IDELAY_VALUE[0]",
    "=IDELAY_VALUE[1]",
    "=IDELAY_VALUE[2]",
    "=IDELAY_VALUE[3]",
    "=IDELAY_VALUE[4]",
    "=ZIDELAY_VALUE[0]",
    "=ZIDELAY_VALUE[1]",
    "=ZIDELAY_VALUE[2]",
    "=ZIDELAY_VALUE[3]",
    "=ZIDELAY_VALUE[4]",
];

/// The controls grounded when unconnected or tied low, by pin role.
const GROUNDED: [&str; 6] = ["ce", "inc", "ld", "ldpipeen", "regrst", "cinvctrl"];

/// An IO logic tile type's side and whether it is a `_SING` tile.
fn side_of(tile_type: &str) -> Option<(&'static str, bool)> {
    Some(match tile_type {
        "LIOI3" | "LIOI3_TBYTESRC" | "LIOI3_TBYTETERM" => ("LIOI", false),
        "RIOI3" | "RIOI3_TBYTESRC" | "RIOI3_TBYTETERM" => ("RIOI", false),
        "LIOI3_SING" => ("LIOI", true),
        "RIOI3_SING" => ("RIOI", true),
        _ => return None,
    })
}

/// The `IDELAYE2` bels of an IO logic tile type, or nothing. The wires are
/// `tile_type_LIOI3.json`'s site pins, numbered as `super::iologic`'s are: a
/// `_SING` tile's one half is wire 0.
pub(super) fn delay_bels(tile_type: &str) -> Vec<ExtraBel> {
    let Some((side, single)) = side_of(tile_type) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for n in 0..2u32 {
        let w = if single { 0 } else { n };
        let pin = |role: &'static str, wire: String| BelPin { role, wire };
        let mut pins = vec![
            pin("idatain", format!("{side}_IDELAY{w}_IDATAIN")),
            pin("dataout", format!("{side}_IDELAY{w}_DATAOUT")),
            pin("datain", format!("IOI_IDELAY{w}_DATAIN")),
            pin("c", format!("IOI_IDELAY{w}_C")),
            pin("ce", format!("IOI_IDELAY{w}_CE")),
            pin("inc", format!("IOI_IDELAY{w}_INC")),
            pin("ld", format!("IOI_IDELAY{w}_LD")),
            pin("ldpipeen", format!("IOI_IDELAY{w}_LDPIPEEN")),
            pin("regrst", format!("IOI_IDELAY{w}_REGRST")),
            pin("cinvctrl", format!("IOI_IDELAY{w}_CINVCTRL")),
        ];
        const IN: [&str; 5] = [
            "cntvaluein0",
            "cntvaluein1",
            "cntvaluein2",
            "cntvaluein3",
            "cntvaluein4",
        ];
        const OUT: [&str; 5] = [
            "cntvalueout0",
            "cntvalueout1",
            "cntvalueout2",
            "cntvalueout3",
            "cntvalueout4",
        ];
        for (k, (input, output)) in IN.iter().zip(OUT).enumerate() {
            pins.push(pin(input, format!("IOI_IDELAY{w}_CNTVALUEIN{k}")));
            pins.push(pin(output, format!("IOI_IDELAY{w}_CNTVALUEOUT{k}")));
        }
        out.push(ExtraBel {
            name: format!("IDELAY_Y{n}"),
            kind: DELAY_KIND,
            pins,
            cells: FEATURES
                .iter()
                .map(|name| (*name, vec![format!("IDELAY_Y{n}.{}", &name[1..])]))
                .collect(),
        });
    }
    out
}

/// The controller of a clock row's IO tile, `HCLK_IOI3`, or nothing. Its
/// pins are `tile_type_HCLK_IOI3.json`'s; it has no features.
pub(super) fn controller_bels(tile_type: &str) -> Vec<ExtraBel> {
    if tile_type != "HCLK_IOI3" {
        return Vec::new();
    }
    let pin = |role: &'static str, wire: &str| BelPin {
        role,
        wire: wire.to_owned(),
    };
    vec![ExtraBel {
        name: "IDELAYCTRL".to_owned(),
        kind: CONTROLLER_KIND,
        pins: vec![
            pin("refclk", "HCLK_IOI_IDELAYCTRL_REFCLK"),
            pin("rst", "HCLK_IOI_IDELAYCTRL_RST"),
            pin("rdy", "HCLK_IOI_IDELAYCTRL_RDY"),
        ],
        cells: Vec::new(),
    }]
}

/// The `ILOGIC`'s two ways on from a delay: to the fabric, on `O`, which the
/// pad's own way there shares and `IDELMUXE3` chooses between; and to its
/// registers, by way of `D`, which is what the register's pin is here, and
/// `IFFDELMUXE3`. Declared only for a design with a delay
/// (`XrayOptions::io_delays`), since they are wires the router can see.
///
/// `D` from `DDLY` is not a wire on the part: it stands for the register's
/// own input mux, which chooses between the two. A route through it to `O`
/// would cost `IFFDELMUXE3.P0` and `ZINV_D` where Vivado sets `IDELMUXE3.P0`
/// alone, and the router does not take it while the one hop is there.
pub(super) fn pass_throughs(tile_type: &str) -> Vec<PassThrough> {
    let Some((side, single)) = side_of(tile_type) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for n in 0..2u32 {
        let w = if single { 0 } else { n };
        out.push(PassThrough {
            to: format!("IOI_ILOGIC{w}_O"),
            from: format!("{side}_ILOGIC{w}_DDLY"),
            features: vec![format!("ILOGIC_Y{n}.IDELMUXE3.P0")],
        });
        out.push(PassThrough {
            to: register_input(side, w),
            from: format!("{side}_ILOGIC{w}_DDLY"),
            features: vec![format!("ILOGIC_Y{n}.IFFDELMUXE3.P0")],
        });
        out.push(PassThrough {
            to: register_input(side, w),
            from: format!("{side}_ILOGIC{w}_D"),
            features: Vec::new(),
        });
    }
    out
}

/// The wire this flow invents for the input of an `ILOGIC`'s registers,
/// behind the mux between the pad's `D` and the delay's `DDLY`: what an
/// `IDDR`'s `d` is on, in a design with a delay ([`register_inputs`]).
fn register_input(side: &str, wire: u32) -> String {
    format!("{side}_ILOGIC{wire}_IFFD")
}

/// Moves each `IDDR` site's `d` onto [`register_input`], so that a delay's
/// way to the register does not pass through the pad's `D`, from where the
/// pad's own way to the fabric would carry it on as if it were the pad. Only
/// for a design with a delay (`XrayOptions::io_delays`).
pub(super) fn register_inputs(bels: &mut [crate::fpga::arch::BelDecl]) {
    for bel in bels.iter_mut().filter(|b| b.kind == "ddr_in") {
        for (role, wire) in &mut bel.pins {
            if role == "d"
                && let Some(stem) = wire.name.strip_suffix("_D")
            {
                wire.name = format!("{stem}_IFFD");
            }
        }
    }
}

/// The features an `IDELAYE2` needs, from its parameters, without the
/// `IDELAY_Y<n>.` prefix. **Every line is Vivado's**; see the module
/// documentation.
fn delay_features(
    text: &dyn Fn(&str, &str) -> String,
    int: &dyn Fn(&str, i64) -> i64,
) -> Result<Vec<String>, String> {
    let mut out = vec!["IN_USE".to_owned()];
    let fabric = match text("DELAY_SRC", "IDATAIN").as_str() {
        "IDATAIN" => false,
        "DATAIN" => true,
        other => return Err(format!("`DELAY_SRC(\"{other}\")`")),
    };
    out.push(
        if fabric {
            "DELAY_SRC_DATAIN"
        } else {
            "DELAY_SRC_IDATAIN"
        }
        .to_owned(),
    );
    let loaded = match text("IDELAY_TYPE", "FIXED").as_str() {
        "FIXED" => false,
        "VARIABLE" => {
            out.push("IDELAY_TYPE_VARIABLE".to_owned());
            false
        }
        "VAR_LOAD" | "VAR_LOAD_PIPE" => {
            out.push("IDELAY_TYPE_VAR_LOAD".to_owned());
            true
        }
        other => return Err(format!("`IDELAY_TYPE(\"{other}\")`")),
    };
    let value = int("IDELAY_VALUE", 0);
    if !(0..=31).contains(&value) {
        return Err(format!(
            "`IDELAY_VALUE({value})`, which is not 0 to 31; Vivado warns of it and takes 0"
        ));
    }
    let value = if loaded { 0 } else { value };
    for bit in 0..5 {
        out.push(if value >> bit & 1 == 1 {
            format!("IDELAY_VALUE[{bit}]")
        } else {
            format!("ZIDELAY_VALUE[{bit}]")
        });
    }
    for (name, feature) in [
        ("HIGH_PERFORMANCE_MODE", "HIGH_PERFORMANCE_MODE"),
        ("CINVCTRL_SEL", "CINVCTRL_SEL"),
        ("PIPE_SEL", "PIPE_SEL"),
    ] {
        match text(name, "FALSE").as_str() {
            "FALSE" => {}
            "TRUE" => out.push(feature.to_owned()),
            other => return Err(format!("`{name}(\"{other}\")`")),
        }
    }
    match text("SIGNAL_PATTERN", "DATA").as_str() {
        "DATA" | "CLOCK" => {}
        other => return Err(format!("`SIGNAL_PATTERN(\"{other}\")`")),
    }
    // `DATAIN`'s inverter whatever the source, and the pad's only from the
    // pad (`idelay_more`, `datain_idatain_inverted`).
    if int("IS_DATAIN_INVERTED", 0) & 1 == 1 {
        out.push("IS_DATAIN_INVERTED".to_owned());
    }
    if !fabric && int("IS_IDATAIN_INVERTED", 0) & 1 == 1 {
        out.push("IS_IDATAIN_INVERTED".to_owned());
    }
    // A bit no `segbits` file names: see [`derived_features`].
    if int("IS_C_INVERTED", 0) & 1 == 1 {
        out.push("IS_C_INVERTED".to_owned());
    }
    Ok(out)
}

/// The one bit of a delay the database does not name: `IS_C_INVERTED`, a bit
/// of each half that came and went with it and nothing else (`c_inverted`,
/// `idelay_sweep`), frame 34 bit 88 on `IDELAY_Y0` and its mirror, frame 35
/// bit 39, on `IDELAY_Y1`, as every other feature of the two halves is
/// mirrored. A `_SING` tile's half is these, moved by its alias, as
/// measured on both of bank 35's (`RIOI3_SING_X57Y100` 34_24,
/// `RIOI3_SING_X57Y149` 35_39). **The name is mine**, and says what set it.
pub(super) fn derived_features(tile_type: &str) -> Vec<super::Feature> {
    if !matches!(
        tile_type,
        "LIOI3"
            | "LIOI3_TBYTESRC"
            | "LIOI3_TBYTETERM"
            | "RIOI3"
            | "RIOI3_TBYTESRC"
            | "RIOI3_TBYTETERM"
    ) {
        return Vec::new();
    }
    [("IDELAY_Y0", 34, 88), ("IDELAY_Y1", 35, 39)]
        .into_iter()
        .map(|(half, frame, bit)| super::Feature {
            name: format!("{half}.IS_C_INVERTED"),
            ones: vec![crate::fpga::arch::ConfigBit::new(frame, bit)],
            zeros: Vec::new(),
        })
        .collect()
}

/// Why a delay or a controller could not be configured or placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IoDelayError {
    /// A setting this flow refuses, with Vivado's rule where it has one.
    Unsupported {
        /// The instance.
        instance: String,
        /// What it asks for.
        message: String,
    },
    /// The database has no feature this needs.
    MissingFeature {
        /// The feature.
        feature: String,
    },
    /// A bit fell outside its tile.
    Bitstream(BitstreamError),
}

impl std::fmt::Display for IoDelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoDelayError::Unsupported { instance, message } => write!(f, "`{instance}`: {message}"),
            IoDelayError::MissingFeature { feature } => write!(
                f,
                "the chip database has no `{feature}`; the database is not the one this flow \
                 was written against"
            ),
            IoDelayError::Bitstream(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for IoDelayError {}

impl From<BitstreamError> for IoDelayError {
    fn from(e: BitstreamError) -> Self {
        IoDelayError::Bitstream(e)
    }
}

/// The ties a delay or a controller wants, for `super::bram`'s pass: a
/// control left open or tied low is grounded, as Vivado grounds it, and one
/// tied high is left to read the one it reads unrouted.
pub(super) fn wants(
    instance: &crate::fpga::place::Instance,
    site: &crate::fpga::arch::ArchSite,
    netlist: &Netlist,
    wants: &mut Vec<super::bram::Want>,
) {
    let roles: Vec<String> = match instance.primitive.as_str() {
        DELAY => GROUNDED
            .iter()
            .map(|r| (*r).to_owned())
            .chain((0..5).map(|k| format!("cntvaluein{k}")))
            .collect(),
        CONTROLLER => vec!["rst".to_owned()],
        _ => return,
    };
    for role in roles {
        let pins: Vec<&crate::fpga::place::NetPin> = instance
            .pins
            .iter()
            .map(|p| &netlist.pins[*p])
            .filter(|p| p.role == role)
            .collect();
        // A `CNTVALUEIN` left open is left; every other is grounded.
        let low = if pins.is_empty() {
            !role.starts_with("cntvaluein")
        } else {
            pins.iter()
                .all(|p| p.signal.is_none() && p.constant == Some(Bit::Zero))
        };
        if low && let Some(node) = site.pin(&role) {
            wants.push((
                super::bram::TiePolicy::Zero,
                instance.name.clone(),
                role,
                node,
            ));
        }
    }
}

impl XrayFabric {
    /// Configures every `IDELAYE2` the placement put on a delay site, and says
    /// how many bits that set. A controller has no bits of its own.
    ///
    /// # Errors
    ///
    /// [`IoDelayError`]: a setting the module documentation lists as refused,
    /// or a database without the features.
    pub fn configure_io_delays(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        bits: &mut Bitstream,
    ) -> Result<usize, IoDelayError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut count = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.primitive != DELAY {
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
            let text = |name: &str, default: &str| {
                params
                    .and_then(|p| p.get(name))
                    .and_then(AttrValue::as_str)
                    .unwrap_or(default)
                    .to_ascii_uppercase()
            };
            let int = |name: &str, default: i64| {
                params
                    .and_then(|p| p.get(name))
                    .and_then(AttrValue::as_int)
                    .unwrap_or(default)
            };
            let names =
                delay_features(&text, &int).map_err(|message| IoDelayError::Unsupported {
                    instance: instance.name.clone(),
                    message,
                })?;
            for name in names {
                let tag = format!("={name}");
                let entry = bel.config.iter().find_map(|entry| match entry {
                    ConfigEntry::Cell { primitive, bits } if *primitive == tag => Some(bits),
                    _ => None,
                });
                let Some(ones) = entry else {
                    return Err(IoDelayError::MissingFeature {
                        feature: format!("{}.{name}", site.bel),
                    });
                };
                for bit in ones {
                    bits.set(site.tile, *bit)?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Puts each controller [`io_delay_controllers`] planned in the clock row
    /// of the region it serves, and a delay of the fabric into a region a
    /// controller of its group serves, on a delay whose `ILOGIC` and pad are
    /// free. Run after placement and before routing; says how many it moved.
    ///
    /// # Errors
    ///
    /// A message when a region has no controller site loaded, or two groups
    /// want one region's.
    pub fn place_io_delay_controllers(
        &self,
        plan: &IoDelayPlan,
        netlist: &Netlist,
        graph: &RoutingGraph,
        placement: &mut Placement,
    ) -> Result<usize, String> {
        if plan.controllers.is_empty() {
            return Ok(0);
        }
        let regions = self.clock_regions();
        let width = self.arch.width;
        let region_of = |site: usize| -> Option<u16> {
            let (x, y) = graph.sites[site].tile;
            regions.get((y * width + x) as usize).copied()
        };
        let by_name: BTreeMap<&str, usize> = netlist
            .instances
            .iter()
            .enumerate()
            .map(|(i, inst)| (inst.name.as_str(), i))
            .collect();
        // The controller sites, by region.
        let mut controller_sites: BTreeMap<u16, usize> = BTreeMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            if site.kind == CONTROLLER_KIND
                && let Some(region) = region_of(index)
            {
                controller_sites.entry(region).or_insert(index);
            }
        }
        let mut moved = 0usize;
        // Each group's regions: those planned, or the first fabric delay's.
        let mut served: BTreeMap<&str, Vec<u16>> = BTreeMap::new();
        for c in &plan.controllers {
            if let Some(region) = c.region {
                served.entry(c.group.as_str()).or_default().push(region);
            }
        }
        for (group, delays) in &plan.floating {
            if served.contains_key(group.as_str()) {
                continue;
            }
            let first = delays
                .iter()
                .filter_map(|d| by_name.get(d.as_str()))
                .filter_map(|i| placement.site_of(*i))
                .find_map(region_of);
            if let Some(region) = first {
                served.insert(group.as_str(), vec![region]);
            }
        }
        // The fabric delays, each into a region its group is served in.
        for (group, delays) in &plan.floating {
            let Some(regions_served) = served.get(group.as_str()) else {
                continue;
            };
            for name in delays {
                let Some(&index) = by_name.get(name.as_str()) else {
                    continue;
                };
                let here = placement.site_of(index);
                if here.is_some_and(|s| {
                    region_of(s).is_some_and(|r| regions_served.contains(&r))
                        && self.delay_site_free(graph, placement, s, Some(index))
                }) {
                    continue;
                }
                let target = graph.sites.iter().enumerate().position(|(s, site)| {
                    site.kind == DELAY_KIND
                        && region_of(s).is_some_and(|r| regions_served.contains(&r))
                        && placement.instance_at(s).is_none()
                        && self.delay_site_free(graph, placement, s, None)
                });
                let Some(target) = target else {
                    return Err(format!(
                        "`{name}`: no free delay where its controller is, for a delay of the \
                         fabric"
                    ));
                };
                placement.unplace(index);
                placement.place(index, target);
                moved += 1;
            }
        }
        // The controllers, each to its region's site.
        let mut wanted: Vec<(usize, u16)> = Vec::new();
        for c in &plan.controllers {
            let Some(&index) = by_name.get(c.name.as_str()) else {
                continue;
            };
            let region = c.region.or_else(|| {
                served
                    .get(c.group.as_str())
                    .and_then(|r| r.first().copied())
            });
            let Some(region) = region else {
                continue;
            };
            wanted.push((index, region));
        }
        for (index, _) in &wanted {
            placement.unplace(*index);
        }
        for (index, region) in wanted {
            let name = &netlist.instances[index].name;
            let Some(&site) = controller_sites.get(&region) else {
                return Err(format!(
                    "`{name}`: the clock region its delays are in has no `IDELAYCTRL` site loaded"
                ));
            };
            if let Some(other) = placement.instance_at(site) {
                return Err(format!(
                    "`{name}` and `{}` both serve one clock region, which has one `IDELAYCTRL` \
                     (`{}`)",
                    netlist.instances[other].name, graph.sites[site].name
                ));
            }
            placement.place(index, site);
            moved += 1;
        }
        Ok(moved)
    }

    /// True when a delay site's `ILOGIC` and pad hold nothing but `this`, so
    /// that a delay of the fabric there has the `ILOGIC`'s way out to itself.
    fn delay_site_free(
        &self,
        graph: &RoutingGraph,
        placement: &Placement,
        site: usize,
        this: Option<usize>,
    ) -> bool {
        let at = &graph.sites[site];
        let half = &at.bel[at.bel.len() - 1..];
        let (x, y) = at.tile;
        graph.sites.iter().enumerate().all(|(s, other)| {
            let near = other.tile.1 == y && other.tile.0.abs_diff(x) <= 1;
            let same_half =
                other.bel == format!("ILOGIC_Y{half}") || other.bel == format!("IOB_Y{half}");
            !(near && same_half) || placement.instance_at(s).is_none_or(|i| Some(i) == this)
        })
    }
}

/// One controller of [`IoDelayPlan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedController {
    /// The cell's name.
    pub name: String,
    /// Its `IODELAY_GROUP`, or empty.
    pub group: String,
    /// The clock region it serves, when a pad's delay said which.
    pub region: Option<u16>,
}

/// What [`io_delay_controllers`] decided, for
/// [`XrayFabric::place_io_delay_controllers`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IoDelayPlan {
    /// Every controller left, the replicas among them.
    pub controllers: Vec<PlannedController>,
    /// The delays of the fabric, by the group whose controller serves them.
    pub floating: BTreeMap<String, Vec<String>>,
    /// How many replicas were made, and how many controllers taken away.
    pub replicated: usize,
    /// See [`IoDelayPlan::replicated`].
    pub removed: usize,
    /// How many delays nothing read were taken away.
    pub unread: usize,
}

/// A cell's `IODELAY_GROUP`, an attribute or a parameter, or empty.
fn group_of(cell: &crate::ir::Cell) -> String {
    cell.attrs
        .get("IODELAY_GROUP")
        .or_else(|| cell.params.get("IODELAY_GROUP"))
        .and_then(AttrValue::as_str)
        .unwrap_or("")
        .to_owned()
}

/// Gives every clock region a delay of a group is in a controller of that
/// group, as Vivado does: the instantiated one serves the first, a replica
/// each other, with `RDY` the AND of theirs; a controller no delay needs is
/// taken away and its `RDY` is a one. Refuses what Vivado refuses before
/// placing: a delay no controller serves (`PLIDC-10`), two controllers of no
/// group (`PLIDC-7`), and a delay whose source is unconnected (`REQP-83`,
/// `REQP-84`).
///
/// A delay's region is its pad's, read off a netlist of the design as it
/// stands, so this runs before the one that is placed is built.
///
/// # Errors
///
/// The refusal, with the rule's name.
pub fn io_delay_controllers(
    design: &mut Design,
    top: ModuleId,
    device: &crate::fpga::Device,
    fabric: &XrayFabric,
    graph: &RoutingGraph,
) -> Result<IoDelayPlan, String> {
    let mut plan = IoDelayPlan::default();
    let Some(m) = design.modules.get(top) else {
        return Ok(plan);
    };
    let is = |cell: &crate::ir::Cell, name: &str| matches!(&cell.kind, crate::ir::CellKind::Blackbox(p) if p.as_str() == name);
    if !m
        .cells
        .iter()
        .any(|(_, c)| is(c, DELAY) || is(c, CONTROLLER))
    {
        return Ok(plan);
    }
    // The sources, before anything is built.
    for (_, cell) in m.cells.iter().filter(|(_, c)| is(c, DELAY)) {
        let fabric_source = cell
            .params
            .get("DELAY_SRC")
            .and_then(AttrValue::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("DATAIN"));
        let (port, rule) = if fabric_source {
            ("DATAIN", "REQP-84")
        } else {
            ("IDATAIN", "REQP-83")
        };
        let connected = cell
            .input(port)
            .is_some_and(|e| m.expr(e).as_const().is_none());
        if !connected {
            return Err(format!(
                "`{}`: for `DELAY_SRC` {} the `{port}` input of an `IDELAYE2` must be connected, \
                 which Vivado refuses too ({rule})",
                cell.name,
                if fabric_source { "DATAIN" } else { "IDATAIN" }
            ));
        }
    }
    let netlist = Netlist::build(design, top, device, graph).map_err(|e| e.to_string())?;
    let regions = fabric.clock_regions();
    let width = fabric.arch.width;
    // Each delay's group and, for a pad's, its region.
    let mut delays: BTreeMap<String, Vec<(String, Option<u16>)>> = BTreeMap::new();
    let mut controllers: BTreeMap<String, Vec<crate::ir::CellId>> = BTreeMap::new();
    let m = &design.modules[top];
    for (id, cell) in m.cells.iter() {
        if is(cell, CONTROLLER) {
            controllers.entry(group_of(cell)).or_default().push(id);
        }
    }
    // A delay nothing reads, which Vivado takes away (`idelay_more`), and with
    // it the reason its pad is read.
    let read = |instance: &crate::fpga::place::Instance, roles: &dyn Fn(&str) -> bool| {
        instance.pins.iter().map(|p| &netlist.pins[*p]).any(|p| {
            p.output
                && roles(&p.role)
                && p.signal
                    .is_some_and(|s| !netlist.signals[s].sinks.is_empty())
        })
    };
    let mut unread: BTreeSet<crate::ir::CellId> = BTreeSet::new();
    for instance in netlist.instances.iter().filter(|i| i.primitive == DELAY) {
        if !read(instance, &|_| true) {
            unread.insert(instance.cell);
            continue;
        }
        // A pad read around its delay as well as through it, both to the
        // fabric: the one `ILOGIC` output for two signals, which Vivado's
        // router gives up on (`pad_and_delay_fabric`).
        let to_fabric = |signal: usize, except: usize| {
            netlist.signals[signal].sinks.iter().any(|s| {
                let pin = &netlist.pins[*s];
                pin.instance != except && netlist.instances[pin.instance].kind != "ddr_in"
            })
        };
        let me = netlist
            .instances
            .iter()
            .position(|i| std::ptr::eq(i, instance))
            .unwrap_or(usize::MAX);
        let pin_signal = |role: &str| {
            instance
                .pins
                .iter()
                .map(|p| &netlist.pins[*p])
                .find(|p| p.role == role)
                .and_then(|p| p.signal)
        };
        // A delay into a serialiser's `DDLY` and to the fabric as well: the
        // second goes through the `ILOGIC` as a route-thru that Vivado allows
        // only with `IOBDELAY("BOTH")` (`ddly_and_fabric`, and `IBUF` and
        // `NONE` the same).
        if let Some(out) = pin_signal("dataout")
            && to_fabric(out, me)
        {
            for sink in &netlist.signals[out].sinks {
                let pin = &netlist.pins[*sink];
                let serdes = &netlist.instances[pin.instance];
                if pin.role != "ddly" || serdes.primitive != "ISERDESE2" {
                    continue;
                }
                let iobdelay = m
                    .cells
                    .get(serdes.cell)
                    .and_then(|c| c.params.get("IOBDELAY"))
                    .and_then(AttrValue::as_str)
                    .unwrap_or("NONE")
                    .to_ascii_uppercase();
                if iobdelay != "BOTH" {
                    return Err(format!(
                        "`{}`: its delay `{}` goes to the fabric as well as to its `DDLY`, \
                         which takes `IOBDELAY(\"BOTH\")` and not `{iobdelay}`; Vivado refuses \
                         it too (PDIL-1)",
                        serdes.name, instance.name
                    ));
                }
            }
        }
        if let (Some(pad), Some(out)) = (pin_signal("idatain"), pin_signal("dataout"))
            && to_fabric(out, me)
            && to_fabric(pad, me)
        {
            return Err(format!(
                "`{}`: its pad is read by the fabric both through the delay and around it, \
                 which is one `ILOGIC` output for two signals, and which Vivado cannot route \
                 either (Route 35-7)",
                instance.name
            ));
        }
    }
    for instance in netlist.instances.iter().filter(|i| i.primitive == DELAY) {
        if unread.contains(&instance.cell) {
            continue;
        }
        let Some(cell) = m.cells.get(instance.cell) else {
            continue;
        };
        let region = instance
            .pins
            .iter()
            .map(|p| &netlist.pins[*p])
            .find(|p| p.role == "idatain")
            .and_then(|p| p.signal)
            .and_then(|s| netlist.signals[s].driver)
            .map(|d| &netlist.instances[netlist.pins[d].instance])
            .and_then(|io| io.pin.as_deref())
            .and_then(|ball| fabric.arch.site_of_pin(ball))
            .and_then(|site| graph.site_index(site))
            .and_then(|site| {
                let (x, y) = graph.sites[site].tile;
                regions.get((y * width + x) as usize).copied()
            });
        // A delay of a group no controller is in is the ungrouped one's.
        let mut group = group_of(cell);
        if !controllers.contains_key(&group) && controllers.contains_key("") {
            group.clear();
        }
        delays
            .entry(group)
            .or_default()
            .push((instance.name.clone(), region));
    }
    for (group, list) in &delays {
        let Some(cells) = controllers.get(group) else {
            return Err(format!(
                "`{}`: an `IDELAYE2` and no `IDELAYCTRL`{} to calibrate it, which Vivado refuses \
                 too (PLIDC-10)",
                list[0].0,
                if group.is_empty() {
                    String::new()
                } else {
                    format!(" of group `{group}`")
                }
            ));
        };
        if cells.len() > 1 {
            return Err(format!(
                "{} `IDELAYCTRL`s of {}, which Vivado refuses (PLIDC-7): give each delay and \
                 its controller an `IODELAY_GROUP`",
                cells.len(),
                if group.is_empty() {
                    "no group".to_owned()
                } else {
                    format!("group `{group}`")
                }
            ));
        }
    }
    let m = &mut design.modules[top];
    // Taken away at the end: a cell's id is its place, which a removal moves.
    let mut gone = unread.clone();
    plan.unread = unread.len();
    for (group, cells) in &controllers {
        let Some(list) = delays.get(group) else {
            // No delay: taken away, and its `RDY` is a one.
            for id in cells {
                let rdy = m.cells[*id]
                    .outputs
                    .iter()
                    .find(|(port, _)| port.as_str() == "RDY")
                    .map(|(_, net)| *net);
                let span = m.cells[*id].span;
                // Driven by a lookup table of ones, as mapping drives every
                // constant a cell's data pin reads
                // (`techcells::drive_constant_data`), and not by an
                // assignment: this runs after mapping, and a flip-flop that
                // read the `RDY` took a constant `D` no pass then built a
                // driver for. Its data mux was left with neither input chosen
                // (`idelayctrl_probe`: Vivado set `B5FFMUX.IN_B`, from the
                // bypass input, which reads one unrouted, and this flow
                // nothing).
                if let Some(net) = rdy {
                    let inputs: Vec<(crate::ir::Name, crate::ir::ExprId)> = (0..6)
                        .map(|i| {
                            (
                                crate::ir::Name::new(format!("I{i}")),
                                super::super::primitives::const_expr(
                                    m,
                                    crate::ir::Const::zero(1),
                                    span,
                                ),
                            )
                        })
                        .collect();
                    let lut = super::super::primitives::add_cell(
                        m,
                        "rdy$const1$lut",
                        crate::ir::CellKind::Blackbox(crate::ir::Name::new("LUT6")),
                        inputs,
                        vec![(crate::ir::Name::new("O"), net)],
                        span,
                    );
                    m.cells[lut].params.set(
                        "INIT",
                        crate::ir::AttrValue::Const(crate::ir::Const::ones(64)),
                    );
                }
                plan.removed += 1;
            }
            gone.extend(cells.iter().copied());
            continue;
        };
        let id = cells[0];
        let mut served: Vec<u16> = list.iter().filter_map(|(_, r)| *r).collect();
        served.sort_unstable();
        served.dedup();
        let floating: Vec<String> = list
            .iter()
            .filter(|(_, r)| r.is_none())
            .map(|(n, _)| n.clone())
            .collect();
        if !floating.is_empty() {
            plan.floating.insert(group.clone(), floating);
        }
        let name = m.cells[id].name.as_str().to_owned();
        plan.controllers.push(PlannedController {
            name: name.clone(),
            group: group.clone(),
            region: served.first().copied(),
        });
        if served.len() < 2 {
            continue;
        }
        // A replica for each region past the first, as Vivado names them,
        // and `RDY` the AND of all of theirs.
        let span = m.cells[id].span;
        let rdy = m.cells[id]
            .outputs
            .iter()
            .position(|(port, _)| port.as_str() == "RDY");
        let mut readies = Vec::new();
        if let Some(at) = rdy {
            let net = m.nets.push(crate::ir::Net {
                name: crate::ir::Name::new(format!("{name}$rdy0")),
                ty: crate::ir::Type::bit(),
                kind: crate::ir::NetKind::Wire,
                attrs: crate::ir::Attrs::new(),
                span,
            });
            let original = std::mem::replace(&mut m.cells[id].outputs[at].1, net);
            readies.push((original, net));
        }
        for (k, region) in served.iter().enumerate().skip(1) {
            let mut copy = m.cells[id].clone();
            let copy_name = format!("{name}_REPLICATED_{}", k - 1);
            copy.name = crate::ir::Name::new(copy_name.clone());
            if let Some(at) = rdy {
                let net = m.nets.push(crate::ir::Net {
                    name: crate::ir::Name::new(format!("{copy_name}$rdy")),
                    ty: crate::ir::Type::bit(),
                    kind: crate::ir::NetKind::Wire,
                    attrs: crate::ir::Attrs::new(),
                    span,
                });
                copy.outputs[at].1 = net;
                readies.push((readies[0].0, net));
            }
            m.cells.push(copy);
            plan.controllers.push(PlannedController {
                name: copy_name,
                group: group.clone(),
                region: Some(*region),
            });
            plan.replicated += 1;
        }
        if let Some(&(original, _)) = readies.first() {
            // A six-input table, unused inputs low: an unrouted one reads one.
            let n = readies.len();
            let mask = (1u64 << n) - 1;
            let init = (0..64u64)
                .filter(|i| i & mask == mask)
                .fold(0u64, |acc, i| acc | 1 << i);
            let mut inputs = Vec::new();
            for k in 0..6 {
                let value = match readies.get(k) {
                    Some((_, net)) => m.add_expr(crate::ir::Expr::new(
                        crate::ir::ExprKind::Net(*net),
                        crate::ir::Type::bit(),
                        span,
                    )),
                    None => m.add_expr(crate::ir::Expr::new(
                        crate::ir::ExprKind::Const(crate::ir::Const::zero(1)),
                        crate::ir::Type::bit(),
                        span,
                    )),
                };
                inputs.push((crate::ir::Name::new(format!("I{k}")), value));
            }
            let mut params = crate::ir::Attrs::new();
            params.set(
                "INIT",
                AttrValue::Const(crate::ir::Const::from_u64(init, 64)),
            );
            m.cells.push(crate::ir::Cell {
                name: crate::ir::Name::new(format!("{name}_TOP_AND")),
                kind: crate::ir::CellKind::Blackbox(crate::ir::Name::new("LUT6")),
                inputs,
                outputs: vec![(crate::ir::Name::new("O"), original)],
                params,
                attrs: crate::ir::Attrs::new(),
                span,
            });
        }
    }
    m.cells.retain(|id, _| !gone.contains(&id));
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features(params: &[(&str, &str)]) -> Result<Vec<String>, String> {
        let text = |name: &str, default: &str| {
            params
                .iter()
                .find(|(k, _)| *k == name)
                .map_or(default, |(_, v)| v)
                .to_owned()
        };
        let int = |name: &str, default: i64| {
            params
                .iter()
                .find(|(k, _)| *k == name)
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(default)
        };
        let mut out = delay_features(&text, &int)?;
        out.sort();
        Ok(out)
    }

    /// The table of the module documentation, a few lines of it. Would catch
    /// a value written the wrong way round, a loaded delay keeping its value,
    /// and the inverter of the source not in use set; the whole of it is
    /// `tests/fpga_xray_idelay.rs`'s against Vivado.
    #[test]
    fn the_settings_vivado_wrote() {
        let five = features(&[("IDELAY_VALUE", "5")]).unwrap();
        assert!(five.contains(&"IDELAY_VALUE[0]".to_owned()));
        assert!(five.contains(&"ZIDELAY_VALUE[1]".to_owned()));
        assert!(five.contains(&"IDELAY_VALUE[2]".to_owned()));
        let loaded = features(&[("IDELAY_VALUE", "9"), ("IDELAY_TYPE", "VAR_LOAD_PIPE")]).unwrap();
        assert!(loaded.contains(&"IDELAY_TYPE_VAR_LOAD".to_owned()));
        assert!((0..5).all(|b| loaded.contains(&format!("ZIDELAY_VALUE[{b}]"))));
        let other = features(&[("DELAY_SRC", "DATAIN"), ("IS_IDATAIN_INVERTED", "1")]).unwrap();
        assert!(!other.iter().any(|f| f.starts_with("IS_")));
        let pad = features(&[("IS_DATAIN_INVERTED", "1")]).unwrap();
        assert!(pad.contains(&"IS_DATAIN_INVERTED".to_owned()));
        assert!(features(&[("IDELAY_VALUE", "32")]).is_err());
        assert!(features(&[("IDELAY_TYPE", "SOMETIMES")]).is_err());
    }
}
