//! The configuration no routed net carries: IO buffers, IO banks, and the
//! defaults of a logic slice whose flip-flops are empty.
//!
//! A LUT's truth table and a pip are named in the database by position,
//! so [`super::ApiculaDatabase::load`] turns them into bits directly.
//! Everything here is named instead by *attribute*: an IO buffer is
//! `IO_TYPE=LVCMOS33, DRIVE=8, ...`, and the database maps each
//! `(attribute id, value id)` to a code through `logicinfo`, then a set of
//! codes to bits through a `shortval` or `longval` table
//! ([`super::fuses_for`]). The ids' names are Project Apicula's
//! `attrids.py`, transcribed in [`super::attrids`].
//!
//! # What is configured, and after whom
//!
//! This follows `gowin_pack` (apycula 0.33), function by function, and a
//! test compares the result with its output bit for bit:
//!
//! - an **input** buffer gets `default_ibuf_attrs`, an **output** buffer
//!   `default_obuf_attrs`, and each also gets its bank's `IO_TYPE` and
//!   `BANK_VCCIO` (`process_IBUF`, `process_OBUF`). These are not bel
//!   entries, because a bel's entries belong to its tile *type* and one
//!   type of IO tile sits in several banks;
//! - every **unused** IO gets only its bank's `IO_TYPE` and `BANK_VCCIO`
//!   (`get_unused_io_fuses`); a bank with nothing in it is set to
//!   `LVCMOS18` at 1.8 V;
//! - every **bank** gets its `IO_TYPE`, `BANK_VCCIO` and, when used,
//!   `PULL_STRENGTH=UNKNOWN` (`get_io_bank_fuses`, `check_io_banks`);
//! - a **slice** with a LUT and no flip-flop gets `LSRONMUX=0`,
//!   `CLKMUX_1=1`, `REG0_REGSET=RESET` and `REG1_REGSET=RESET`, the
//!   state of an empty register pair (`get_slice_fuses`, `no_dff`).
//!
//! An attribute value the `logicinfo` table has no code for is dropped,
//! exactly as `add_attr_val` drops it: that is how most defaults work,
//! since a default is usually the state of a fuse left alone.
//!
//! # Where this departs from `gowin_pack`, on purpose
//!
//! **A used bank's `BANK_VCCIO` follows its IO standard, inputs or not.**
//! `gowin_pack` means to take the bank's voltage from its outputs and to
//! fall back to 1.2 V when there are none, but a mutable default argument
//! (`make_IoBelDesc(bel, flags={})`) marks every buffer processed after the
//! first output as an output too, so which of the two a bank of inputs
//! gets depends on the order the cells appear in its input file. A bank's
//! `BANK_VCCIO` describes the supply the board wires to it, and an
//! `LVCMOS33` input sits in a bank supplied at 3.3 V; that is also what
//! `gowin_pack` writes whenever an output happens to come first.
//!
//! # One IO standard per bank
//!
//! A bank has one supply, so it has one standard, and every used IO in it
//! is configured for that standard: [`ApiculaOptions::bank_io_standards`]
//! names it bank by bank, and [`ApiculaOptions::io_standard`] is the one a
//! bank gets when nothing names it. `gowin_pack` refuses two standards in
//! one bank, and so does the command line. The standard has to match what
//! the *board* supplies the bank with, which no database says: a Tang
//! Primer 20K dock runs bank 4 at 1.5 V and banks 0, 1 and 3 at 3.3 V.
//!
//! [`ApiculaOptions::bank_io_standards`]: super::ApiculaOptions::bank_io_standards
//! [`ApiculaOptions::io_standard`]: super::ApiculaOptions::io_standard

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::attrids;
use super::parse::{self, CodeRow};
use super::{ApiculaDatabase, ApiculaError};
use crate::fpga::arch::ConfigBit;

/// The single-ended standards this configures, and the bank voltage each
/// implies: `BankDesc._vcc_ios`, the `LVCMOS` rows of it.
pub const IO_STANDARDS: &[(&str, &str)] = &[
    ("LVCMOS10", "1.0"),
    ("LVCMOS12", "1.2"),
    ("LVCMOS15", "1.5"),
    ("LVCMOS18", "1.8"),
    ("LVCMOS25", "2.5"),
    ("LVCMOS33", "3.3"),
];

/// What a bank with no IO in it is set to: `get_default_unused_io_type`.
const IDLE_BANK_STANDARD: &str = "LVCMOS18";

/// `Device.default_ibuf_attrs`.
const IBUF_DEFAULTS: &[(&str, &str)] = &[
    ("PADDI", "PADDI"),
    ("HYSTERESIS", "NONE"),
    ("PULLMODE", "UP"),
    ("SLEWRATE", "SLOW"),
    ("DRIVE", "0"),
    ("CLAMP", "OFF"),
    ("OPENDRAIN", "OFF"),
    ("DIFFRESISTOR", "OFF"),
    ("VREF", "OFF"),
    ("LVDS_OUT", "OFF"),
];

/// `Device.default_obuf_attrs`.
const OBUF_DEFAULTS: &[(&str, &str)] = &[
    ("ODMUX_1", "1"),
    ("PULLMODE", "UP"),
    ("SLEWRATE", "FAST"),
    ("DRIVE", "8"),
    ("HYSTERESIS", "NONE"),
    ("CLAMP", "OFF"),
    ("SINGLERESISTOR", "OFF"),
    ("LVDS_OUT", "OFF"),
    ("DDR_DYNTERM", "NA"),
    ("TO", "INV"),
    ("OPENDRAIN", "OFF"),
];

/// `Device.default_slice_attrvals['no_dff']`: both registers of a slice
/// empty.
const SLICE_NO_DFF: &[(&str, &str)] = &[
    ("LSRONMUX", "0"),
    ("CLKMUX_1", "1"),
    ("REG0_REGSET", "RESET"),
    ("REG1_REGSET", "RESET"),
];

/// `no_dff0` and `no_dff1`: one register of the pair empty.
const SLICE_NO_DFF0: &[(&str, &str)] = &[("REG0_REGSET", "RESET")];
const SLICE_NO_DFF1: &[(&str, &str)] = &[("REG1_REGSET", "RESET")];

/// The slices a logic tile has, `CLS0` to `CLS3`; LUTs and flip-flops
/// `2i` and `2i+1` are slice `i`.
const SLICES: u32 = 4;

/// The IO standard's bank voltage, or the error that lists the ones
/// this configures.
pub fn bank_vccio(standard: &str) -> Result<&'static str, ApiculaError> {
    IO_STANDARDS
        .iter()
        .find(|(s, _)| *s == standard)
        .map(|(_, v)| *v)
        .ok_or_else(|| ApiculaError::UnsupportedIoStandard {
            standard: standard.to_owned(),
            known: IO_STANDARDS.iter().map(|(s, _)| (*s).to_owned()).collect(),
        })
}

/// A `logicinfo` table with the names to look it up by.
pub(super) struct Codes {
    table: HashMap<(i64, i64), i64>,
    attrs: &'static [(&'static str, i64)],
    values: &'static [(&'static str, i64)],
}

impl Codes {
    /// The IO block's: `logicinfo['IOB']`, keyed by `iob_attrids`.
    pub(super) fn iob(db: &ApiculaDatabase) -> Codes {
        Codes::new(db, "IOB", attrids::IOB_ATTRS, attrids::IOB_VALUES)
    }

    /// The logic slice's: `logicinfo['SLICE']`, keyed by `cls_attrids`.
    pub(super) fn slice(db: &ApiculaDatabase) -> Codes {
        Codes::new(db, "SLICE", attrids::CLS_ATTRS, attrids::CLS_VALUES)
    }

    fn new(
        db: &ApiculaDatabase,
        table: &str,
        attrs: &'static [(&'static str, i64)],
        values: &'static [(&'static str, i64)],
    ) -> Codes {
        Codes {
            table: db.logicinfo(table).into_iter().collect(),
            attrs,
            values,
        }
    }

    /// The code for one attribute value, or `None` when the table has
    /// none (or it is zero), which is `add_attr_val` leaving it out.
    ///
    /// A *name* that is not in `attrids` is a mistake in this file rather
    /// than in the database, and a test checks every name used here.
    pub(super) fn code(&self, attr: &str, value: &str) -> Option<i64> {
        let id = |table: &[(&str, i64)], name: &str| {
            table.iter().find(|(n, _)| *n == name).map(|(_, id)| *id)
        };
        let key = (id(self.attrs, attr)?, id(self.values, value)?);
        self.table.get(&key).copied().filter(|c| *c != 0)
    }

    /// The codes for a list of attribute values.
    pub(super) fn set(&self, pairs: &[(&str, &str)]) -> BTreeSet<i64> {
        pairs.iter().filter_map(|(a, v)| self.code(a, v)).collect()
    }
}

/// The bits one set of codes selects from a table.
fn bits(table: &[CodeRow], codes: &BTreeSet<i64>) -> Vec<ConfigBit> {
    parse::fuses_for(table, codes)
        .into_iter()
        .map(|(r, c)| ConfigBit::new(r, c))
        .collect()
}

/// An IO buffer's codes: its defaults, then the bank's standard and
/// voltage.
fn buffer_codes(
    codes: &Codes,
    defaults: &[(&str, &str)],
    standard: &str,
    vccio: &str,
) -> BTreeSet<i64> {
    let mut set = codes.set(defaults);
    set.extend(codes.set(&[("IO_TYPE", standard), ("BANK_VCCIO", vccio)]));
    set
}

/// One IO the part has, and its bits in every state it can be in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoSite {
    /// Its tile, as `(x, y)`: grid column and row.
    pub tile: (u32, u32),
    /// Its bel, `IOBA` or `IOBB`.
    pub bel: String,
    /// The bank it belongs to.
    pub bank: i64,
    /// The standard its bank is configured for when used.
    pub standard: String,
    /// Its bits with an `IBUF` on it.
    pub as_input: Vec<ConfigBit>,
    /// Its bits with an `OBUF` on it.
    pub as_output: Vec<ConfigBit>,
    /// Its bits unused, in a bank with a used IO.
    pub idle_in_used_bank: Vec<ConfigBit>,
    /// Its bits unused, in a bank with none, and so `LVCMOS18` at 1.8 V.
    pub idle_in_idle_bank: Vec<ConfigBit>,
}

/// One IO bank and its two possible settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bank {
    /// The bank's number.
    pub number: i64,
    /// The tile its bits are in, as `(x, y)`.
    pub tile: (u32, u32),
    /// The standard it is configured for when used.
    pub standard: String,
    /// Its bits with a used IO in it.
    pub used: Vec<ConfigBit>,
    /// Its bits with none.
    pub idle: Vec<ConfigBit>,
}

/// A slice: its tile, as `(x, y)`, and its index in the tile.
type SliceAt = ((u32, u32), u32);

/// Bits in one tile, the tile as `(x, y)`.
pub type TileBits = ((u32, u32), Vec<ConfigBit>);

/// Everything outside a placed cell's own bits, worked out once per load
/// so that writing a stream is only a matter of choosing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Periphery {
    /// Every IO with a bank, in the database's `io_cfg` order.
    pub ios: Vec<IoSite>,
    /// Every bank, in order of number.
    pub banks: Vec<Bank>,
    /// Per logic tile type, per slice, the bits of a slice whose LUTs are
    /// used and whose registers are `[neither, only 1, only 0]` used:
    /// `no_dff`, `no_dff0`, `no_dff1`.
    pub slices: BTreeMap<u32, Vec<[Vec<ConfigBit>; 3]>>,
    /// The grid's tile types, row by row, to find a tile's slice tables.
    pub grid: Vec<Vec<u32>>,
}

/// Why [`Periphery::bits`] refused a design.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeripheryError {
    /// A primitive on an IO bel this flow does not configure.
    UnsupportedIo {
        /// The bel, `X<x>Y<y>/<bel>`.
        site: String,
        /// The primitive.
        primitive: String,
    },
}

impl std::fmt::Display for PeripheryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeripheryError::UnsupportedIo { site, primitive } => write!(
                f,
                "`{primitive}` on {site} is not an IO buffer this flow configures; it \
                 configures IBUF and OBUF"
            ),
        }
    }
}

impl std::error::Error for PeripheryError {}

impl Periphery {
    /// The IO on `tile`'s bel `bel`, if the part has one there.
    pub fn io(&self, tile: (u32, u32), bel: &str) -> Option<&IoSite> {
        self.ios.iter().find(|io| io.tile == tile && io.bel == bel)
    }

    /// The bits no routed net carries, for a design whose cells occupy
    /// `used`: `(tile, bel)` to the primitive placed there, the tile as
    /// `(x, y)`.
    ///
    /// # Errors
    ///
    /// [`PeripheryError::UnsupportedIo`] for anything but an `IBUF` or an
    /// `OBUF` on an IO bel.
    pub fn bits(
        &self,
        used: &BTreeMap<((u32, u32), String), String>,
    ) -> Result<Vec<TileBits>, PeripheryError> {
        let mut out = Vec::new();
        let placed = |io: &IoSite| used.get(&(io.tile, io.bel.clone()));
        let used_banks: BTreeSet<i64> = self
            .ios
            .iter()
            .filter(|io| placed(io).is_some())
            .map(|io| io.bank)
            .collect();
        for io in &self.ios {
            let bits = match placed(io).map(String::as_str) {
                Some("IBUF") => &io.as_input,
                Some("OBUF") => &io.as_output,
                Some(other) => {
                    return Err(PeripheryError::UnsupportedIo {
                        site: format!("X{}Y{}/{}", io.tile.0, io.tile.1, io.bel),
                        primitive: other.to_owned(),
                    });
                }
                None if used_banks.contains(&io.bank) => &io.idle_in_used_bank,
                None => &io.idle_in_idle_bank,
            };
            out.push((io.tile, bits.clone()));
        }
        for bank in &self.banks {
            let bits = if used_banks.contains(&bank.number) {
                &bank.used
            } else {
                &bank.idle
            };
            out.push((bank.tile, bits.clone()));
        }

        // Slices: a used LUT0..5 marks its slice, a used DFF marks its
        // half of the register pair. Keyed by (tile, slice): (a LUT,
        // register 0, register 1) used.
        let mut slices: BTreeMap<SliceAt, (bool, bool, bool)> = BTreeMap::new();
        for (tile, bel) in used.keys() {
            let (index, is_lut) = if let Some(n) = bel.strip_prefix("LUT") {
                (n, true)
            } else if let Some(n) = bel.strip_prefix("DFF") {
                (n, false)
            } else {
                continue;
            };
            let Ok(index) = index.parse::<u32>() else {
                continue;
            };
            if index >= 2 * SLICES || (is_lut && index >= 6) {
                continue;
            }
            let entry = slices.entry((*tile, index / 2)).or_default();
            if is_lut {
                entry.0 = true;
            } else if index % 2 == 0 {
                entry.1 = true;
            } else {
                entry.2 = true;
            }
        }
        for (((x, y), slice), (_, dff0, dff1)) in slices {
            let Some(ttyp) = self
                .grid
                .get(y as usize)
                .and_then(|row| row.get(x as usize))
            else {
                continue;
            };
            let Some(tables) = self.slices.get(ttyp).and_then(|s| s.get(slice as usize)) else {
                continue;
            };
            let which = match (dff0, dff1) {
                (false, false) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (true, true) => continue,
            };
            out.push(((x, y), tables[which].clone()));
        }
        Ok(out)
    }
}

impl ApiculaDatabase {
    /// Everything [`Periphery`] needs: every bank configured for the
    /// standard `per_bank` names, or `default` when it names none.
    ///
    /// # Errors
    ///
    /// [`ApiculaError::UnsupportedIoStandard`] for a standard not in
    /// [`IO_STANDARDS`].
    pub fn periphery(
        &self,
        default: &str,
        per_bank: &BTreeMap<i64, String>,
    ) -> Result<Periphery, ApiculaError> {
        bank_vccio(default)?;
        for standard in per_bank.values() {
            bank_vccio(standard)?;
        }
        let standard_of = |bank: i64| per_bank.get(&bank).map_or(default, String::as_str);
        let idle_vccio = bank_vccio(IDLE_BANK_STANDARD)?;
        let iob = Codes::iob(self);
        let slice = Codes::slice(self);
        let grid = self.grid();
        let (rows, cols) = self.grid_size();
        let corners = self.corner_tiles();

        // IOLOC (without its half) to tile, built forwards over the ring
        // as `ioloc_sites` does.
        let mut tiles: HashMap<String, (u32, u32)> = HashMap::new();
        for (row, line) in grid.iter().enumerate() {
            for col in 0..line.len() {
                let (x, y) = (
                    u32::try_from(col).unwrap_or(0),
                    u32::try_from(row).unwrap_or(0),
                );
                if let Some(name) = parse::ioloc(rows, cols, &corners, y, x) {
                    tiles.insert(name, (x, y));
                }
            }
        }
        let pin_bank: HashMap<String, i64> = self
            .root
            .get("pin_bank")
            .map(|m| {
                m.pairs()
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_owned(), v.as_i64()?)))
                    .collect()
            })
            .unwrap_or_default();
        let ttyp_at = |(x, y): (u32, u32)| grid.get(y as usize)?.get(x as usize).copied();
        let bank_codes = |standard: &str| {
            let vccio = bank_vccio(standard).unwrap_or("3.3");
            iob.set(&[("IO_TYPE", standard), ("BANK_VCCIO", vccio)])
        };
        let idle_codes = iob.set(&[("IO_TYPE", IDLE_BANK_STANDARD), ("BANK_VCCIO", idle_vccio)]);

        let mut ios = Vec::new();
        let io_cfg = self
            .root
            .get("io_cfg")
            .map(|m| m.pairs())
            .unwrap_or_default();
        for (key, _) in io_cfg {
            let Some(key) = key.as_str() else { continue };
            let Some((loc, half)) = key.split_at_checked(key.len().saturating_sub(1)) else {
                continue;
            };
            let Some(&tile) = tiles.get(loc) else {
                continue;
            };
            // `loc2bank`: the tile's bank, read from its A pin if it has one.
            let Some(&bank) = pin_bank
                .get(&format!("{loc}A"))
                .or_else(|| pin_bank.get(&format!("{loc}B")))
            else {
                continue;
            };
            let Some(ttyp) = ttyp_at(tile) else { continue };
            let bel = format!("IOB{half}");
            let table = self.code_table("longval", ttyp, &bel);
            let standard = standard_of(bank);
            let vccio = bank_vccio(standard)?;
            ios.push(IoSite {
                tile,
                bel,
                bank,
                standard: standard.to_owned(),
                as_input: bits(&table, &buffer_codes(&iob, IBUF_DEFAULTS, standard, vccio)),
                as_output: bits(&table, &buffer_codes(&iob, OBUF_DEFAULTS, standard, vccio)),
                idle_in_used_bank: bits(&table, &bank_codes(standard)),
                idle_in_idle_bank: bits(&table, &idle_codes),
            });
        }

        // `Device.bank_tiles`, which is a property and not in the file:
        // the tile holding a `BANK<n>` bel, the last one in row-major
        // order when several do.
        let mut bank_tiles: BTreeMap<i64, (u32, u32)> = BTreeMap::new();
        for (row, line) in grid.iter().enumerate() {
            for (col, ttyp) in line.iter().enumerate() {
                let Some(bels) = self.tile(*ttyp).and_then(|t| t.get("bels")) else {
                    continue;
                };
                for (name, _) in bels.pairs() {
                    if let Some(number) = name
                        .as_str()
                        .and_then(|n| n.strip_prefix("BANK"))
                        .and_then(|n| n.parse().ok())
                    {
                        let tile = (
                            u32::try_from(col).unwrap_or(0),
                            u32::try_from(row).unwrap_or(0),
                        );
                        bank_tiles.insert(number, tile);
                    }
                }
            }
        }

        let mut banks = Vec::new();
        let pull = iob.set(&[("PULL_STRENGTH", "UNKNOWN")]);
        for (number, tile) in bank_tiles {
            let Some(ttyp) = ttyp_at(tile) else { continue };
            // `get_bank_fuses`: the rows of this bank, their first code
            // (the bank number) dropped.
            let table: Vec<CodeRow> = self
                .code_table("longval", ttyp, "BANK")
                .into_iter()
                .filter(|(key, _)| key.first() == Some(&number))
                .map(|(key, bits)| (key[1..].to_vec(), bits))
                .collect();
            // `get_bank_io_fuses`: an IO table in the bank's own tile.
            let io_table = ["IOBA", "IOBB"]
                .iter()
                .map(|t| self.code_table("longval", ttyp, t))
                .find(|t| !t.is_empty())
                .unwrap_or_default();
            let both = |codes: &BTreeSet<i64>| {
                let mut out = bits(&table, codes);
                out.extend(bits(&io_table, codes));
                out
            };
            let standard = standard_of(number);
            let mut used_codes = bank_codes(standard);
            used_codes.extend(pull.iter().copied());
            banks.push(Bank {
                number,
                tile,
                standard: standard.to_owned(),
                used: both(&used_codes),
                idle: both(&idle_codes),
            });
        }

        let mut slices = BTreeMap::new();
        let states = [
            slice.set(SLICE_NO_DFF),
            slice.set(SLICE_NO_DFF0),
            slice.set(SLICE_NO_DFF1),
        ];
        for ttyp in grid.iter().flatten().copied().collect::<BTreeSet<u32>>() {
            let mut per_slice = Vec::new();
            for index in 0..SLICES {
                let table = self.code_table("shortval", ttyp, &format!("CLS{index}"));
                if table.is_empty() {
                    break;
                }
                per_slice.push(states.clone().map(|codes| bits(&table, &codes)));
            }
            if !per_slice.is_empty() {
                slices.insert(ttyp, per_slice);
            }
        }

        Ok(Periphery {
            ios,
            banks,
            slices,
            grid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IBUF_DEFAULTS, IDLE_BANK_STANDARD, IO_STANDARDS, OBUF_DEFAULTS, SLICE_NO_DFF, bank_vccio,
    };
    use crate::fpga::apicula::attrids;

    /// Every name this module looks up is in the transcribed tables; a
    /// typo would otherwise drop an attribute silently, which is exactly
    /// what a *missing code* does on purpose.
    #[test]
    fn every_name_used_here_is_one_apicula_defines() {
        let has = |table: &[(&str, i64)], name: &str| table.iter().any(|(n, _)| *n == name);
        let mut iob: Vec<(&str, &str)> =
            IBUF_DEFAULTS.iter().chain(OBUF_DEFAULTS).copied().collect();
        for (standard, vccio) in IO_STANDARDS {
            iob.push(("IO_TYPE", standard));
            iob.push(("BANK_VCCIO", vccio));
        }
        iob.push(("PULL_STRENGTH", "UNKNOWN"));
        for (attr, value) in iob {
            assert!(has(attrids::IOB_ATTRS, attr), "iob attribute {attr}");
            assert!(has(attrids::IOB_VALUES, value), "iob value {value}");
        }
        for (attr, value) in SLICE_NO_DFF {
            assert!(has(attrids::CLS_ATTRS, attr), "cls attribute {attr}");
            assert!(has(attrids::CLS_VALUES, value), "cls value {value}");
        }
    }

    /// The ids `docs/fpga-gowin.md` and the tests quote.
    #[test]
    fn the_transcription_has_the_ids_the_flow_depends_on() {
        let id = |table: &[(&str, i64)], name: &str| {
            table.iter().find(|(n, _)| *n == name).map(|(_, id)| *id)
        };
        assert_eq!(id(attrids::IOB_ATTRS, "IO_TYPE"), Some(0));
        assert_eq!(id(attrids::IOB_ATTRS, "BANK_VCCIO"), Some(10));
        assert_eq!(id(attrids::IOB_VALUES, "LVCMOS33"), Some(33));
        assert_eq!(id(attrids::IOB_VALUES, "3.3"), Some(93));
        assert_eq!(id(attrids::CLS_ATTRS, "REG0_REGSET"), Some(13));
        assert_eq!(id(attrids::CLS_VALUES, "RESET"), Some(17));
    }

    #[test]
    fn a_standard_implies_its_bank_voltage() {
        assert_eq!(bank_vccio("LVCMOS33").unwrap(), "3.3");
        assert_eq!(bank_vccio(IDLE_BANK_STANDARD).unwrap(), "1.8");
        let err = bank_vccio("SSTL15").unwrap_err().to_string();
        assert!(err.contains("SSTL15") && err.contains("LVCMOS33"), "{err}");
    }
}
