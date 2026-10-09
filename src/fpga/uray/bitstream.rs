//! Writing an UltraScale+ `.bit`, and the `.bin` the Linux FPGA manager
//! loads.
//!
//! # Where the sequence comes from
//!
//! The packets, the registers and the CRC are the 7-series ones (UG470),
//! and [`xc7`] builds them. What differs is the recipe: Vivado's ZCU104
//! bitstream (`docs/fpga-uray.md`, "The oracle") was read word by word,
//! and [`write_bit`] writes the same words in the same order with only
//! the frames and the IDCODE as inputs. `tests/fpga_uray.rs` holds it to
//! that: given that bitstream's own header and frames, the output is
//! identical to it, byte for byte.
//!
//! What UG570 documents is each register; what the order is, and the
//! values of `COR0`, `COR1` and `CTL0` below, are Vivado's choice, copied.

use super::{BitWindow, FrameAddress, FrameLayout, TileGrid, UrayError, WORDS_PER_FRAME};
use crate::fpga::arch::ConfigBit;
use crate::fpga::xc7::{self, BitHeader, Command, Register};

/// Dummy words before the bus-width pattern: sixteen, where a 7-series
/// `.bit` from Vivado has eight. Measured.
pub const LEADING_DUMMY_WORDS: usize = 16;

/// The IDCODE of an XCZU7EV, as Vivado writes it and as the database's
/// readme gives the die.
pub const IDCODE_XCZU7EV: u32 = 0x04A5_A093;

/// Vivado's `COR0`, `COR1`, `CTL0` and the masks around it, verbatim.
const COR0: u32 = 0x3800_3FE5;
const COR1: u32 = 0x0040_0000;
const CTL0: u32 = 0x0000_0101;
const CTL0_MASK: u32 = 0x0000_0001;
const STARTUP_MASK: u32 = 0x0000_0101;
/// `RBCRC`, register 19, which UG570 names and UG470 reserves.
const RBCRC: u32 = 19;
/// Where `FAR` is left after configuration: block type 7, which no frame
/// has.
const PARKED_FAR: u32 = 0x07FC_0000;

/// Writes a `.bit` holding `frames`, which must be the whole stream
/// `layout` describes, pad frames included.
///
/// # Errors
///
/// [`UrayError::WrongSize`] when `frames` is not the layout's length.
pub fn write_bit(
    header: &BitHeader,
    idcode: u32,
    layout: &FrameLayout,
    frames: &[u32],
) -> Result<Vec<u8>, UrayError> {
    let expected = layout.frames() * WORDS_PER_FRAME;
    if frames.len() != expected {
        return Err(UrayError::WrongSize {
            expected,
            found: frames.len(),
        });
    }
    let mut stream = xc7::Stream::new();
    stream.nops(2);
    stream.write(Register::Timer, 0);
    stream.write(Register::Wbstar, 0);
    stream.command(Command::Null);
    stream.nops(1);
    stream.command(Command::Rcrc);
    stream.nops(2);
    stream.write(Register::Far, 0);
    stream.write_raw(RBCRC, 0);
    stream.write(Register::Cor0, COR0);
    stream.write(Register::Cor1, COR1);
    stream.write(Register::Idcode, idcode);
    stream.command(Command::Switch);
    stream.nops(1);
    stream.write(Register::Mask, CTL0_MASK);
    stream.write(Register::Ctl0, CTL0);
    stream.write(Register::Mask, 0);
    stream.write(Register::Ctl1, 0);
    stream.nops(8);
    stream.write(Register::Far, 0);
    stream.command(Command::Wcfg);
    stream.nops(1);
    stream.frames(frames);
    stream.checkpoint();
    stream.nops(2);
    stream.command(Command::Grestore);
    stream.nops(2);
    stream.command(Command::Lfrm);
    stream.nops(20);
    stream.command(Command::Start);
    stream.nops(1);
    stream.write(Register::Far, PARKED_FAR);
    stream.write(Register::Mask, STARTUP_MASK);
    stream.write(Register::Ctl0, CTL0);
    stream.checkpoint();
    stream.nops(2);
    stream.command(Command::Desync);
    stream.nops(400);
    Ok(xc7::wrap(header, LEADING_DUMMY_WORDS, &stream.words))
}

/// The `.bin` the Linux FPGA manager loads, from a `.bit`: everything
/// after the header, every 32-bit word byte-swapped.
///
/// Measured: this of Vivado's ZCU104 `base.bit` is identical to the
/// `base.bin` PYNQ loads at boot, all 19 311 092 bytes.
///
/// # Errors
///
/// [`UrayError::Bit`] for a file that is not a `.bit`, and
/// [`UrayError::Malformed`] for one whose stream is not whole words.
pub fn bin_from_bit(bit: &[u8]) -> Result<Vec<u8>, UrayError> {
    let start = xc7::payload_start(bit)?;
    let body = &bit[start..];
    if !body.len().is_multiple_of(4) {
        return Err(UrayError::Malformed {
            path: "the .bit".to_owned(),
            message: format!("its stream is {} byte(s), not whole words", body.len()),
        });
    }
    Ok(body
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|w| [w[3], w[2], w[1], w[0]])
        .collect())
}

/// Paints tile bits into a frame stream.
///
/// `bits` names each tile by its index in `grid` and each bit as the
/// tile's own `(frame, bit)` — a `segbits` line's `<frame>_<bit>`. A tile
/// with two windows (a block RAM: its configuration, then its contents)
/// numbers its frames through the first window and on into the second.
///
/// # Errors
///
/// [`UrayError::Malformed`] for a bit outside the tile's windows, or a
/// window outside the layout.
pub fn frames_from_tile_bits(
    grid: &TileGrid,
    layout: &FrameLayout,
    bits: impl IntoIterator<Item = (usize, ConfigBit)>,
) -> Result<Vec<u32>, UrayError> {
    let mut frames = vec![0u32; layout.frames() * WORDS_PER_FRAME];
    for (tile, bit) in bits {
        let Some(t) = grid.tiles().get(tile) else {
            return Err(outside(format!("there is no tile {tile}")));
        };
        let (window, frame) = locate(&t.windows, bit.row)
            .ok_or_else(|| outside(format!("`{}` has no frame {}", t.name, bit.row)))?;
        if bit.col >= window.bits {
            return Err(outside(format!(
                "`{}` has {} bit(s) per frame and bit {} was asked for",
                t.name, window.bits, bit.col
            )));
        }
        let address = FrameAddress {
            minor: u8::try_from(u32::from(window.base.minor) + frame).map_err(|_| {
                outside(format!("`{}` frame {} is past minor 255", t.name, bit.row))
            })?,
            ..window.base
        };
        let position = layout
            .position(address)
            .ok_or_else(|| outside(format!("the layout has no frame {address}")))?;
        let index = usize::try_from(window.offset + bit.col).unwrap_or(usize::MAX);
        frames[position * WORDS_PER_FRAME + index / 32] |= 1 << (index % 32);
    }
    Ok(frames)
}

/// The window a tile frame falls in, and the frame within it.
fn locate(windows: &[BitWindow], mut row: u32) -> Option<(&BitWindow, u32)> {
    for window in windows {
        if row < window.frames {
            return Some((window, row));
        }
        row -= window.frames;
    }
    None
}

fn outside(message: String) -> UrayError {
    UrayError::Malformed {
        path: "tile bits".to_owned(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::uray::{Tile, TileGrid};

    fn grid() -> TileGrid {
        let window = |block, frames, offset, bits| BitWindow {
            base: FrameAddress {
                block,
                row: 0,
                column: 0,
                minor: 0,
            },
            frames,
            offset,
            bits,
        };
        TileGrid::from_tiles(vec![Tile {
            name: "BRAM_X0Y0".to_owned(),
            kind: "BRAM".to_owned(),
            grid: (0, 0),
            windows: vec![window(0, 2, 64, 48), window(1, 3, 0, 48)],
        }])
    }

    #[test]
    fn a_tile_bit_lands_in_its_window_and_the_second_window_follows_the_first() {
        let grid = grid();
        let layout = FrameLayout::from_grid(&grid);
        let frames = frames_from_tile_bits(
            &grid,
            &layout,
            [(0, ConfigBit::new(1, 5)), (0, ConfigBit::new(3, 0))],
        )
        .unwrap();
        // Frame 1 of block 0, bit 64 + 5.
        assert_eq!(frames[WORDS_PER_FRAME + 2], 1 << 5);
        // Tile frame 3 is frame 1 of the second window, block 1, which
        // follows block 0's two frames and two pad frames.
        assert_eq!(frames[(2 + 2 + 1) * WORDS_PER_FRAME], 1);
        assert_eq!(frames.iter().map(|w| w.count_ones()).sum::<u32>(), 2);
        assert!(frames_from_tile_bits(&grid, &layout, [(0, ConfigBit::new(5, 0))]).is_err());
        assert!(frames_from_tile_bits(&grid, &layout, [(0, ConfigBit::new(0, 48))]).is_err());
    }

    #[test]
    fn a_bin_is_the_stream_with_its_words_swapped() {
        let grid = grid();
        let layout = FrameLayout::from_grid(&grid);
        let frames = vec![0u32; layout.frames() * WORDS_PER_FRAME];
        let bit = write_bit(
            &BitHeader::new("t", "xczu7ev"),
            IDCODE_XCZU7EV,
            &layout,
            &frames,
        )
        .unwrap();
        let bin = bin_from_bit(&bit).unwrap();
        // Sixteen dummy words, then the bus-width pattern, swapped.
        assert_eq!(&bin[..4], &[0xFF; 4]);
        assert_eq!(&bin[64..72], &[0xBB, 0, 0, 0, 0x44, 0x00, 0x22, 0x11]);
        let read = crate::fpga::uray::read_bit(&bit).unwrap();
        assert_eq!(read.idcode, Some(IDCODE_XCZU7EV));
        assert_eq!(read.frames, frames);
    }
}
