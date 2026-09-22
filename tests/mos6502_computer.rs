//! `examples/mos6502_computer`, driven through the same chain as
//! `examples/soc`.
//!
//! The RISC-V system on chip of `examples/soc` proves phase 8's
//! completion criterion. This is the *second* system built the same way,
//! and it exists because the two are nothing alike above the manifest:
//! one processor is a 32-bit load/store machine with two memory ports and
//! a request/ready handshake on each, the other an 8-bit accumulator
//! machine from 1975 with one bus, one access per cycle, its stack nailed
//! to page one and its reset vector nailed to `$FFFC`. The project
//! manifest, the elaboration, the synthesis, the block RAM mapping and
//! the nextpnr export do not care, which is the point.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `hello_hex_is_the_assembled_source` | the checked-in ROM image is `sw/hello.s` assembled by the opcode matrix in `tests/mos6502_asm`, with the three vectors where the part reads them |
//! | `the_project_resolves_and_elaborates` | the manifest builds through `ip::resolve` and `ip::elaborate`, from exactly two library packages and one user source |
//! | `the_computer_synthesises_without_errors_or_latches` | generic synthesis reports no error, no warning and no latch, and the ROM's initial contents are `sw/hello.hex`, loaded by the design's own `$readmemh` |
//! | `the_line_comes_out_of_the_serial_wire` | the testbench runs and `Hello from Reticle\n` is decoded from the waveform of `uart_tx` |
//! | `the_computer_maps_onto_the_hx8k_and_exports_for_nextpnr` | the iCE40 flow fits it on an HX8K, every cell a device primitive, the program and the vectors inside the ROM's block RAMs, and the JSON and PCF `nextpnr-ice40` reads |
//! | `reticle_build_builds_the_project` | the same through the binary |
//! | `reticle_sim_runs_the_testbench` | and the line comes out of `reticle sim` too |
//! | `reticle_fpga_exports_the_rom_with_the_program_in_it` | and out of `reticle fpga` |
//!
//! What it cannot prove is the board: none is attached here, and
//! Reticle's iCE40 place-and-route uses a synthetic fabric that cannot
//! program a real part (`docs/fpga.md`). The chain is proved up to the
//! files `nextpnr-ice40` reads;
//! `examples/mos6502_computer/README.md` has the commands that take it
//! from there.
//!
//! Two things here are specific to a 6502 and have no counterpart in
//! `tests/soc.rs`.
//!
//! The first is **the vectors**. A RISC-V core is told where to start by
//! a parameter; a 6502 is not told at all — it reads `$FFFC` and goes
//! there, so the top six bytes of the address space have to be ROM and
//! have to be right before anything else can be. `the_vectors_point_at_the
//! _programs_handlers` is not a separate test, it is part of
//! `hello_hex_is_the_assembled_source`, because a hex file whose vectors
//! are wrong is not a program at all.
//!
//! The second is **the hex file's `@` addresses**. The ROM is 2 KiB and
//! the program is a few dozen bytes at the bottom of it plus six bytes at
//! the top, so `sw/hello.hex` uses the `@hex` address specifications
//! IEEE 1364-2005 §17.2.9 defines rather than two thousand lines of
//! padding. Everything downstream handles the holes: the simulator leaves
//! those elements `x`, synthesis leaves them `x` in `Memory::init`, and
//! the iCE40 block RAM mapping writes them as zero into `INIT_*`.
//!
//! No defect in Reticle was found building this example. That is worth
//! saying rather than leaving out: the eight that `examples/soc` found
//! are fixed, and the second system through the same path found nothing
//! new, which is what a fixed defect is supposed to look like.
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
use reticle::ir::{Bit, Cell, CellKind, Delay, Design, ModuleId, TimeUnit};
use reticle::logic::Logic;
use reticle::sim::{MemoryFiles, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run as synth_run};

/// The 6502 assembler built from the documented opcode matrix, shared
/// with `tests/ip_library.rs`.
#[path = "mos6502_asm/mod.rs"]
mod asm;

/// The 8N1 receiver that reads the line off the pin's waveform, shared
/// with `tests/soc.rs`.
#[path = "serial/mod.rs"]
mod serial;

use serial::{Waveform, decode_uart};

/// The line `sw/hello.s` prints.
const HELLO: &str = "Hello from Reticle\n";

/// The testbench's clock: half a period, in nanoseconds.
const HALF_NS: u64 = 42;

/// Clock cycles per UART bit, as `computer_top` and the testbench
/// configure it.
const CLK_DIV: u64 = 104;

/// The part the project targets.
const DEVICE: &str = "ice40-hx8k-ct256";

/// The first address the ROM answers to, which is element 0 of the ROM
/// and therefore offset 0 of `sw/hello.hex`.
const ROM_BASE: u32 = 0xF800;

/// How many bytes the ROM holds, as `rtl/computer_top.v` declares it.
const ROM_BYTES: u32 = 2048;

/// The three vectors, where the part has always had them.
const VEC_NMI: u16 = 0xFFFA;
const VEC_RES: u16 = 0xFFFC;
const VEC_IRQ: u16 = 0xFFFE;

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

/// `examples/mos6502_computer`, when this copy of the crate has it.
fn example() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/mos6502_computer");
    if dir.join("reticle.proj").is_file() {
        Some(dir)
    } else {
        println!("skipping: examples/mos6502_computer is not in this copy of the crate");
        None
    }
}

/// A file of the example, as text.
fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel))
        .unwrap_or_else(|e| panic!("examples/mos6502_computer/{rel}: {e}"))
}

/// The ROM image as `$readmemh` reads it: a header, then runs of bytes,
/// each run introduced by the `@` offset it starts at.
///
/// The program is two runs a couple of kilobytes apart — the code and
/// the string at the bottom of the ROM, the vectors at the very top — so
/// the addresses are what keeps the file to its own length instead of
/// the ROM's.
fn render_hex(image: &BTreeMap<u32, u8>) -> String {
    let mut out = String::from(
        "// hello.hex: sw/hello.s assembled, one byte per line, in the ROM's\n\
         // own numbering: offset 0 is address $F800 and an `@` line moves the\n\
         // next byte to that offset, which is how the vectors reach the top of\n\
         // the ROM without two thousand lines of padding in front of them.\n\
         // Generated by tests/mos6502_computer.rs; do not edit.\n",
    );
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

/// The bytes of `sw/hello.hex`, by ROM offset, read the way `$readmemh`
/// would.
fn rom_image(dir: &Path) -> BTreeMap<u32, u8> {
    let text = read(dir, "sw/hello.hex");
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

/// The design `reticle build` produces: `computer_top` and the library.
fn computer_design(dir: &Path) -> Design {
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
        project.top = Some("computer_tb".to_owned());
    });
    built.elaboration.design.expect("the testbench elaborates")
}

/// The files the design reads, for the simulator and synthesis: the ROM
/// image `computer_top`'s `$readmemh` names, relative to the project.
fn example_files(dir: &Path) -> MemoryFiles {
    let mut files = MemoryFiles::new();
    files.insert("sw/hello.hex", read(dir, "sw/hello.hex"));
    files
}

/// Synthesis options that let `$readmemh` load the ROM.
fn synth_options(dir: &Path) -> SynthOptions {
    SynthOptions {
        files: Some(Rc::new(example_files(dir))),
        ..SynthOptions::default()
    }
}

/// The initial contents of `computer_top`'s ROM, byte by byte, with
/// `None` for an element the hex file did not name.
fn rom_init(design: &Design) -> Option<Vec<Option<u8>>> {
    let module = design
        .modules
        .iter()
        .map(|(_, m)| m)
        // A testbench that overrides a parameter gets a specialised copy,
        // `computer_top$CLK_DIV_104`; there is only ever one.
        .find(|m| m.name.as_str().split('$').next() == Some("computer_top"))
        .expect("computer_top is in the design");
    let rom = module
        .memories
        .iter()
        .map(|(_, m)| m)
        .find(|m| m.name.as_str() == "rom")
        .expect("computer_top has a memory called rom");
    rom.init.as_ref().map(|init| {
        init.iter()
            .map(|w| w.to_u64().map(|v| u8::try_from(v).expect("a byte")))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// The program
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
fn hello_hex_is_the_assembled_source() {
    let Some(dir) = example() else { return };
    let source = read(&dir, "sw/hello.s");
    let image = asm::assemble(&source).unwrap_or_else(|e| panic!("sw/hello.s: {e}"));

    // Everything the program places is inside the ROM, which is the only
    // place a 6502 can find anything before it has run.
    let lowest = u32::from(*image.bytes.keys().next().expect("a program"));
    let highest = u32::from(*image.bytes.keys().next_back().expect("a program"));
    assert_eq!(lowest, ROM_BASE, "the program does not start at the ROM");
    assert_eq!(highest, ROM_BASE + ROM_BYTES - 1, "the last byte is $FFFF");

    // The three vectors are where the part reads them and point at the
    // program's own entry points. This is the whole reset story on a
    // 6502: there is no parameter saying where to start.
    assert_eq!(word_at(&image, VEC_RES), image.label("reset"));
    assert_eq!(word_at(&image, VEC_NMI), image.label("nmi"));
    assert_eq!(word_at(&image, VEC_IRQ), image.label("irq"));
    // The two handlers are the same RTI; `irq` is also where BRK lands.
    assert_eq!(image.label("nmi"), image.label("irq"));

    // The string is where the program says it is, terminated, so a
    // reader of the hex can find it.
    let message = image.label("message");
    let bytes: Vec<u8> = (0..HELLO.len() + 1)
        .map(|i| image.bytes[&(message + u16::try_from(i).expect("a short string"))])
        .collect();
    assert_eq!(&bytes[..HELLO.len()], HELLO.as_bytes());
    assert_eq!(bytes[HELLO.len()], 0, "the string is terminated");

    let offsets: BTreeMap<u32, u8> = image
        .bytes
        .iter()
        .map(|(a, b)| (u32::from(*a) - ROM_BASE, *b))
        .collect();
    let expected = render_hex(&offsets);
    let path = dir.join("sw/hello.hex");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        fs::write(&path, &expected).expect("write sw/hello.hex");
    }
    let actual = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "sw/hello.hex is not sw/hello.s assembled; rerun with UPDATE_EXPECT=1 and commit both"
    );
    // And the hex reads back as what was assembled, `@` addresses and
    // all, which is what the simulator and synthesis will do with it.
    assert_eq!(rom_image(&dir), offsets);
}

#[test]
fn the_assembler_places_a_string_and_its_terminator() {
    // `.string` is the one directive `tests/mos6502_asm` gained for this
    // example, so it is checked apart from the program: the bytes, the
    // escape, the NUL, and a `;` inside a string that is not a comment.
    let image = asm::assemble(".org $1000\n.string \"hi;\\n\"\nnop\n").expect("assembles");
    assert_eq!(image.bytes[&0x1000], b'h');
    assert_eq!(image.bytes[&0x1001], b'i');
    assert_eq!(image.bytes[&0x1002], b';');
    assert_eq!(image.bytes[&0x1003], b'\n');
    assert_eq!(image.bytes[&0x1004], 0);
    // The instruction after it is placed past the terminator, so the two
    // passes agreed about how long the string was.
    assert_eq!(image.bytes[&0x1005], 0xEA, "NOP follows the string");
    let bad = asm::assemble(".string nope")
        .err()
        .expect("an unquoted string is an error");
    assert!(bad.contains("line 1"), "{bad}");
}

// ---------------------------------------------------------------------------
// Building and synthesis
// ---------------------------------------------------------------------------

#[test]
fn the_project_resolves_and_elaborates() {
    let Some(dir) = example() else { return };
    let built = build(&dir, |_| {});
    assert_eq!(built.project.top.as_deref(), Some("computer_top"));
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
            ("mos6502_computer", "rtl/computer_top.v"),
        ],
        "the user's only HDL is rtl/computer_top.v; everything else is the library"
    );
    assert!(built.elaboration.blackboxes.is_empty());
    assert!(built.elaboration.skipped.is_empty());
    let design = built.elaboration.design.expect("a design");
    let top = design.top_module().expect("a top");
    assert_eq!(top.name.as_str(), "computer_top");
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
fn the_computer_synthesises_without_errors_or_latches() {
    let Some(dir) = example() else { return };
    let mut design = computer_design(&dir);
    assert_eq!(rom_init(&design), None, "the ROM starts empty");
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &synth_options(&dir), &mut diags);
    assert!(
        complaints(&diags).is_empty(),
        "synthesis complained:\n  {}",
        complaints(&diags).join("\n  ")
    );

    // `$readmemh` put the program into the ROM, byte for byte, and every
    // element the file did not name is still `x` — which is the whole
    // point of the `@` addresses: nothing invented the padding.
    let init = rom_init(&design).expect("the ROM was loaded");
    let image = rom_image(&dir);
    assert_eq!(
        u32::try_from(init.len()).expect("a small ROM"),
        ROM_BYTES,
        "the last element the file named is the ROM's last"
    );
    for (offset, byte) in init.iter().enumerate() {
        let offset = u32::try_from(offset).expect("a small ROM");
        assert_eq!(
            *byte,
            image.get(&offset).copied(),
            "ROM offset {offset:#05x}"
        );
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
// Simulation
// ---------------------------------------------------------------------------

#[test]
fn the_line_comes_out_of_the_serial_wire() {
    let Some(dir) = example() else { return };
    let bench = read(&dir, "tb/computer_tb.v");
    assert!(bench.contains(&format!("localparam CLK_DIV  = {CLK_DIV};")));
    assert!(bench.contains(&format!("localparam HALF     = {HALF_NS};")));

    let design = testbench_design(&dir);
    let options = SimOptions {
        files: Some(Box::new(example_files(&dir))),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).expect("the testbench simulates");

    // The waveform of the pin, and nothing else: every change of
    // `uart_tx` with the time it happened.
    let tx = sim
        .net("computer_tb.uart_tx")
        .expect("the testbench has uart_tx");
    let wave: Rc<RefCell<Waveform>> = Rc::default();
    let sink = Rc::clone(&wave);
    sim.on_change(tx, move |time, value| {
        let level = value.to_u64().map(|v| v == 1);
        sink.borrow_mut().push((time, level));
    });
    sim.run();
    assert!(sim.finished(), "the testbench did not reach $finish");

    let bit = sim.ticks(Delay::new(2 * HALF_NS * CLK_DIV, TimeUnit::Ns));
    let bytes = decode_uart(&wave.borrow(), bit).unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8(bytes).expect("ASCII from the wire");
    assert_eq!(text, HELLO, "the serial line did not carry the line");

    // The testbench's own receiver agrees, and printed it.
    assert_eq!(sim.output(), HELLO);

    // `$readmemh` loaded the program, and nothing else was said.
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert!(messages.is_empty(), "simulator messages: {messages:?}");
    let rom = sim.memory("computer_tb.dut.rom").expect("the ROM");
    for (offset, byte) in rom_image(&dir) {
        let got = sim.get_mem(rom, u64::from(offset)).and_then(|v| v.to_u64());
        assert_eq!(got, Some(u64::from(byte)), "ROM offset {offset:#05x}");
    }
}

// ---------------------------------------------------------------------------
// The iCE40 flow
// ---------------------------------------------------------------------------

/// The value of initialisation parameter `name` of `cell`, as bits.
fn init_param(cell: &Cell, name: &str) -> Logic {
    match cell.params.get(name) {
        Some(reticle::ir::AttrValue::Const(value)) => value.clone(),
        other => panic!("`{}` has no constant {name}: {other:?}", cell.name),
    }
}

/// The `SB_RAM40_4K` cells built for memory `memory`, in cell order.
fn block_rams_of<'a>(design: &'a Design, top: ModuleId, memory: &str) -> Vec<&'a Cell> {
    design.modules[top]
        .cells
        .iter()
        .map(|(_, c)| c)
        .filter(|c| matches!(&c.kind, CellKind::Blackbox(p) if p.as_str() == "SB_RAM40_4K"))
        .filter(|c| c.attrs.get("memory").and_then(|v| v.as_str()) == Some(memory))
        .collect()
}

/// Byte `offset` of a byte-wide memory spread over `blocks`, read back
/// out of their `INIT_*` parameters.
///
/// `src/fpga/devices/ice40.dev` states the layout, and this is the only
/// place in the test that depends on it: a block is 256 rows of 16 bits,
/// `INIT_<k>` holds rows 16k..16k+15 with row 16k+i in bits 16i+15..16i,
/// and in a mode narrower than sixteen bits the low eight address bits
/// pick the row, the bits above them pick the word `s` inside it, and
/// data bit `j` of that word lives in row bit `(16 / width) * j + s`.
fn sb_ram_byte(blocks: &[&Cell], mode: (u32, u32), offset: u32) -> u8 {
    let (width, depth) = mode;
    let cell = blocks[usize::try_from(offset / depth).expect("a depth slice")];
    let inside = offset % depth;
    let row = inside % 256;
    let word = inside / 256;
    let spacing = 16 / width;
    let init = init_param(cell, &format!("INIT_{:X}", row / 16));
    assert_eq!(init.width(), 256);
    let base = 16 * (row % 16);
    (0..8)
        .filter(|j| init.bit(base + spacing * j + word) == Bit::One)
        .map(|j| 1u8 << j)
        .sum()
}

#[test]
fn the_computer_maps_onto_the_hx8k_and_exports_for_nextpnr() {
    let Some(dir) = example() else { return };
    let mut design = computer_design(&dir);
    let top = design.top.expect("a top");

    let device = fpga::target(DEVICE).expect("the HX8K is a built-in device");
    let mut map = SourceMap::new();
    let rcf = read(&dir, "board/hx8k_breakout.rcf");
    let file = map
        .add("board/hx8k_breakout.rcf", rcf.clone())
        .expect("the constraints fit");
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(&rcf, file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    constraints.check(&design, device, &mut diags);
    // The board's clock and serial pins are all in the device database.
    assert!(diags.is_empty(), "{}", diags.render(&map));

    let mut diags = Diagnostics::new();
    let options = FpgaOptions {
        synth: synth_options(&dir),
        ..FpgaOptions::default()
    };
    let flow = fpga::synthesize_for(&mut design, top, device, &constraints, &options, &mut diags)
        .unwrap_or_else(|e| panic!("the HX8K flow failed: {e:?}"));
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    assert!(errors.is_empty(), "the HX8K flow reported: {errors:?}");

    // The resource use, for the README and for anyone reading the log.
    println!("{DEVICE}:");
    for (cell, count) in &flow.netlist {
        println!("  {count:>5} x {cell}");
    }
    println!("  {} LUTs, depth {}", flow.luts, flow.lut_depth);

    // The flow flattened the core and the UART into computer_top.
    assert!(flow.inlined >= 2, "{} instance(s) inlined", flow.inlined);
    assert!(
        !diags.iter().any(|d| d.code == Some("F0305")),
        "a memory lost its initial contents: {}",
        diags.render(&map)
    );

    let luts = flow.count("SB_LUT4");
    let brams = flow.count("SB_RAM40_4K");
    assert!(luts > 1000, "{luts} LUTs is too few for a processor");
    assert!(luts <= 7680, "{luts} LUTs does not fit the HX8K's 7680");
    assert!(brams >= 1, "nothing went into block RAM");
    assert!(brams <= 32, "{brams} block RAMs does not fit the HX8K's 32");
    assert!(
        flow.netlist.iter().all(|(cell, _)| !cell.starts_with('$')),
        "a generic cell survived: {:?}",
        flow.netlist
    );

    // The ROM's blocks hold the program, byte for byte, and the six
    // bytes at the very top of it are the vectors — which is the part a
    // board would notice first, since the core reads $FFFC before it
    // does anything else at all.
    let rom = flow
        .primitives
        .block_rams
        .iter()
        .find(|b| b.memory == "rom")
        .expect("the ROM went into block RAM");
    println!(
        "  rom -> {} x SB_RAM40_4K in {}x{} mode",
        rom.blocks(),
        rom.mode.0,
        rom.mode.1
    );
    assert!(rom.initialised, "the ROM's blocks carry no contents");
    assert_eq!(rom.copies, 1, "one bus means one read port means one copy");
    assert_eq!(rom.wide, 1, "a byte fits one block's data width");
    let blocks = block_rams_of(&design, top, "rom");
    assert_eq!(
        u32::try_from(blocks.len()).expect("a few blocks"),
        rom.blocks()
    );
    let ordered: Vec<&Cell> = (0..rom.deep)
        .map(|d| {
            let name = format!("rom$ram_w0_d{d}");
            *blocks
                .iter()
                .find(|c| c.name.as_str() == name)
                .unwrap_or_else(|| panic!("no block `{name}`"))
        })
        .collect();
    let image = rom_image(&dir);
    for offset in 0..ROM_BYTES {
        // An element the hex file did not name is zero in the netlist:
        // block RAM has no `x`.
        let want = image.get(&offset).copied().unwrap_or(0);
        assert_eq!(
            sb_ram_byte(&ordered, rom.mode, offset),
            want,
            "ROM offset {offset:#05x} in the netlist"
        );
    }
    for vector in [VEC_NMI, VEC_RES, VEC_IRQ] {
        let offset = u32::from(vector) - ROM_BASE;
        assert_ne!(
            (
                sb_ram_byte(&ordered, rom.mode, offset),
                sb_ram_byte(&ordered, rom.mode, offset + 1)
            ),
            (0, 0),
            "the vector at {vector:#06x} is blank in the netlist"
        );
    }

    // Every cell is a primitive the device has, and nothing is wrong.
    let problems: Vec<String> = fpga::check_nextpnr_json(&design, top, device, &constraints)
        .into_iter()
        .map(|p| format!("{}: {}", p.object, p.message))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    let inputs = fpga::export_nextpnr(&design, top, device, &constraints).expect("exports");
    assert_eq!(inputs.constraints_name, "computer_top.pcf");
    assert_eq!(
        inputs.args.join(" "),
        "nextpnr-ice40 --hx8k --package ct256 --json computer_top.json --pcf computer_top.pcf \
         --asc computer_top.asc"
    );
    for pin in ["set_io clk J3", "set_io uart_tx B12", "set_io uart_rx B10"] {
        assert!(inputs.pcf_or_lpf.contains(pin), "the PCF lacks `{pin}`");
    }
    assert!(inputs.json.contains("\"SB_RAM40_4K\""));
    assert!(inputs.json.contains("\"INIT_0\": \""));

    // The files a board needs, where a reader can pick them up.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mos6502-computer");
    fs::create_dir_all(&out).expect("a scratch directory");
    fs::write(out.join("computer_top.json"), &inputs.json).expect("write the netlist");
    fs::write(out.join(&inputs.constraints_name), &inputs.pcf_or_lpf).expect("write the PCF");
    println!(
        "wrote {}/computer_top.json and computer_top.pcf",
        out.display()
    );
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
const LIBRARY: [&str; 4] = [
    "../../ip/mos6502/rtl/mos6502.v",
    "../../ip/uart/rtl/uart_tx.v",
    "../../ip/uart/rtl/uart_rx.v",
    "../../ip/uart/rtl/uart.v",
];

#[cfg(feature = "cli")]
#[test]
fn reticle_build_builds_the_project() {
    let Some(dir) = example() else { return };
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mos6502-computer-build");
    fs::create_dir_all(&scratch).expect("a scratch directory");
    let lock = scratch.join("reticle.lock");
    let lock_arg = lock.to_string_lossy().into_owned();
    let (code, _, err) = reticle(
        &dir,
        &["build", "--synth", "--lock", &lock_arg, "reticle.proj"],
    );
    assert_eq!(code, 0, "reticle build failed:\n{err}");
    assert!(
        err.contains("note: built `mos6502_computer`: 5 module(s) from 5 source(s)"),
        "{err}"
    );
    // The binary hands synthesis a file provider rooted at the manifest's
    // directory, so `$readmemh` loads the ROM image and there is nothing
    // to say about it.
    assert!(
        !err.contains("could not be loaded"),
        "the ROM image was not loaded:\n{err}"
    );
    assert!(!err.contains("simulation-only statement dropped"), "{err}");
    let lock = fs::read_to_string(&lock).expect("a lock file");
    assert!(lock.contains("mos6502") && lock.contains("uart"), "{lock}");
}

#[cfg(feature = "cli")]
#[test]
fn reticle_sim_runs_the_testbench() {
    let Some(dir) = example() else { return };
    let mut args = vec!["sim", "--quiet", "tb/computer_tb.v", "rtl/computer_top.v"];
    args.extend(LIBRARY);
    let (code, out, err) = reticle(&dir, &args);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(HELLO.trim_end()),
        "the line did not come out of `reticle sim`:\n{out}\n{err}"
    );
}

#[cfg(feature = "cli")]
#[test]
fn reticle_fpga_exports_the_rom_with_the_program_in_it() {
    let Some(dir) = example() else { return };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mos6502-computer-fpga");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("a scratch directory");
    let out_arg = out.to_string_lossy().into_owned();
    let mut args = vec![
        "fpga",
        "--device",
        DEVICE,
        "--constraints",
        "board/hx8k_breakout.rcf",
        "--output-dir",
        &out_arg,
        "--quiet",
        "rtl/computer_top.v",
    ];
    args.extend(LIBRARY);
    let (code, _, err) = reticle(&dir, &args);
    assert_eq!(code, 0, "reticle fpga failed:\n{err}");

    let json = fs::read_to_string(out.join("computer_top.json")).expect("the netlist");
    // The ROM's block RAMs carry `INIT_*` parameters holding the program;
    // a blank ROM would have every one of them all zeroes.
    let rom_inits: Vec<&str> = json
        .split("\"INIT_")
        .skip(1)
        .filter_map(|rest| rest.split('"').nth(2))
        .collect();
    assert!(
        !rom_inits.is_empty(),
        "no block RAM carries INIT parameters"
    );
    assert!(
        rom_inits.iter().any(|v| v.chars().any(|c| c != '0')),
        "every INIT parameter is zero: the ROM was exported blank"
    );
}
