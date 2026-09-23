//! `examples/apple2`, driven through the same chain as
//! `examples/mos6502_computer` — with one test that neither of the other
//! examples could have.
//!
//! `examples/soc` and `examples/mos6502_computer` both end at a serial
//! wire: a string goes in one end of the design and comes out of a pin,
//! and the test decodes the pin's waveform. This machine ends at a
//! *screen*, and the analogous test is
//! `the_screen_comes_out_of_the_video_signal`: it samples the RGB and DE
//! the video generator hands to `dvi_tx` across a whole frame, rebuilds
//! the 640 x 480 picture from the raster's own arithmetic, undoes the
//! pixel doubling, cuts the result into 7 x 8 character cells, matches
//! each one against the font *as drawn in `sw/font.txt`*, and asserts the
//! forty columns of twenty-four rows that come out — glyph *and*
//! polarity, so the inverse and flashing attributes are read back too,
//! the second of them from a row of a second frame. Nothing in it looks
//! at the text page, at the character generator or at any register inside
//! the design.
//!
//! That matters most for the one thing this example exists to build: the
//! Apple II's interleaved text page, where line N lives at $0400 + 128 *
//! (N mod 8) + 40 * (N div 8) and not at $0400 + 40 * N. Two tests hold
//! it from opposite sides and neither can agree with a wrong
//! implementation:
//!
//! * `the_monitors_line_table_is_the_documented_interleave` computes the
//!   forty-eight bytes from that formula, here, and compares them with
//!   the table assembled into the ROM. The monitor is therefore known to
//!   put row N where the documentation says row N is.
//! * `the_screen_comes_out_of_the_video_signal` reads the picture off the
//!   video signal and finds, in row N, what the monitor wrote to row N.
//!   The screen the monitor paints has a different pattern on every row
//!   and a different character in every column, on purpose, so no
//!   permutation of the rows and no shift of the columns survives it.
//!
//! The hardware computes the address by concatenation and the test
//! computes it by arithmetic; they meet in the middle, on a wire.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `font_hex_is_the_drawn_font` | `sw/font.hex` is `sw/font.txt`'s art, dot for dot |
//! | `the_font_tells_its_glyphs_apart` | no two of the 128 patterns a cell can show — 64 glyphs, normal and inverse — are the same, so reading a cell back off the screen has one answer |
//! | `monitor_hex_is_the_assembled_source` | `sw/monitor.hex` is `sw/monitor.s` assembled, inside the ROM, with the three vectors pointing at the labels the source names |
//! | `the_monitors_line_table_is_the_documented_interleave` | the ROM's line table is $0400 + 128 * (N mod 8) + 40 * (N div 8), computed here from the formula |
//! | `the_project_resolves_and_elaborates` | the manifest builds from exactly three library packages and three user sources |
//! | `the_machine_synthesises_without_errors_or_latches` | synthesis has no error, no warning and no latch, and both memories hold what their files say |
//! | `the_screen_comes_out_of_the_video_signal` | the whole chain: forty by twenty-four characters decoded from two frames of video, the session decoded again off `uart_tx` (which is also what proves the keyboard, since the testbench typed it), and the speaker counted |
//! | `the_assembler_takes_the_low_and_high_byte_of_an_address` | the `<` and `>` operators `tests/mos6502_asm` gained for this example |
//! | `the_machine_maps_onto_the_ecp5_and_exports_for_nextpnr` | the ECP5 flow fits it, puts the monitor and the font in block RAM, builds the PLL and the DDR outputs, and writes the JSON and LPF `nextpnr-ecp5` reads |
//! | `the_machine_does_not_fit_the_ice40_hx8k` | it does not fit the part the other two examples use, and the reason is the 48 KiB of RAM |
//! | `reticle_build_builds_the_project` | the same through the binary |
//! | `reticle_sim_runs_the_testbench` | the banner comes out of `reticle sim` too |
//! | `reticle_fpga_exports_the_rom_with_the_monitor_in_it` | and `reticle fpga` exports memories that are not blank |
//!
//! Two defects in Reticle turned up building this, both in
//! `src/fpga/constraints.rs` and both about a design that is still
//! hierarchical when its board file is checked: a port whose attributes
//! carry IO options but no pin was reported as having its (empty) pin
//! overridden, and the reference clock of a PLL request was reported as
//! clocking nothing, because "nothing drives this net" was being answered
//! by a rule that counts a net an instance merely *reads*. Both are fixed
//! with a regression test next to the code.
//!
//! What none of it proves is the board: none is attached here, Reticle's
//! place and route uses a synthetic fabric, and
//! `examples/apple2/README.md` ends with a section on exactly that.
//!
//! `examples/` is not in the published crate, so every test that needs
//! the example skips with a message when it is absent.

#![cfg(all(
    feature = "ip",
    feature = "verilog",
    feature = "sim",
    feature = "synth",
    feature = "fpga"
))]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use reticle::diag::{Diagnostics, Severity};
use reticle::fpga::{self, Constraints, FpgaOptions};
use reticle::ip::{self, Elaboration, PathProvider, Project, SourceEntry};
use reticle::ir::{CellKind, Delay, Design, TimeUnit};
use reticle::sim::{MemoryFiles, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run as synth_run};

/// The 6502 assembler built from the documented opcode matrix, shared
/// with `tests/mos6502_computer.rs` and `tests/ip_library.rs`.
#[path = "mos6502_asm/mod.rs"]
mod asm;

/// The 8N1 receiver that reads a line off a pin's waveform, shared with
/// `tests/soc.rs` and `tests/mos6502_computer.rs`.
#[path = "serial/mod.rs"]
mod serial;

use serial::{Waveform, decode_uart};

// ---------------------------------------------------------------------------
// The machine's shape, stated here rather than read out of the design
// ---------------------------------------------------------------------------

/// The part the project targets.
const DEVICE: &str = "ecp5-45f-CABGA381";

/// The part `examples/soc` and `examples/mos6502_computer` target, which
/// this one is measured against and does not fit.
const SMALL_DEVICE: &str = "ice40-hx8k-ct256";

/// The testbench's clock: half a period, in nanoseconds, and one pixel
/// per period.
const HALF_NS: u64 = 20;

/// Clock cycles per serial bit, as `tb/apple2_tb.v` configures the
/// machine and its own terminal.
const CLK_DIV: u64 = 16;

/// The first address the ROM answers to, which is element 0 of the ROM
/// and therefore offset 0 of `sw/monitor.hex`.
const ROM_BASE: u32 = 0xF800;

/// How many bytes the ROM holds, as `rtl/apple2.v` declares it.
const ROM_BYTES: u32 = 2048;

/// The three vectors, where the part has always had them.
const VEC_NMI: u16 = 0xFFFA;
const VEC_RES: u16 = 0xFFFC;
const VEC_IRQ: u16 = 0xFFFE;

/// The VESA DMT timing of the mode `dvi_tx` calls MODE 0, written out
/// here rather than read from `video_timing`: 640 active pixels, 16 front
/// porch, 96 sync, 48 back porch; 480 active lines, 10, 2 and 33.
const H_ACTIVE: i64 = 640;
const H_TOTAL: i64 = 640 + 16 + 96 + 48;
const V_ACTIVE: i64 = 480;
const V_TOTAL: i64 = 480 + 10 + 2 + 33;
/// Pixel slots in one frame, blanking and all.
const FRAME_PIXELS: i64 = H_TOTAL * V_TOTAL;

/// The text screen: 40 columns of 24 rows, in cells seven dots wide and
/// eight scan lines tall, every dot drawn twice.
const COLS: usize = 40;
const ROWS: usize = 24;
const CELL_W: usize = 7;
const CELL_H: usize = 8;
const SCALE: i64 = 2;
/// Where the picture sits on the raster, centred.
const PIC_W: i64 = (COLS * CELL_W) as i64 * SCALE;
const PIC_H: i64 = (ROWS * CELL_H) as i64 * SCALE;
const X_LEFT: i64 = (H_ACTIVE - PIC_W) / 2;
const Y_TOP: i64 = (V_ACTIVE - PIC_H) / 2;

/// A lit dot, in text: white.
const WHITE: u32 = 0x00FF_FFFF;

/// The display code of the monitor's cursor: flashing, glyph $1F, which
/// is ASCII `_`.
const CURSOR_GLYPH: usize = 0x1F;

/// The first row the monitor's `T` command paints over: the banner is
/// three rows and the session the testbench types is six more.
const CARD_TOP: usize = 9;

/// How many times `beep` in `sw/monitor.s` touches $C030, and therefore
/// how many times the speaker pin moves during the run.
const BEEP_TOGGLES: usize = 8;

/// The documented address of column 0 of text row `row`, on page one.
///
/// This is the formula and nothing else: it is not read back from the
/// RTL, from the ROM or from a table.
fn text_base(row: usize) -> u16 {
    let row = u16::try_from(row).expect("a small row");
    0x0400 + 128 * (row % 8) + 40 * (row / 8)
}

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

/// `examples/apple2`, when this copy of the crate has it.
fn example() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/apple2");
    if dir.join("reticle.proj").is_file() {
        Some(dir)
    } else {
        println!("skipping: examples/apple2 is not in this copy of the crate");
        None
    }
}

/// A file of the example, as text.
fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("examples/apple2/{rel}: {e}"))
}

/// An image as `$readmemh` reads it: a header, then runs of bytes, each
/// run introduced by the `@` offset it starts at.
fn render_hex(header: &str, image: &BTreeMap<u32, u8>) -> String {
    let mut out = String::from(header);
    let mut next = None;
    for (offset, byte) in image {
        if next != Some(*offset) {
            out.push_str(&format!("@{offset:03x}\n"));
        }
        out.push_str(&format!("{byte:02x}\n"));
        next = Some(offset + 1);
    }
    out
}

/// The bytes of a hex file, by offset, read the way `$readmemh` would.
fn hex_image(dir: &Path, rel: &str) -> BTreeMap<u32, u8> {
    let text = read(dir, rel);
    let mut out = BTreeMap::new();
    let mut at = 0u32;
    for line in text.lines() {
        for token in line.split("//").next().unwrap_or("").split_whitespace() {
            match token.strip_prefix('@') {
                Some(address) => at = u32::from_str_radix(address, 16).expect("an @ address"),
                None => {
                    out.insert(at, u8::from_str_radix(token, 16).expect("a hex byte"));
                    at += 1;
                }
            }
        }
    }
    out
}

/// Writes `expected` to `path` under `UPDATE_EXPECT`, and otherwise
/// insists the file already says it.
fn expect_file(path: &Path, expected: &str, how: &str) {
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        fs::write(path, expected).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
    let actual = fs::read_to_string(path).unwrap_or_default();
    assert!(
        actual == expected,
        "{} is not {how}; rerun with UPDATE_EXPECT=1 and commit both",
        path.display()
    );
}

/// A resolved and elaborated project.
struct Built {
    project: Project,
    elaboration: Elaboration,
}

/// Loads `reticle.proj`, lets `adjust` change it, then resolves and
/// elaborates it, requiring no diagnostics at error level.
fn build(dir: &Path, adjust: impl FnOnce(&mut Project)) -> Built {
    let text = read(dir, "reticle.proj");
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut project =
        ip::load_project(&mut map, "reticle.proj", &text, &mut diags).expect("reticle.proj parses");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    adjust(&mut project);

    let root = dir.to_path_buf();
    let mut provider = PathProvider::new(".", move |path: &str| {
        fs::read_to_string(root.join(path)).ok()
    });
    let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
    assert!(
        resolved.is_complete(),
        "the project does not resolve:\n{}",
        diags.render(resolved.source_map())
    );
    let elaboration = ip::elaborate(&project, &mut resolved, &mut diags);
    assert!(
        !diags.has_errors(),
        "the project does not build:\n{}",
        diags.render(resolved.source_map())
    );
    Built {
        project,
        elaboration,
    }
}

/// The design `reticle build` produces: `apple2_top` and the library.
fn machine_design(dir: &Path) -> Design {
    build(dir, |_| {})
        .elaboration
        .design
        .expect("the project elaborates to a design")
}

/// The testbench's design: the project with its `testbench` line added as
/// a source and the testbench as the top.
fn testbench_design(dir: &Path) -> Design {
    let built = build(dir, |project| {
        let bench = project
            .testbenches
            .first()
            .expect("a testbench line")
            .clone();
        project.sources.push(SourceEntry {
            path: bench,
            language: None,
            encrypted: false,
            span: project.span,
        });
        project.top = Some("apple2_tb".to_owned());
    });
    built.elaboration.design.expect("the testbench elaborates")
}

/// The files the design reads: the monitor and the character generator.
fn example_files(dir: &Path) -> MemoryFiles {
    let mut files = MemoryFiles::new();
    files.insert("sw/monitor.hex", read(dir, "sw/monitor.hex"));
    files.insert("sw/font.hex", read(dir, "sw/font.hex"));
    files
}

/// Synthesis options that let `$readmemh` load both memories.
fn synth_options(dir: &Path) -> SynthOptions {
    SynthOptions {
        files: Some(Rc::new(example_files(dir))),
        ..SynthOptions::default()
    }
}

/// The initial contents of the memory called `name`, wherever in the
/// design it is, byte by byte, with `None` for an element no file named.
fn memory_init(design: &Design, name: &str) -> Option<Vec<Option<u8>>> {
    let memory = design
        .modules
        .iter()
        .flat_map(|(_, m)| m.memories.iter().map(|(_, mem)| mem))
        .find(|m| m.name.as_str() == name)
        .unwrap_or_else(|| panic!("no memory called `{name}` in the design"));
    memory.init.as_ref().map(|init| {
        init.iter()
            .map(|w| w.to_u64().map(|v| u8::try_from(v).expect("a byte")))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// The font, as drawn
// ---------------------------------------------------------------------------

/// One glyph: eight rows of seven dots.
type Glyph = [[bool; CELL_W]; CELL_H];

/// Reads `sw/font.txt`: every `glyph` line and the eight lines after it.
///
/// The eight-lines-after rule is the whole grammar, and it has to be,
/// because an art line may itself begin with `#`.
fn font_art(text: &str) -> Vec<Glyph> {
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

/// The 512 bytes of the character generator: eight rows for each of 64
/// glyphs, the leftmost dot in bit 0.
fn font_bytes(glyphs: &[Glyph]) -> Vec<u8> {
    assert_eq!(glyphs.len(), 64, "the font is 64 glyphs");
    let mut out = Vec::with_capacity(512);
    for glyph in glyphs {
        for row in glyph {
            let mut byte = 0u8;
            for (column, dot) in row.iter().enumerate() {
                if *dot {
                    byte |= 1 << column;
                }
            }
            out.push(byte);
        }
    }
    out
}

/// The ASCII a glyph index stands for: 00..1F are $40..$5F and 20..3F are
/// $20..$3F, which is how the display code's low six bits address the
/// character generator.
fn glyph_ascii(index: usize) -> u8 {
    let index = u8::try_from(index).expect("a small glyph index");
    if index < 32 {
        0x40 + index
    } else {
        0x20 + (index - 32)
    }
}

/// The glyph index of an ASCII character, the other way round.
fn ascii_glyph(c: u8) -> usize {
    assert!((0x20..=0x5F).contains(&c), "{c:#04x} is not in the font");
    if c >= 0x40 {
        (c - 0x40) as usize
    } else {
        (c - 0x20) as usize + 32
    }
}

#[test]
fn font_hex_is_the_drawn_font() {
    let Some(dir) = example() else { return };
    let glyphs = font_art(&read(&dir, "sw/font.txt"));
    let bytes = font_bytes(&glyphs);

    // A space is blank and a full stop is not, which is the cheapest
    // possible check that the art was read the right way up.
    assert!(
        glyphs[ascii_glyph(b' ')]
            .iter()
            .all(|r| !r.iter().any(|d| *d))
    );
    assert!(
        glyphs[ascii_glyph(b'.')]
            .iter()
            .any(|r| r.iter().any(|d| *d))
    );
    // Every glyph fits: the two right-hand columns and the bottom row are
    // the gap between characters, which is what makes the screen
    // readable rather than a wall.
    for (index, glyph) in glyphs.iter().enumerate() {
        let ascii = glyph_ascii(index) as char;
        assert!(
            glyph[CELL_H - 1].iter().all(|d| !*d),
            "glyph {index:02X} (`{ascii}`) draws on its bottom row"
        );
        assert!(
            glyph.iter().all(|r| !r[5] && !r[6]),
            "glyph {index:02X} (`{ascii}`) draws in the gap"
        );
        assert!(
            bytes[index * CELL_H..(index + 1) * CELL_H]
                .iter()
                .all(|b| b & 0x80 == 0),
            "glyph {index:02X} sets bit 7, and a cell is seven dots wide"
        );
    }

    let image: BTreeMap<u32, u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| (u32::try_from(i).expect("a small font"), *b))
        .collect();
    let expected = render_hex(
        "// font.hex: sw/font.txt's art, one byte per row of one glyph,\n\
         // eight rows per glyph and 64 glyphs, with the leftmost dot in bit 0.\n\
         // Generated by tests/apple2.rs; do not edit — edit the art.\n",
        &image,
    );
    expect_file(&dir.join("sw/font.hex"), &expected, "sw/font.txt's art");
    assert_eq!(hex_image(&dir, "sw/font.hex"), image);
}

#[test]
fn the_font_tells_its_glyphs_apart() {
    let Some(dir) = example() else { return };
    let glyphs = font_art(&read(&dir, "sw/font.txt"));
    // A cell on the screen is a glyph or a glyph inverted, and the test
    // that reads the screen back has to be able to say which. If two of
    // those 128 patterns were the same, it could not, and the failure
    // would look like a video bug rather than like a font bug.
    let mut seen: BTreeMap<Vec<bool>, (usize, bool)> = BTreeMap::new();
    for (index, glyph) in glyphs.iter().enumerate() {
        for inverse in [false, true] {
            let pattern: Vec<bool> = glyph
                .iter()
                .flat_map(|row| row.iter().map(|d| *d ^ inverse))
                .collect();
            if let Some((other, other_inv)) = seen.insert(pattern, (index, inverse)) {
                panic!(
                    "glyph {index:02X} (inverse {inverse}) looks exactly like glyph \
                     {other:02X} (inverse {other_inv})"
                );
            }
        }
    }
    assert_eq!(seen.len(), 128);
}

// ---------------------------------------------------------------------------
// The monitor
// ---------------------------------------------------------------------------

/// The little-endian word the assembled image holds at `address`.
fn word_at(image: &asm::Image, address: u16) -> u16 {
    let byte = |a: u16| {
        *image
            .bytes
            .get(&a)
            .unwrap_or_else(|| panic!("{a:#06x} is not in the image"))
    };
    u16::from(byte(address)) | (u16::from(byte(address + 1)) << 8)
}

/// The monitor, assembled.
fn monitor(dir: &Path) -> asm::Image {
    let source = read(dir, "sw/monitor.s");
    asm::assemble(&source).unwrap_or_else(|e| panic!("sw/monitor.s: {e}"))
}

#[test]
fn monitor_hex_is_the_assembled_source() {
    let Some(dir) = example() else { return };
    let image = monitor(&dir);

    // Everything the monitor places is inside the ROM, which is the only
    // place a 6502 can find anything before it has run.
    let lowest = u32::from(*image.bytes.keys().next().expect("a program"));
    let highest = u32::from(*image.bytes.keys().next_back().expect("a program"));
    assert_eq!(lowest, ROM_BASE, "the monitor does not start at the ROM");
    assert_eq!(highest, ROM_BASE + ROM_BYTES - 1, "the last byte is $FFFF");

    // The three vectors are where the part reads them and point at the
    // monitor's own entry points.
    assert_eq!(word_at(&image, VEC_RES), image.label("reset"));
    assert_eq!(word_at(&image, VEC_NMI), image.label("nmi"));
    assert_eq!(word_at(&image, VEC_IRQ), image.label("irq"));
    assert_eq!(image.label("nmi"), image.label("irq"));

    let offsets: BTreeMap<u32, u8> = image
        .bytes
        .iter()
        .map(|(a, b)| (u32::from(*a) - ROM_BASE, *b))
        .collect();
    let expected = render_hex(
        "// monitor.hex: sw/monitor.s assembled, one byte per line, in the ROM's\n\
         // own numbering: offset 0 is address $F800 and an `@` line moves the next\n\
         // byte to that offset, which is how the vectors reach the top of the ROM\n\
         // without two thousand lines of padding in front of them.\n\
         // Generated by tests/apple2.rs; do not edit.\n",
        &offsets,
    );
    expect_file(
        &dir.join("sw/monitor.hex"),
        &expected,
        "sw/monitor.s assembled",
    );
    assert_eq!(hex_image(&dir, "sw/monitor.hex"), offsets);
}

#[test]
fn the_assembler_takes_the_low_and_high_byte_of_an_address() {
    // `<` and `>` are what `tests/mos6502_asm` gained for this example,
    // so they are checked apart from the monitor: a program cannot put
    // the address of a label into a zero-page pointer without them, and
    // `putstr` in sw/monitor.s does it four times.
    let image =
        asm::assemble(".org $1234\nstart: lda #<start\nldx #>start\nnop\n").expect("assembles");
    assert_eq!(image.bytes[&0x1234], 0xA9, "LDA immediate");
    assert_eq!(image.bytes[&0x1235], 0x34, "the low byte of $1234");
    assert_eq!(image.bytes[&0x1236], 0xA2, "LDX immediate");
    assert_eq!(image.bytes[&0x1237], 0x12, "the high byte of $1234");
    // Both forms are two bytes even where the label is not yet known, so
    // the two passes agree about how long the instruction is.
    assert_eq!(image.bytes[&0x1238], 0xEA, "NOP follows");
    // And a forward reference works the same way round.
    let forward = asm::assemble(".org $200\nlda #>later\nlater: nop\n").expect("assembles");
    assert_eq!(forward.bytes[&0x0201], 0x02);
}

#[test]
fn the_monitors_line_table_is_the_documented_interleave() {
    let Some(dir) = example() else { return };
    let image = monitor(&dir);
    let lo = image.label("linelo");
    let hi = image.label("linehi");
    let rows = u16::try_from(ROWS).expect("a small screen");
    assert_eq!(hi, lo + rows, "the two halves are 24 bytes apart");

    for row in 0..ROWS {
        let offset = u16::try_from(row).expect("a small row");
        let from_rom =
            u16::from(image.bytes[&(lo + offset)]) | (u16::from(image.bytes[&(hi + offset)]) << 8);
        assert_eq!(
            from_rom,
            text_base(row),
            "row {row}: the ROM says {from_rom:#06x}, the formula says {:#06x}",
            text_base(row)
        );
    }

    // The shape the formula has, stated as facts rather than as the
    // formula again: the page is 1 KiB, the rows do not overlap, and the
    // sixty-four bytes no row reaches are the screen holes.
    let mut reached = BTreeSet::new();
    for row in 0..ROWS {
        for column in 0..COLS {
            let address = text_base(row) + u16::try_from(column).expect("a small column");
            assert!(
                (0x0400..0x0800).contains(&address),
                "row {row} column {column} is at {address:#06x}, outside the page"
            );
            assert!(
                reached.insert(address),
                "{address:#06x} is on the screen twice"
            );
        }
    }
    assert_eq!(reached.len(), ROWS * COLS);
    assert_eq!(0x0400 - reached.len(), 64, "the screen holes are 64 bytes");
    // And the thing the whole example is about: it is not 40 * N.
    assert_ne!(
        text_base(1),
        text_base(0) + u16::try_from(COLS).expect("a small screen")
    );
    assert_eq!(text_base(1), text_base(0) + 128);
    assert_eq!(text_base(8), text_base(0) + 40);
}

// ---------------------------------------------------------------------------
// Building and synthesis
// ---------------------------------------------------------------------------

#[test]
fn the_project_resolves_and_elaborates() {
    let Some(dir) = example() else { return };
    let built = build(&dir, |_| {});
    assert_eq!(built.project.top.as_deref(), Some("apple2_top"));
    assert_eq!(built.project.device.as_deref(), Some(DEVICE));
    let owners: Vec<(&str, &str)> = built
        .elaboration
        .sources
        .iter()
        .map(|(owner, path, _)| (owner.as_str(), path.as_str()))
        .collect();
    assert_eq!(
        owners,
        [
            ("mos6502", "rtl/mos6502.v"),
            ("uart", "rtl/uart_tx.v"),
            ("uart", "rtl/uart_rx.v"),
            ("uart", "rtl/uart.v"),
            ("dvi_tx", "rtl/tmds_encoder.v"),
            ("dvi_tx", "rtl/video_timing.v"),
            ("dvi_tx", "rtl/dvi_tx.v"),
            ("apple2", "rtl/apple2_video.v"),
            ("apple2", "rtl/apple2.v"),
            ("apple2", "rtl/apple2_top.v"),
        ],
        "the user's HDL is the three files in rtl/; everything else is the library"
    );
    assert!(built.elaboration.blackboxes.is_empty());
    assert!(built.elaboration.skipped.is_empty());
    let design = built.elaboration.design.expect("a design");
    let top = design.top_module().expect("a top");
    assert_eq!(top.name.as_str(), "apple2_top");
}

/// Everything generic synthesis said at warning level or above.
fn complaints(diags: &Diagnostics) -> Vec<String> {
    diags
        .iter()
        .filter(|d| d.severity >= Severity::Warning)
        .map(|d| format!("{}: {}", d.code.unwrap_or("-"), d.message))
        .collect()
}

#[test]
fn the_machine_synthesises_without_errors_or_latches() {
    let Some(dir) = example() else { return };
    let mut design = machine_design(&dir);
    assert_eq!(memory_init(&design, "rom"), None, "the ROM starts empty");
    assert_eq!(memory_init(&design, "font"), None, "the font starts empty");
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &synth_options(&dir), &mut diags);
    assert!(
        complaints(&diags).is_empty(),
        "synthesis complained:\n  {}",
        complaints(&diags).join("\n  ")
    );

    // `$readmemh` put both files where they belong, byte for byte, and
    // every element neither file named is still `x`.
    for (memory, file, size) in [
        ("rom", "sw/monitor.hex", ROM_BYTES),
        ("font", "sw/font.hex", 512),
    ] {
        let init = memory_init(&design, memory).expect("the memory was loaded");
        let image = hex_image(&dir, file);
        assert_eq!(
            u32::try_from(init.len()).expect("a small memory"),
            size,
            "{memory} is not {size} bytes"
        );
        for (offset, byte) in init.iter().enumerate() {
            let offset = u32::try_from(offset).expect("a small memory");
            assert_eq!(
                *byte,
                image.get(&offset).copied(),
                "{memory}[{offset:#05x}]"
            );
        }
    }
    assert!(!diags.iter().any(|d| d.code == Some("S0014")));

    let latches: usize = design
        .modules
        .iter()
        .map(|(_, m)| {
            m.cells
                .iter()
                .filter(|(_, c)| matches!(c.kind, CellKind::Dlatch))
                .count()
        })
        .sum();
    assert_eq!(latches, 0, "synthesis inferred a latch");
}

// ---------------------------------------------------------------------------
// Reading the screen off the video signal
// ---------------------------------------------------------------------------

/// A pixel's colour, or `None` if the signal was `x` there.
type Pixel = Option<u32>;

/// A colour bus's changes, as `(time, value)`, which is what
/// `serial::Waveform` is for a one-bit net.
type Colours = Vec<(u64, Pixel)>;

/// What one character cell turned out to be: a glyph of the font, and
/// whether it was drawn inverted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Cell {
    glyph: usize,
    inverse: bool,
}

/// The video signal, tied to the raster.
///
/// Nothing here reads the design: the raster's arithmetic is the VESA
/// timing at the top of this file, the pixel period is the testbench's
/// clock, and the picture's place on the raster is 560 x 384 centred in
/// 640 x 480. The only thing taken from the simulation is when each
/// signal changed.
struct Video {
    /// Every change of the colour, by pixel number.
    samples: Vec<(i64, Pixel)>,
    /// The last pixel the run reached.
    end: i64,
}

impl Video {
    fn new(de_wave: &[(u64, Option<bool>)], rgb_wave: &Colours, period: u64, end: u64) -> Video {
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

        // The pixel each change of the colour belongs to. A change lands
        // on a clock edge, which is the start of a pixel.
        let mut samples: Vec<(i64, Pixel)> = Vec::with_capacity(rgb_wave.len());
        // Everything before the raster was let go is the machine booting
        // against a frozen `x` and `y`; only the colour it left behind
        // matters, and it becomes the colour pixel 0 starts from.
        let mut before = None;
        for (when, value) in rgb_wave {
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
    fn border_is_black(&self, frame: i64) {
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
    fn rows(&self, frame: i64, rows: std::ops::Range<usize>, glyphs: &[Glyph]) -> Vec<Vec<Cell>> {
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
                            Some(WHITE) => true,
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
}

/// The glyph, and the polarity, a cell was drawn with.
fn match_glyph(pattern: &Glyph, glyphs: &[Glyph], column: usize, row: usize) -> Cell {
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
fn expect_row(text: &str) -> Vec<Cell> {
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
fn test_card(row: usize, column: usize) -> Cell {
    Cell {
        glyph: (7 * row + column) & 0x3F,
        inverse: false,
    }
}

/// The screen as text, one row per line, for a message.
fn as_text(rows: &[Vec<Cell>]) -> String {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|c| glyph_ascii(c.glyph) as char)
                .collect::<String>()
                + "\n"
        })
        .collect()
}

#[test]
fn the_screen_comes_out_of_the_video_signal() {
    let Some(dir) = example() else { return };
    let bench = read(&dir, "tb/apple2_tb.v");
    assert!(bench.contains(&format!("localparam HALF        = {HALF_NS};")));
    assert!(bench.contains(&format!("localparam CLK_DIV     = {CLK_DIV};")));
    // The testbench turns the flashing rate right up so that a flashing
    // cell is in its other state one frame later rather than eight.
    assert!(bench.contains("localparam FLASH_SHIFT = 0;"));
    let glyphs = font_art(&read(&dir, "sw/font.txt"));
    let rom = monitor(&dir);

    let design = testbench_design(&dir);
    let options = SimOptions {
        files: Some(Box::new(example_files(&dir))),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).expect("the testbench simulates");

    let de = sim.net("apple2_tb.de").expect("the testbench has de");
    let rgb = sim.net("apple2_tb.rgb").expect("the testbench has rgb");
    let tx = sim
        .net("apple2_tb.uart_tx")
        .expect("the testbench has uart_tx");

    let de_wave: Rc<RefCell<Waveform>> = Rc::default();
    let sink = Rc::clone(&de_wave);
    sim.on_change(de, move |time, value| {
        sink.borrow_mut()
            .push((time, value.to_u64().map(|v| v == 1)));
    });
    let rgb_wave: Rc<RefCell<Colours>> = Rc::default();
    let sink = Rc::clone(&rgb_wave);
    sim.on_change(rgb, move |time, value| {
        let colour = value
            .to_u64()
            .map(|v| u32::try_from(v).expect("twenty-four bits"));
        sink.borrow_mut().push((time, colour));
    });
    let tx_wave: Rc<RefCell<Waveform>> = Rc::default();
    let sink = Rc::clone(&tx_wave);
    sim.on_change(tx, move |time, value| {
        sink.borrow_mut()
            .push((time, value.to_u64().map(|v| v == 1)));
    });
    let speaker = sim
        .net("apple2_tb.speaker")
        .expect("the testbench has speaker");
    let beeps: Rc<RefCell<Vec<Option<bool>>>> = Rc::default();
    let sink = Rc::clone(&beeps);
    sim.on_change(speaker, move |_, value| {
        sink.borrow_mut().push(value.to_u64().map(|v| v == 1));
    });

    sim.run();
    assert!(sim.finished(), "the testbench did not reach $finish");
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert!(messages.is_empty(), "simulator messages: {messages:?}");

    let period = sim.ticks(Delay::new(2 * HALF_NS, TimeUnit::Ns));
    let video = Video::new(&de_wave.borrow(), &rgb_wave.borrow(), period, sim.time());

    // What the monitor drew, and why. Rows 0 to 2 are its banner; rows 3
    // to 8 are the session the testbench typed; row 9 onwards is the test
    // card the `T` command paints over whatever is left, with the next
    // prompt and its cursor on top of the first two cells of it.
    let mut expected: Vec<Vec<Cell>> = Vec::new();

    // Row 0, the title, in inverse across the whole width.
    let mut title = expect_row("    RETICLE APPLE ][ COMPATIBLE 6502    ");
    for cell in &mut title {
        cell.inverse = true;
    }
    expected.push(title);

    // Row 1 shows the three attributes at once: one word in each.
    let mut attributes = expect_row("ATTRIBUTES: NORMAL INVERSE FLASHING");
    for cell in &mut attributes[19..27] {
        cell.inverse = true;
    }
    let flashing = 27..35;
    expected.push(attributes);

    expected.push(expect_row("TEXT 40X24  $0400 INTERLEAVED"));

    // The first eight bytes of the ROM are the monitor's own first
    // instructions, which is where the test knows them from.
    let dump = |address: u16, bytes: [u8; 8]| {
        let mut text = format!("{address:04X}:");
        for byte in bytes {
            text.push_str(&format!(" {byte:02X}"));
        }
        text
    };
    let rom_head: [u8; 8] = std::array::from_fn(|i| {
        let offset = u16::try_from(i).expect("a short dump");
        rom.bytes[&(u16::try_from(ROM_BASE).expect("a 16-bit address") + offset)]
    });
    let deposited: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    expected.push(expect_row("]E F800"));
    expected.push(expect_row(&dump(0xF800, rom_head)));
    expected.push(expect_row("]D 0300 11 22 33 44 55 66 77 88"));
    expected.push(expect_row("]E 0300"));
    expected.push(expect_row(&dump(0x0300, deposited)));
    expected.push(expect_row("]T"));

    // Row 9 onwards: the test card, with the prompt written over the
    // first cell of row 9 and the cursor over the second.
    for row in CARD_TOP..ROWS {
        expected.push((0..COLS).map(|column| test_card(row, column)).collect());
    }
    expected[CARD_TOP][0] = Cell {
        glyph: ascii_glyph(b']'),
        inverse: false,
    };
    expected[CARD_TOP][1] = Cell {
        glyph: CURSOR_GLYPH,
        inverse: false,
    };
    assert_eq!(expected.len(), ROWS);

    // Frame 0. The flashing attribute is off in it, because the frame
    // counter starts at zero when the raster does and the testbench asks
    // for a state per frame.
    video.border_is_black(0);
    let screen = video.rows(0, 0..ROWS, &glyphs);
    for row in 0..ROWS {
        assert_eq!(
            screen[row],
            expected[row],
            "row {row} of the screen reads\n{}",
            as_text(&screen[row..row + 1])
        );
    }

    // Frame 1, far enough down to show row 1 again. The eight cells the
    // monitor wrote with the flashing attribute have swapped over and
    // nothing else has, which is the whole of what flashing is.
    let later = video.rows(1, 1..2, &glyphs);
    for (column, cell) in later[0].iter().enumerate() {
        let mut want = expected[1][column];
        if flashing.contains(&column) {
            want.inverse = !want.inverse;
        }
        assert_eq!(
            *cell, want,
            "column {column} of row 1 changed the wrong way between frames"
        );
    }
    assert!(
        later[0][flashing.start].inverse,
        "the flashing cells did not flash"
    );

    // The session, decoded off the serial pin as well, by the same
    // decoder `examples/soc` and `examples/mos6502_computer` use.
    let bit = sim.ticks(Delay::new(2 * HALF_NS * CLK_DIV, TimeUnit::Ns));
    let bytes = decode_uart(&tx_wave.borrow(), bit).unwrap_or_else(|e| panic!("{e}"));
    let transcript = String::from_utf8(bytes).expect("ASCII from the wire");
    for line in [
        "RETICLE APPLE ][ COMPATIBLE 6502",
        "ATTRIBUTES: NORMAL INVERSE FLASHING",
        "TEXT 40X24  $0400 INTERLEAVED",
        "]E F800",
        &dump(0xF800, rom_head),
        "]D 0300 11 22 33 44 55 66 77 88",
        "]E 0300",
        &dump(0x0300, deposited),
        "]T",
    ] {
        assert!(
            transcript.contains(line),
            "the serial transcript has no `{line}`:\n{transcript}"
        );
    }
    // The testbench's own receiver agrees, and printed what it read.
    let printed = sim.output();
    assert!(
        printed.contains("]D 0300 11 22 33 44 55 66 77 88"),
        "{printed}"
    );

    // And the machine beeped when it woke up. `beep` in sw/monitor.s
    // touches $C030 eight times and nothing else in the session touches
    // it, so the speaker pin moved eight times — which is also the proof
    // that a soft switch answers a *read*, since `bit $C030` never writes
    // anything.
    let moves = beeps
        .borrow()
        .windows(2)
        .filter(|w| w[0].is_some() && w[1].is_some() && w[0] != w[1])
        .count();
    assert_eq!(moves, BEEP_TOGGLES, "the machine did not beep");
}

// ---------------------------------------------------------------------------
// The ECP5 flow
// ---------------------------------------------------------------------------

#[test]
fn the_machine_maps_onto_the_ecp5_and_exports_for_nextpnr() {
    let Some(dir) = example() else { return };
    let mut design = machine_design(&dir);
    let top = design.top.expect("a top");

    let device = fpga::target(DEVICE).expect("the ECP5 45F is a built-in device");
    let mut map = SourceMap::new();
    let rcf = read(&dir, "board/ulx3s.rcf");
    let file = map
        .add("board/ulx3s.rcf", rcf.clone())
        .expect("the constraints fit");
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(&rcf, file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    constraints.check(&design, device, &mut diags);
    // Every pin the board file names is one the device database records,
    // which is the whole reason it names those nine and no others, and
    // every port of the top has a pin. The messages are listed rather
    // than rendered because `merge_attrs` labels them with spans from the
    // design's source map and this one only holds the board file.
    let said: Vec<String> = diags
        .iter()
        .map(|d| format!("{}: {}", d.code.unwrap_or("-"), d.message))
        .collect();
    assert!(said.is_empty(), "the constraints were not clean: {said:#?}");

    let mut diags = Diagnostics::new();
    let options = FpgaOptions {
        synth: synth_options(&dir),
        ..FpgaOptions::default()
    };
    let flow = fpga::synthesize_for(&mut design, top, device, &constraints, &options, &mut diags)
        .unwrap_or_else(|e| panic!("the ECP5 flow failed: {e:?}"));
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    assert!(errors.is_empty(), "the ECP5 flow reported: {errors:?}");

    // The resource use, for the README and for anyone reading the log.
    println!("{DEVICE}:");
    for (cell, count) in &flow.netlist {
        println!("  {count:>5} x {cell}");
    }
    println!("  {} LUTs, depth {}", flow.luts, flow.lut_depth);
    for bram in &flow.primitives.block_rams {
        println!(
            "  {} -> {} x DP16KD in {}x{} mode, {} cop(ies), initialised {}",
            bram.memory,
            bram.blocks(),
            bram.mode.0,
            bram.mode.1,
            bram.copies,
            bram.initialised
        );
    }

    let luts = flow.count("LUT4");
    assert!(luts > 2000, "{luts} LUTs is too few for this machine");
    assert!(luts <= 43848, "{luts} LUTs does not fit the 45F's 43848");
    assert!(
        flow.netlist.iter().all(|(cell, _)| !cell.starts_with('$')),
        "a generic cell survived: {:?}",
        flow.netlist
    );
    assert!(
        !diags.iter().any(|d| d.code == Some("F0305")),
        "a memory lost its initial contents: {}",
        diags.render(&map)
    );

    // The PLL that makes the pixel clock, and the four lanes that leave
    // through double-data-rate registers on it.
    let pll = flow.primitives.plls.first().expect("no PLL was built");
    assert_eq!(pll.primitive, "EHXPLLL");
    assert_eq!(pll.net, "clk_x5");
    assert_eq!(pll.source, "clk_ref");
    assert_eq!(pll.input_hz, 25_000_000);
    assert_eq!(pll.requested_hz, 126_000_000);
    assert!(
        pll.error_ppm().abs() < 10_000.0,
        "{} Hz is more than 1 % off",
        pll.achieved_hz
    );
    for lane in ["tmds_d0", "tmds_d1", "tmds_d2", "tmds_clk"] {
        let io = flow
            .primitives
            .io_buffers
            .iter()
            .find(|buffer| buffer.port == lane)
            .unwrap_or_else(|| panic!("no buffer for {lane}"));
        assert_eq!(io.bits, 1, "{lane} is one pin");
        let (clock, register) = io.ddr.clone().expect("a DDR lane");
        assert_eq!(clock, "clk_x5", "{lane}");
        assert_eq!(register, "ODDRX1F", "{lane}");
    }

    // The monitor and the character generator went into block RAM with
    // their contents, and the 48 KiB of main memory went into block RAM
    // with two read ports.
    // The flow reports a memory by its path through the hierarchy, so
    // `ram` is `u_machine.ram`.
    let named = |name: &str| {
        flow.primitives
            .block_rams
            .iter()
            .find(|b| b.memory.rsplit('.').next() == Some(name))
            .unwrap_or_else(|| panic!("`{name}` is not in block RAM"))
    };
    assert!(named("rom").initialised, "the ROM's blocks carry nothing");
    assert!(named("font").initialised, "the font's blocks carry nothing");
    let ram = named("ram");
    assert_eq!(
        ram.copies, 2,
        "two read ports — the processor's and the video's — means two copies"
    );

    // Every cell is a primitive the device has, and nothing is wrong.
    let problems: Vec<String> = fpga::check_nextpnr_json(&design, top, device, &constraints)
        .into_iter()
        .map(|p| format!("{}: {}", p.object, p.message))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    let inputs = fpga::export_nextpnr(&design, top, device, &constraints).expect("exports");
    assert_eq!(inputs.constraints_name, "apple2_top.lpf");
    assert_eq!(
        inputs.args.join(" "),
        "nextpnr-ecp5 --45k --package CABGA381 --json apple2_top.json --lpf apple2_top.lpf \
         --textcfg apple2_top.config"
    );
    for pin in ["clk_ref", "tmds_d0", "uart_tx", "speaker"] {
        assert!(
            inputs
                .pcf_or_lpf
                .contains(&format!("LOCATE COMP \"{pin}\"")),
            "the LPF lacks {pin}"
        );
    }
    assert!(inputs.json.contains("\"DP16KD\""));
    assert!(inputs.json.contains("\"EHXPLLL\""));
    assert!(inputs.json.contains("\"ODDRX1F\""));

    // The files a board needs, where a reader can pick them up.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("apple2");
    fs::create_dir_all(&out).expect("a scratch directory");
    fs::write(out.join("apple2_top.json"), &inputs.json).expect("write the netlist");
    fs::write(out.join(&inputs.constraints_name), &inputs.pcf_or_lpf).expect("write the LPF");
    println!("wrote {}/apple2_top.json and apple2_top.lpf", out.display());
    println!("then: {}", inputs.args.join(" "));
}

#[test]
fn the_machine_does_not_fit_the_ice40_hx8k() {
    let Some(dir) = example() else { return };
    // The HX8K is what `examples/soc` and `examples/mos6502_computer`
    // target, so the honest thing is to run the flow for it and say what
    // comes out rather than to leave the question open. No pins are
    // assigned — the HX8K breakout has no DVI connector and there is no
    // pinout worth inventing — but the board's 12 MHz oscillator is
    // named, so the PLL that makes the pixel clock is in the count.
    let mut design = machine_design(&dir);
    let top = design.top.expect("a top");
    let device = fpga::target(SMALL_DEVICE).expect("the HX8K is a built-in device");
    let mut map = SourceMap::new();
    let rcf = "create_clock -name osc -period 83.333333 clk_ref\n";
    let file = map.add("hx8k.rcf", rcf).expect("the constraints fit");
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(rcf, file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    let options = FpgaOptions {
        synth: synth_options(&dir),
        ..FpgaOptions::default()
    };
    let flow = fpga::synthesize_for(&mut design, top, device, &constraints, &options, &mut diags);

    println!("{SMALL_DEVICE}:");
    match &flow {
        Ok(report) => {
            for (cell, count) in &report.netlist {
                println!("  {count:>5} x {cell}");
            }
            println!("  {} LUTs, depth {}", report.luts, report.lut_depth);
        }
        Err(e) => println!("  the flow failed: {e:?}"),
    }
    for message in diags.iter().map(|d| &d.message) {
        println!("  {message}");
    }

    // Whatever else happens, it does not fit, and the reason is the
    // memory: the part has 32 block RAMs of 4 kbit, 16 KiB in total, and
    // this machine wants 48 KiB of it *twice* — once per read port —
    // plus the monitor and the character generator.
    let blocks = flow.as_ref().map(|r| r.count("SB_RAM40_4K")).unwrap_or(0);
    let over_budget = blocks > 32 || diags.has_errors();
    assert!(
        over_budget,
        "{blocks} block RAMs fits the HX8K's 32 after all, which would be news"
    );
}

// ---------------------------------------------------------------------------
// The binary
// ---------------------------------------------------------------------------

/// Runs the binary in `dir` and returns its exit code, stdout and stderr.
#[cfg(feature = "cli")]
fn reticle(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_reticle"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the reticle binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The library sources the project pulls in, as `reticle sim` and
/// `reticle fpga` want them on a command line.
#[cfg(feature = "cli")]
const LIBRARY: [&str; 7] = [
    "../../ip/mos6502/rtl/mos6502.v",
    "../../ip/uart/rtl/uart_tx.v",
    "../../ip/uart/rtl/uart_rx.v",
    "../../ip/uart/rtl/uart.v",
    "../../ip/dvi_tx/rtl/tmds_encoder.v",
    "../../ip/dvi_tx/rtl/video_timing.v",
    "../../ip/dvi_tx/rtl/dvi_tx.v",
];

#[cfg(feature = "cli")]
#[test]
fn reticle_build_builds_the_project() {
    let Some(dir) = example() else { return };
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("apple2-build");
    fs::create_dir_all(&scratch).expect("a scratch directory");
    let lock = scratch.join("reticle.lock");
    let lock_arg = lock.to_string_lossy().into_owned();
    let (code, _, err) = reticle(
        &dir,
        &["build", "--synth", "--lock", &lock_arg, "reticle.proj"],
    );
    assert_eq!(code, 0, "reticle build failed:\n{err}");
    assert!(err.contains("note: built `apple2`:"), "{err}");
    // The binary hands synthesis a file provider rooted at the manifest's
    // directory, so both `$readmemh`s load and there is nothing to say
    // about them.
    assert!(
        !err.contains("could not be loaded"),
        "a memory image was not loaded:\n{err}"
    );
    assert!(!err.contains("simulation-only statement dropped"), "{err}");
    let lock = fs::read_to_string(&lock).expect("a lock file");
    for package in ["mos6502", "uart", "dvi_tx"] {
        assert!(lock.contains(package), "{lock}");
    }
}

#[cfg(feature = "cli")]
#[test]
fn reticle_sim_runs_the_testbench() {
    let Some(dir) = example() else { return };
    // `--until` stops the run eight and a half milliseconds in, by which
    // time the machine has cleared its screen and printed its banner, and
    // long before the two frames of video the testbench records for
    // `the_screen_comes_out_of_the_video_signal`. Running the whole thing
    // again here would double the slowest test in the file to prove
    // something it has already proved; what is left is the claim this
    // test is for, which is that the binary runs the example's
    // testbench.
    let mut args = vec![
        "sim",
        "--quiet",
        "--until",
        "8500000",
        "tb/apple2_tb.v",
        "rtl/apple2.v",
        "rtl/apple2_video.v",
    ];
    args.extend(LIBRARY);
    let (code, out, err) = reticle(&dir, &args);
    assert_eq!(code, 0, "{err}");
    for line in [
        "RETICLE APPLE ][ COMPATIBLE 6502",
        "ATTRIBUTES: NORMAL INVERSE FLASHING",
        "TEXT 40X24  $0400 INTERLEAVED",
    ] {
        assert!(
            out.contains(line),
            "`{line}` did not come out of `reticle sim`:\n{out}\n{err}"
        );
    }
}

#[cfg(feature = "cli")]
#[test]
fn reticle_fpga_exports_the_rom_with_the_monitor_in_it() {
    let Some(dir) = example() else { return };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("apple2-fpga");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("a scratch directory");
    let out_arg = out.to_string_lossy().into_owned();
    let mut args = vec![
        "fpga",
        "--device",
        DEVICE,
        "--constraints",
        "board/ulx3s.rcf",
        "--output-dir",
        &out_arg,
        "--quiet",
        "rtl/apple2_top.v",
        "rtl/apple2.v",
        "rtl/apple2_video.v",
    ];
    args.extend(LIBRARY);
    let (code, _, err) = reticle(&dir, &args);
    assert_eq!(code, 0, "reticle fpga failed:\n{err}");

    let json = fs::read_to_string(out.join("apple2_top.json")).expect("the netlist");
    // The block RAMs carry INITVAL parameters holding the monitor and the
    // font; a blank machine would have every one of them all zeroes.
    let inits: Vec<&str> = json
        .split("\"INITVAL_")
        .skip(1)
        .filter_map(|rest| rest.split('"').nth(2))
        .collect();
    assert!(!inits.is_empty(), "no block RAM carries INITVAL parameters");
    assert!(
        inits.iter().any(|v| v.chars().any(|c| c != '0')),
        "every INITVAL parameter is zero: the memories were exported blank"
    );
}
