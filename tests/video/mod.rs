//! A 40 x 24 text screen read back off a video signal's own waveform,
//! shared by the tests that decode what a machine draws.
//!
//! This is to a picture what `tests/serial/mod.rs` is to a line of text:
//! the decoder that turns what a pin did into what a person would have
//! seen, written from the raster's arithmetic rather than from the
//! design. Nothing here reads the RTL. The timings are the VESA DMT
//! numbers for 640 x 480 at 60 Hz, typed out below; the picture's place
//! on the raster is 560 x 384 centred in 640 x 480; the pixel period is
//! the testbench's clock; and the only thing taken from a simulation is
//! when each signal changed.
//!
//! `examples/apple2` has two video paths and the same screen comes out
//! of both, so both tests use this: one samples the 24-bit colour bus
//! the machine hands to `dvi_tx`, the other the twelve VGA pins
//! `vga_out` drives. [`Signal`] is the whole of the difference — what
//! value counts as white, and how many pixels the block's output
//! register puts between the raster and the pin.

#![allow(dead_code)]

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// The raster, stated here rather than read out of any design
// ---------------------------------------------------------------------------

/// The VESA DMT timing of 640 x 480 at 60 Hz, which is what `dvi_tx`
/// and `vga_out` both call MODE 0: 640 active pixels, 16 front porch,
/// 96 sync, 48 back porch; 480 active lines, 10, 2 and 33.
pub(crate) const H_ACTIVE: i64 = 640;
pub(crate) const H_TOTAL: i64 = 640 + 16 + 96 + 48;
pub(crate) const V_ACTIVE: i64 = 480;
pub(crate) const V_TOTAL: i64 = 480 + 10 + 2 + 33;
/// Pixel slots in one frame, blanking and all.
pub(crate) const FRAME_PIXELS: i64 = H_TOTAL * V_TOTAL;

/// The text screen: 40 columns of 24 rows, in cells seven dots wide and
/// eight scan lines tall, every dot drawn twice.
pub(crate) const COLS: usize = 40;
pub(crate) const ROWS: usize = 24;
pub(crate) const CELL_W: usize = 7;
pub(crate) const CELL_H: usize = 8;
pub(crate) const SCALE: i64 = 2;
/// Where the picture sits on the raster, centred.
pub(crate) const PIC_W: i64 = (COLS * CELL_W) as i64 * SCALE;
pub(crate) const PIC_H: i64 = (ROWS * CELL_H) as i64 * SCALE;
pub(crate) const X_LEFT: i64 = (H_ACTIVE - PIC_W) / 2;
pub(crate) const Y_TOP: i64 = (V_ACTIVE - PIC_H) / 2;

// ---------------------------------------------------------------------------
// The font, as drawn
// ---------------------------------------------------------------------------

/// One glyph: eight rows of seven dots.
pub(crate) type Glyph = [[bool; CELL_W]; CELL_H];

/// Reads `sw/font.txt`: every `glyph` line and the eight lines after it.
///
/// The eight-lines-after rule is the whole grammar, and it has to be,
/// because an art line may itself begin with `#`.
pub(crate) fn font_art(text: &str) -> Vec<Glyph> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("glyph ") else {
            continue;
        };
        let index =
            usize::from_str_radix(rest.split_whitespace().next().expect("a glyph index"), 16)
                .expect("a hexadecimal glyph index");
        assert_eq!(index, out.len(), "the glyphs are out of order");
        let mut glyph = [[false; CELL_W]; CELL_H];
        for (row, art) in glyph.iter_mut().enumerate() {
            let source = lines
                .get(at + 1 + row)
                .unwrap_or_else(|| panic!("glyph {index:02X} is cut short"));
            let dots: Vec<char> = source.chars().collect();
            assert_eq!(
                dots.len(),
                CELL_W,
                "glyph {index:02X} row {row} is `{source}`, which is not {CELL_W} dots"
            );
            for (column, dot) in art.iter_mut().enumerate() {
                *dot = match dots[column] {
                    '#' => true,
                    '.' => false,
                    other => panic!("glyph {index:02X} row {row} has a `{other}` in it"),
                };
            }
        }
        out.push(glyph);
    }
    out
}

/// The ASCII a glyph index stands for: 00..1F are $40..$5F and 20..3F are
/// $20..$3F, which is how the display code's low six bits address the
/// character generator.
pub(crate) fn glyph_ascii(index: usize) -> u8 {
    let index = u8::try_from(index).expect("a small glyph index");
    if index < 32 {
        0x40 + index
    } else {
        0x20 + (index - 32)
    }
}

/// The glyph index of an ASCII character, the other way round.
pub(crate) fn ascii_glyph(c: u8) -> usize {
    assert!((0x20..=0x5F).contains(&c), "{c:#04x} is not in the font");
    if c >= 0x40 {
        (c - 0x40) as usize
    } else {
        (c - 0x20) as usize + 32
    }
}

// ---------------------------------------------------------------------------
// Reading the screen off the signal
// ---------------------------------------------------------------------------

/// A pixel's colour, or `None` if the signal was `x` there.
pub(crate) type Pixel = Option<u32>;

/// A colour bus's changes, as `(time, value)`, which is what
/// `serial::Waveform` is for a one-bit net.
pub(crate) type Colours = Vec<(u64, Pixel)>;

/// What one character cell turned out to be: a glyph of the font, and
/// whether it was drawn inverted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Cell {
    pub(crate) glyph: usize,
    pub(crate) inverse: bool,
}

/// How one video path carries a black-and-white picture.
///
/// Two numbers, because that is all that differs between reading the
/// colour bus a DVI transmitter is fed and reading the pins a VGA block
/// drives: what value a lit dot has, and how far behind the raster the
/// signal runs.
#[derive(Clone, Copy)]
pub(crate) struct Signal {
    /// The value a lit dot carries. Anything else must be 0.
    pub(crate) white: u32,
    /// Pixels between the raster naming a pixel and the signal carrying
    /// its colour. A block whose outputs are registered has one.
    pub(crate) delay: i64,
}

impl Signal {
    /// The 24-bit colour bus `apple2` hands to `dvi_tx`, which is
    /// combinational and so is in step with the raster.
    pub(crate) const DVI: Signal = Signal {
        white: 0x00FF_FFFF,
        delay: 0,
    };

    /// The twelve VGA pins of a four-bit-per-channel ladder. `vga_out`
    /// registers its pins, so they run one pixel behind — which is why
    /// the block truncating rather than rounding matters here: white in
    /// is `0xFFFFFF`, and the top four bits of each channel are `0xFFF`.
    pub(crate) const VGA4: Signal = Signal {
        white: 0x0FFF,
        delay: 1,
    };
}

/// The video signal, tied to the raster.
pub(crate) struct Video {
    /// Every change of the colour, by pixel number.
    samples: Vec<(i64, Pixel)>,
    /// The last pixel the run reached.
    end: i64,
    /// What the path carries.
    signal: Signal,
    /// The simulation time at which this path carries pixel 0 of frame
    /// 0, and how long a pixel is in the same units.
    origin: i64,
    period: i64,
}

impl Video {
    pub(crate) fn new(
        de_wave: &[(u64, Option<bool>)],
        colour_wave: &Colours,
        period: u64,
        end: u64,
        signal: Signal,
    ) -> Video {
        let period = i64::try_from(period).expect("a sane period");

        // `de` is high from the moment the raster leaves reset, because
        // it starts at x = 0, y = 0 and that pixel is visible. So the
        // first *falling* edge is the end of the first visible line, 640
        // pixels in, and that is what ties simulation time to pixel
        // number.
        let falls: Vec<u64> = de_wave
            .iter()
            .filter(|(_, level)| *level == Some(false))
            .map(|(when, _)| *when)
            .collect();
        assert!(!falls.is_empty(), "`de` never went low: there is no raster");
        let origin = i64::try_from(falls[0]).expect("a sane time") - H_ACTIVE * period;

        // Every one of those falling edges is at the same place on its
        // own line, so the whole raster is checked by checking them: a
        // line is H_TOTAL pixels and a frame is V_ACTIVE visible lines
        // followed by V_TOTAL - V_ACTIVE blank ones.
        for (index, when) in falls.iter().enumerate() {
            let line = i64::try_from(index).expect("a sane count");
            let pixel = (line / V_ACTIVE) * FRAME_PIXELS + (line % V_ACTIVE) * H_TOTAL + H_ACTIVE;
            assert_eq!(
                i64::try_from(*when).expect("a sane time"),
                origin + pixel * period,
                "the {index}th end of a visible line is not where the raster says"
            );
        }

        // Where pixel 0's colour appears on *this* path, which is the
        // raster's origin plus however many pixels the path is behind.
        let origin = origin + signal.delay * period;

        // The pixel each change of the colour belongs to. A change lands
        // on a clock edge, which is the start of a pixel.
        let mut samples: Vec<(i64, Pixel)> = Vec::with_capacity(colour_wave.len());
        // Everything before the raster was let go is the machine booting
        // against a frozen `x` and `y`; only the colour it left behind
        // matters, and it becomes the colour pixel 0 starts from.
        let mut before = None;
        for (when, value) in colour_wave {
            let offset = i64::try_from(*when).expect("a sane time") - origin;
            if offset < 0 {
                before = Some(*value);
                continue;
            }
            assert_eq!(
                offset.rem_euclid(period),
                0,
                "the colour changed between pixels, at {when}"
            );
            samples.push((offset / period, *value));
        }
        samples.sort_by_key(|(pixel, _)| *pixel);
        samples.insert(0, (-1, before.expect("the colour was never driven")));
        assert!(samples.len() > 1, "the colour never changed");
        Video {
            samples,
            end: (i64::try_from(end).expect("a sane time") - origin) / period,
            signal,
            origin,
            period,
        }
    }

    /// The colour of one pixel of the raster. `cursor` walks forward, so
    /// the caller must ask in raster order.
    fn at(&self, cursor: &mut usize, pixel: i64) -> Pixel {
        assert!(
            pixel <= self.end,
            "pixel {pixel} is past the end of the run"
        );
        while *cursor + 1 < self.samples.len() && self.samples[*cursor + 1].0 <= pixel {
            *cursor += 1;
        }
        assert!(
            self.samples[*cursor].0 <= pixel,
            "nothing drove the colour before pixel {pixel}"
        );
        self.samples[*cursor].1
    }

    /// Everything of frame `frame` that is visible and outside the
    /// picture is black, which is what proves the picture is centred
    /// rather than merely the right size.
    pub(crate) fn border_is_black(&self, frame: i64) {
        let base = frame * FRAME_PIXELS;
        let mut cursor = 0usize;
        for y in 0..V_ACTIVE {
            for x in [0, X_LEFT - 1, X_LEFT + PIC_W, H_ACTIVE - 1] {
                if (Y_TOP..Y_TOP + PIC_H).contains(&y) && (X_LEFT..X_LEFT + PIC_W).contains(&x) {
                    continue;
                }
                assert_eq!(
                    self.at(&mut cursor, base + y * H_TOTAL + x),
                    Some(0),
                    "the border at ({x}, {y}) of frame {frame} is not black"
                );
            }
        }
    }

    /// Text rows `rows` of frame `frame`, read back as characters.
    ///
    /// On the way it undoes the pixel doubling and proves it *was*
    /// doubling: every dot of the picture is a square of four identical
    /// pixels.
    pub(crate) fn rows(
        &self,
        frame: i64,
        rows: std::ops::Range<usize>,
        glyphs: &[Glyph],
    ) -> Vec<Vec<Cell>> {
        let base = frame * FRAME_PIXELS;
        let mut cursor = 0usize;
        let mut out = Vec::new();
        let scale = usize::try_from(SCALE).expect("a small scale");
        for row in rows {
            // The sixteen scan lines of this row, 560 pixels each.
            let top = Y_TOP + i64::try_from(row * CELL_H * scale).expect("a small screen");
            let tall = i64::try_from(CELL_H * scale).expect("a small cell");
            let mut lines = Vec::with_capacity(CELL_H * scale);
            for line in 0..tall {
                let y = top + line;
                let pixels: Vec<Pixel> = (0..PIC_W)
                    .map(|column| self.at(&mut cursor, base + y * H_TOTAL + X_LEFT + column))
                    .collect();
                lines.push(pixels);
            }
            // Each dot is a 2 x 2 square.
            let mut dots = vec![vec![None; CELL_W * COLS]; CELL_H];
            for (dy, line) in dots.iter_mut().enumerate() {
                for (dx, dot) in line.iter_mut().enumerate() {
                    let colour = lines[dy * scale][dx * scale];
                    for (oy, ox) in [(0, 1), (1, 0), (1, 1)] {
                        assert_eq!(
                            lines[dy * scale + oy][dx * scale + ox],
                            colour,
                            "the dot at ({dx}, {dy}) of row {row} is not a square of four pixels"
                        );
                    }
                    *dot = colour;
                }
            }
            // And cut it into cells, matching each against the font.
            let mut line = Vec::with_capacity(COLS);
            for column in 0..COLS {
                let mut pattern = [[false; CELL_W]; CELL_H];
                for (dy, art) in pattern.iter_mut().enumerate() {
                    for (dx, lit) in art.iter_mut().enumerate() {
                        *lit = match dots[dy][column * CELL_W + dx] {
                            Some(0) => false,
                            Some(value) if value == self.signal.white => true,
                            other => panic!(
                                "the cell at ({column}, {row}) has {other:?} in it, which is \
                                 neither black nor white"
                            ),
                        };
                    }
                }
                line.push(match_glyph(&pattern, glyphs, column, row));
            }
            out.push(line);
        }
        out
    }

    /// Every line of frame `frame` sees the syncs where the raster puts
    /// them, and at the polarity the mode uses.
    ///
    /// The DVI path carries the syncs inside its TMDS control symbols
    /// and has no pin for them; a VGA path has two, and this is what
    /// reads them. `hsync` and `vsync` are those two pins' waveforms and
    /// `active` is the level a pulse has — low, for 640 x 480. Four
    /// places on each of the 525 lines are checked: the first visible
    /// pixel, the last one, the first pixel of the sync, and the last
    /// pixel of the back porch.
    pub(crate) fn syncs_are_the_rasters(
        &self,
        frame: i64,
        hsync: &[(u64, Option<bool>)],
        vsync: &[(u64, Option<bool>)],
        active: bool,
    ) {
        // The pins are sampled the way the colour is: the value a pin
        // held at a pixel is the last change at or before it, and the
        // syncs leave through the same register as the colour, so
        // `origin` is theirs too.
        let level = |wave: &[(u64, Option<bool>)], pixel: i64| -> bool {
            let when = self.origin + pixel * self.period;
            wave.iter()
                .take_while(|(t, _)| i64::try_from(*t).expect("a sane time") <= when)
                .last()
                .and_then(|(_, level)| *level)
                .unwrap_or_else(|| panic!("the sync pin was x at pixel {pixel}"))
        };
        let base = frame * FRAME_PIXELS;
        let mut h_pulses = 0i64;
        let mut v_lines = BTreeSet::new();
        for y in 0..V_TOTAL {
            for x in [0, H_ACTIVE - 1, H_ACTIVE + 16, H_TOTAL - 1] {
                let pixel = base + y * H_TOTAL + x;
                assert!(pixel <= self.end, "the run is shorter than a whole frame");
                let want_h = (H_ACTIVE + 16..H_ACTIVE + 16 + 96).contains(&x);
                assert_eq!(
                    level(hsync, pixel) == active,
                    want_h,
                    "hsync at ({x}, {y}) of frame {frame}"
                );
                let want_v = (V_ACTIVE + 10..V_ACTIVE + 10 + 2).contains(&y);
                assert_eq!(
                    level(vsync, pixel) == active,
                    want_v,
                    "vsync at ({x}, {y}) of frame {frame}"
                );
                if want_h {
                    h_pulses += 1;
                }
                if want_v {
                    v_lines.insert(y);
                }
            }
        }
        assert_eq!(h_pulses, V_TOTAL, "one hsync pulse per line");
        assert_eq!(v_lines.len(), 2, "two lines of vsync");
    }
}

/// The glyph, and the polarity, a cell was drawn with.
pub(crate) fn match_glyph(pattern: &Glyph, glyphs: &[Glyph], column: usize, row: usize) -> Cell {
    let found = glyphs.iter().enumerate().find_map(|(glyph, art)| {
        for inverse in [false, true] {
            if (0..CELL_H).all(|dy| (0..CELL_W).all(|dx| art[dy][dx] ^ inverse == pattern[dy][dx]))
            {
                return Some(Cell { glyph, inverse });
            }
        }
        None
    });
    found.unwrap_or_else(|| {
        let art: String = pattern
            .iter()
            .map(|r| {
                r.iter()
                    .map(|d| if *d { '#' } else { '.' })
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        panic!("the cell at ({column}, {row}) is in no font:\n{art}")
    })
}

/// A row of the expected screen: text, left-aligned, the rest spaces.
pub(crate) fn expect_row(text: &str) -> Vec<Cell> {
    assert!(text.len() <= COLS, "`{text}` is wider than the screen");
    let mut row: Vec<Cell> = text
        .bytes()
        .map(|c| Cell {
            glyph: ascii_glyph(c),
            inverse: false,
        })
        .collect();
    while row.len() < COLS {
        row.push(Cell {
            glyph: ascii_glyph(b' '),
            inverse: false,
        });
    }
    row
}

/// The test card the monitor's `T` command paints: glyph 7 * row + column,
/// so that no two rows are alike and no two columns of a row are.
pub(crate) fn test_card(row: usize, column: usize) -> Cell {
    Cell {
        glyph: (7 * row + column) & 0x3F,
        inverse: false,
    }
}

/// The screen as text, one row per line, for a message.
pub(crate) fn as_text(rows: &[Vec<Cell>]) -> String {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|c| glyph_ascii(c.glyph) as char)
                .collect::<String>()
                + "\n"
        })
        .collect()
}
