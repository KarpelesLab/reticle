//! An IO buffer's pad, configured per buffer: its standard, its drive and
//! slew, its pull, and a differential pair.
//!
//! # Why per buffer
//!
//! Until NewAE's ChipWhisperer-Husky this flow configured one IO standard
//! for the whole design, chosen at load time and attached to every pad as
//! a fixed feature list. The Husky has three on one part — LVCMOS33,
//! LVCMOS25 and LVDS_25 — four of its LVCMOS33 outputs at a drive of 8 mA
//! rather than the default 12, pull-ups on some pads and a pull-down on
//! one. So what a pad's bits are is read here, from the buffer cell's own
//! attributes (`io_standard`, `drive`, `slew`, `pullup`, `pulldown`, set
//! from its constraint by [`crate::fpga::primitives`]), once placement
//! says which pad it is on.
//!
//! # Where every recipe below came from
//!
//! **Vivado**, wherever it could be: NewAE's shipped Husky image decoded
//! through this database, pad by pad against `cwhusky.xdc`. Each line of
//! the table says whether Vivado wrote it or only the database's feature
//! names say it:
//!
//! | Request | Features | Source |
//! |---|---|---|
//! | LVCMOS33 or LVCMOS25 input | `IN_ONLY`, `LVCMOS25_LVCMOS33_LVTTL.IN`, `PULLTYPE.NONE` | Vivado, 24 pads |
//! | LVCMOS33 output, 12 mA | `SLEW.SLOW`, `LVCMOS33_LVTTL.DRIVE.I12_I16`, `PULLTYPE.NONE` | Vivado, 8 pads |
//! | LVCMOS33 output, 8 mA | the same with `LVCMOS33_LVTTL.DRIVE.I12_I8` | Vivado, 4 pads |
//! | LVCMOS25 output, 12 mA | `SLEW.SLOW`, `LVCMOS25.DRIVE.I12`, `PULLTYPE.NONE` | Vivado, 6 pads |
//! | bidirectional | the output's, plus `LVCMOS25_LVCMOS33_LVTTL.IN` and not `IN_ONLY` | Vivado, 11 pads |
//! | a pull-up | `PULLTYPE.PULLUP` in place of `NONE` | Vivado, 9 pads |
//! | a pull-down | no `PULLTYPE` feature at all: `PULLDOWN` is the field with every bit clear | Vivado, 1 pad |
//! | LVDS_25 input pair | both halves `IN_ONLY`, `LVDS_25_SSTL135_SSTL15.IN_DIFF`, `PULLTYPE.NONE`; the tile's `DIFF.ZIBUF_LOW_PWR` when `IBUF_LOW_PWR` is false | Vivado, 9 pairs |
//! | LVDS_25 output pair | the master `LVDS_25.OUT`, `LVDS_25.DRIVE.I_FIXED`, `PULLTYPE.NONE`; the slave `IN_ONLY`, `PULLTYPE.NONE` | Vivado, 1 pair |
//! | other drives (4, 16 mA) and a fast slew | the feature of that name | **the database's names only** |
//!
//! A standard this table has no line for is refused with its name: an IO
//! standard is a voltage, and guessing one is how a board gets damaged.

use std::collections::{BTreeMap, HashMap};

use super::XrayFabric;
use crate::fpga::arch::{ConfigBit, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::ir::{AttrValue, Design, ModuleId};

/// The pad features of every IO tile type, by name, for
/// [`XrayFabric::configure_io`]: the per-half ones (`IOB_Y0.PULLTYPE.NONE`)
/// and the ones that belong to the pair (`DIFF.ZIBUF_LOW_PWR`), which no
/// bel holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IoTables {
    /// Tile type to feature name to the bits that must be one. A feature
    /// with no ones (`PULLTYPE.PULLDOWN`) is kept with an empty list, so
    /// that asking for it is not mistaken for asking for a feature the
    /// database lacks.
    pub features: BTreeMap<String, BTreeMap<String, Vec<ConfigBit>>>,
    /// The standard a buffer whose constraint names none is given.
    pub default_standard: String,
}

impl IoTables {
    /// Gathers the tables from every tile type that has an `IOB_Y0`.
    pub(super) fn new(
        features: &HashMap<String, super::FeatureSet>,
        default_standard: &str,
    ) -> IoTables {
        let mut out = IoTables {
            features: BTreeMap::new(),
            default_standard: default_standard.to_owned(),
        };
        for (tile_type, set) in features {
            if !set.features().iter().any(|f| f.name.starts_with("IOB_Y0.")) {
                continue;
            }
            let map: BTreeMap<String, Vec<ConfigBit>> = set
                .features()
                .iter()
                .map(|f| (f.name.clone(), f.ones.clone()))
                .collect();
            out.features.insert(tile_type.clone(), map);
        }
        out
    }
}

/// Which way a buffer drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    /// The pad drives the fabric (`IBUF`).
    In,
    /// The fabric drives the pad (`OBUF`, `OBUFT`).
    Out,
    /// Both (`IOBUF`).
    InOut,
}

/// A pull on the pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pull {
    /// None, which is `PULLTYPE.NONE`.
    None,
    /// `PULLTYPE.PULLUP`.
    Up,
    /// `PULLTYPE.PULLDOWN`, whose bits are all clear.
    Down,
}

/// What a buffer asks of its pad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct IoRequest {
    /// The standard, as a constraint names it.
    pub standard: String,
    /// Which way.
    pub direction: Direction,
    /// Drive in milliamperes; `None` is the standard's default of 12.
    pub drive: Option<u32>,
    /// `true` for a fast slew rate; the default is slow.
    pub fast: bool,
    /// The pull.
    pub pull: Pull,
    /// `IBUF_LOW_PWR`; the library default is true.
    pub low_power: bool,
}

/// The features a pad needs: on its own half, on the other half of its
/// tile (a differential pair's partner), and on the tile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct IoRecipe {
    /// Relative to the half: `PULLTYPE.NONE`.
    pub this: Vec<String>,
    /// Relative to the other half.
    pub other: Vec<String>,
    /// Whole-tile names: `DIFF.ZIBUF_LOW_PWR`.
    pub tile: Vec<String>,
}

const IN_ONLY: &str =
    "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVDS_25_LVTTL_SSTL135_SSTL15_TMDS_33.IN_ONLY";
const RECEIVER: &str = "LVCMOS25_LVCMOS33_LVTTL.IN";
const SLOW: &str = "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL_SSTL135_SSTL15.SLEW.SLOW";
const FAST: &str = "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL.SLEW.FAST";

/// The drive feature of a single-ended standard at `ma` milliamperes.
fn drive_feature(standard: &str, ma: u32) -> Option<&'static str> {
    Some(match (standard, ma) {
        ("LVCMOS33", 4) => "LVCMOS33_LVTTL.DRIVE.I4",
        ("LVCMOS33", 8) => "LVCMOS33_LVTTL.DRIVE.I12_I8",
        ("LVCMOS33", 12) => "LVCMOS33_LVTTL.DRIVE.I12_I16",
        ("LVCMOS33", 16) => "LVCMOS33.DRIVE.I16",
        ("LVCMOS25", 4) => "LVCMOS15_LVCMOS18_LVCMOS25.DRIVE.I4",
        ("LVCMOS25", 8) => "LVCMOS12_LVCMOS25.DRIVE.I8",
        ("LVCMOS25", 12) => "LVCMOS25.DRIVE.I12",
        ("LVCMOS25", 16) => "LVCMOS25.DRIVE.I16",
        _ => return None,
    })
}

/// The standards this flow knows the pad bits of, for an error message.
pub fn io_standards() -> Vec<&'static str> {
    vec!["LVCMOS33", "LVCMOS25", "LVDS_25"]
}

/// What `request` costs, in feature names.
///
/// # Errors
///
/// A sentence naming what is not known: the standard, or a drive it does
/// not have.
pub(super) fn recipe(request: &IoRequest) -> Result<IoRecipe, String> {
    let pull = match request.pull {
        Pull::None => Some("PULLTYPE.NONE"),
        Pull::Up => Some("PULLTYPE.PULLUP"),
        Pull::Down => Some("PULLTYPE.PULLDOWN"),
    };
    let mut out = IoRecipe::default();
    match request.standard.as_str() {
        "LVCMOS33" | "LVCMOS25" => {
            if request.direction != Direction::In {
                let ma = request.drive.unwrap_or(12);
                let drive = drive_feature(&request.standard, ma).ok_or_else(|| {
                    format!(
                        "{} has no drive of {ma} mA this flow knows the bits of",
                        request.standard
                    )
                })?;
                out.this
                    .push(if request.fast { FAST } else { SLOW }.to_owned());
                out.this.push(drive.to_owned());
            }
            match request.direction {
                Direction::In => {
                    out.this.push(IN_ONLY.to_owned());
                    out.this.push(RECEIVER.to_owned());
                }
                Direction::InOut => out.this.push(RECEIVER.to_owned()),
                Direction::Out => {}
            }
            out.this.extend(pull.map(str::to_owned));
        }
        "LVDS_25" => {
            if request.pull != Pull::None {
                return Err("a pull on a differential pad".to_owned());
            }
            match request.direction {
                Direction::In => {
                    for half in [&mut out.this, &mut out.other] {
                        half.push(IN_ONLY.to_owned());
                        half.push("LVDS_25_SSTL135_SSTL15.IN_DIFF".to_owned());
                        half.push("PULLTYPE.NONE".to_owned());
                    }
                    if !request.low_power {
                        out.tile.push("DIFF.ZIBUF_LOW_PWR".to_owned());
                    }
                }
                Direction::Out => {
                    out.this.push("LVDS_25.OUT".to_owned());
                    out.this.push("LVDS_25.DRIVE.I_FIXED".to_owned());
                    out.this.push("PULLTYPE.NONE".to_owned());
                    out.other.push(IN_ONLY.to_owned());
                    out.other.push("PULLTYPE.NONE".to_owned());
                }
                Direction::InOut => {
                    return Err("a bidirectional LVDS_25 pad".to_owned());
                }
            }
        }
        other => {
            return Err(format!(
                "no bits are known for the io standard `{other}`; this flow can configure {}",
                io_standards().join(", ")
            ));
        }
    }
    Ok(out)
}

/// Why a pad could not be configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IoError {
    /// The request is not one the recipes in this module have bits for.
    Unsupported {
        /// The buffer.
        instance: String,
        /// Why.
        message: String,
    },
    /// A differential pad on the half that cannot lead a pair.
    SlaveHalf {
        /// The buffer.
        instance: String,
    },
    /// The database has no feature of that name for the tile.
    MissingFeature {
        /// The tile type.
        tile_type: String,
        /// The feature.
        feature: String,
    },
    /// A bit fell outside its tile.
    Bitstream(BitstreamError),
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::Unsupported { instance, message } => write!(f, "`{instance}`: {message}"),
            IoError::SlaveHalf { instance } => write!(
                f,
                "`{instance}` is a differential pad on the `_N` ball of its pair; the `P` port \
                 must be constrained to the `_P` ball, which leads the pair"
            ),
            IoError::MissingFeature { tile_type, feature } => write!(
                f,
                "the chip database has no `{feature}` in a `{tile_type}`; the database is not \
                 the one this flow was written against"
            ),
            IoError::Bitstream(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for IoError {}

impl From<BitstreamError> for IoError {
    fn from(e: BitstreamError) -> Self {
        IoError::Bitstream(e)
    }
}

/// The request a placed buffer makes, from its primitive and attributes.
fn request(primitive: &str, attrs: &crate::ir::Attrs, default: &str) -> IoRequest {
    let text = |key: &str| {
        attrs
            .get(key)
            .and_then(AttrValue::as_str)
            .map(str::to_owned)
    };
    let flag = |key: &str| attrs.get(key).and_then(AttrValue::as_int).map(|v| v != 0);
    let direction = match primitive {
        "IBUF" => Direction::In,
        "IOBUF" => Direction::InOut,
        _ => Direction::Out,
    };
    let pull = if flag("pullup") == Some(true) {
        Pull::Up
    } else if flag("pulldown") == Some(true) {
        Pull::Down
    } else {
        Pull::None
    };
    IoRequest {
        standard: text("io_standard")
            .unwrap_or_else(|| default.to_owned())
            .to_ascii_uppercase(),
        direction,
        drive: attrs
            .get("drive")
            .and_then(AttrValue::as_int)
            .and_then(|d| u32::try_from(d).ok()),
        fast: text("slew").is_some_and(|s| s.eq_ignore_ascii_case("fast")),
        pull,
        low_power: flag("ibuf_low_pwr") != Some(false),
    }
}

impl XrayFabric {
    /// Configures every IO buffer's pad for the standard, drive, slew and
    /// pull its cell asks for, and says how many bits that set.
    ///
    /// A pass after [`crate::fpga::bitstream::generate`] because what a
    /// pad's bits are depends on the cell's attributes, which no
    /// [`ConfigEntry`](crate::fpga::ConfigEntry) reads, and a differential
    /// pair writes both halves of its tile.
    ///
    /// # Errors
    ///
    /// [`IoError`]: a standard or drive with no recipe, a differential
    /// pad on its pair's slave half, or a database without the features.
    pub fn configure_io(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        bits: &mut Bitstream,
    ) -> Result<usize, IoError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut count = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "io" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let half = site.bel.as_str();
            let other = match half {
                "IOB_Y0" => "IOB_Y1",
                "IOB_Y1" => "IOB_Y0",
                _ => continue,
            };
            let Some(cell) = m.cells.get(instance.cell) else {
                continue;
            };
            let request = request(&instance.primitive, &cell.attrs, &self.io.default_standard);
            let recipe = recipe(&request).map_err(|message| IoError::Unsupported {
                instance: instance.name.clone(),
                message,
            })?;
            // A pair is led by its `_P` ball, which prjxray calls `IOB_Y0`
            // in every two-site tile (`docs/fpga-xray.md`, "A global clock
            // with no buffer driving it").
            if !recipe.other.is_empty() && half != "IOB_Y0" {
                return Err(IoError::SlaveHalf {
                    instance: instance.name.clone(),
                });
            }
            let tile_type = self.arch.tile_types[site.tile_type].name.as_str();
            let Some(table) = self.io.features.get(tile_type) else {
                return Err(IoError::MissingFeature {
                    tile_type: tile_type.to_owned(),
                    feature: format!("{half}.*"),
                });
            };
            let names = recipe
                .this
                .iter()
                .map(|f| format!("{half}.{f}"))
                .chain(recipe.other.iter().map(|f| format!("{other}.{f}")))
                .chain(recipe.tile.iter().cloned());
            for name in names {
                let Some(ones) = table.get(&name) else {
                    return Err(IoError::MissingFeature {
                        tile_type: tile_type.to_owned(),
                        feature: name,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(standard: &str, direction: Direction) -> IoRequest {
        IoRequest {
            standard: standard.to_owned(),
            direction,
            drive: None,
            fast: false,
            pull: Pull::None,
            low_power: true,
        }
    }

    /// The table in the module documentation, a line at a time, as
    /// Vivado wrote each one into NewAE's Husky image. Would catch an
    /// input given a drive, a bidirectional pad given `IN_ONLY`, a pull
    /// that adds to `NONE` rather than replacing it, and a differential
    /// pair configured on one half only; would not catch a wrong reading
    /// of a feature name the image never shows (the 4 and 16 mA drives).
    #[test]
    fn the_recipes_are_the_ones_vivado_wrote() {
        let input = recipe(&ask("LVCMOS33", Direction::In)).unwrap();
        assert_eq!(input.this, [IN_ONLY, RECEIVER, "PULLTYPE.NONE"]);
        assert_eq!(recipe(&ask("LVCMOS25", Direction::In)).unwrap(), input);

        let output = recipe(&ask("LVCMOS33", Direction::Out)).unwrap();
        assert_eq!(
            output.this,
            [SLOW, "LVCMOS33_LVTTL.DRIVE.I12_I16", "PULLTYPE.NONE"]
        );
        let mut eight = ask("LVCMOS33", Direction::Out);
        eight.drive = Some(8);
        assert!(
            recipe(&eight)
                .unwrap()
                .this
                .contains(&"LVCMOS33_LVTTL.DRIVE.I12_I8".to_owned())
        );
        assert_eq!(
            recipe(&ask("LVCMOS25", Direction::Out)).unwrap().this,
            [SLOW, "LVCMOS25.DRIVE.I12", "PULLTYPE.NONE"]
        );

        let both = recipe(&ask("LVCMOS33", Direction::InOut)).unwrap();
        assert_eq!(
            both.this,
            [
                SLOW,
                "LVCMOS33_LVTTL.DRIVE.I12_I16",
                RECEIVER,
                "PULLTYPE.NONE"
            ]
        );
        let mut up = ask("LVCMOS33", Direction::InOut);
        up.pull = Pull::Up;
        assert!(
            recipe(&up)
                .unwrap()
                .this
                .ends_with(&["PULLTYPE.PULLUP".to_owned()])
        );

        let mut pair = ask("LVDS_25", Direction::In);
        pair.low_power = false;
        let pair = recipe(&pair).unwrap();
        let diff = [IN_ONLY, "LVDS_25_SSTL135_SSTL15.IN_DIFF", "PULLTYPE.NONE"];
        assert_eq!(pair.this, diff);
        assert_eq!(pair.other, diff);
        assert_eq!(pair.tile, ["DIFF.ZIBUF_LOW_PWR"]);

        let out = recipe(&ask("LVDS_25", Direction::Out)).unwrap();
        assert_eq!(
            out.this,
            ["LVDS_25.OUT", "LVDS_25.DRIVE.I_FIXED", "PULLTYPE.NONE"]
        );
        assert_eq!(out.other, [IN_ONLY, "PULLTYPE.NONE"]);

        assert!(recipe(&ask("SSTL15", Direction::In)).is_err());
        let mut odd = ask("LVCMOS33", Direction::Out);
        odd.drive = Some(6);
        assert!(recipe(&odd).is_err());
    }
}
