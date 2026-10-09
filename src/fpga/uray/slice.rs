//! The inside of an UltraScale+ slice, as far as it is known.
//!
//! # A lookup table's 64 bits
//!
//! The 2026 database names 62 of each six-input lookup table's 64 `INIT`
//! bits: `INIT[31]` and `INIT[63]` are missing from every one of the
//! eight. Its `defaults_<type>.db` lists 16 bits "set when a feature is
//! not used", two per lookup table. They are the same bits.
//!
//! **Measured, two ways.**
//!
//! - **The layout.** The 62 named bits sit on a perfect grid: `INIT[i]` is
//!   frame `f0 + 3 − i mod 4`, bit `c0 + 15 − ⌊i / 4⌋`, with `(f0, c0)`
//!   the table's corner, (8, 0) for `A`. Continued to the two missing
//!   indices, the grid lands `INIT[63]` on (8, 0) and `INIT[31]` on
//!   (8, 8) — both `defaults` bits — and likewise for every table.
//! - **Vivado's constant zeros.** Where Vivado ties an unused processor
//!   input low, it uses a lookup table of `INIT = 0` in a slice beside the
//!   interface column. In `base.bit` those tables have their two bits
//!   clear, and every unused table of the same slice has them set.
//!
//! So the two bits are ordinary `INIT` bits, stored like the rest, and
//! Vivado gives an unused table the contents `0x8000_0000_8000_0000`. The
//! database's generator saw them set whenever no table was placed, and
//! filed them as defaults. [`lut_init_bits`] fills them in from the grid,
//! and refuses a database whose named bits are not on one.
//!
//! A five-input table is the lower half of its six-input one: the
//! database names `A5LUT.INIT[i]` at `A6LUT.INIT[i]`'s bit for every `i`
//! it names, and names no `E5LUT` at all.

use super::{TileTypeBits, UrayError};
use crate::fpga::arch::ConfigBit;

/// The contents Vivado leaves in a lookup table nothing uses.
pub const UNUSED_LUT_INIT: u64 = 0x8000_0000_8000_0000;

/// The eight lookup tables of a slice, in the order the site names them.
pub const LUT_LETTERS: [char; 8] = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H'];

/// The tile bit of each of `letter`'s 64 `INIT` bits, index by index, in
/// a slice tile whose features are `bits`.
///
/// # Errors
///
/// [`UrayError::Malformed`] when the database names fewer than two of the
/// table's bits, or names one off the grid the rest are on — either of
/// which would make filling in the missing two a guess.
pub fn lut_init_bits(bits: &TileTypeBits, letter: char) -> Result<[ConfigBit; 64], UrayError> {
    let prefix = format!("SLICE_X0Y0.{letter}6LUT.INIT[");
    let mut named: Vec<(u32, ConfigBit)> = Vec::new();
    for feature in &bits.features {
        let Some(rest) = feature.name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(index) = rest.strip_suffix(']').and_then(|i| i.parse::<u32>().ok()) else {
            continue;
        };
        if let [bit] = feature.ones.as_slice() {
            named.push((index, *bit));
        }
    }
    let bad = |message: String| UrayError::Malformed {
        path: format!("{letter}6LUT"),
        message,
    };
    if named.len() < 2 {
        return Err(bad(format!(
            "the database names {} of its bits",
            named.len()
        )));
    }
    // The corner: INIT[i] = (f0 + 3 - i % 4, c0 + 15 - i / 4).
    let (index, bit) = named[0];
    let f0 = i64::from(bit.row) - 3 + i64::from(index % 4);
    let c0 = i64::from(bit.col) - 15 + i64::from(index / 4);
    let at = |i: u32| -> Option<ConfigBit> {
        let row = u32::try_from(f0 + 3 - i64::from(i % 4)).ok()?;
        let col = u32::try_from(c0 + 15 - i64::from(i / 4)).ok()?;
        Some(ConfigBit::new(row, col))
    };
    for (i, b) in &named {
        if at(*i) != Some(*b) {
            return Err(bad(format!(
                "INIT[{i}] is at {b:?}, off the grid the database's other bits are on"
            )));
        }
    }
    let mut out = [ConfigBit::new(0, 0); 64];
    for (i, slot) in out.iter_mut().enumerate() {
        let i = u32::try_from(i).unwrap_or(0);
        *slot = at(i).ok_or_else(|| bad(format!("INIT[{i}] falls off the tile")))?;
    }
    Ok(out)
}

/// The contents of `letter`'s table in a tile whose set bits are `set`.
pub fn read_lut(layout: &[ConfigBit; 64], set: &impl Fn(ConfigBit) -> bool) -> u64 {
    layout
        .iter()
        .enumerate()
        .filter(|(_, b)| set(**b))
        .fold(0u64, |acc, (i, _)| acc | (1 << i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::xray::Feature;

    fn table(skip: &[u32]) -> TileTypeBits {
        let mut features = Vec::new();
        for i in 0..64u32 {
            if skip.contains(&i) {
                continue;
            }
            features.push(Feature {
                name: format!("SLICE_X0Y0.A6LUT.INIT[{i}]"),
                ones: vec![ConfigBit::new(8 + 3 - i % 4, 15 - i / 4)],
                zeros: Vec::new(),
            });
        }
        TileTypeBits {
            features,
            defaults: Vec::new(),
        }
    }

    #[test]
    fn the_two_missing_bits_are_filled_in_from_the_grid() {
        let layout = lut_init_bits(&table(&[31, 63]), 'A').unwrap();
        assert_eq!(layout[63], ConfigBit::new(8, 0));
        assert_eq!(layout[31], ConfigBit::new(8, 8));
        assert_eq!(layout[0], ConfigBit::new(11, 15));
        let set = |b: ConfigBit| b == layout[63] || b == layout[31];
        assert_eq!(read_lut(&layout, &set), UNUSED_LUT_INIT);
    }

    #[test]
    fn a_bit_off_the_grid_is_refused() {
        let mut bits = table(&[31, 63]);
        bits.features[5].ones = vec![ConfigBit::new(0, 0)];
        assert!(lut_init_bits(&bits, 'A').is_err());
        assert!(lut_init_bits(&table(&(1..64).collect::<Vec<_>>()), 'A').is_err());
    }
}

/// Which control pins serve which flip-flops.
///
/// A slice has two clocks, four clock enables and two set/resets for its
/// sixteen flip-flops. The database settles which flip-flops *share* a
/// "used" bit: its `CE_ACTIVE` and `SR_ACTIVE` features give the same bit
/// to the first flip-flops of `A`–`D`, to the second flip-flops of
/// `A`–`D`, to the first of `E`–`H` and to the second of `E`–`H`, and
/// [`slice_model`] refuses a database where they do not. Which *pin* each
/// group listens to is not in the database. `AFF` on `CLK1`, `CKEN1` and
/// `SRST1` is **measured** (`one_flip_flop` in `tests/fpga_uray_board.rs`);
/// the rest of this table is the reading that follows from it and has not
/// been run.
pub fn control_pins(letter: char, second: bool) -> (&'static str, &'static str, &'static str) {
    let upper = matches!(letter, 'E' | 'F' | 'G' | 'H');
    let clock = if upper { "CLK2" } else { "CLK1" };
    let reset = if upper { "SRST2" } else { "SRST1" };
    let enable = match (upper, second) {
        (false, false) => "CKEN1",
        (false, true) => "CKEN2",
        (true, false) => "CKEN3",
        (true, true) => "CKEN4",
    };
    (clock, enable, reset)
}

/// What a slice tile contributes to the fabric: its bels, the wires it
/// invents inside the site, and the pips that join them to the site pins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SliceModel {
    /// The eight six-input lookup tables and sixteen flip-flops.
    pub bels: Vec<crate::fpga::arch::BelDecl>,
    /// Wires that exist only inside the site: one per control group.
    pub wires: Vec<String>,
    /// From a site pin to a control group's wire, with the group's "used"
    /// bit, so a group's enable or reset costs its bit exactly when it is
    /// routed.
    pub pips: Vec<crate::fpga::arch::PipDecl>,
}

/// The slice of a tile whose site pins are `pins` (pin name to tile wire)
/// and whose features are `bits`.
///
/// Each lookup table is a `lut` bel: `i0`..`i5` on `A1`..`A6`, `o` on
/// `A_O`, and `INIT` written through [`lut_init_bits`]. Each flip-flop is
/// an `ff` bel taking its data from its bypass pin (`AX`, or `A_I` for the
/// second): a lookup table's output reaches it through the fabric, as it
/// did in the counters that ran on the part. Its clock is the slice's clock
/// pin; its enable and reset are its group's wire. An `FDRE` sets the
/// bypass select, the group's synchronous bit and its reset value of zero,
/// and its `INIT` of zero sets `FFINIT=INIT0`.
///
/// # Errors
///
/// [`UrayError::Malformed`] for a feature or pin the model needs and the
/// database lacks, or for a control group whose members do not share their
/// bit.
pub fn slice_model(
    pins: &[(String, String)],
    bits: &TileTypeBits,
) -> Result<SliceModel, UrayError> {
    use crate::fpga::arch::{BelDecl, ConfigEntry, PipDecl, WireRef};
    let bad = |message: String| UrayError::Malformed {
        path: "slice".to_owned(),
        message,
    };
    let wire = |pin: &str| -> Result<String, UrayError> {
        pins.iter()
            .find(|(p, _)| p == pin)
            .map(|(_, w)| w.clone())
            .ok_or_else(|| bad(format!("the site has no pin `{pin}`")))
    };
    let feature = |name: &str| -> Option<&Vec<ConfigBit>> {
        bits.features
            .iter()
            .find(|f| f.name == format!("SLICE_X0Y0.{name}"))
            .map(|f| &f.ones)
    };
    let need = |name: &str| -> Result<Vec<ConfigBit>, UrayError> {
        feature(name)
            .cloned()
            .ok_or_else(|| bad(format!("the database has no `SLICE_X0Y0.{name}`")))
    };
    // A group's bit, from whichever members name it, all of which must agree.
    let group_bit = |members: &[String], tail: &str| -> Result<Vec<ConfigBit>, UrayError> {
        let mut found: Option<Vec<ConfigBit>> = None;
        for m in members {
            if let Some(b) = feature(&format!("{m}.{tail}")) {
                match &found {
                    None => found = Some(b.clone()),
                    Some(f) if f == b => {}
                    Some(f) => {
                        return Err(bad(format!(
                            "`{m}.{tail}` is {b:?} where the rest of its group has {f:?}"
                        )));
                    }
                }
            }
        }
        found.ok_or_else(|| bad(format!("no member of {members:?} names `{tail}`")))
    };

    let mut model = SliceModel::default();
    for letter in LUT_LETTERS {
        let mut lut = BelDecl::new(format!("{letter}6LUT"), "lut");
        for i in 1..=6 {
            lut.pins.push((
                format!("i{}", i - 1),
                WireRef::local(wire(&format!("{letter}{i}"))?),
            ));
        }
        lut.pins.push((
            "o".to_owned(),
            WireRef::local(wire(&format!("{letter}_O"))?),
        ));
        for (index, at) in lut_init_bits(bits, letter)?.iter().enumerate() {
            lut.config.push(ConfigEntry::Param {
                name: "INIT".to_owned(),
                index: u32::try_from(index).unwrap_or(0),
                at: *at,
            });
        }
        model.bels.push(lut);
    }

    for upper in [false, true] {
        let letters: &[char] = if upper {
            &['E', 'F', 'G', 'H']
        } else {
            &['A', 'B', 'C', 'D']
        };
        let half = if upper { "EFGH" } else { "ABCD" };
        let all: Vec<String> = letters
            .iter()
            .flat_map(|l| [format!("{l}FF"), format!("{l}FF2")])
            .collect();
        let sync = group_bit(&all, "SYNC_ATTR=SYNC")?;
        for second in [false, true] {
            let members: Vec<String> = letters
                .iter()
                .map(|l| format!("{l}FF{}", if second { "2" } else { "" }))
                .collect();
            let (clock, enable, reset) = control_pins(letters[0], second);
            let group = format!("{half}{}", if second { "2" } else { "1" });
            let ce = format!("RETICLE_SLICE_CE_{group}");
            let sr = format!("RETICLE_SLICE_SR_{group}");
            model.pips.push(PipDecl {
                from: WireRef::local(wire(enable)?),
                to: WireRef::local(ce.clone()),
                bits: group_bit(&members, "CE_ACTIVE=TRUE")?,
            });
            model.pips.push(PipDecl {
                from: WireRef::local(wire(reset)?),
                to: WireRef::local(sr.clone()),
                bits: group_bit(&members, "SR_ACTIVE=TRUE")?,
            });
            model.wires.push(ce.clone());
            model.wires.push(sr.clone());
            for (letter, name) in letters.iter().zip(&members) {
                let n = if second { "2" } else { "1" };
                let mut ff = BelDecl::new(name.clone(), "ff");
                let data = if second {
                    format!("{letter}_I")
                } else {
                    format!("{letter}X")
                };
                let q = if second {
                    format!("{letter}Q2")
                } else {
                    format!("{letter}Q")
                };
                ff.pins
                    .push(("clk".to_owned(), WireRef::local(wire(clock)?)));
                ff.pins.push(("d".to_owned(), WireRef::local(wire(&data)?)));
                ff.pins.push(("q".to_owned(), WireRef::local(wire(&q)?)));
                ff.pins.push(("en".to_owned(), WireRef::local(ce.clone())));
                ff.pins.push(("rst".to_owned(), WireRef::local(sr.clone())));
                let mut fdre = need(&format!("FFMUX{letter}{n}.SP.BYP.OUT{n}"))?;
                fdre.extend(need(&format!("{name}.FFSR=SRLOW"))?);
                fdre.extend(sync.iter().copied());
                ff.config.push(ConfigEntry::Cell {
                    primitive: "FDRE".to_owned(),
                    bits: fdre,
                });
                for at in need(&format!("{name}.FFINIT=INIT0"))? {
                    ff.config.push(ConfigEntry::ParamZero {
                        name: "INIT".to_owned(),
                        index: 0,
                        at,
                    });
                }
                model.bels.push(ff);
            }
        }
    }
    Ok(model)
}
