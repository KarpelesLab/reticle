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
