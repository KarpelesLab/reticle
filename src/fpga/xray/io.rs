//! An IO buffer's pad, configured per buffer: its standard, its drive and
//! slew, its pull, its terminations, and a differential pair.
//!
//! # Why per buffer
//!
//! Until NewAE's ChipWhisperer-Husky this flow configured one IO standard
//! for the whole design, chosen at load time and attached to every pad as
//! a fixed feature list. The Husky has three on one part — LVCMOS33,
//! LVCMOS25 and LVDS_25 — four of its LVCMOS33 outputs at a drive of 8 mA
//! rather than the default 12, pull-ups on some pads and a pull-down on
//! one. So what a pad's bits are is read here, from the buffer cell's own
//! attributes (`io_standard`, `drive`, `slew`, `pullup`, `pulldown`,
//! `keeper`, `in_term`, `diff_term`, `ibuf_low_pwr`, set from its constraint
//! by [`crate::fpga::primitives`]), once placement says which pad it is on.
//!
//! # Where every recipe below came from
//!
//! **Vivado 2026.1, every line.** `tools/vivado-parity/gen_io.py` wrote a
//! design per standard and direction with a pad per setting, on both balls of
//! a pair, and `testdata/fpga/vivado/io/vivado.txt` is what each pad's bits
//! decoded to. `tests/fpga_xray_pads.rs` holds [`recipe`] to every line of it,
//! and builds them. `docs/fpga-vivado-parity.md` has the tables. This module's
//! table used to be read from NewAE's shipped Husky image, which agreed with all
//! of it but one line:
//!
//! **An LVDS output, not an LVDS input, sets the bank's two features.** This
//! said that a bank with LVDS *inputs* has `LVDS_25_IN_USE` and
//! `ONLY_DIFF_IN_USE` on its clock-row tile (`HCLK_IOI3`), read off the Husky
//! Plus's bank 15 on 2026-10-28: without them "every one of those six inputs
//! read a constant on the part", and with them the design worked. Vivado sets
//! neither on a bank of LVDS inputs alone (`lvds_in_alone`), with or without
//! `DIFF_TERM`, and sets both on one LVDS output (`lvds_out_alone`). That bank
//! has one, `ADC_CLKP`, and nothing here had looked for it. Why the inputs read a
//! constant without them is still not known; the part said they did, and that
//! bank's bits are unchanged by this.
//!
//! # The bits no `segbits` file names
//!
//! Project X-Ray fuzzed six kinds of standard, and Vivado writes others with
//! bits it never saw: a `--decode` of `HSTL_II` lists them as `?`. So
//! [`measured_features`] names them, as a `segbits` file would, and every bit of
//! every standard decodes. Each name is one group of bits that came and went
//! together across the whole sweep, with a clear bit added only where one group
//! holds another. **The names are mine and say nothing of what the bits do.**
//!
//! A standard this table has no line for is refused with its name: an IO
//! standard is a voltage, and guessing one is how a board gets damaged.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::XrayFabric;
use crate::fpga::arch::{ConfigBit, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::fpga::xc7::pads::{self, Standard, Way};
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
    /// A pad tile's grid position to a key for its bank: its column and its
    /// row over fifty, by `bank_of_name`.
    pub banks: BTreeMap<(u32, u32), (u32, u32)>,
}

impl IoTables {
    /// Gathers the tables from every tile type that has an IO buffer:
    /// an `IOB_Y0`, or an `IOB_Y1` alone. A `_SING` tile keeps one half of
    /// the two-IOB tile's bits (`TileAlias`), and the top one of a column,
    /// `start_offset` 0, keeps `IOB_Y1`'s: taken only for `IOB_Y0`, its
    /// table was missing, and the Apple II for the Basys 3, with a pad in
    /// `LIOB33_SING_X0Y99`, placed and routed and then wrote no bitstream.
    pub(super) fn new(
        features: &HashMap<String, super::FeatureSet>,
        default_standard: &str,
    ) -> IoTables {
        let mut out = IoTables {
            features: BTreeMap::new(),
            default_standard: default_standard.to_owned(),
            banks: BTreeMap::new(),
        };
        for (tile_type, set) in features {
            // The pads' tiles, and a bank's clock-row tile for its
            // `LVDS_25_IN_USE`.
            if !set.features().iter().any(|f| {
                f.name.starts_with("IOB_Y0.")
                    || f.name.starts_with("IOB_Y1.")
                    || f.name == "LVDS_25_IN_USE"
            }) {
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

/// Bits of a tile, as frame and bit.
type Bits = &'static [(u32, u32)];

/// One half's measured feature: its bits as `IOB_Y0` has them, and the clear
/// bits that keep it from being read off a larger one.
const HALF_FEATURES: [(&str, Bits, Bits); 9] = [
    (
        "HSTL_II.DRIVE.I_FIXED",
        &[
            (38, 64),
            (38, 112),
            (38, 118),
            (38, 126),
            (39, 65),
            (39, 113),
            (39, 117),
            (39, 119),
        ],
        &[],
    ),
    (
        "HSTL_II_18.DRIVE.I_FIXED",
        &[
            (38, 64),
            (38, 112),
            (38, 126),
            (39, 65),
            (39, 113),
            (39, 117),
            (39, 119),
            (39, 127),
        ],
        &[],
    ),
    (
        "PCI33_3.DRIVE.I_FIXED",
        &[
            (38, 64),
            (38, 106),
            (38, 110),
            (38, 112),
            (38, 116),
            (38, 126),
            (39, 65),
            (39, 107),
            (39, 111),
            (39, 117),
            (39, 127),
        ],
        &[],
    ),
    ("PCI33_3.IN", &[(39, 81)], &[]),
    (
        "HSTL.SLEW.FAST",
        &[(39, 105), (39, 107), (39, 111)],
        &[(39, 109)],
    ),
    (
        "HSTL_II_18_SSTL18.SLEW.FAST",
        &[(39, 105), (39, 107), (39, 109), (39, 111)],
        &[(38, 110)],
    ),
    (
        "PPDS_25_RSDS_25.DRIVE.I_FIXED",
        &[
            (38, 64),
            (38, 100),
            (38, 102),
            (38, 118),
            (39, 65),
            (39, 97),
            (39, 113),
            (39, 119),
            (39, 125),
        ],
        &[],
    ),
    (
        "MINI_LVDS_25.DRIVE.I_FIXED",
        &[
            (38, 64),
            (38, 98),
            (38, 100),
            (38, 102),
            (38, 118),
            (39, 65),
            (39, 97),
            (39, 113),
            (39, 119),
            (39, 125),
        ],
        &[],
    ),
    (
        "DIFF_TERM",
        &[
            (38, 100),
            (38, 118),
            (39, 97),
            (39, 113),
            (39, 119),
            (39, 125),
        ],
        &[],
    ),
];

/// The bits of `IOB_Y0` a driver's field has: what the database's
/// `LVDS_25.DRIVE.I_FIXED` lists, set or clear. A driver in
/// [`HALF_FEATURES`] clears the ones it does not set, as those of the database
/// do, or `DIFF_TERM` reads off `RSDS_25`'s driver.
const DRIVER_FIELD: [(u32, u32); 18] = [
    (38, 64),
    (38, 74),
    (38, 76),
    (38, 98),
    (38, 100),
    (38, 102),
    (38, 112),
    (38, 118),
    (38, 126),
    (39, 65),
    (39, 75),
    (39, 97),
    (39, 101),
    (39, 113),
    (39, 117),
    (39, 119),
    (39, 125),
    (39, 127),
];

/// A bit of `IOB_Y0` as `IOB_Y1` has it: **measured** to be the mirror of it,
/// frames 38 and 39 and the tile's 128 bits, on every group in
/// [`HALF_FEATURES`], and `tests/fpga_xray_pads.rs` reads both.
const fn mirrored(bit: (u32, u32)) -> (u32, u32) {
    (77 - bit.0, 127 - bit.1)
}

/// What Vivado sets in a bank's clock-row tile for a pair of true differential
/// outputs, beyond the database's `LVDS_25_IN_USE`. One standard takes the
/// first of two biases, the other the second, and a `PPDS_25` has a value of
/// its own in each.
const BANK_FEATURES: [(&str, Bits, Bits); 3] = [
    (
        "PPDS_25_IN_USE",
        &[
            (38, 23),
            (38, 24),
            (38, 25),
            (39, 23),
            (40, 25),
            (40, 27),
            (40, 29),
            (40, 30),
            (41, 15),
            (41, 16),
            (41, 17),
            (41, 18),
            (41, 27),
            (41, 28),
            (41, 31),
        ],
        &[(38, 27), (41, 14), (41, 29)],
    ),
    (
        "SECOND_LVDS_25_IN_USE",
        &[
            (38, 18),
            (38, 19),
            (38, 20),
            (38, 31),
            (39, 31),
            (40, 15),
            (40, 16),
            (40, 17),
            (40, 18),
            (40, 21),
            (40, 22),
            (40, 23),
            (40, 24),
        ],
        &[],
    ),
    (
        "SECOND_PPDS_25_IN_USE",
        &[
            (38, 18),
            (38, 19),
            (38, 20),
            (38, 31),
            (40, 15),
            (40, 16),
            (40, 17),
            (40, 18),
            (40, 22),
            (40, 23),
        ],
        &[(39, 31), (40, 21), (40, 24)],
    ),
];

/// The features of an IO or clock-row tile type that this flow measured and no
/// `segbits` file has; see the module's documentation. The same on both sides of
/// the die, which `left_*` in `testdata/fpga/vivado/io/` checks.
pub(super) fn measured_features(tile_type: &str) -> Vec<super::Feature> {
    let bits = |list: &[(u32, u32)]| {
        list.iter()
            .map(|(row, col)| ConfigBit::new(*row, *col))
            .collect::<Vec<_>>()
    };
    let mirror = |list: &[(u32, u32)]| {
        list.iter()
            .map(|bit| {
                let (row, col) = mirrored(*bit);
                ConfigBit::new(row, col)
            })
            .collect::<Vec<_>>()
    };
    let mut out = Vec::new();
    match tile_type {
        "LIOB33" | "RIOB33" => {
            for (name, ones, zeros) in HALF_FEATURES {
                let mut zeros = zeros.to_vec();
                if name.ends_with("DRIVE.I_FIXED") || name == "DIFF_TERM" {
                    zeros.extend(DRIVER_FIELD.iter().filter(|bit| !ones.contains(bit)));
                }
                out.push(super::Feature {
                    name: format!("IOB_Y0.{name}"),
                    ones: bits(ones),
                    zeros: bits(&zeros),
                });
                out.push(super::Feature {
                    name: format!("IOB_Y1.{name}"),
                    ones: mirror(ones),
                    zeros: mirror(&zeros),
                });
            }
            // A `TMDS_33` pair's `_N` half, which the database has for
            // `IOB_Y0` alone.
            out.push(super::Feature {
                name: "IOB_Y1.TMDS_33.IN_DIFF".to_owned(),
                ones: mirror(&[(38, 86)]),
                zeros: mirror(&[(39, 85), (39, 87)]),
            });
            // A `VREF` ball an unread input has configured: always the `_N`
            // half, so this half alone has been seen.
            out.push(super::Feature {
                name: "IOB_Y1.VREF_BALL".to_owned(),
                ones: bits(&[(39, 39)]),
                zeros: Vec::new(),
            });
            // Which pair of a bank takes the second bias.
            out.push(super::Feature {
                name: "OUT_DIFF_SECOND_BIAS".to_owned(),
                ones: bits(&[(38, 24), (39, 103)]),
                zeros: Vec::new(),
            });
        }
        "HCLK_IOI3" => {
            for (name, ones, zeros) in BANK_FEATURES {
                out.push(super::Feature {
                    name: name.to_owned(),
                    ones: bits(ones),
                    zeros: bits(zeros),
                });
            }
        }
        _ => {}
    }
    out
}

/// Which way a buffer drives.
pub(super) use crate::fpga::xc7::pads::Way as Direction;

/// A pull on the pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pull {
    /// None, which is `PULLTYPE.NONE`.
    None,
    /// `PULLTYPE.PULLUP`.
    Up,
    /// `PULLTYPE.PULLDOWN`, whose bits are all clear.
    Down,
    /// `PULLTYPE.KEEPER`.
    Keeper,
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
    /// `IN_TERM`, other than `NONE`.
    pub in_term: Option<String>,
    /// `DIFF_TERM`.
    pub diff_term: bool,
    /// An input nothing reads: Vivado removes the buffer and keeps the pad's
    /// direction and pull, whatever else it was given (`unread_*`).
    pub unread: bool,
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
const LOW_RECEIVER: &str = "LVCMOS12_LVCMOS15_LVCMOS18.IN";
const REFERENCED_RECEIVER: &str = "SSTL135_SSTL15.IN";
const DIFF_RECEIVER: &str = "LVDS_25_SSTL135_SSTL15.IN_DIFF";
const SLOW: &str = "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL_SSTL135_SSTL15.SLEW.SLOW";
const FAST: &str = "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL.SLEW.FAST";
const FAST_REFERENCED: &str = "SSTL135_SSTL15.SLEW.FAST";
/// Set on every pad of a bank whose supply is 1.8 V or lower, and in its
/// clock-row tile.
pub(super) const STEPDOWN: &str = "LVCMOS12_LVCMOS15_LVCMOS18_SSTL135_SSTL15.STEPDOWN";

/// The drive feature of a standard at `ma` milliamperes, or `None` for a drive
/// it does not have. A standard with one output has no choice, and what it was
/// asked for is a warning elsewhere.
fn drive_feature(standard: &str, ma: Option<u32>) -> Option<&'static str> {
    Some(match (standard, ma) {
        ("LVTTL", Some(4)) => "LVCMOS33_LVTTL.DRIVE.I4",
        ("LVTTL", None | Some(8 | 12)) => "LVCMOS33_LVTTL.DRIVE.I12_I8",
        ("LVTTL", Some(16)) => "LVCMOS33_LVTTL.DRIVE.I12_I16",
        ("LVTTL", Some(24)) => "LVTTL.DRIVE.I24",
        ("LVCMOS33", Some(4)) => "LVCMOS33_LVTTL.DRIVE.I4",
        ("LVCMOS33", Some(8)) => "LVCMOS33_LVTTL.DRIVE.I12_I8",
        ("LVCMOS33", None | Some(12)) => "LVCMOS33_LVTTL.DRIVE.I12_I16",
        ("LVCMOS33", Some(16)) => "LVCMOS33.DRIVE.I16",
        ("LVCMOS25", Some(4)) => "LVCMOS15_LVCMOS18_LVCMOS25.DRIVE.I4",
        ("LVCMOS25", Some(8)) => "LVCMOS12_LVCMOS25.DRIVE.I8",
        ("LVCMOS25", None | Some(12)) => "LVCMOS25.DRIVE.I12",
        ("LVCMOS25", Some(16)) => "LVCMOS25.DRIVE.I16",
        ("LVCMOS18", Some(4)) => "LVCMOS15_LVCMOS18_LVCMOS25.DRIVE.I4",
        ("LVCMOS18", None | Some(8 | 12)) => "LVCMOS18.DRIVE.I12_I8",
        ("LVCMOS18", Some(16)) => "LVCMOS18.DRIVE.I16",
        ("LVCMOS18", Some(24)) => "LVCMOS18.DRIVE.I24",
        ("LVCMOS15", Some(4)) => "LVCMOS15_LVCMOS18_LVCMOS25.DRIVE.I4",
        ("LVCMOS15", Some(8)) => "LVCMOS15.DRIVE.I8",
        ("LVCMOS15", None | Some(12)) => "LVCMOS15.DRIVE.I12",
        ("LVCMOS15", Some(16)) => "LVCMOS15_SSTL15.DRIVE.I16_I_FIXED",
        ("LVCMOS12", Some(4)) => "LVCMOS12.DRIVE.I4",
        ("LVCMOS12", Some(8)) => "LVCMOS12_LVCMOS25.DRIVE.I8",
        ("LVCMOS12", None | Some(12)) => "LVCMOS12.DRIVE.I12",
        ("LVTTL" | "LVCMOS33" | "LVCMOS25" | "LVCMOS18" | "LVCMOS15" | "LVCMOS12", _) => {
            return None;
        }
        ("HSUL_12" | "DIFF_HSUL_12", _) => "LVCMOS12.DRIVE.I12",
        ("HSTL_I" | "DIFF_HSTL_I" | "SSTL15_R" | "DIFF_SSTL15_R", _) => "LVCMOS15.DRIVE.I8",
        ("HSTL_II" | "DIFF_HSTL_II", _) => "HSTL_II.DRIVE.I_FIXED",
        ("HSTL_I_18" | "DIFF_HSTL_I_18", _) => "LVCMOS18.DRIVE.I16",
        ("HSTL_II_18" | "DIFF_HSTL_II_18", _) => "HSTL_II_18.DRIVE.I_FIXED",
        ("SSTL18_I" | "DIFF_SSTL18_I" | "MOBILE_DDR" | "DIFF_MOBILE_DDR", _) => {
            "LVCMOS18.DRIVE.I12_I8"
        }
        ("SSTL18_II" | "DIFF_SSTL18_II" | "BLVDS_25", _) => "LVCMOS25.DRIVE.I16",
        ("SSTL15" | "DIFF_SSTL15", _) => "LVCMOS15_SSTL15.DRIVE.I16_I_FIXED",
        ("SSTL135" | "DIFF_SSTL135", _) => "SSTL135.DRIVE.I_FIXED",
        ("SSTL135_R" | "DIFF_SSTL135_R", _) => "LVCMOS12_LVCMOS25.DRIVE.I8",
        ("PCI33_3", _) => "PCI33_3.DRIVE.I_FIXED",
        ("LVDS_25", _) => "LVDS_25.DRIVE.I_FIXED",
        ("TMDS_33", _) => "TMDS_33.DRIVE.I_FIXED",
        ("RSDS_25" | "PPDS_25", _) => "PPDS_25_RSDS_25.DRIVE.I_FIXED",
        ("MINI_LVDS_25", _) => "MINI_LVDS_25.DRIVE.I_FIXED",
        _ => return None,
    })
}

/// A driver's slew features, slow or fast. The same bits never mean both
/// across standards: the fast form is a family's own, and the single-ended
/// ones' fast is the field with every bit clear.
fn slew_features(standard: &Standard, fast: bool) -> &'static [&'static str] {
    if !standard.slew {
        return &[];
    }
    if !fast {
        return &[SLOW];
    }
    match standard.name {
        "LVTTL" | "LVCMOS33" | "LVCMOS25" | "LVCMOS18" | "LVCMOS15" | "LVCMOS12" | "MOBILE_DDR"
        | "DIFF_MOBILE_DDR" => &[FAST],
        "HSTL_I" | "HSTL_II" | "HSTL_I_18" | "DIFF_HSTL_I" | "DIFF_HSTL_II" | "DIFF_HSTL_I_18" => {
            &["HSTL.SLEW.FAST"]
        }
        "HSTL_II_18" | "DIFF_HSTL_II_18" | "SSTL18_I" | "SSTL18_II" | "DIFF_SSTL18_I"
        | "DIFF_SSTL18_II" => &["HSTL_II_18_SSTL18.SLEW.FAST"],
        _ => &[FAST_REFERENCED],
    }
}

/// The receiver a single-ended standard uses.
fn receiver_features(standard: &Standard) -> &'static [&'static str] {
    match standard.name {
        "LVTTL" | "LVCMOS33" | "LVCMOS25" => &[RECEIVER],
        "PCI33_3" => &[RECEIVER, "PCI33_3.IN"],
        "LVCMOS18" | "LVCMOS15" | "LVCMOS12" | "MOBILE_DDR" => &[LOW_RECEIVER],
        _ => &[REFERENCED_RECEIVER],
    }
}

/// The standards this flow knows the pad bits of, for an error message.
pub fn io_standards() -> Vec<&'static str> {
    pads::STANDARDS.iter().map(|s| s.name).collect()
}

/// What `request` costs, in feature names.
///
/// # Errors
///
/// A sentence naming what is not known: the standard, or a drive it does
/// not have.
pub(super) fn recipe(request: &IoRequest) -> Result<IoRecipe, String> {
    let Some(standard) = pads::standard(&request.standard) else {
        return Err(format!(
            "no bits are known for the io standard `{}`; this flow can configure {}",
            request.standard,
            io_standards().join(", ")
        ));
    };
    let pull = match request.pull {
        Pull::None => "PULLTYPE.NONE",
        Pull::Up => "PULLTYPE.PULLUP",
        Pull::Down => "PULLTYPE.PULLDOWN",
        Pull::Keeper => "PULLTYPE.KEEPER",
    };
    let driver = || -> Result<Vec<String>, String> {
        let ma = (!standard.drives.is_empty())
            .then_some(request.drive)
            .flatten();
        let drive = drive_feature(standard.name, ma).ok_or_else(|| {
            format!(
                "`{}` has no drive of {} mA",
                request.standard,
                request.drive.unwrap_or(0)
            )
        })?;
        let mut out: Vec<String> = slew_features(standard, request.fast)
            .iter()
            .map(|f| (*f).to_owned())
            .collect();
        out.push(drive.to_owned());
        Ok(out)
    };
    let term = request
        .in_term
        .as_ref()
        .filter(|_| standard.in_term)
        .map(|t| format!("IN_TERM.{t}"));
    let mut out = IoRecipe::default();
    if request.unread {
        out.this = vec![IN_ONLY.to_owned(), pull.to_owned()];
        if standard.differential {
            out.other = out.this.clone();
        }
        return Ok(out);
    }
    if !standard.differential {
        if request.direction != Way::In {
            out.this = driver()?;
        }
        if request.direction != Way::Out {
            if request.direction == Way::In {
                out.this.push(IN_ONLY.to_owned());
            }
            out.this
                .extend(receiver_features(standard).iter().map(|f| (*f).to_owned()));
            if !request.low_power && standard.vref.is_some() {
                out.this.push("ZIBUF_LOW_PWR".to_owned());
            }
            out.this.extend(term);
        }
        out.this.push(pull.to_owned());
        return Ok(out);
    }
    let receiver = if standard.name == "TMDS_33" {
        "TMDS_33.IN_DIFF"
    } else {
        DIFF_RECEIVER
    };
    // True differential drivers are the `_P` half's, and `OUT_DIFF` makes the
    // others from two single-ended ones.
    let one_driver = pads::is_true_differential_output(standard.name);
    if request.direction != Way::In {
        out.this = driver()?;
        if standard.name == "LVDS_25" {
            out.this.push("LVDS_25.OUT".to_owned());
        } else if standard.name == "TMDS_33" {
            out.this.push("TMDS_33.OUT".to_owned());
        }
        if one_driver {
            if request.direction == Way::Out {
                out.other.push(IN_ONLY.to_owned());
            }
        } else {
            out.other = out.this.clone();
            out.tile.push("OUT_DIFF".to_owned());
        }
    }
    match request.direction {
        Way::Out => {}
        Way::In => {
            // A `DIFF_TERM` is on the `_P` half, and takes the place of its
            // `IN_ONLY`, whose bits it holds.
            if request.diff_term && standard.diff_term {
                out.this.push("DIFF_TERM".to_owned());
            } else {
                out.this.push(IN_ONLY.to_owned());
            }
            out.other.push(IN_ONLY.to_owned());
            out.this.push(receiver.to_owned());
            out.other.push(receiver.to_owned());
        }
        Way::InOut => {
            if one_driver {
                out.other.push(IN_ONLY.to_owned());
            }
            out.this.push(receiver.to_owned());
            out.other.push(receiver.to_owned());
        }
    }
    if request.direction != Way::Out {
        if !request.low_power {
            out.tile.push("DIFF.ZIBUF_LOW_PWR".to_owned());
        }
        out.this.extend(term.clone());
        out.other.extend(term);
    }
    out.this.push(pull.to_owned());
    out.other.push(pull.to_owned());
    Ok(out)
}

/// What a bank wants of its clock-row tile and of every pad in it, for
/// [`XrayDatabase::finish_io_banks`](super::XrayDatabase::finish_io_banks): the
/// part of a pad that is not the pad's, and that may be outside the loaded
/// region, since a bank is fifty rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IoBankUse {
    /// The grid position of one of its pads' tiles, which says which bank.
    pub pad: (u32, u32),
    /// Features of its `HCLK_IOI3` tile.
    pub clock_row: BTreeSet<String>,
    /// `STEPDOWN` on all of its pads.
    pub stepdown: bool,
    /// Its `VREF` balls configured as inputs. Vivado does this for a
    /// referenced input nothing reads, and not for one something does
    /// (`unread_sstl`, `sstl15_in`), which is its rule and has no reason here.
    pub vref_balls: bool,
}

/// What a bank's pads make of its clock-row tile, in Vivado's terms, and which
/// of them take the second bias, if there are two true differential output
/// standards. `pads` are in port-name order, and are the request and the
/// pad's name. **Fitted:** the last port's standard takes the first bias, on
/// fourteen designs with the two standards in every order and place; it is not
/// the site's order. What a tool does with two names a sort cannot tell apart is
/// not known.
fn bank_use(pads: &[&IoRequest]) -> (BTreeSet<String>, bool, Option<String>, bool) {
    let mut clock_row = BTreeSet::new();
    let mut stepdown = false;
    let mut reference = None;
    let mut outputs: Vec<&str> = Vec::new();
    let mut vref_balls = false;
    for request in pads {
        let Some(standard) = pads::standard(&request.standard) else {
            continue;
        };
        if let Some(vcco) = standard.vcco_of(
            request.direction,
            request.in_term.is_some(),
            request.diff_term,
        ) {
            stepdown |= vcco <= 1800;
        }
        if request.unread {
            vref_balls |= standard.vref.is_some();
            continue;
        }
        if reference.is_none() {
            reference = standard.vref_of(request.direction);
        }
        if request.direction != Way::In && pads::is_true_differential_output(standard.name) {
            outputs.retain(|s| *s != standard.name);
            outputs.push(standard.name);
        }
    }
    let name = match reference {
        Some(600) => Some("VREF.V_600_MV"),
        Some(675) => Some("VREF.V_675_MV"),
        Some(750) => Some("VREF.V_750_MV"),
        Some(900) => Some("VREF.V_900_MV"),
        _ => None,
    };
    clock_row.extend(name.map(str::to_owned));
    // The last one named is the first bias.
    let first = outputs.pop();
    let second = outputs.pop();
    match first {
        Some("PPDS_25") => {
            clock_row.insert("PPDS_25_IN_USE".to_owned());
        }
        Some("TMDS_33") => {
            clock_row.insert("ONLY_DIFF_IN_USE".to_owned());
            clock_row.insert("TMDS_33_IN_USE".to_owned());
        }
        Some(_) => {
            clock_row.insert("LVDS_25_IN_USE".to_owned());
            clock_row.insert("ONLY_DIFF_IN_USE".to_owned());
        }
        None => {}
    }
    match second {
        Some("PPDS_25") => {
            clock_row.insert("SECOND_PPDS_25_IN_USE".to_owned());
        }
        Some(_) => {
            clock_row.insert("SECOND_LVDS_25_IN_USE".to_owned());
        }
        None => {}
    }
    (clock_row, stepdown, second.map(str::to_owned), vref_balls)
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
    /// A pair's second ball is not its first ball's partner, where Vivado moves
    /// the first without a word (`not_a_pair`).
    NotAPair {
        /// The buffer.
        instance: String,
        /// The second ball.
        ball: String,
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
            IoError::NotAPair { instance, ball } => write!(
                f,
                "`{instance}` and ball {ball} are not a pair: a differential pair's balls are \
                 the two of one IO tile"
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
fn request(primitive: &str, attrs: &crate::ir::Attrs, default: &str, unread: bool) -> IoRequest {
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
    } else if flag("keeper") == Some(true) {
        Pull::Keeper
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
        in_term: text("in_term")
            .map(|t| t.to_ascii_uppercase())
            .filter(|t| t != "NONE"),
        diff_term: flag("diff_term") == Some(true),
        unread,
    }
}

/// A key for the bank a pad's tile is in, by its name: its column and the
/// tile's `Y` over fifty. A bank is fifty rows, cut at multiples of fifty
/// (UG475, quoted; **checked** here on banks 14, 15, 34 and 35 of the XC7A100T,
/// whose `STEPDOWN` covers exactly these).
pub(super) fn bank_of_name(tile: &str) -> Option<(u32, u32)> {
    let (_, at) = tile.rsplit_once("_X")?;
    let (x, y) = at.split_once('Y')?;
    Some((x.parse().ok()?, y.parse::<u32>().ok()? / 50))
}

/// What a pad asks, in Vivado's words: [`pad_features`]'s question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadRequest {
    /// `IOSTANDARD`.
    pub standard: String,
    /// Which way.
    pub way: Way,
    /// `DRIVE`.
    pub drive: Option<u32>,
    /// `SLEW FAST`.
    pub fast: bool,
    /// `PULLTYPE`, if stated: `PULLUP`, `PULLDOWN`, `KEEPER`.
    pub pull: Option<String>,
    /// `IBUF_LOW_PWR`.
    pub low_power: bool,
    /// `IN_TERM`, other than `NONE`.
    pub in_term: Option<String>,
    /// `DIFF_TERM`.
    pub diff_term: bool,
}

impl Default for PadRequest {
    fn default() -> Self {
        PadRequest {
            standard: String::new(),
            way: Way::In,
            drive: None,
            fast: false,
            pull: None,
            low_power: true,
            in_term: None,
            diff_term: false,
        }
    }
}

/// What [`pad_features`] answers: the features of a pad's own half, of the other
/// half of its pair, whether it is a pair, and of the tile.
pub type PadFeatures = (Vec<String>, Vec<String>, bool, Vec<String>);

/// The features a pad asking `request` is given, for a test to hold against
/// what Vivado wrote.
///
/// # Errors
///
/// A sentence naming what is not known: the standard, or a drive it does not
/// have.
pub fn pad_features(request: &PadRequest) -> Result<PadFeatures, String> {
    let pull = match request.pull.as_deref() {
        Some("PULLUP") => Pull::Up,
        Some("PULLDOWN") => Pull::Down,
        Some("KEEPER") => Pull::Keeper,
        _ => Pull::None,
    };
    let out = recipe(&IoRequest {
        standard: request.standard.clone(),
        direction: request.way,
        drive: request.drive,
        fast: request.fast,
        pull,
        low_power: request.low_power,
        in_term: request.in_term.clone(),
        diff_term: request.diff_term,
        unread: false,
    })?;
    Ok((out.this, out.other.clone(), !out.other.is_empty(), out.tile))
}

/// A placed pad, by what it asks and where it is.
struct PlacedPad {
    name: String,
    request: IoRequest,
    half: String,
    tile: (u32, u32),
    tile_type: String,
    /// The second ball a constraint gave the pair.
    pair_ball: Option<String>,
}

impl XrayFabric {
    /// Every pad a placement filled, by its bank and then by name.
    fn placed_pads(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
    ) -> BTreeMap<(u32, u32), Vec<PlacedPad>> {
        let mut banks: BTreeMap<(u32, u32), Vec<PlacedPad>> = BTreeMap::new();
        let Some(m) = design.modules.get(module) else {
            return banks;
        };
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "io" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            if !matches!(site.bel.as_str(), "IOB_Y0" | "IOB_Y1") {
                continue;
            }
            let Some(cell) = m.cells.get(instance.cell) else {
                continue;
            };
            let at = self.io.banks.get(&site.tile).copied().unwrap_or(site.tile);
            banks.entry(at).or_default().push(PlacedPad {
                name: instance.name.clone(),
                request: request(
                    &instance.primitive,
                    &cell.attrs,
                    &self.io.default_standard,
                    instance.primitive == "IBUF"
                        && !instance.pins.iter().any(|p| {
                            let pin = &netlist.pins[*p];
                            pin.role == "din"
                                && pin
                                    .signal
                                    .is_some_and(|s| !netlist.signals[s].sinks.is_empty())
                        }),
                ),
                half: site.bel.clone(),
                tile: site.tile,
                tile_type: self.arch.tile_types[site.tile_type].name.clone(),
                pair_ball: cell
                    .attrs
                    .get("io_pair_ball")
                    .and_then(AttrValue::as_str)
                    .map(str::to_owned),
            });
        }
        for pads in banks.values_mut() {
            pads.sort_by(|a, b| a.name.cmp(&b.name));
        }
        banks
    }

    /// Configures every IO buffer's pad for the standard, drive, slew,
    /// pull and terminations its cell asks for, and says how many bits that
    /// set.
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
        let mut count = 0usize;
        let mut set = |tile_type: &str, tile: (u32, u32), name: &str| -> Result<(), IoError> {
            let Some(ones) = self.io.features.get(tile_type).and_then(|t| t.get(name)) else {
                return Err(IoError::MissingFeature {
                    tile_type: tile_type.to_owned(),
                    feature: name.to_owned(),
                });
            };
            for bit in ones {
                bits.set(tile, *bit)?;
                count += 1;
            }
            Ok(())
        };
        for pads in self
            .placed_pads(design, module, graph, netlist, placement)
            .values()
        {
            for pad in pads {
                let recipe = recipe(&pad.request).map_err(|message| IoError::Unsupported {
                    instance: pad.name.clone(),
                    message,
                })?;
                // A pair is led by its `_P` ball, which prjxray calls `IOB_Y0`
                // in every two-site tile (`docs/fpga-xray.md`, "A global clock
                // with no buffer driving it").
                if !recipe.other.is_empty() && pad.half != "IOB_Y0" {
                    return Err(IoError::SlaveHalf {
                        instance: pad.name.clone(),
                    });
                }
                if let Some(ball) = &pad.pair_ball {
                    let wanted = format!("X{}Y{}/IOB_Y1", pad.tile.0, pad.tile.1);
                    if self
                        .arch
                        .pinmap
                        .iter()
                        .any(|(pin, site)| pin == ball && *site != wanted)
                    {
                        return Err(IoError::NotAPair {
                            instance: pad.name.clone(),
                            ball: ball.clone(),
                        });
                    }
                }
                let other = if pad.half == "IOB_Y0" {
                    "IOB_Y1"
                } else {
                    "IOB_Y0"
                };
                let names = recipe
                    .this
                    .iter()
                    .map(|f| format!("{}.{f}", pad.half))
                    .chain(recipe.other.iter().map(|f| format!("{other}.{f}")))
                    .chain(recipe.tile.iter().cloned());
                for name in names {
                    set(&pad.tile_type, pad.tile, &name)?;
                }
            }
            let requests: Vec<&IoRequest> = pads.iter().map(|p| &p.request).collect();
            if let (_, _, Some(second), _) = bank_use(&requests) {
                for pad in pads
                    .iter()
                    .filter(|p| p.request.direction != Way::In && p.request.standard == second)
                {
                    set(&pad.tile_type, pad.tile, "OUT_DIFF_SECOND_BIAS")?;
                }
            }
        }
        Ok(count)
    }

    /// What each bank these pads are in wants beyond its pads, for
    /// [`XrayDatabase::finish_io_banks`](super::XrayDatabase::finish_io_banks).
    pub fn io_banks(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
    ) -> Vec<IoBankUse> {
        let mut out = Vec::new();
        for pads in self
            .placed_pads(design, module, graph, netlist, placement)
            .values()
        {
            let requests: Vec<&IoRequest> = pads.iter().map(|p| &p.request).collect();
            let (clock_row, stepdown, _, vref_balls) = bank_use(&requests);
            if !clock_row.is_empty() || stepdown || vref_balls {
                out.push(IoBankUse {
                    pad: pads[0].tile,
                    clock_row,
                    stepdown,
                    vref_balls,
                });
            }
        }
        out
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
            in_term: None,
            diff_term: false,
            unread: false,
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
            ["LVDS_25.DRIVE.I_FIXED", "LVDS_25.OUT", "PULLTYPE.NONE"]
        );
        assert_eq!(out.other, [IN_ONLY, "PULLTYPE.NONE"]);

        assert!(recipe(&ask("SSTL15", Direction::In)).is_ok());
        assert!(recipe(&ask("HSTL_III", Direction::In)).is_err());
        let mut odd = ask("LVCMOS33", Direction::Out);
        odd.drive = Some(6);
        assert!(recipe(&odd).is_err());
    }
}
