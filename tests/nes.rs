//! `examples/nes`, an NES-compatible console, driven through the same
//! chain as `examples/mos6502_computer`.
//!
//! The console is **not** a game machine here: it is
//! [`ip/mos6502`](../ip/mos6502) with `DECIMAL_MODE = 0` — which is what
//! the processor in an NES actually is — plus
//! [`ip/ppu2c02`](../ip/ppu2c02), 2 KiB of work RAM, an NROM cartridge
//! and a sprite DMA engine, running a demo written for this example. No
//! part of any commercial cartridge is in this repository and none is
//! needed.
//!
//! The test that matters is
//! `the_frame_comes_out_of_the_video_port`: it runs the console for two
//! whole frames, records the video port pin by pin, and compares every
//! one of the 61,440 pixels of the second against a frame buffer this
//! file computes from the nametable, the pattern table and the palette
//! by the documented rules — the rules written out in `expected_frame`
//! below, which knows nothing about how the hardware fetches anything.
//! It is the analogue of `the_line_comes_out_of_the_serial_wire` in
//! `tests/mos6502_computer.rs`, and `docs/writing-a-cpu.md` section 3
//! argues at length for why it has to be built that way.
//!
//! `the_screen_doubles_the_console_onto_the_tmds_lanes` is the same idea
//! for the other half: it paints the frame buffer, runs `nes_top`, and
//! reads the four output pins two bits at a time — finding the symbol
//! boundary from the clock channel, decoding the ten-bit symbols back
//! into bytes with the inverse of what DVI 1.0 specifies, and checking
//! that every one of them is the colour the palette gives the console
//! pixel two screen columns wide underneath it.
//!
//! The console has a second video path and a second board, and
//! `the_frame_comes_out_of_the_vga_pins` is the two halves joined: the
//! demo's own frame, through the frame buffer and the doubling and
//! `vga_out`, read back off the twelve colour pins and the two sync
//! pins of a Digilent Basys 3. It can be one simulation where the DVI
//! path could not be, because VGA needs no clock at five times the
//! pixel rate. What it compares against is the *truncated* palette:
//! the board renders four bits a channel, so the test throws away the
//! low four bits of the reference colours before comparing, and
//! `four_bits_a_channel_merges_one_pair_of_the_palette` says what that
//! costs — 54 distinct colours become 53, and the pair that merges is
//! $09 and $0B.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `demo_hex_is_the_assembled_source` | the checked-in program image is `sw/demo.s` assembled by the opcode matrix in `tests/mos6502_asm`, with the three vectors where the part reads them |
//! | `chr_hex_is_the_assembled_source` | and the pattern image is `sw/chr.s` assembled |
//! | `the_tiles_are_the_art_drawn_above_them` | every tile's sixteen bytes are the eight rows of art in the comment over it, worked out again here from the two-bitplane format |
//! | `the_project_resolves_and_elaborates` | the manifest builds through `ip::resolve` and `ip::elaborate` from two library packages and the example's own HDL |
//! | `the_console_synthesises_without_errors_or_latches` | generic synthesis reports no error, no warning and no latch, and both ROMs hold what the hex files say |
//! | `the_frame_comes_out_of_the_video_port` | two frames of the demo, every pixel of the second compared against an independently computed framebuffer |
//! | `the_screen_doubles_the_console_onto_the_tmds_lanes` | and the other half of the chain: a painted frame buffer read back off the four output pins, TMDS symbols decoded, doubled and centred with a black border |
//! | `the_frame_comes_out_of_the_vga_pins` | both halves at once on the other board: every one of the 307,200 pixels of a 640 x 480 frame decoded off `vga_out`'s twelve colour pins, against the same model doubled and truncated to four bits a channel, with the syncs read off their own pins on every line |
//! | `the_palette_is_the_one_the_tmds_test_reads_by_hand` | the two transcriptions of `ppu_palette`'s table in this file agree |
//! | `four_bits_a_channel_merges_one_pair_of_the_palette` | what the Basys 3's four bits a channel cost the palette, as a number and a pair |
//! | `sprite_zero_hits_on_the_dot_the_pixels_meet` | the flag goes up on exactly the dot where sprite zero's opaque pixel meets an opaque background pixel, and not a dot earlier |
//! | `nine_sprites_on_a_line_set_the_overflow_flag` | eight sprites on one line and nine on the next, and the line the flag goes up on |
//! | `the_write_latch_is_shared_by_2005_and_2006` | the single `w` toggle, the interleaving it allows, and a read of $2002 putting it back |
//! | `the_data_port_reads_one_access_behind_and_steps_by_what_2000_says` | a $2007 read gives the byte fetched for the previous one, the palette is not buffered and $3F10 is $3F00, and $2000's increment and nametable bits |
//! | `oam_dma_copies_a_page_and_stops_the_processor` | $4014 costs the documented 513 processor cycles and the processor retires nothing in them |
//! | `the_console_maps_onto_the_ecp5_and_exports_for_nextpnr` | the ECP5 flow fits it, every cell a device primitive, every memory in block RAM, and the JSON and LPF `nextpnr-ecp5` reads — and the block RAM arithmetic that says why the HX8K of the other two examples is out |
//! | `the_console_maps_onto_the_artix7_for_the_basys3` | the 7-series flow fits `nes_basys3` on the Basys 3's XC7A35T with no PLL and no DDR register anywhere, and writes the netlist, the XDC and the Vivado script |
//! | `reticle_build_builds_the_project` | the same through the binary |
//! | `reticle_sim_runs_the_testbench` | and the picture comes out of `reticle sim`, hashed by a testbench that watches the same pins |
//! | `reticle_fpga_exports_the_cartridge_with_the_program_in_it` | and out of `reticle fpga` |
//!
//! Two defects in Reticle turned up building this, and both have a
//! regression test at the bottom of this file:
//! `dce_keeps_no_reference_to_a_memory_it_removed` and
//! `lowering_many_small_memories_is_not_quadratic`.
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
use std::collections::BTreeMap;
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
/// with `tests/ip_library.rs` and `tests/mos6502_computer.rs`.
#[path = "mos6502_asm/mod.rs"]
mod asm;

/// The raster decoder that turns a video signal's waveform into pixels,
/// shared with `tests/apple2.rs`. Only the raster half of it is used
/// here — `examples/nes` draws a frame buffer and not a page of text —
/// but that half is the whole of what ties a simulation's times to a
/// 640 x 480 raster's pixels, and writing it twice would be writing the
/// VESA numbers twice.
#[path = "video/mod.rs"]
mod video;

// ---------------------------------------------------------------------------
// The machine's constants, as the documentation of the part states them
// ---------------------------------------------------------------------------

/// The part the project targets.
const DEVICE: &str = "ecp5-45f-CABGA381";

/// The part on a Digilent Basys 3, which `nes_basys3` and
/// `board/basys3.rcf` are the second target for.
const BASYS3_DEVICE: &str = "xc7a35t-cpg236";

/// The small iCE40 the other two examples fit on.
const SMALL_DEVICE: &str = "ice40-hx8k-ct256";

/// NROM-128: 16 KiB of program ROM, seen at $8000 and again at $C000.
/// `sw/demo.s` is assembled into the upper copy, so offset 0 of the
/// image is $C000.
const PRG_BASE: u32 = 0xC000;
const PRG_BYTES: u32 = 16384;

/// 8 KiB of pattern memory: two tables of 256 tiles of sixteen bytes.
const CHR_BYTES: u32 = 8192;

/// The three vectors, where a 6502 has always had them.
const VEC_NMI: u16 = 0xFFFA;
const VEC_RES: u16 = 0xFFFC;
const VEC_IRQ: u16 = 0xFFFE;

/// The raster: 341 dots by 262 scanlines, 256 by 240 of them visible.
const DOTS_PER_LINE: u64 = 341;
const LINES_PER_FRAME: u64 = 262;
const DOTS_PER_FRAME: u64 = DOTS_PER_LINE * LINES_PER_FRAME;
const SCREEN_W: usize = 256;
const SCREEN_H: usize = 240;

// ---------------------------------------------------------------------------
// The example on disk
// ---------------------------------------------------------------------------

/// `examples/nes`, when this copy of the crate has it.
fn example() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/nes");
    if dir.join("reticle.proj").is_file() {
        Some(dir)
    } else {
        println!("skipping: examples/nes is not in this copy of the crate");
        None
    }
}

/// A file of the example, as text.
fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("examples/nes/{rel}: {e}"))
}

/// The ROM image as `$readmemh` reads it: a header, then runs of bytes,
/// each run introduced by the `@` offset it starts at.
fn render_hex(what: &str, image: &BTreeMap<u32, u8>) -> String {
    let mut out = format!(
        "// {what}, one byte per line, in the memory's own numbering: an\n\
         // `@` line moves the next byte to that offset, which is how a\n\
         // sparse image reaches the top of its memory without thousands of\n\
         // lines of padding in front of it.\n\
         // Generated by tests/nes.rs; do not edit.\n"
    );
    let mut next = None;
    for (offset, byte) in image {
        if next != Some(*offset) {
            out.push_str(&format!("@{offset:04x}\n"));
        }
        out.push_str(&format!("{byte:02x}\n"));
        next = Some(offset + 1);
    }
    out
}

/// The bytes of a hex image, by offset, read the way `$readmemh` would.
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

/// Assembles one of the example's sources, failing with its message.
fn assemble(dir: &Path, rel: &str) -> asm::Image {
    let source = read(dir, rel);
    asm::assemble(&source).unwrap_or_else(|e| panic!("examples/nes/{rel}: {e}"))
}

/// Checks a hex file against the image the source assembles to, and
/// rewrites it under `UPDATE_EXPECT`.
fn check_hex(dir: &Path, hex: &str, what: &str, base: u32, image: &asm::Image) {
    let offsets: BTreeMap<u32, u8> = image
        .bytes
        .iter()
        .map(|(a, b)| (u32::from(*a) - base, *b))
        .collect();
    let expected = render_hex(what, &offsets);
    let path = dir.join(hex);
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        fs::write(&path, &expected).unwrap_or_else(|e| panic!("write {hex}: {e}"));
    }
    let actual = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "{hex} is not its source assembled; rerun with UPDATE_EXPECT=1 and commit both"
    );
    assert_eq!(hex_image(dir, hex), offsets, "{hex} does not read back");
}

// ---------------------------------------------------------------------------
// The program and the tiles
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

#[test]
fn demo_hex_is_the_assembled_source() {
    let Some(dir) = example() else { return };
    let image = assemble(&dir, "sw/demo.s");

    // Everything the program places is inside the cartridge's upper
    // copy, which is where the vectors have to be.
    let lowest = u32::from(*image.bytes.keys().next().expect("a program"));
    let highest = u32::from(*image.bytes.keys().next_back().expect("a program"));
    assert_eq!(lowest, PRG_BASE, "the program does not start at $C000");
    assert_eq!(highest, PRG_BASE + PRG_BYTES - 1, "the last byte is $FFFF");

    // The three vectors point at the program's own entry points. There
    // is no parameter saying where a 6502 starts: it reads $FFFC.
    assert_eq!(word_at(&image, VEC_RES), image.label("reset"));
    assert_eq!(word_at(&image, VEC_NMI), image.label("nmi"));
    assert_eq!(word_at(&image, VEC_IRQ), image.label("irq"));
    assert_ne!(
        image.label("nmi"),
        image.label("irq"),
        "the frame handler and the IRQ handler are different code here"
    );

    check_hex(
        &dir,
        "sw/demo.hex",
        "demo.hex: sw/demo.s assembled",
        PRG_BASE,
        &image,
    );
}

#[test]
fn chr_hex_is_the_assembled_source() {
    let Some(dir) = example() else { return };
    let image = assemble(&dir, "sw/chr.s");
    let highest = u32::from(*image.bytes.keys().next_back().expect("tiles"));
    assert!(highest < CHR_BYTES, "a tile is outside the 8 KiB of CHR");
    // Tile zero of the sprite table has to exist and has to be blank: a
    // slot with no sprite in it still fetches something.
    for offset in 0x1000..0x1010u16 {
        assert_eq!(image.bytes.get(&offset), Some(&0), "sprite tile 0 is blank");
    }
    check_hex(&dir, "sw/chr.hex", "chr.hex: sw/chr.s assembled", 0, &image);
}

/// One tile's sixteen bytes worked out from eight rows of art, where
/// `.` is colour 0 and `1`, `2` and `3` are the other three: the low
/// bit of each pixel goes into the first eight bytes and the high bit
/// into the next eight, leftmost pixel in bit 7.
fn tile_bytes(art: &[String]) -> Vec<u8> {
    let mut planes = vec![0u8; 16];
    for (row, line) in art.iter().enumerate() {
        for (x, c) in line.chars().enumerate() {
            let value = match c {
                '.' => 0u8,
                d => u8::try_from(d.to_digit(4).expect("a colour from 0 to 3"))
                    .expect("a colour from 0 to 3"),
            };
            let bit = 0x80u8 >> x;
            if value & 1 != 0 {
                planes[row] |= bit;
            }
            if value & 2 != 0 {
                planes[row + 8] |= bit;
            }
        }
    }
    planes
}

#[test]
fn the_tiles_are_the_art_drawn_above_them() {
    let Some(dir) = example() else { return };
    let source = read(&dir, "sw/chr.s");
    let image = assemble(&dir, "sw/chr.s");

    // A run of eight comment lines that are each exactly eight
    // characters of `.123` is a tile, and the label under it names where
    // it was placed.
    let mut art: Vec<String> = Vec::new();
    let mut tiles = 0usize;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(';') {
            let row = rest.trim();
            if row.len() == 8 && row.chars().all(|c| ".123".contains(c)) {
                art.push(row.to_owned());
                continue;
            }
        }
        if let Some(label) = trimmed.strip_suffix(':')
            && art.len() == 8
        {
            let at = image.label(label);
            let want = tile_bytes(&art);
            let got: Vec<u8> = (0..16)
                .map(|i| image.bytes[&(at + u16::try_from(i).expect("sixteen bytes"))])
                .collect();
            assert_eq!(got, want, "`{label}` is not the art drawn above it");
            tiles += 1;
        }
        if !trimmed.starts_with(';') {
            art.clear();
        }
    }
    assert_eq!(tiles, 11, "every tile in sw/chr.s is checked");

    // And the reading of the format itself, against a tile worked out by
    // hand: the row `.123....` is 0,1,2,3,0,0,0,0, so the low plane's
    // first byte is 0b0101_0000 and the high plane's 0b0011_0000.
    let hand: Vec<String> = std::iter::once(".123....".to_owned())
        .chain(std::iter::repeat_n("........".to_owned(), 7))
        .collect();
    let bytes = tile_bytes(&hand);
    assert_eq!(bytes[0], 0b0101_0000);
    assert_eq!(bytes[8], 0b0011_0000);
    assert!(bytes[1..8].iter().all(|b| *b == 0));
    assert!(bytes[9..16].iter().all(|b| *b == 0));
}

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

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

/// The design `reticle build` produces.
fn console_design(dir: &Path) -> Design {
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

/// The files the design reads: the two hex images its `$readmemh`
/// statements name, relative to the project.
fn example_files(dir: &Path) -> MemoryFiles {
    let mut files = MemoryFiles::new();
    files.insert("sw/demo.hex", read(dir, "sw/demo.hex"));
    files.insert("sw/chr.hex", read(dir, "sw/chr.hex"));
    files
}

fn synth_options(dir: &Path) -> SynthOptions {
    SynthOptions {
        files: Some(Rc::new(example_files(dir))),
        ..SynthOptions::default()
    }
}

/// Everything synthesis said at warning level or above.
fn complaints(diags: &Diagnostics) -> Vec<String> {
    diags
        .iter()
        .filter(|d| d.severity >= Severity::Warning)
        .map(|d| format!("{}: {}", d.code.unwrap_or("-"), d.message))
        .collect()
}

#[test]
fn the_project_resolves_and_elaborates() {
    let Some(dir) = example() else { return };
    let built = build(&dir, |_| {});
    assert_eq!(built.project.top.as_deref(), Some("nes_top"));
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
            ("ppu2c02", "rtl/ppu_palette.v"),
            ("ppu2c02", "rtl/ppu2c02.v"),
            ("dvi_tx", "rtl/tmds_encoder.v"),
            ("dvi_tx", "rtl/video_timing.v"),
            ("dvi_tx", "rtl/dvi_tx.v"),
            ("vga_out", "rtl/vga_out.v"),
            ("nes", "rtl/nes_console.v"),
            ("nes", "rtl/nes_video.v"),
            ("nes", "rtl/nes_top.v"),
            ("nes", "rtl/nes_basys3.v"),
        ],
        "the processor, the picture unit and the two video outputs come from the library"
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
    assert_eq!(top.name.as_str(), "nes_top");
}

#[test]
fn the_console_synthesises_without_errors_or_latches() {
    let Some(dir) = example() else { return };
    let mut design = console_design(&dir);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &synth_options(&dir), &mut diags);
    assert!(
        complaints(&diags).is_empty(),
        "synthesis complained:\n  {}",
        complaints(&diags).join("\n  ")
    );

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

    // The cartridge's two memories hold what the hex files say, loaded
    // by the design's own `$readmemh`, with every byte the files did not
    // name still `x` — so nothing invented the padding.
    for (memory, rel, size) in [
        ("prg", "sw/demo.hex", PRG_BYTES),
        ("chr", "sw/chr.hex", CHR_BYTES),
    ] {
        let init = memory_init(&design, "nes_console", memory).expect("the memory was loaded");
        assert!(
            u32::try_from(init.len()).expect("a small memory") <= size,
            "{memory} was given more elements than the console declares"
        );
        let image = hex_image(&dir, rel);
        for offset in 0..size {
            let got = init
                .get(usize::try_from(offset).expect("a small memory"))
                .copied()
                .flatten();
            assert_eq!(
                got,
                image.get(&offset).copied(),
                "{memory} at {offset:#06x}"
            );
        }
    }
}

/// The initial contents of a memory of `module`, byte by byte, with
/// `None` for an element nothing named.
fn memory_init(design: &Design, module: &str, memory: &str) -> Option<Vec<Option<u8>>> {
    let m = design
        .modules
        .iter()
        .map(|(_, m)| m)
        .find(|m| m.name.as_str().split('$').next() == Some(module))
        .unwrap_or_else(|| panic!("{module} is in the design"));
    let mem = m
        .memories
        .iter()
        .map(|(_, m)| m)
        .find(|m| m.name.as_str() == memory)
        .unwrap_or_else(|| panic!("{module} has a memory called {memory}"));
    mem.init.as_ref().map(|init| {
        init.iter()
            .map(|w| w.to_u64().map(|v| u8::try_from(v).expect("a byte")))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// The simulator support
// ---------------------------------------------------------------------------

fn sim_options(dir: &Path) -> SimOptions {
    SimOptions {
        files: Some(Box::new(example_files(dir))),
        ..SimOptions::default()
    }
}

/// A simulation of one module of the example, driven a clock at a time.
struct Run<'d> {
    sim: Simulator<'d>,
}

impl<'d> Run<'d> {
    fn new(design: &'d Design, top: &str, options: SimOptions) -> Run<'d> {
        let options = SimOptions {
            top: Some(top.to_owned()),
            ..options
        };
        Run {
            sim: Simulator::new(design, options).expect("the design simulates"),
        }
    }

    fn net(&self, name: &str) -> reticle::sim::NetHandle {
        self.sim
            .net(name)
            .unwrap_or_else(|| panic!("no net `{name}`"))
    }

    /// The value of a net, when every bit of it is known.
    fn maybe(&self, handle: reticle::sim::NetHandle) -> Option<u64> {
        self.sim.get(handle).to_u64()
    }

    fn get(&self, handle: reticle::sim::NetHandle) -> u64 {
        self.sim.get(handle).to_u64().unwrap_or_else(|| {
            panic!(
                "`{}` is not a number: {}",
                self.sim.net_name(handle),
                self.sim.get(handle)
            )
        })
    }

    fn set(&mut self, handle: reticle::sim::NetHandle, value: u64, width: u32) {
        self.sim
            .set(handle, reticle::logic::Logic::from_u64(value, width));
    }
}

const HALF: u64 = 1;

// The rest of the tests are built on these two helpers, which turn the
// event simulator into "one clock edge at a time" without a testbench.
fn tick(run: &mut Run<'_>, clk: reticle::sim::NetHandle) {
    run.sim.run_for(HALF);
    run.set(clk, 1, 1);
    run.sim.run_for(HALF);
    run.set(clk, 0, 1);
}

// ---------------------------------------------------------------------------
// The picture, worked out from the documentation
//
// Everything in this section is the *reference*: given the bytes in the
// nametable, the pattern table and the palette, and the scroll the
// program set, it says what the 61,440 pixels of a frame have to be. It
// is written from the published description of the 2C02 — where a tile
// comes from, where its attribute comes from, how the two bitplanes
// become a colour index, which sprite wins — and it knows nothing about
// how `ip/ppu2c02` fetches anything, which is the only reason comparing
// the two means anything at all. `docs/writing-a-cpu.md` section 3 is
// the argument.
// ---------------------------------------------------------------------------

/// A palette entry, with the four mirrors the part has: $3F10, $3F14,
/// $3F18 and $3F1C are not entries of their own, they are $3F00, $3F04,
/// $3F08 and $3F0C seen through the sprite half.
fn palette_at(palette: &[u8; 32], index: usize) -> u8 {
    let index = if index.is_multiple_of(4) {
        index & 0x0F
    } else {
        index
    };
    palette[index] & 0x3F
}

/// The byte the console's nametable memory answers with for a picture
/// bus address, under vertical mirroring: the two 1 KiB tables sit side
/// by side, so address bit 10 is the one that chooses and bit 11 is not
/// decoded at all.
fn nametable_at(nt: &[u8], address: usize) -> u8 {
    let bank = (address >> 10) & 1;
    nt[bank * 1024 + (address & 0x3FF)]
}

/// The two bits of one pixel of a tile: bit `7 - column` of the tile's
/// low-plane byte for `row`, and of the byte eight further on for the
/// high plane.
fn tile_pixel(chr: &[u8], base: usize, tile: u8, row: usize, column: usize) -> u8 {
    let at = base + usize::from(tile) * 16 + row;
    let lo = (chr[at] >> (7 - column)) & 1;
    let hi = (chr[at + 8] >> (7 - column)) & 1;
    (hi << 1) | lo
}

/// One sprite as OAM holds it.
#[derive(Clone, Copy)]
struct Sprite {
    y: u8,
    tile: u8,
    attr: u8,
    x: u8,
}

/// What the video port has to carry for frame `n` of the demo: 240 rows
/// of 256 palette indices.
///
/// The demo scrolls `n` pixels left and writes no vertical scroll, so
/// the tile under screen pixel (x, y) is the one at horizontal position
/// `n + x` of a 512-pixel-wide pair of nametables and at vertical
/// position `y` of the 240-line one.
fn expected_frame(
    n: u32,
    nt: &[u8],
    chr: &[u8],
    palette: &[u8; 32],
    oam: &[Sprite; 64],
) -> Vec<u8> {
    // $2000 = $88 in the demo: the background comes out of pattern
    // table 0 and the sprites out of table 1.
    const BG_BASE: usize = 0x0000;
    const SPRITE_BASE: usize = 0x1000;

    let mut out = vec![0u8; SCREEN_W * SCREEN_H];
    for y in 0..SCREEN_H {
        // The eight sprites of this line, in the order the evaluator
        // finds them: OAM order, first eight only, and none at all on
        // line 0, because evaluation does not run on the pre-render
        // line. A sprite whose Y is `sy` is drawn on lines sy+1 to
        // sy+8.
        let mut line_sprites: Vec<(Sprite, usize)> = Vec::new();
        for sprite in oam.iter() {
            let top = usize::from(sprite.y) + 1;
            if y >= top && y < top + 8 && y > 0 {
                if line_sprites.len() == 8 {
                    break;
                }
                line_sprites.push((*sprite, y - top));
            }
        }

        for x in 0..SCREEN_W {
            // --- the background ---------------------------------
            let column = n as usize + x;
            let bank = (column >> 8) & 1;
            let tile_x = (column >> 3) & 31;
            let fine_x = column & 7;
            let tile_y = y / 8;
            let fine_y = y % 8;

            let base = 0x2000 + bank * 0x400;
            let tile = nametable_at(nt, base + tile_y * 32 + tile_x);
            let attribute = nametable_at(nt, base + 0x3C0 + (tile_y / 4) * 8 + (tile_x / 4));
            let quadrant = ((tile_y & 2) << 1) | (tile_x & 2);
            let bg_palette = (attribute >> quadrant) & 3;
            let bg_bits = tile_pixel(chr, BG_BASE, tile, fine_y, fine_x);

            // --- the sprites ------------------------------------
            let mut sprite_bits = 0u8;
            let mut sprite_palette = 0u8;
            let mut behind = false;
            for (sprite, row) in &line_sprites {
                if x < usize::from(sprite.x) || x >= usize::from(sprite.x) + 8 {
                    continue;
                }
                let column = x - usize::from(sprite.x);
                let column = if sprite.attr & 0x40 != 0 {
                    7 - column
                } else {
                    column
                };
                let row = if sprite.attr & 0x80 != 0 {
                    7 - row
                } else {
                    *row
                };
                let bits = tile_pixel(chr, SPRITE_BASE, sprite.tile, row, column);
                if bits != 0 {
                    sprite_bits = bits;
                    sprite_palette = sprite.attr & 3;
                    behind = sprite.attr & 0x20 != 0;
                    break;
                }
            }

            // --- which of them is on top ------------------------
            // $2001 = $1E in the demo: background and sprites both
            // shown, both of them in the leftmost eight pixels too, and
            // no greyscale.
            let bg_opaque = bg_bits != 0;
            let sprite_opaque = sprite_bits != 0;
            out[y * SCREEN_W + x] = if !bg_opaque && !sprite_opaque {
                palette_at(palette, 0)
            } else if sprite_opaque && (!bg_opaque || !behind) {
                palette_at(
                    palette,
                    16 + usize::from(sprite_palette) * 4 + usize::from(sprite_bits),
                )
            } else {
                palette_at(palette, usize::from(bg_palette) * 4 + usize::from(bg_bits))
            };
        }
    }
    out
}

/// The hash `tb/nes_tb.v` folds a frame into, computed again here: a
/// shift and a polynomial over the palette index of every visible pixel,
/// in the order the raster puts them out.
fn frame_hash(frame: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for colour in frame {
        let carry = hash & 0x8000_0000 != 0;
        hash = (hash << 1) ^ if carry { 0x04c1_1db7 } else { 0 } ^ u32::from(*colour);
    }
    hash
}

/// The demo's palette and its four sprites, read out of the program it
/// was assembled into rather than typed again here.
fn demo_tables(image: &asm::Image) -> ([u8; 32], [Sprite; 4]) {
    let byte = |at: u16| image.bytes[&at];

    let base = image.label("palette");
    let mut palette = [0u8; 32];
    for i in 0..32usize {
        // The program writes those 32 bytes through $2007 from $3F00, so
        // four of them land on the mirrors and overwrite entries 0, 4, 8
        // and 12 on the way past.
        let index = if i.is_multiple_of(4) { i & 0x0F } else { i };
        palette[index] = byte(base + u16::try_from(i).expect("32 bytes")) & 0x3F;
    }

    let base = image.label("sprites");
    let mut sprites = [Sprite {
        y: 0,
        tile: 0,
        attr: 0,
        x: 0,
    }; 4];
    for (i, sprite) in sprites.iter_mut().enumerate() {
        let at = base + u16::try_from(i * 4).expect("sixteen bytes");
        *sprite = Sprite {
            y: byte(at),
            tile: byte(at + 1),
            attr: byte(at + 2),
            x: byte(at + 3),
        };
    }
    (palette, sprites)
}

/// OAM as the demo leaves it for frame `n`: the four sprites it uses,
/// each moved `n` pixels right of where the table puts it, and sixty
/// more parked on line 249, where nothing can see them.
fn demo_oam(sprites: &[Sprite; 4], n: u32) -> [Sprite; 64] {
    let mut oam = [Sprite {
        y: 0xF8,
        tile: 0xF8,
        attr: 0xF8,
        x: 0xF8,
    }; 64];
    for (slot, sprite) in oam.iter_mut().zip(sprites.iter()) {
        *slot = Sprite {
            x: sprite
                .x
                .wrapping_add(u8::try_from(n & 0xFF).expect("a byte")),
            ..*sprite
        };
    }
    oam
}

/// The colour the demo has written into $3F02 by frame `n`: it advances
/// every eighth frame through eight of the palette's bright hues.
fn demo_star_colour(n: u32) -> u8 {
    0x21 + u8::try_from((n >> 3) & 7).expect("three bits")
}

// ---------------------------------------------------------------------------
// The picture unit on its own
//
// `Ppu` puts the test on the other side of the picture bus: it holds the
// 16 KiB the cartridge and the console board would answer with, presents
// the byte at the address of the *previous* dot — which is the contract
// `ppu2c02` states and the shape a synchronous memory has — and takes
// whatever the block writes. Everything these tests look at is on the
// block's pins.
// ---------------------------------------------------------------------------

/// The picture unit with a memory behind it, stepped one dot at a time.
struct Ppu<'d> {
    run: Run<'d>,
    clk: reticle::sim::NetHandle,
    rst_n: reticle::sim::NetHandle,
    reg_addr: reticle::sim::NetHandle,
    reg_din: reticle::sim::NetHandle,
    reg_dout: reticle::sim::NetHandle,
    reg_we: reticle::sim::NetHandle,
    reg_re: reticle::sim::NetHandle,
    vram_addr: reticle::sim::NetHandle,
    vram_dout: reticle::sim::NetHandle,
    vram_we: reticle::sim::NetHandle,
    vram_din: reticle::sim::NetHandle,
    dbg_dot: reticle::sim::NetHandle,
    dbg_line: reticle::sim::NetHandle,
    /// Pattern memory at $0000-$1FFF and nametable memory at $2000-$3FFF.
    mem: Vec<u8>,
    /// The address the block put on the bus during the previous dot.
    last: usize,
    /// What it put there during the dot that just ran.
    bus: usize,
    dots: u64,
}

impl<'d> Ppu<'d> {
    fn boot(design: &'d Design) -> Ppu<'d> {
        let run = Run::new(design, "ppu2c02", SimOptions::default());
        let ppu = |name: &str| format!("ppu2c02.{name}");
        let clk = run.net(&ppu("clk"));
        let rst_n = run.net(&ppu("rst_n"));
        let en = run.net(&ppu("en"));
        let me = Ppu {
            clk,
            rst_n,
            reg_addr: run.net(&ppu("reg_addr")),
            reg_din: run.net(&ppu("reg_din")),
            reg_dout: run.net(&ppu("reg_dout")),
            reg_we: run.net(&ppu("reg_we")),
            reg_re: run.net(&ppu("reg_re")),
            vram_addr: run.net(&ppu("vram_addr")),
            vram_dout: run.net(&ppu("vram_dout")),
            vram_we: run.net(&ppu("vram_we")),
            vram_din: run.net(&ppu("vram_din")),
            dbg_dot: run.net(&ppu("dbg_dot")),
            dbg_line: run.net(&ppu("dbg_line")),
            mem: vec![0u8; 0x4000],
            last: 0,
            bus: 0,
            dots: 0,
            run,
        };
        let mut me = me;
        // Every dot, no register access, held in reset for a few edges.
        me.run.set(en, 1, 1);
        me.run.set(me.rst_n, 0, 1);
        me.run.set(me.reg_addr, 0, 3);
        me.run.set(me.reg_din, 0, 8);
        me.run.set(me.reg_we, 0, 1);
        me.run.set(me.reg_re, 0, 1);
        me.run.set(me.vram_din, 0, 8);
        for _ in 0..4 {
            tick(&mut me.run, clk);
        }
        me.run.set(me.rst_n, 1, 1);
        me
    }

    /// One dot: the memory answers the address of the previous dot, the
    /// block is clocked, and whatever it wrote lands in the memory.
    /// Returns what a processor reading `reg_addr` would have taken,
    /// which is `None` while the register it names has bits nothing has
    /// written — OAM comes out of configuration undefined, as it does on
    /// a real part.
    fn step(&mut self) -> Option<u8> {
        let answer = u64::from(self.mem[self.last]);
        self.run.set(self.vram_din, answer, 8);
        self.run.sim.run_for(HALF);
        let addr = usize::try_from(self.run.get(self.vram_addr)).expect("fourteen bits");
        let write = self.run.get(self.vram_we) == 1;
        let value = self.run.get(self.vram_dout);
        let read = self.run.maybe(self.reg_dout);
        self.run.set(self.clk, 1, 1);
        self.run.sim.run_for(HALF);
        self.run.set(self.clk, 0, 1);
        if write {
            self.mem[addr] = u8::try_from(value).expect("a byte");
        }
        self.last = addr;
        self.bus = addr;
        self.dots += 1;
        read.map(|v| u8::try_from(v).expect("a byte"))
    }

    /// One dot with a write to $2000 + `reg`.
    fn write(&mut self, reg: u8, value: u8) {
        self.run.set(self.reg_addr, u64::from(reg), 3);
        self.run.set(self.reg_din, u64::from(value), 8);
        self.run.set(self.reg_we, 1, 1);
        self.step();
        self.run.set(self.reg_we, 0, 1);
    }

    /// One dot with a read of $2000 + `reg`, and what it gave back.
    fn read(&mut self, reg: u8) -> u8 {
        self.run.set(self.reg_addr, u64::from(reg), 3);
        self.run.set(self.reg_re, 1, 1);
        let value = self.step();
        self.run.set(self.reg_re, 0, 1);
        value.expect("the register has a value")
    }

    fn dot(&self) -> u64 {
        self.run.get(self.dbg_dot)
    }

    fn line(&self) -> u64 {
        self.run.get(self.dbg_line)
    }
}

/// The picture unit on its own, with the project's sources behind it.
fn ppu_design(dir: &Path) -> Design {
    build(dir, |project| project.top = Some("ppu2c02".to_owned()))
        .elaboration
        .design
        .expect("ppu2c02 elaborates")
}

#[test]
fn the_write_latch_is_shared_by_2005_and_2006() {
    let Some(dir) = example() else { return };
    let design = ppu_design(&dir);
    let mut ppu = Ppu::boot(&design);

    // Rendering is off, so the block puts `v` on the picture bus every
    // dot and nothing else does. That is the whole of this test's
    // instrumentation: no register is read out of the block, the
    // address pins say what `v` is.
    let v = |ppu: &mut Ppu<'_>| {
        ppu.step();
        ppu.bus
    };
    assert_eq!(v(&mut ppu), 0, "`v` starts at zero");

    // A plain pair of writes to $2006 is the ordinary way to point `v`
    // somewhere: high six bits, then low eight.
    ppu.write(6, 0x21);
    assert_eq!(
        v(&mut ppu),
        0,
        "the first write of a pair moves `t`, not `v`"
    );
    ppu.write(6, 0x08);
    assert_eq!(v(&mut ppu), 0x2108, "the second write copies `t` into `v`");

    // Now the part everyone gets wrong. $2005 and $2006 share **one**
    // latch: a write to $2005 with the latch down is a *first* write and
    // puts the latch up, and the next write to $2006 is therefore a
    // *second* write, which copies `t` into `v` — with the fine and
    // coarse scroll the $2005 write just put into `t` still in it.
    ppu.read(2); // the latch down
    ppu.write(5, 0x18); // a *first* write: coarse X into `t`, fine X into `x`
    ppu.write(6, 0x34); // and so this is a *second* write, not a first
    assert_eq!(
        v(&mut ppu),
        0x2134,
        "a $2005 write and a $2006 write make one pair: if the two registers \
         had a latch each, this write would have been a first one and `v` \
         would still be $2108"
    );

    // And a read of $2002 puts the latch back, so a $2006 write after
    // one is a first write again and leaves `v` where it was.
    ppu.write(6, 0x21); // first: the latch goes up
    ppu.read(2); // ...and straight back down
    ppu.write(6, 0x08); // so this is a first write, not a second
    assert_eq!(
        v(&mut ppu),
        0x2134,
        "a first write does not touch `v`, whatever the program meant"
    );
    ppu.write(6, 0x40); // *now* the second write
    assert_eq!(
        v(&mut ppu),
        0x0840,
        "`v` takes the high bits of the last first write, which was $08"
    );

    // Two writes to $2005 are a pair of their own and never touch `v`.
    ppu.read(2);
    ppu.write(5, 0xFF);
    ppu.write(5, 0xFF);
    assert_eq!(v(&mut ppu), 0x0840, "$2005 writes `t` and `x`, never `v`");
    // ...and the `x` of that pair is the fine scroll, which nothing on
    // these pins can show. `the_frame_comes_out_of_the_video_port` is
    // where it is proved, one pixel at a time.

    // A $2007 access moves `v` on by one, which is the other thing the
    // address pins show.
    ppu.write(7, 0xAA);
    assert_eq!(
        ppu.mem[0x0840], 0xAA,
        "the write went to the address `v` named"
    );
    assert_eq!(v(&mut ppu), 0x0841, "and `v` advanced by one");
}

#[test]
fn sprite_zero_hits_on_the_dot_the_pixels_meet() {
    let Some(dir) = example() else { return };
    let design = ppu_design(&dir);
    let mut ppu = Ppu::boot(&design);

    // A background that is opaque everywhere: every nametable entry is
    // tile 1, whose low plane is solid, and every attribute byte is 0.
    for byte in ppu.mem[0x2000..0x2400].iter_mut() {
        *byte = 1;
    }
    for byte in ppu.mem[0x23C0..0x2400].iter_mut() {
        *byte = 0;
    }
    for row in 0..8 {
        ppu.mem[0x0010 + row] = 0xFF;
        ppu.mem[0x0018 + row] = 0x00;
    }
    // A sprite tile whose leftmost opaque column is column 3: bits 4
    // down to 0 of each row, which is $1F.
    const FIRST_COLUMN: usize = 3;
    for row in 0..8 {
        ppu.mem[0x1010 + row] = 0x1F;
        ppu.mem[0x1018 + row] = 0x00;
    }

    // OAM, all 256 bytes of it: sprite zero where we want it and the
    // other 63 parked on line 249.
    const SPRITE_X: u64 = 100;
    const SPRITE_Y: u64 = 50;
    ppu.write(3, 0);
    for index in 0..64u64 {
        let entry = if index == 0 {
            [
                u8::try_from(SPRITE_Y).unwrap(),
                1,
                0,
                u8::try_from(SPRITE_X).unwrap(),
            ]
        } else {
            [0xF8, 0xF8, 0xF8, 0xF8]
        };
        for byte in entry {
            ppu.write(4, byte);
        }
    }

    // Sprites out of pattern table 1, background out of table 0, and
    // both of them shown.
    ppu.write(0, 0x08);
    ppu.write(1, 0x1E);

    // A sprite whose Y is `sy` is drawn on lines sy+1 to sy+8, and its
    // leftmost opaque column lands on pixel `sx + column`. The pixel of
    // dot `d` is screen pixel `d - 1`, so the two meet on the dot below
    // — and the flag is a flip-flop, so a processor reading $2002 sees
    // it on the dot *after* that one.
    let hit_line = SPRITE_Y + 1;
    let hit_dot = SPRITE_X + u64::try_from(FIRST_COLUMN).unwrap() + 1;

    let mut seen: Option<(u64, u64)> = None;
    while ppu.line() < hit_line + 2 {
        let line = ppu.line();
        let dot = ppu.dot();
        let status = ppu.read(2);
        if status & 0x40 != 0 && seen.is_none() {
            seen = Some((line, dot));
        }
    }
    assert_eq!(
        seen,
        Some((hit_line, hit_dot + 1)),
        "sprite zero hit is not on the dot the two opaque pixels meet"
    );

    // And with the background turned off there is nothing for it to
    // meet, so the flag never goes up at all.
    let mut ppu = Ppu::boot(&design);
    for byte in ppu.mem[0x2000..0x2400].iter_mut() {
        *byte = 1;
    }
    for byte in ppu.mem[0x23C0..0x2400].iter_mut() {
        *byte = 0;
    }
    for row in 0..8 {
        ppu.mem[0x0010 + row] = 0xFF;
        ppu.mem[0x1010 + row] = 0x1F;
    }
    ppu.write(3, 0);
    for index in 0..64u64 {
        let entry = if index == 0 {
            [
                u8::try_from(SPRITE_Y).unwrap(),
                1,
                0,
                u8::try_from(SPRITE_X).unwrap(),
            ]
        } else {
            [0xF8, 0xF8, 0xF8, 0xF8]
        };
        for byte in entry {
            ppu.write(4, byte);
        }
    }
    ppu.write(0, 0x08);
    ppu.write(1, 0x10); // sprites only
    while ppu.line() < hit_line + 2 {
        assert_eq!(ppu.read(2) & 0x40, 0, "a hit with no background under it");
    }
}

#[test]
fn nine_sprites_on_a_line_set_the_overflow_flag() {
    let Some(dir) = example() else { return };
    let design = ppu_design(&dir);
    let mut ppu = Ppu::boot(&design);

    // Eight sprites on line 41 and a ninth on line 42, so the flag has
    // to be clear for one line and set for the next.
    ppu.write(3, 0);
    for index in 0..64u64 {
        let entry = match index {
            0..=7 => [40, 1, 0, u8::try_from(index * 8).unwrap()],
            8 => [41, 1, 0, 200],
            _ => [0xF8, 0xF8, 0xF8, 0xF8],
        };
        for byte in entry {
            ppu.write(4, byte);
        }
    }
    ppu.write(0, 0x08);
    ppu.write(1, 0x1E);

    // Evaluation for line L happens during line L - 1, so the eight of
    // line 41 are counted on line 40 and the nine of line 42 on line 41.
    let mut set_on: Option<u64> = None;
    while ppu.line() < 45 {
        let line = ppu.line();
        if ppu.read(2) & 0x20 != 0 && set_on.is_none() {
            set_on = Some(line);
        }
    }
    assert_eq!(
        set_on,
        Some(41),
        "the overflow flag goes up on the line that finds a ninth sprite"
    );
}

// ---------------------------------------------------------------------------
// The whole console
// ---------------------------------------------------------------------------

/// The frame the test looks at: the second one the console draws. The
/// first is the one `sw/demo.s` spends filling the nametables, and the
/// second is the first drawn from a complete picture, with the scroll
/// and the sprites one step along.
const FRAME: u32 = 1;

/// The hash `tb/nes_tb.v` prints for that frame. It is here rather than
/// only in the testbench so that the two implementations of the picture
/// — the Verilog that watches the pins and the Rust that works out what
/// they should say — have one number between them.
const FRAME_HASH: u32 = 0x7f6f_6b48;

/// The console on its own: the project with `nes_console` as the top,
/// which is the machine without the frame buffer or the transmitter.
fn console_only(dir: &Path) -> Design {
    build(dir, |project| {
        project.top = Some("nes_console".to_owned());
    })
    .elaboration
    .design
    .expect("nes_console elaborates")
}

/// The pattern memory as a flat 8 KiB, from the assembled source rather
/// than from anything the hardware did with it.
fn chr_bytes(dir: &Path) -> Vec<u8> {
    let image = assemble(dir, "sw/chr.s");
    let mut chr = vec![0u8; usize::try_from(CHR_BYTES).expect("8 KiB")];
    for (at, byte) in &image.bytes {
        chr[usize::from(*at)] = *byte;
    }
    chr
}

/// A frame as a page of text, for a failure message: one character per
/// four pixels across and per eight down, so 240 lines fit in a screen.
fn sketch(frame: &[u8]) -> String {
    const SHADES: &[u8] = b" .:-=+*#%@";
    let mut out = String::new();
    for y in (0..SCREEN_H).step_by(8) {
        for x in (0..SCREEN_W).step_by(4) {
            let colour = frame[y * SCREEN_W + x];
            let level = usize::from(colour >> 4) * 3 + usize::from(colour & 0x0F) / 6;
            out.push(char::from(SHADES[level.min(SHADES.len() - 1)]));
        }
        out.push('\n');
    }
    out
}

#[test]
fn the_frame_comes_out_of_the_video_port() {
    let Some(dir) = example() else { return };
    let design = console_only(&dir);
    let mut run = Run::new(&design, "nes_console", sim_options(&dir));

    let clk = run.net("nes_console.clk");
    let rst_n = run.net("nes_console.rst_n");
    let en = run.net("nes_console.en");
    let vid_de = run.net("nes_console.vid_de");
    let vid_x = run.net("nes_console.vid_x");
    let vid_y = run.net("nes_console.vid_y");
    let vid_color = run.net("nes_console.vid_color");
    let dbg_dot = run.net("nes_console.dbg_dot");
    let dbg_line = run.net("nes_console.dbg_line");

    // One dot per clock, and a reset long enough for the processor to
    // see it.
    run.set(en, 1, 1);
    run.set(rst_n, 0, 1);
    for _ in 0..4 {
        tick(&mut run, clk);
    }
    run.set(rst_n, 1, 1);

    // Everything below comes off those five pins and nothing else.
    let mut got = vec![0u8; SCREEN_W * SCREEN_H];
    let mut pixels = 0usize;
    let mut frame = 0u32;
    let mut dots = 0u64;
    loop {
        tick(&mut run, clk);
        dots += 1;
        assert!(
            dots < 4 * DOTS_PER_FRAME,
            "the raster did not come round twice in four frames' worth of dots"
        );
        if run.get(dbg_dot) == 0 && run.get(dbg_line) == 0 {
            frame += 1;
            if frame > FRAME {
                break;
            }
        }
        if frame == FRAME && run.get(vid_de) == 1 {
            let x = usize::try_from(run.get(vid_x)).expect("a column");
            let y = usize::try_from(run.get(vid_y)).expect("a row");
            got[y * SCREEN_W + x] = u8::try_from(run.get(vid_color)).expect("six bits");
            pixels += 1;
        }
    }
    assert_eq!(
        pixels,
        SCREEN_W * SCREEN_H,
        "the video port did not carry 256 x 240 pixels"
    );

    // The picture is made out of the nametable the program wrote, the
    // pattern table the cartridge holds and the palette the program
    // sent to $3F00. The first comes out of the console's own memory,
    // which is where a program put it; the other two are read from the
    // sources, assembled here.
    let nt_handle = run
        .sim
        .memory("nes_console.nt")
        .expect("the console has nametable memory");
    let nt: Vec<u8> = (0..2048)
        .map(|i| {
            let value = run
                .sim
                .get_mem(nt_handle, i)
                .and_then(|v| v.to_u64())
                .unwrap_or_else(|| panic!("nametable byte {i} was never written"));
            u8::try_from(value).expect("a byte")
        })
        .collect();
    assert!(
        nt.contains(&5) && nt.contains(&6),
        "the program's own decoration is not in the nametable"
    );

    let chr = chr_bytes(&dir);
    let program = assemble(&dir, "sw/demo.s");
    let (mut palette, sprites) = demo_tables(&program);
    // ...and by this frame the program has advanced the star colour
    // once for every eight frames gone by.
    palette[2] = demo_star_colour(FRAME);
    let oam = demo_oam(&sprites, FRAME);

    let want = expected_frame(FRAME, &nt, &chr, &palette, &oam);

    // The hash the Verilog testbench prints comes from this side too, so
    // the constant is the model's and not the hardware's.
    assert_eq!(
        frame_hash(&want),
        FRAME_HASH,
        "FRAME_HASH is not the hash of the frame the documentation describes"
    );

    if got != want {
        let bad = got
            .iter()
            .zip(want.iter())
            .position(|(a, b)| a != b)
            .expect("a difference");
        panic!(
            "the picture is not the one the nametable, the tiles and the palette \
             describe.\nfirst wrong pixel at ({}, {}): the port said {:#04x}, the \
             documentation says {:#04x}\n\nwhat came out:\n{}\nwhat should have:\n{}",
            bad % SCREEN_W,
            bad / SCREEN_W,
            got[bad],
            want[bad],
            sketch(&got),
            sketch(&want)
        );
    }
    assert_eq!(frame_hash(&got), FRAME_HASH);
}

#[test]
fn oam_dma_copies_a_page_and_stops_the_processor() {
    let Some(dir) = example() else { return };
    let design = console_only(&dir);
    let mut run = Run::new(&design, "nes_console", sim_options(&dir));

    let clk = run.net("nes_console.clk");
    let rst_n = run.net("nes_console.rst_n");
    let en = run.net("nes_console.en");
    let dbg_dma = run.net("nes_console.dbg_dma");
    let dbg_retire = run.net("nes_console.dbg_retire");

    run.set(en, 1, 1);
    run.set(rst_n, 0, 1);
    for _ in 0..4 {
        tick(&mut run, clk);
    }
    run.set(rst_n, 1, 1);

    // The demo does its first DMA at the end of its own setup, which is
    // most of the way through the first frame.
    let mut dots = 0u64;
    while run.get(dbg_dma) == 0 {
        tick(&mut run, clk);
        dots += 1;
        assert!(dots < 2 * DOTS_PER_FRAME, "the program never wrote $4014");
    }

    // From there the processor is stopped until the 256 bytes are in.
    // One dummy cycle and then 256 pairs of a read and a write is 513
    // processor cycles, and the console gives the processor one cycle
    // in three dots.
    let mut held = 0u64;
    let mut retired = 0u64;
    while run.get(dbg_dma) == 1 {
        tick(&mut run, clk);
        held += 1;
        retired += run.get(dbg_retire);
        assert!(held < 4096, "the DMA never finished");
    }
    assert_eq!(held, 513 * 3, "the DMA is not 513 processor cycles long");
    assert_eq!(
        retired, 0,
        "the processor retired an instruction while the DMA had the bus"
    );
}

// ---------------------------------------------------------------------------
// The ECP5 flow
// ---------------------------------------------------------------------------

#[test]
fn the_console_maps_onto_the_ecp5_and_exports_for_nextpnr() {
    let Some(dir) = example() else { return };
    let mut design = console_design(&dir);
    let top = design.top.expect("a top");

    let device = fpga::target(DEVICE).expect("the ECP5 45F is a built-in device");
    let mut map = SourceMap::new();
    let rcf = read(&dir, "board/ecp5_dvi.rcf");
    let file = map
        .add("board/ecp5_dvi.rcf", rcf.clone())
        .expect("the constraints fit");
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(&rcf, file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    constraints.check(&design, device, &mut diags);
    let problems: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Warning)
        .map(|d| d.message.clone())
        .collect();
    assert!(problems.is_empty(), "the constraints: {problems:?}");

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

    println!("{DEVICE}:");
    for (cell, count) in &flow.netlist {
        println!("  {count:>5} x {cell}");
    }
    println!("  {} LUTs, depth {}", flow.luts, flow.lut_depth);

    // The processor, the picture unit, the frame buffer and the
    // transmitter all flattened into one netlist.
    assert!(flow.inlined >= 8, "{} instance(s) inlined", flow.inlined);
    let luts = flow.count("LUT4");
    assert!(luts > 3000, "{luts} LUTs is too few for a console");
    assert!(luts <= 43848, "{luts} LUTs does not fit the 45F's 43848");
    assert!(
        flow.netlist.iter().all(|(cell, _)| !cell.starts_with('$')),
        "a generic cell survived: {:?}",
        flow.netlist
    );

    // Every memory the machine has went into block RAM, and the two the
    // cartridge holds carry the program and the tiles.
    let mut in_block: Vec<&str> = flow
        .primitives
        .block_rams
        .iter()
        .map(|b| b.memory.as_str())
        .collect();
    in_block.sort_unstable();
    assert_eq!(
        in_block,
        [
            "u_console.chr",
            "u_console.nt",
            "u_console.prg",
            "u_console.ram",
            "u_console.u_ppu.oam",
            "u_video.fb",
        ],
        "a memory did not become block RAM"
    );
    for name in ["u_console.prg", "u_console.chr"] {
        let rom = flow
            .primitives
            .block_rams
            .iter()
            .find(|b| b.memory == name)
            .expect("the cartridge's memory");
        assert!(rom.initialised, "{name} carries no contents");
        println!(
            "  {name} -> {} x DP16KD in {}x{} mode",
            rom.blocks(),
            rom.mode.0,
            rom.mode.1
        );
    }

    // Every cell is a primitive the device has, and nothing is wrong.
    let problems: Vec<String> = fpga::check_nextpnr_json(&design, top, device, &constraints)
        .into_iter()
        .map(|p| format!("{}: {}", p.object, p.message))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    let inputs = fpga::export_nextpnr(&design, top, device, &constraints).expect("exports");
    assert_eq!(inputs.constraints_name, "nes_top.lpf");
    assert_eq!(
        inputs.args.join(" "),
        "nextpnr-ecp5 --45k --package CABGA381 --json nes_top.json --lpf nes_top.lpf \
         --textcfg nes_top.config"
    );
    for pin in ["clk_x5", "tmds_d0", "tmds_clk"] {
        assert!(
            inputs.pcf_or_lpf.contains(pin),
            "the LPF lacks `{pin}`:\n{}",
            inputs.pcf_or_lpf
        );
    }
    assert!(inputs.json.contains("\"DP16KD\""));

    // The files a board needs, where a reader can pick them up.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nes");
    fs::create_dir_all(&out).expect("a scratch directory");
    fs::write(out.join("nes_top.json"), &inputs.json).expect("write the netlist");
    fs::write(out.join(&inputs.constraints_name), &inputs.pcf_or_lpf).expect("write the LPF");
    println!("wrote {}/nes_top.json and nes_top.lpf", out.display());
    println!("then: {}", inputs.args.join(" "));

    // And the honest other half: the part the other two examples use
    // could hold this much logic and nowhere near this much memory. A
    // DP16KD is 16 Kibit and an SB_RAM40_4K is 4; the HX8K has 32 of
    // them, which is 128 Kibit against the 624 the console and its frame
    // buffer want.
    let want_bits = u64::try_from(flow.count("DP16KD")).expect("a few blocks") * 16_384;
    let hx8k_bits = 32u64 * 4096;
    println!("block RAM: {want_bits} bits wanted, {hx8k_bits} on {SMALL_DEVICE}",);
    assert!(
        want_bits > 4 * hx8k_bits,
        "the memory story against the HX8K is not what the README says"
    );
}

// ---------------------------------------------------------------------------
// The Artix-7 flow, for the Basys 3
// ---------------------------------------------------------------------------

#[test]
fn the_console_maps_onto_the_artix7_for_the_basys3() {
    let Some(dir) = example() else { return };
    let mut design = build(&dir, |project| {
        project.top = Some("nes_basys3".to_owned());
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
    //
    // The last measured footprint was 3786 LUT6, 80 CARRY4, 1082
    // flip-flops and 39 RAMB18E1, depth 23. The same console behind
    // `dvi_tx` on the same part is 4184 LUT6, 137 CARRY4, 1154
    // flip-flops and the same 39 RAMB18E1, depth 24 — a part it cannot
    // actually be built for, but a fair measure of what the three TMDS
    // encoders, their running-disparity registers and the three
    // serialisers cost. Dropping them is the whole difference: 398
    // LUT6, 57 CARRY4 and 72 flip-flops, and the memory is untouched
    // because the frame buffer is on the other side of the swap. The
    // bounds below are loose enough to survive an unrelated change and
    // tight enough that the carry chain or the saving silently going
    // away fails here.
    let luts = flow.count("LUT6");
    let brams = flow.count("RAMB18E1");
    let carries = flow.count("CARRY4");
    let flops: usize = ["FDRE", "FDSE", "FDCE", "FDPE"]
        .iter()
        .map(|kind| flow.count(kind))
        .sum();
    assert!(
        (55..=110).contains(&carries),
        "{carries} CARRY4 is not the carry chain this console has"
    );
    assert!(luts > 2500, "{luts} LUT6 is too few for a console");
    assert!(
        luts < 4184,
        "{luts} LUT6 is no better than the DVI build, which is the saving this target is for"
    );
    assert!(
        flops < 1154,
        "{flops} flip-flops is no better than the DVI build's 1154"
    );
    assert_eq!(brams, 39, "the memory is not the ECP5 build's 39 blocks");
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

    // Every memory the machine has is still in block RAM, and the
    // cartridge still carries the program and the tiles.
    let mut in_block: Vec<&str> = flow
        .primitives
        .block_rams
        .iter()
        .map(|b| b.memory.as_str())
        .collect();
    in_block.sort_unstable();
    assert_eq!(
        in_block,
        [
            "u_console.chr",
            "u_console.nt",
            "u_console.prg",
            "u_console.ram",
            "u_console.u_ppu.oam",
            "u_video.fb",
        ],
        "a memory did not become block RAM"
    );
    for name in ["u_console.prg", "u_console.chr"] {
        let rom = flow
            .primitives
            .block_rams
            .iter()
            .find(|b| b.memory == name)
            .expect("the cartridge's memory");
        assert!(rom.initialised, "{name} carries no contents");
    }

    // Every cell is a primitive the part has, wired to pins it has.
    let problems: Vec<String> = fpga::check_nextpnr_json(&design, top, device, &constraints)
        .into_iter()
        .map(|p| format!("{}: {}", p.object, p.message))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");

    // The three files Vivado reads, and the command line that runs them.
    let inputs = fpga::export_vendor(&design, top, device, &constraints).expect("the export");
    assert!(
        inputs.script.contains("-part xc7a35tcpg236-1"),
        "{}",
        inputs.script
    );
    assert_eq!(
        inputs.args,
        vec!["vivado", "-mode", "batch", "-source", "nes_basys3.tcl"]
    );
    for step in [
        "read_verilog nes_basys3.v",
        "read_xdc nes_basys3.xdc",
        "synth_design -top nes_basys3",
        "place_design",
        "route_design",
        "write_bitstream -force nes_basys3.bit",
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
    for pin in ["vga_r[0]", "vga_b[3]", "vga_hsync", "vga_vsync", "clk"] {
        assert!(
            inputs.xdc.contains(&format!("[get_ports {{{pin}}}]")),
            "the XDC lacks {pin}"
        );
    }

    // The files a board needs, where a reader can pick them up.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nes-basys3");
    fs::create_dir_all(&out).expect("a scratch directory");
    fs::write(out.join("nes_basys3.v"), &inputs.verilog).expect("write the netlist");
    fs::write(out.join("nes_basys3.xdc"), &inputs.xdc).expect("write the XDC");
    fs::write(out.join("nes_basys3.tcl"), &inputs.script).expect("write the script");
    println!("wrote {}/nes_basys3.{{v,xdc,tcl}}", out.display());
    println!("then: {}", inputs.args.join(" "));
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
const LIBRARY: [&str; 6] = [
    "../../ip/mos6502/rtl/mos6502.v",
    "../../ip/ppu2c02/rtl/ppu_palette.v",
    "../../ip/ppu2c02/rtl/ppu2c02.v",
    "../../ip/dvi_tx/rtl/tmds_encoder.v",
    "../../ip/dvi_tx/rtl/video_timing.v",
    "../../ip/dvi_tx/rtl/dvi_tx.v",
];

#[cfg(feature = "cli")]
#[test]
fn reticle_build_builds_the_project() {
    let Some(dir) = example() else { return };
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nes-build");
    fs::create_dir_all(&scratch).expect("a scratch directory");
    let lock = scratch.join("reticle.lock");
    let lock_arg = lock.to_string_lossy().into_owned();
    let (code, _, err) = reticle(
        &dir,
        &["build", "--synth", "--lock", &lock_arg, "reticle.proj"],
    );
    assert_eq!(code, 0, "reticle build failed:\n{err}");
    assert!(
        err.contains("note: built `nes`: 11 module(s) from 11 source(s)"),
        "{err}"
    );
    assert!(
        !err.contains("could not be loaded"),
        "a cartridge image was not loaded:\n{err}"
    );
    assert!(!err.contains("simulation-only statement dropped"), "{err}");
    let lock = fs::read_to_string(&lock).expect("a lock file");
    for package in ["mos6502", "ppu2c02", "dvi_tx", "vga_out"] {
        assert!(lock.contains(package), "{lock}");
    }
}

#[cfg(feature = "cli")]
#[test]
fn reticle_sim_runs_the_testbench() {
    let Some(dir) = example() else { return };
    let mut args = vec!["sim", "--quiet", "tb/nes_tb.v", "rtl/nes_console.v"];
    args.extend(LIBRARY);
    let (code, out, err) = reticle(&dir, &args);
    assert_eq!(code, 0, "{err}");
    let expected = format!(
        "frame 2: {} pixels, hash {FRAME_HASH:08x}",
        SCREEN_W * SCREEN_H
    );
    assert!(
        out.contains(&expected),
        "`reticle sim` did not print `{expected}`:\n{out}\n{err}"
    );
}

#[cfg(feature = "cli")]
#[test]
fn reticle_fpga_exports_the_cartridge_with_the_program_in_it() {
    let Some(dir) = example() else { return };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nes-fpga");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("a scratch directory");
    let out_arg = out.to_string_lossy().into_owned();
    let mut args = vec![
        "fpga",
        "--device",
        DEVICE,
        "--constraints",
        "board/ecp5_dvi.rcf",
        "--output-dir",
        &out_arg,
        "--quiet",
        "rtl/nes_top.v",
        "rtl/nes_console.v",
        "rtl/nes_video.v",
    ];
    args.extend(LIBRARY);
    let (code, _, err) = reticle(&dir, &args);
    assert_eq!(code, 0, "reticle fpga failed:\n{err}");

    let json = fs::read_to_string(out.join("nes_top.json")).expect("the netlist");
    // The cartridge's block RAMs carry `INITVAL_*` parameters holding
    // the program and the tiles; a blank cartridge would have every one
    // of them all zeroes.
    let inits: Vec<&str> = json
        .split("\"INITVAL_")
        .skip(1)
        .filter_map(|rest| rest.split('"').nth(2))
        .collect();
    assert!(!inits.is_empty(), "no block RAM carries INITVAL parameters");
    assert!(
        inits.iter().any(|v| v.chars().any(|c| c != '0')),
        "every INITVAL parameter is zero: the cartridge was exported blank"
    );
}

// ---------------------------------------------------------------------------
// The compiler defects this example ran into
//
// `examples/soc` documents eight and `examples/mos6502_computer` found
// none. This one found two, both in the FPGA flow, and both fixed in
// `src/`; what follows is the smallest HDL that shows each one, so that
// a regression is a failing test here rather than a slow afternoon.
// ---------------------------------------------------------------------------

/// The HDL of a regression, elaborated and taken through the ECP5 flow.
fn flow_of(name: &str, source: &str) -> (Design, reticle::fpga::FlowReport) {
    use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

    let mut map = SourceMap::new();
    let file = map.add(format!("{name}.v"), source).expect("it fits");
    let mut diags = Diagnostics::new();
    let ast = parse_source(
        &mut map,
        file,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    let mut design =
        elaborate(&[&ast], &ElabOptions::default(), &mut diags).expect("it elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let top = design.top.expect("a top");
    let device = fpga::target(DEVICE).expect("a device");
    let constraints = Constraints::default();
    let report = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow runs");
    (design, report)
}

/// Dead code elimination used to panic with `live memory kept` when an
/// expression that was itself dead still named a memory that had just
/// been removed. Two register arrays, one of which nothing reads, is
/// enough: the mapper lowers both, the unread one dies, and its `MemRead`
/// expression is still in the module when the memories are renumbered.
#[test]
fn dce_keeps_no_reference_to_a_memory_it_removed() {
    let (_, report) = flow_of(
        "dce_dead_memory",
        "module dce_dead_memory (\n\
         \x20   input  wire       clk,\n\
         \x20   input  wire [2:0] slot,\n\
         \x20   input  wire [7:0] value,\n\
         \x20   output reg  [7:0] out\n\
         );\n\
         \x20   reg [7:0] kept [0:7];\n\
         \x20   reg [7:0] dropped [0:7];\n\
         \x20   always @(posedge clk) begin\n\
         \x20       kept[slot]    <= value;\n\
         \x20       dropped[slot] <= value;\n\
         \x20       out           <= kept[slot];\n\
         \x20   end\n\
         endmodule\n",
    );
    assert!(
        report
            .netlist
            .iter()
            .all(|(cell, _)| !cell.starts_with('$')),
        "the flow left a generic cell: {:?}",
        report.netlist
    );
}

/// The shape that `unique_name` used to choke on: several small arrays
/// in one module, each lowered into hundreds of cells that all want the
/// same base name. Every candidate was looked up with a scan of the
/// whole module, so the cost grew with the square of the cell count and
/// twelve arrays took six minutes.
fn many_small_memories_source(arrays: usize) -> String {
    let mut source = String::from(
        "module many_small_memories (\n\
         \x20   input  wire       clk,\n\
         \x20   input  wire [2:0] slot,\n\
         \x20   input  wire [7:0] value,\n\
         \x20   output reg  [7:0] out\n\
         );\n\
         \x20   integer i;\n",
    );
    for i in 0..arrays {
        source.push_str(&format!("    reg [7:0] m{i} [0:7];\n"));
    }
    source.push_str("    always @(posedge clk) begin\n");
    for i in 0..arrays {
        source.push_str(&format!("        m{i}[slot] <= value + 8'd{i};\n"));
    }
    source.push_str("    end\n    always @* begin\n        out = 8'd0;\n");
    source.push_str("        for (i = 0; i < 8; i = i + 1) begin\n");
    for i in 0..arrays {
        source.push_str(&format!("            out = out ^ m{i}[i];\n"));
    }
    source.push_str("        end\n    end\nendmodule\n");
    source
}

/// Lowering that shape must not cost the square of the cell count.
///
/// This asks about the *shape* of the cost, not its size, because a wall
/// clock says as much about the machine as about the code. The first
/// version of this test allowed sixty seconds, which is five times what
/// the shape costs here and less than a loaded Windows CI runner needs:
/// it went red on Windows while the code was correct.
///
/// So the run is timed twice, at four arrays and at sixteen, in the same
/// process on the same machine, and the two are compared to each other.
/// Four times the arrays is four times the cells, so lowering that is
/// linear in the cells takes about four times as long and the quadratic
/// version took about sixteen. Measured here the ratio is 5.0 rather
/// than 4.0, because the pass is not perfectly linear and the part of
/// the flow that does not scale is small; eight sits between 5 and 16
/// with room on both sides, and unlike a number of seconds it means the
/// same thing on every machine.
#[test]
fn lowering_many_small_memories_is_not_quadratic() {
    let run = |arrays: usize| {
        let source = many_small_memories_source(arrays);
        let start = std::time::Instant::now();
        let (_, report) = flow_of("many_small_memories", &source);
        let took = start.elapsed();
        assert!(
            report
                .netlist
                .iter()
                .all(|(cell, _)| !cell.starts_with('$')),
            "the flow left a generic cell with {arrays} arrays: {:?}",
            report.netlist
        );
        took
    };

    let small = run(4);
    let big = run(16);
    let ratio = big.as_secs_f64() / small.as_secs_f64().max(f64::MIN_POSITIVE);
    assert!(
        ratio < 8.0,
        "four times the arrays multiplied the time by {ratio:.1} \
         ({small:?} for four, {big:?} for sixteen), which is the quadratic \
         lowering coming back rather than a slow machine"
    );
}

// ---------------------------------------------------------------------------
// The screen
//
// The console's own video port is proved pixel by pixel above. What is
// left is the other half of the chain: the frame buffer, the doubling
// onto a 640 x 480 raster, the palette, and the transmitter that turns
// three bytes into three ten-bit symbols. This test takes the whole of
// `nes_top` and reads its four output pins.
// ---------------------------------------------------------------------------

/// A ten-bit TMDS symbol decoded back into the byte it carries, or
/// `None` when it is one of the four control symbols the link sends
/// during blanking.
///
/// This is the inverse of what DVI 1.0 section 3.2.2 specifies, written
/// from the specification rather than from `ip/dvi_tx`: bit 9 says the
/// low eight bits were inverted, bit 8 says they were combined with XOR
/// rather than XNOR, and bit 0 of the data is bit 0 of the result
/// unchanged. `tests/ip_library.rs` is where the encoder itself is
/// checked against the specification for all 256 bytes in every
/// disparity; here it only has to be undone.
fn tmds_decode(symbol: u16) -> Option<u8> {
    match symbol {
        0x354 | 0x0AB | 0x154 | 0x2AB => return None,
        _ => {}
    }
    let inverted = symbol & 0x200 != 0;
    let xor = symbol & 0x100 != 0;
    let q = u8::try_from(symbol & 0xFF).expect("a byte") ^ if inverted { 0xFF } else { 0x00 };
    let mut out = q & 1;
    for i in 1..8u8 {
        let a = (q >> i) & 1;
        let b = (q >> (i - 1)) & 1;
        let bit = if xor { a ^ b } else { (a ^ b) ^ 1 };
        out |= bit << i;
    }
    Some(out)
}

/// Eight palette entries and the colours `ip/ppu2c02`'s table gives
/// them, read off that table by hand. The test paints the frame buffer
/// with these and no others, so it needs no second copy of all 64.
const KNOWN: [(u8, [u8; 3]); 8] = [
    (0x00, [0x54, 0x54, 0x54]), // the dark grey the table starts with
    (0x0F, [0x00, 0x00, 0x00]), // $xF is black on every real part
    (0x20, [0xEC, 0xEE, 0xEC]), // and $20 is white
    (0x12, [0x30, 0x32, 0xEC]), // blue
    (0x2A, [0x4C, 0xD0, 0x20]), // green
    (0x30, [0xEC, 0xEE, 0xEC]), // white again, from the pale row
    (0x16, [0x98, 0x22, 0x20]), // red
    (0x3D, [0xA0, 0xA2, 0xA0]), // the pale row's grey
];

#[test]
fn the_screen_doubles_the_console_onto_the_tmds_lanes() {
    let Some(dir) = example() else { return };
    let design = console_design(&dir);
    let mut run = Run::new(&design, "nes_top", sim_options(&dir));
    let clk = run.net("nes_top.clk_x5");
    let lanes = [
        run.net("nes_top.tmds_d0"),
        run.net("nes_top.tmds_d1"),
        run.net("nes_top.tmds_d2"),
        run.net("nes_top.tmds_clk"),
    ];

    // The frame buffer's top row, painted with the eight colours above.
    // The console is running too and will overwrite this row as its own
    // first line comes out, but it writes one pixel every 24 cycles of
    // `clk_x5` and the screen reads one every ten, so it never catches
    // up with the part of the row this test looks at.
    let fb = run
        .sim
        .memory("nes_top.u_video.fb")
        .expect("the frame buffer");
    for x in 0..256u64 {
        let colour = KNOWN[usize::try_from(x).expect("a column") % KNOWN.len()].0;
        assert!(
            run.sim
                .set_mem(fb, x, reticle::logic::Logic::from_u64(u64::from(colour), 6))
        );
    }

    // One line of 800 pixels is 4000 cycles; a few hundred more reach
    // into the blanking after it, which is what the alignment below
    // needs.
    const CYCLES: usize = 4300;
    let mut stream = [const { Vec::<u8>::new() }; 4];
    for _ in 0..CYCLES {
        tick(&mut run, clk);
        for (lane, bits) in lanes.iter().zip(stream.iter_mut()) {
            let pair = run.sim.get(*lane).to_u64().unwrap_or(0);
            bits.push(u8::try_from(pair & 1).expect("a bit"));
            bits.push(u8::try_from((pair >> 1) & 1).expect("a bit"));
        }
    }

    // Where the symbols start. The clock channel carries the pixel clock
    // as a symbol of five ones and then five zeros, sent bit 0 first, so
    // the offset at which it reads as five ones followed by five zeros
    // is the offset the other three channels' symbols start on.
    let symbol_at = |bits: &[u8], at: usize| -> u16 {
        (0..10).fold(0u16, |acc, i| acc | (u16::from(bits[at + i]) << i))
    };
    // The search starts 200 cycles in, past the power-on reset that
    // holds the serialisers at zero for the first few edges; 400 is a
    // multiple of ten, so the offset it finds there is the offset from
    // the beginning as well.
    let offset = (0..10)
        .find(|offset| symbol_at(&stream[3], 400 + *offset) == 0b00000_11111)
        .expect("the clock channel is not five ones and five zeros");
    assert!(
        (offset + 400..stream[3].len() - 10)
            .step_by(10)
            .all(|at| symbol_at(&stream[3], at) == 0b00000_11111),
        "the clock channel does not stay on its symbol"
    );

    // Every symbol of the three data channels, decoded. A run of bytes
    // is the visible part of a line and a run of `None` is the blanking
    // between them.
    let decoded: Vec<[Option<u8>; 3]> = (offset..stream[0].len() - 10)
        .step_by(10)
        .map(|at| {
            [
                tmds_decode(symbol_at(&stream[0], at)),
                tmds_decode(symbol_at(&stream[1], at)),
                tmds_decode(symbol_at(&stream[2], at)),
            ]
        })
        .collect();

    // The first run of pixels is line 0, and its last pixel is column
    // 639: counting back from the blanking that follows is what puts a
    // column number on each symbol without having to know how many
    // cycles the transmitter's own pipeline is.
    let end = decoded
        .iter()
        .position(|c| c[0].is_none())
        .expect("the line never ends");
    let start = end - 640;
    assert!(
        decoded[start..end].iter().all(|c| c[0].is_some()),
        "the visible part of the line is not 640 pixels of data"
    );

    // Two screen pixels per console pixel, 64 columns of border either
    // side. Only the columns the console has not caught up with yet are
    // compared, which is everything past its own 40th pixel.
    let mut checked = 0usize;
    for column in 0..640usize {
        let cell = decoded[start + column];
        let (blue, green, red) = (
            cell[0].expect("a byte"),
            cell[1].expect("a byte"),
            cell[2].expect("a byte"),
        );
        if !(64..576).contains(&column) {
            assert_eq!(
                [red, green, blue],
                [0, 0, 0],
                "column {column} is outside the picture and is not black"
            );
            continue;
        }
        let nes_x = (column - 64) / 2;
        if nes_x < 40 {
            continue;
        }
        let (_, want) = KNOWN[nes_x % KNOWN.len()];
        assert_eq!(
            [red, green, blue],
            want,
            "column {column} is console pixel {nes_x}, palette entry {:#04x}",
            KNOWN[nes_x % KNOWN.len()].0
        );
        checked += 1;
    }
    assert!(checked > 400, "only {checked} columns were compared");
}

// ---------------------------------------------------------------------------
// The other screen: the VGA pins of a Digilent Basys 3
//
// `the_screen_doubles_the_console_onto_the_tmds_lanes` above reads a
// painted frame buffer off four TMDS pins, and
// `the_frame_comes_out_of_the_video_port` reads a real frame off the
// console's video port. This section does both jobs at once, on the
// board this repository's owner can actually buy: the demo's own frame,
// through the frame buffer, the doubling, the palette and `vga_out`,
// read back off the twelve colour pins and the two sync pins of a VGA
// socket.
//
// It can be one simulation where the DVI path could not, and the reason
// is arithmetic: `dvi_tx` runs at five times the pixel rate, so one
// 640 x 480 frame through it is 2.1 million cycles, while VGA needs no
// faster clock at all and `tb/nes_vga_tb.v` takes one pixel per clock.
// ---------------------------------------------------------------------------

/// The 2C02's sixty-four colours as `ip/ppu2c02`'s `ppu_palette` gives
/// them, transcribed here and laid out the way that table is: four rows
/// of sixteen, hue across and level down.
///
/// This is the **reference**, not a reading of the design: the test
/// below works out what each pin has to carry from this array, so a
/// colour that came out of the hardware is never compared with itself.
/// `KNOWN` above is eight of these read off the same table
/// independently, and `the_palette_is_the_one_the_tmds_test_reads_by_hand`
/// holds the two transcriptions together.
///
/// $xE and $xF are black on every real part, and `ppu_palette` gives
/// $xD of the two dark rows black as well rather than something below
/// where the signal is supposed to go; the blacks down the right-hand
/// side of the table are the part and not a gap in it.
const PALETTE: [[u8; 3]; 64] = [
    // Level $0x — the dark row.
    [0x54, 0x54, 0x54], // $00
    [0x00, 0x1E, 0x74], // $01
    [0x08, 0x10, 0x90], // $02
    [0x30, 0x00, 0x88], // $03
    [0x44, 0x00, 0x64], // $04
    [0x5C, 0x00, 0x30], // $05
    [0x54, 0x04, 0x00], // $06
    [0x3C, 0x18, 0x00], // $07
    [0x20, 0x2A, 0x00], // $08
    [0x08, 0x3A, 0x00], // $09
    [0x00, 0x40, 0x00], // $0A
    [0x00, 0x3C, 0x00], // $0B
    [0x00, 0x32, 0x3C], // $0C
    [0x00, 0x00, 0x00], // $0D
    [0x00, 0x00, 0x00], // $0E
    [0x00, 0x00, 0x00], // $0F
    // Level $1x.
    [0x98, 0x96, 0x98], // $10
    [0x08, 0x4C, 0xC4], // $11
    [0x30, 0x32, 0xEC], // $12
    [0x5C, 0x1E, 0xE4], // $13
    [0x88, 0x14, 0xB0], // $14
    [0xA0, 0x14, 0x64], // $15
    [0x98, 0x22, 0x20], // $16
    [0x78, 0x3C, 0x00], // $17
    [0x54, 0x5A, 0x00], // $18
    [0x28, 0x72, 0x00], // $19
    [0x08, 0x7C, 0x00], // $1A
    [0x00, 0x76, 0x28], // $1B
    [0x00, 0x66, 0x78], // $1C
    [0x00, 0x00, 0x00], // $1D
    [0x00, 0x00, 0x00], // $1E
    [0x00, 0x00, 0x00], // $1F
    // Level $2x — the bright row most graphics live in.
    [0xEC, 0xEE, 0xEC], // $20
    [0x4C, 0x9A, 0xEC], // $21
    [0x78, 0x7C, 0xEC], // $22
    [0xB0, 0x62, 0xEC], // $23
    [0xE4, 0x54, 0xEC], // $24
    [0xEC, 0x58, 0xB4], // $25
    [0xEC, 0x6A, 0x64], // $26
    [0xD4, 0x88, 0x20], // $27
    [0xA0, 0xAA, 0x00], // $28
    [0x74, 0xC4, 0x00], // $29
    [0x4C, 0xD0, 0x20], // $2A
    [0x38, 0xCC, 0x6C], // $2B
    [0x38, 0xB4, 0xCC], // $2C
    [0x3C, 0x3C, 0x3C], // $2D
    [0x00, 0x00, 0x00], // $2E
    [0x00, 0x00, 0x00], // $2F
    // Level $3x — the pale row.
    [0xEC, 0xEE, 0xEC], // $30
    [0xA8, 0xCC, 0xEC], // $31
    [0xBC, 0xBC, 0xEC], // $32
    [0xD4, 0xB2, 0xEC], // $33
    [0xEC, 0xAE, 0xEC], // $34
    [0xEC, 0xAE, 0xD4], // $35
    [0xEC, 0xB4, 0xB0], // $36
    [0xE4, 0xC4, 0x90], // $37
    [0xCC, 0xD2, 0x78], // $38
    [0xB4, 0xDE, 0x78], // $39
    [0xA8, 0xE2, 0x90], // $3A
    [0x98, 0xE2, 0xB4], // $3B
    [0xA0, 0xD6, 0xE4], // $3C
    [0xA0, 0xA2, 0xA0], // $3D
    [0x00, 0x00, 0x00], // $3E
    [0x00, 0x00, 0x00], // $3F
];

/// A colour as a Basys 3 renders it: the top four bits of each channel,
/// in the order `tb/nes_vga_tb.v`'s `vga_rgb` puts the twelve pins.
///
/// `vga_out` truncates rather than rounds — `ip/vga_out/README.md` says
/// why — so this is a shift and nothing else. It is the whole of what
/// makes the board's picture different from the ECP5's, and the
/// difference is worth a number:
/// `four_bits_a_channel_merges_one_pair_of_the_palette` has it.
fn basys3_colour(rgb: [u8; 3]) -> u32 {
    (u32::from(rgb[0] >> 4) << 8) | (u32::from(rgb[1] >> 4) << 4) | u32::from(rgb[2] >> 4)
}

#[test]
fn the_palette_is_the_one_the_tmds_test_reads_by_hand() {
    for (index, colour) in KNOWN {
        assert_eq!(
            PALETTE[usize::from(index)],
            colour,
            "the two transcriptions of palette entry {index:#04x} disagree"
        );
    }
}

#[test]
fn four_bits_a_channel_merges_one_pair_of_the_palette() {
    // What the board costs the picture, as a number rather than a
    // warning. Sixty-four entries are not sixty-four colours even at
    // full depth — ten of them are black and $20 and $30 are the same
    // white — and truncating to four bits a channel merges exactly one
    // more pair.
    let distinct = |depth: fn([u8; 3]) -> u32| -> BTreeMap<u32, Vec<usize>> {
        let mut out: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (index, colour) in PALETTE.iter().enumerate() {
            out.entry(depth(*colour)).or_default().push(index);
        }
        out
    };
    let full = distinct(|c| (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]));
    let board = distinct(basys3_colour);
    assert_eq!(full.len(), 54, "the full-depth palette is 54 colours");
    assert_eq!(board.len(), 53, "the Basys 3 palette is 53 colours");

    // And which pair, so that README.md can name it: every two entries
    // the board cannot tell apart but the DVI path can.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for entries in board.values() {
        for (i, a) in entries.iter().enumerate() {
            for b in &entries[i + 1..] {
                if PALETTE[*a] != PALETTE[*b] {
                    merged.push((*a, *b));
                }
            }
        }
    }
    assert_eq!(
        merged,
        [(0x09, 0x0B)],
        "some other pair of the palette collides at four bits a channel"
    );
    assert_eq!(PALETTE[0x09], [0x08, 0x3A, 0x00]);
    assert_eq!(PALETTE[0x0B], [0x00, 0x3C, 0x00]);
    assert_eq!(basys3_colour(PALETTE[0x09]), 0x030);
}

/// The VGA testbench's clock: half a period in nanoseconds, and one
/// pixel per period.
const VGA_HALF_NS: u64 = 20;

/// Where the doubled picture sits on the 640 x 480 raster: 256 columns
/// doubled is 512, and centring that leaves 64 either side. This is the
/// arithmetic and not a reading of `rtl/nes_video.v`.
const BAND_LEFT: usize = (640 - 2 * SCREEN_W) / 2;

#[test]
fn the_frame_comes_out_of_the_vga_pins() {
    let Some(dir) = example() else { return };
    let bench = read(&dir, "tb/nes_vga_tb.v");
    assert!(
        bench.contains(&format!("localparam HALF    = {VGA_HALF_NS};")),
        "the testbench's clock is not the one this test decodes"
    );
    // Four bits a channel, which is the Basys 3's resistor ladder.
    assert!(
        bench.contains(".BPC       (4)"),
        "the testbench is not 4 bpc"
    );

    let design = testbench_design(&dir, "tb/nes_vga_tb.v", "nes_vga_tb");
    let mut sim = Simulator::new(&design, sim_options(&dir)).expect("the testbench simulates");

    // Everything sampled here is a **pin**: the twelve colour bits and
    // the two syncs that leave `vga_out`, and `de`, which is the raster
    // the picture is measured against. Nothing inside the console, the
    // frame buffer or the palette is looked at.
    let de = sim.net("nes_vga_tb.de").expect("the testbench has de");
    let rgb = sim
        .net("nes_vga_tb.vga_rgb")
        .expect("the testbench has vga_rgb");
    let hsync = sim
        .net("nes_vga_tb.vga_hsync")
        .expect("the testbench has vga_hsync");
    let vsync = sim
        .net("nes_vga_tb.vga_vsync")
        .expect("the testbench has vga_vsync");

    let watch = |sim: &mut Simulator<'_>, net| -> Rc<RefCell<video::Levels>> {
        let wave: Rc<RefCell<video::Levels>> = Rc::default();
        let sink = Rc::clone(&wave);
        sim.on_change(net, move |time, value| {
            sink.borrow_mut()
                .push((time, value.to_u64().map(|v| v == 1)));
        });
        wave
    };
    let de_wave = watch(&mut sim, de);
    let hsync_wave = watch(&mut sim, hsync);
    let vsync_wave = watch(&mut sim, vsync);
    let rgb_wave: Rc<RefCell<video::Colours>> = Rc::default();
    let sink = Rc::clone(&rgb_wave);
    sim.on_change(rgb, move |time, value| {
        let colour = value
            .to_u64()
            .map(|v| u32::try_from(v).expect("twelve bits"));
        sink.borrow_mut().push((time, colour));
    });

    // The nametable the program wrote. It is the one input to the model
    // that has to come from the run; the tiles and the program are read
    // from the sources below.
    let nt_handle = sim
        .memory("nes_vga_tb.u_console.nt")
        .expect("the console has nametable memory");

    sim.run();
    assert!(sim.finished(), "the testbench did not reach $finish");
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert!(messages.is_empty(), "simulator messages: {messages:?}");
    // The testbench says when it froze the frame buffer and when the
    // raster had drawn a whole frame; reaching the timeout instead says
    // so in the same place.
    let said = sim.output().to_owned();
    print!("{said}");
    assert!(
        said.contains("frame 0 recorded"),
        "the testbench did not record a frame:\n{said}"
    );

    let nt: Vec<u8> = (0..2048)
        .map(|i| {
            let value = sim
                .get_mem(nt_handle, i)
                .and_then(|v| v.to_u64())
                .unwrap_or_else(|| panic!("nametable byte {i} was never written"));
            u8::try_from(value).expect("a byte")
        })
        .collect();

    let period = sim.ticks(Delay::new(2 * VGA_HALF_NS, TimeUnit::Ns));
    let screen = video::Video::new(
        &de_wave.borrow(),
        &rgb_wave.borrow(),
        period,
        sim.time(),
        video::Signal::VGA4_BUFFERED,
    );

    // The two sync pins, on every one of the 525 lines. 640 x 480 at 60
    // Hz has negative syncs, so the level a pulse has is low.
    screen.syncs_are_the_rasters(0, &hsync_wave.borrow(), &vsync_wave.borrow(), false);

    // What the console drew, worked out from the nametable it wrote, the
    // tiles the cartridge holds and the palette the program sent — the
    // same model `the_frame_comes_out_of_the_video_port` uses, and the
    // same hash between the two, so nothing about the picture is stated
    // twice.
    let chr = chr_bytes(&dir);
    let program = assemble(&dir, "sw/demo.s");
    let (mut palette, sprites) = demo_tables(&program);
    palette[2] = demo_star_colour(FRAME);
    let indices = expected_frame(FRAME, &nt, &chr, &palette, &demo_oam(&sprites, FRAME));
    assert_eq!(
        frame_hash(&indices),
        FRAME_HASH,
        "the testbench did not freeze the frame the other tests are about"
    );

    // ...and what that has to look like at a socket: every console pixel
    // twice across and twice down, centred, everything around it black,
    // and every colour truncated to the four bits a channel the board's
    // ladder has. The truncation is done here, on the reference palette;
    // nothing in `want` has been through the design.
    let width = usize::try_from(video::H_ACTIVE).expect("a small raster");
    let height = usize::try_from(video::V_ACTIVE).expect("a small raster");
    let want: Vec<u32> = (0..width * height)
        .map(|at| {
            let (x, y) = (at % width, at / width);
            if !(BAND_LEFT..BAND_LEFT + 2 * SCREEN_W).contains(&x) {
                return 0;
            }
            let index = indices[(y / 2) * SCREEN_W + (x - BAND_LEFT) / 2];
            basys3_colour(PALETTE[usize::from(index)])
        })
        .collect();

    // A tripwire against the whole comparison passing for a boring
    // reason: this frame really is a picture, in several colours, and a
    // one-pixel shift of it would put 8720 pixels wrong.
    let mut colours = want.clone();
    colours.sort_unstable();
    colours.dedup();
    println!(
        "the frame is {} colours at four bits a channel",
        colours.len()
    );
    assert!(
        colours.len() >= 6,
        "the frame is {} colour(s), which is not a picture",
        colours.len()
    );

    let got = screen.visible(0);
    assert_eq!(got.len(), want.len(), "the frame is not 640 x 480");
    if let Some(at) = got
        .iter()
        .zip(want.iter())
        .position(|(a, b)| *a != Some(*b))
    {
        let (x, y) = (at % width, at / width);
        let wrong = got
            .iter()
            .zip(want.iter())
            .filter(|(a, b)| **a != Some(**b))
            .count();
        panic!(
            "the picture at the VGA pins is not the one the nametable, the tiles and \
             the truncated palette describe.\n{wrong} of {} pixels differ; the first \
             is ({x}, {y}), where the pins said {:?} and the documentation says \
             {:#05x}\n\nwhat the console drew:\n{}",
            got.len(),
            got[at],
            want[at],
            sketch(&indices)
        );
    }
}

#[test]
fn the_data_port_reads_one_access_behind_and_steps_by_what_2000_says() {
    let Some(dir) = example() else { return };
    let design = ppu_design(&dir);
    let mut ppu = Ppu::boot(&design);

    let v = |ppu: &mut Ppu<'_>| {
        ppu.step();
        ppu.bus
    };

    // A byte into nametable memory through $2007.
    ppu.read(2);
    ppu.write(6, 0x20);
    ppu.write(6, 0x00);
    ppu.write(7, 0x5A);
    assert_eq!(ppu.mem[0x2000], 0x5A, "the write did not reach the bus");
    assert_eq!(v(&mut ppu), 0x2001, "and `v` stepped by one");

    // Reading it back takes two goes. The part answers a read of $2007
    // with the byte it fetched for the *previous* one and only then
    // starts the fetch, so a program that wants the byte at an address
    // reads twice and throws the first away.
    ppu.read(2);
    ppu.write(6, 0x20);
    ppu.write(6, 0x00);
    let first = ppu.read(7);
    ppu.step();
    let second = ppu.read(7);
    assert_eq!(first, 0x00, "the first read is the buffer, which was empty");
    assert_eq!(second, 0x5A, "the second read is the byte");

    // The palette is not on that bus and is not buffered: a read of it
    // answers straight away.
    ppu.read(2);
    ppu.write(6, 0x3F);
    ppu.write(6, 0x01);
    ppu.write(7, 0x11);
    ppu.read(2);
    ppu.write(6, 0x3F);
    ppu.write(6, 0x01);
    assert_eq!(ppu.read(7), 0x11, "a palette read is answered at once");

    // $3F10 is not an entry of its own: it is $3F00, which is why a
    // program writing all 32 bytes from $3F00 overwrites four of them on
    // the way past.
    ppu.read(2);
    ppu.write(6, 0x3F);
    ppu.write(6, 0x10);
    ppu.write(7, 0x29);
    ppu.read(2);
    ppu.write(6, 0x3F);
    ppu.write(6, 0x00);
    assert_eq!(ppu.read(7), 0x29, "$3F10 is another way of writing $3F00");

    // $2000 bit 2 makes a $2007 access step down a row of the nametable
    // rather than along it, which is how a program writes a column.
    ppu.write(0, 0x04);
    ppu.read(2);
    ppu.write(6, 0x21);
    ppu.write(6, 0x00);
    ppu.write(7, 0x01);
    assert_eq!(v(&mut ppu), 0x2120, "the step is 32 with $2000 bit 2 set");

    // ...and $2000's low two bits are the nametable, which go into `t`
    // and reach `v` through the *second* half of a $2006 pair. Writing
    // $2000 between the two halves is what shows where they go.
    ppu.read(2);
    ppu.write(6, 0x00); // first: t[13:8] = 0, t[14] = 0
    ppu.write(0, 0x02); // nametable 2, which is bit 11 of `t`
    ppu.write(6, 0x55); // second: `v` takes t[14:8] and this byte
    assert_eq!(v(&mut ppu), 0x0855, "$2000 bits 1-0 are bits 11-10 of `t`");
}
