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
//! The machine has two video paths and the same screen comes out of
//! both, so there are two such tests.
//! `the_screen_comes_out_of_the_vga_pins` does the same job one step
//! further down the wire: it reads the picture off the twelve colour
//! pins and the two sync pins `vga_out` drives for a Basys 3, rather
//! than off the colour bus `dvi_tx` is handed. The decoder they share
//! is `tests/video/mod.rs`, the way `tests/serial/mod.rs` is shared by
//! the tests that end at a serial wire.
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
//! | `the_project_resolves_and_elaborates` | the manifest builds from exactly four library packages and four user sources, with `video_timing` pulled in once for both video paths |
//! | `the_machine_synthesises_without_errors_or_latches` | synthesis has no error, no warning and no latch, and both memories hold what their files say |
//! | `the_screen_comes_out_of_the_video_signal` | the whole chain: forty by twenty-four characters decoded from two frames of video, the session decoded again off `uart_tx` (which is also what proves the keyboard, since the testbench typed it), and the speaker counted |
//! | `the_screen_comes_out_of_the_vga_pins` | the same screen decoded off the *pins* `vga_out` drives — twelve colour bits and two syncs — with the syncs checked on every line of the frame |
//! | `the_assembler_takes_the_low_and_high_byte_of_an_address` | the `<` and `>` operators `tests/mos6502_asm` gained for this example |
//! | `the_machine_maps_onto_the_ecp5_and_exports_for_nextpnr` | the ECP5 flow fits it, puts the monitor and the font in block RAM, builds the PLL and the DDR outputs, and writes the JSON and LPF `nextpnr-ecp5` reads |
//! | `the_machine_maps_onto_the_artix7_for_the_basys3` | the 7-series flow fits `apple2_basys3` on the Basys 3's XC7A35T with no PLL and no DDR register anywhere, and writes the netlist, the XDC and the Vivado script |
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

/// The 40 x 24 text screen read back off a video signal's waveform, and
/// the font as drawn. Both video tests below use it: the DVI one samples
/// the colour bus the machine hands to `dvi_tx` and the VGA one samples
/// the pins `vga_out` drives, and `video::Signal` is the whole of the
/// difference between them.
#[path = "video/mod.rs"]
mod video;

use serial::{Waveform, decode_uart};
use video::{
    CELL_H, COLS, Cell, Colours, Glyph, ROWS, Signal, Video, as_text, ascii_glyph, expect_row,
    font_art, glyph_ascii, test_card,
};

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

// The raster, the picture's place on it and the font are in
// `tests/video/mod.rs`, which both of the video tests below share.

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

/// One testbench's design: the project with that `testbench` line added
/// as a source and the testbench as the top.
///
/// The example has two, one per video path, and each is named here so
/// that a project which stopped listing one would fail rather than
/// quietly build the other.
fn testbench_design(dir: &Path, bench: &str, top: &str) -> Design {
    let built = build(dir, |project| {
        assert!(
            project.testbenches.iter().any(|t| t == bench),
            "reticle.proj does not list {bench}"
        );
        project.sources.push(SourceEntry {
            path: bench.to_owned(),
            language: None,
            encrypted: false,
            span: project.span,
        });
        project.top = Some(top.to_owned());
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
            ("vga_out", "rtl/vga_out.v"),
            ("apple2", "rtl/apple2_video.v"),
            ("apple2", "rtl/apple2.v"),
            ("apple2", "rtl/apple2_top.v"),
            ("apple2", "rtl/apple2_basys3.v"),
        ],
        "the user's HDL is the four files in rtl/; everything else is the library"
    );
    // `vga_out` reaches the build through its own `depends dvi_tx`, and
    // `video_timing` appears once for both video paths and not twice.
    assert_eq!(
        owners
            .iter()
            .filter(|(_, path)| *path == "rtl/video_timing.v")
            .count(),
        1,
        "the raster was pulled in twice"
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
//
// The decoder is `tests/video/mod.rs`: the raster's own arithmetic, the
// font as `sw/font.txt` draws it, and a picture cut into character
// cells. Nothing in it reads the design, and both of the tests below
// hand it the same screen off two different signals.

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

    let design = testbench_design(&dir, "tb/apple2_tb.v", "apple2_tb");
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
    let video = Video::new(
        &de_wave.borrow(),
        &rgb_wave.borrow(),
        period,
        sim.time(),
        Signal::DVI,
    );

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

/// The first row the `T` command paints over in `tb/apple2_vga_tb.v`:
/// the banner is three rows and the one command it types is one more.
const VGA_CARD_TOP: usize = 4;

#[test]
fn the_screen_comes_out_of_the_vga_pins() {
    let Some(dir) = example() else { return };
    let bench = read(&dir, "tb/apple2_vga_tb.v");
    assert!(bench.contains(&format!("localparam HALF     = {HALF_NS};")));
    assert!(bench.contains(&format!("localparam CLK_DIV  = {CLK_DIV};")));
    // Four bits a channel, which is the Basys 3's resistor ladder and
    // what `Signal::VGA4` decodes.
    assert!(
        bench.contains(".BPC       (4)"),
        "the testbench is not 4 bpc"
    );
    let glyphs = font_art(&read(&dir, "sw/font.txt"));

    let design = testbench_design(&dir, "tb/apple2_vga_tb.v", "apple2_vga_tb");
    let options = SimOptions {
        files: Some(Box::new(example_files(&dir))),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).expect("the testbench simulates");

    // Everything sampled here is a **pin**: the twelve colour bits and
    // the two syncs that leave `vga_out`, and `de`, which is the raster
    // the picture is measured against. Nothing inside `apple2` is
    // looked at.
    let de = sim.net("apple2_vga_tb.de").expect("the testbench has de");
    let rgb = sim
        .net("apple2_vga_tb.vga_rgb")
        .expect("the testbench has vga_rgb");
    let hsync = sim
        .net("apple2_vga_tb.vga_hsync")
        .expect("the testbench has vga_hsync");
    let vsync = sim
        .net("apple2_vga_tb.vga_vsync")
        .expect("the testbench has vga_vsync");

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
            .map(|v| u32::try_from(v).expect("twelve bits"));
        sink.borrow_mut().push((time, colour));
    });
    let hsync_wave: Rc<RefCell<Waveform>> = Rc::default();
    let sink = Rc::clone(&hsync_wave);
    sim.on_change(hsync, move |time, value| {
        sink.borrow_mut()
            .push((time, value.to_u64().map(|v| v == 1)));
    });
    let vsync_wave: Rc<RefCell<Waveform>> = Rc::default();
    let sink = Rc::clone(&vsync_wave);
    sim.on_change(vsync, move |time, value| {
        sink.borrow_mut()
            .push((time, value.to_u64().map(|v| v == 1)));
    });

    sim.run();
    assert!(sim.finished(), "the testbench did not reach $finish");
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert!(messages.is_empty(), "simulator messages: {messages:?}");

    let period = sim.ticks(Delay::new(2 * HALF_NS, TimeUnit::Ns));
    let video = Video::new(
        &de_wave.borrow(),
        &rgb_wave.borrow(),
        period,
        sim.time(),
        Signal::VGA4,
    );

    // The two sync pins, on every one of the 525 lines. 640 x 480 at 60
    // Hz has negative syncs, so the level a pulse has is low — which is
    // the mode's own polarity arriving at a socket, and is the one thing
    // about a VGA signal that has no counterpart on the DVI path, where
    // the syncs travel inside TMDS control symbols.
    video.syncs_are_the_rasters(0, &hsync_wave.borrow(), &vsync_wave.borrow(), false);

    // What the monitor drew. Rows 0 to 2 are its banner and row 3 is the
    // one command this testbench types; row 4 onwards is the test card
    // the `T` command paints over whatever is left, with the next prompt
    // and its cursor on top of the first two cells of it. Twenty-one of
    // the twenty-four rows are therefore different from every other row,
    // which is what makes this a test of the interleaved line order and
    // not only of the character generator.
    let mut expected: Vec<Vec<Cell>> = Vec::new();

    let mut title = expect_row("    RETICLE APPLE ][ COMPATIBLE 6502    ");
    for cell in &mut title {
        cell.inverse = true;
    }
    expected.push(title);

    let mut attributes = expect_row("ATTRIBUTES: NORMAL INVERSE FLASHING");
    for cell in &mut attributes[19..27] {
        cell.inverse = true;
    }
    // The flashing word is in its dark phase in frame 0, because the
    // machine's frame counter starts at zero and the raster is let go at
    // the top of that frame. `the_screen_comes_out_of_the_video_signal`
    // is the test that watches it change.
    expected.push(attributes);

    expected.push(expect_row("TEXT 40X24  $0400 INTERLEAVED"));
    expected.push(expect_row("]T"));

    for row in VGA_CARD_TOP..ROWS {
        expected.push((0..COLS).map(|column| test_card(row, column)).collect());
    }
    expected[VGA_CARD_TOP][0] = Cell {
        glyph: ascii_glyph(b']'),
        inverse: false,
    };
    expected[VGA_CARD_TOP][1] = Cell {
        glyph: CURSOR_GLYPH,
        inverse: false,
    };
    assert_eq!(expected.len(), ROWS);

    // The picture is centred in the raster and everything around it is
    // black, which on this path is also the proof that the blanking gate
    // did not leak: a pixel outside the picture and a pixel outside the
    // active area are read the same way, off the same pins.
    video.border_is_black(0);
    let screen = video.rows(0, 0..ROWS, &glyphs);
    for row in 0..ROWS {
        assert_eq!(
            screen[row],
            expected[row],
            "row {row} of the VGA screen reads\n{}",
            as_text(&screen[row..row + 1])
        );
    }
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

// ---------------------------------------------------------------------------
// The Artix-7 flow, for a Basys 3
// ---------------------------------------------------------------------------

/// The second target: the part on a Digilent Basys 3.
const BASYS3_DEVICE: &str = "xc7a35t-cpg236";

/// `apple2_basys3` through the 7-series flow, with the files a Basys 3
/// owner would hand to Vivado written out at the end.
///
/// This is the payoff of `ip/vga_out`. The same design with `dvi_tx` on
/// the end stops here with
///
/// ```text
/// port tmds_d0 is not double data rate: `xc7a35t-cpg236` declares
/// neither an IO buffer that registers both edges nor a `ddr_out`
/// register
/// ```
///
/// because the part's device file declares no DDR register — and the
/// board has no connector for one either. Everything else about the
/// machine already fitted; VGA was the only missing piece.
///
/// **Vivado has not been run.** What is checked is what the files say:
/// that every cell is a primitive `src/fpga/devices/xc7.dev` declares,
/// wired to pins it has; that every pin the board file names is one the
/// device database records and every port of the top has a pin; that
/// the memories carried their contents into the netlist; and that the
/// script names the part string Vivado wants. Whether Vivado accepts
/// the result, and whether a monitor locks to what leaves the socket,
/// are separate claims and this test makes neither.
#[test]
fn the_machine_maps_onto_the_artix7_for_the_basys3() {
    let Some(dir) = example() else { return };
    let mut design = build(&dir, |project| {
        project.top = Some("apple2_basys3".to_owned());
    })
    .elaboration
    .design
    .expect("the project elaborates");
    let top = design.top.expect("a top");

    let device = fpga::target(BASYS3_DEVICE).expect("the XC7A35T is a built-in device");
    let mut map = SourceMap::new();
    let rcf = read(&dir, "board/basys3.rcf");
    let file = map
        .add("board/basys3.rcf", rcf.clone())
        .expect("the constraints fit");
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(&rcf, file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    constraints.check(&design, device, &mut diags);
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
        .unwrap_or_else(|e| panic!("the Artix-7 flow failed: {e:?}"));
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    assert!(errors.is_empty(), "the Artix-7 flow reported: {errors:?}");

    // The footprint, for the README and for anyone reading the log.
    println!("{BASYS3_DEVICE}:");
    for (cell, count) in &flow.netlist {
        println!("  {count:>5} x {cell}");
    }
    println!("  {} LUTs, depth {}", flow.luts, flow.lut_depth);
    for bram in &flow.primitives.block_rams {
        println!(
            "  {} -> {} x RAMB18E1 in {}x{} mode, {} cop(ies), initialised {}",
            bram.memory,
            bram.blocks(),
            bram.mode.0,
            bram.mode.1,
            bram.copies,
            bram.initialised
        );
    }

    // It fits, with room. The part has 20800 LUT6, 41600 flip-flops and
    // 100 RAMB18E1, which `the_artix7_matches_its_datasheet_figures` in
    // tests/fpga_flow.rs holds to the datasheet.
    let luts = flow.count("LUT6");
    let brams = flow.count("RAMB18E1");
    // The adders are on the part's own carry chain, four bits to a
    // CARRY4, which is what `tests/fpga_carry.rs` checks the wiring of.
    // The last measured footprint was 1877 LUT6 and 85 CARRY4 over 33
    // adders; before the carry chain was mapped it was 1980 LUT6. The
    // bounds are loose enough to survive an unrelated change and tight
    // enough that the chain silently going away fails here.
    let carries = flow.count("CARRY4");
    assert!(
        (60..=120).contains(&carries),
        "{carries} CARRY4 is not the carry chain this machine has"
    );
    assert!(luts < 1980, "{luts} LUT6 is no better than no carry chain");
    let flops: usize = ["FDRE", "FDSE", "FDCE", "FDPE"]
        .iter()
        .map(|kind| flow.count(kind))
        .sum();
    assert!(luts > 1000, "{luts} LUT6 is too few for this machine");
    assert!(luts <= 20_800, "{luts} LUT6 does not fit the XC7A35T");
    assert!(brams <= 100, "{brams} RAMB18E1 does not fit the XC7A35T");
    assert!(flops <= 41_600, "{flops} flip-flops do not fit");
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

    // No PLL and no double-data-rate register anywhere, which is the
    // whole reason this target exists: the pixel rate is a divider, not
    // a clock, and a colour bit is an ordinary output.
    assert!(
        flow.primitives.plls.is_empty(),
        "the VGA path asked for a PLL"
    );
    for buffer in &flow.primitives.io_buffers {
        assert!(
            buffer.ddr.is_none(),
            "{} came out double data rate",
            buffer.port
        );
    }
    // And `dvi_tx` came along as a dependency of `vga_out` but left
    // nothing behind: the encoders and the serialisers are not
    // instantiated, so no netlist cell belongs to one.
    assert_eq!(flow.count("ODDR"), 0);

    // The monitor and the character generator went into block RAM with
    // their contents, and the 48 KiB of main memory went in twice, once
    // per read port.
    let named = |name: &str| {
        flow.primitives
            .block_rams
            .iter()
            .find(|b| b.memory.rsplit('.').next() == Some(name))
            .unwrap_or_else(|| panic!("`{name}` is not in block RAM"))
    };
    assert!(named("rom").initialised, "the ROM's blocks carry nothing");
    assert!(named("font").initialised, "the font's blocks carry nothing");
    assert_eq!(
        named("ram").copies,
        2,
        "two read ports — the processor's and the video's — means two copies"
    );

    // Every cell is a primitive the part has, wired to pins it has.
    let problems: Vec<String> = fpga::check_nextpnr_json(&design, top, device, &constraints)
        .into_iter()
        .map(|p| format!("{}: {}", p.object, p.message))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");

    // The three files Vivado reads, and the command line that runs them.
    let inputs = fpga::export_vendor(&design, top, device, &constraints).expect("the export");
    assert!(
        inputs.script.contains("-part {xc7a35tcpg236-1}"),
        "{}",
        inputs.script
    );
    assert_eq!(
        inputs.args,
        vec!["vivado", "-mode", "batch", "-source", "apple2_basys3.tcl"]
    );
    // Every name is braced. Tcl substitutes `$NAME` inside a bare word,
    // and a module specialised by `--param` carries a `$`, so an
    // unbraced name would send Vivado looking for a variable.
    for step in [
        "read_verilog {apple2_basys3.v}",
        "read_xdc {apple2_basys3.xdc}",
        "synth_design -top {apple2_basys3}",
        "place_design",
        "route_design",
        "write_bitstream -force {apple2_basys3.bit}",
    ] {
        assert!(inputs.script.contains(step), "the script lacks `{step}`");
    }
    // Every pin of the board file reaches the XDC, VGA and all.
    for pin in &constraints.pins {
        if pin.pin.is_empty() {
            continue;
        }
        assert!(
            device.pin(&pin.pin).is_some(),
            "`{}` is not a pin of the part",
            pin.pin
        );
        let line = format!(
            "set_property PACKAGE_PIN {} [get_ports {{{}}}]",
            pin.pin,
            pin.signal()
        );
        assert!(inputs.xdc.contains(&line), "the XDC lacks `{line}`");
    }
    for pin in ["vga_r[0]", "vga_hsync", "vga_vsync", "clk", "uart_tx"] {
        assert!(
            inputs.xdc.contains(&format!("[get_ports {{{pin}}}]")),
            "the XDC lacks {pin}"
        );
    }

    // The files a board needs, where a reader can pick them up.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("apple2-basys3");
    fs::create_dir_all(&out).expect("a scratch directory");
    fs::write(out.join("apple2_basys3.v"), &inputs.verilog).expect("write the netlist");
    fs::write(out.join("apple2_basys3.xdc"), &inputs.xdc).expect("write the XDC");
    fs::write(out.join("apple2_basys3.tcl"), &inputs.script).expect("write the script");
    println!("wrote {}/apple2_basys3.{{v,xdc,tcl}}", out.display());
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
