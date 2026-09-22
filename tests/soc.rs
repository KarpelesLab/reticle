//! Phase 8's completion criterion, proved on `examples/soc`.
//!
//! ROADMAP.md closes phase 8 with: "a project manifest pulling in a UART
//! and a RISC-V core from the library builds, simulates its testbench, and
//! runs on an iCE40 board with no HDL written by the user beyond a
//! top-level". `examples/soc` is that project — `rv32i` and `uart` from
//! `ip/`, one top-level of the user's (`rtl/soc_top.v`), a program that
//! prints a line over the serial port (`sw/hello.s`, assembled into
//! `sw/hello.hex`) and a testbench — and this file drives it through
//! everything the criterion names:
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `hello_hex_is_the_assembled_source` | the checked-in ROM image is `sw/hello.s` assembled by the shared encoders in `tests/rv32i_asm` |
//! | `the_assembler_matches_hand_encoded_instructions` | the text front end of that assembler, against encodings worked by hand |
//! | `the_project_resolves_and_elaborates` | the manifest builds through `ip::resolve` and `ip::elaborate`, from exactly two library packages and one user source |
//! | `the_soc_synthesises_without_errors_or_latches` | generic synthesis reports no error, no warning and no latch |
//! | `the_line_comes_out_of_the_serial_wire` | the testbench runs, and the exact line is decoded from the waveform of `uart_tx` |
//! | `the_soc_maps_onto_the_hx8k_and_exports_for_nextpnr` | the iCE40 flow fits it on an HX8K, every cell a device primitive, and writes the JSON and PCF `nextpnr-ice40` reads |
//! | `reticle_build_builds_the_project` | the same through the binary |
//! | `reticle_sim_cannot_run_the_testbench_yet` | what the binary does with the testbench today, pinned |
//!
//! What it cannot prove is the board itself: none is attached here, and
//! the iCE40 place-and-route in this repository uses a synthetic fabric
//! that cannot program a real part (`docs/fpga.md`). The chain is proved
//! up to the JSON and PCF that `nextpnr-ice40` reads, and even those would
//! not yet print anything on a board, for the reasons below;
//! `examples/soc/README.md` has the commands that take it from there.
//!
//! # Gaps this found
//!
//! Building the example exposed eight defects in Reticle, each reproduced
//! in a few lines by a test at the end of this file that asserts the gap
//! is *still there*, so fixing one fails its test and points here:
//!
//! - `a_system_task_without_parentheses_is_dropped`: `$finish;` is
//!   lowered as an expression and silently discarded; only `$finish(0);`
//!   stops a simulation. The testbench uses `$finish(0)` for that reason.
//! - `readmemh_in_verilog_never_reaches_the_simulator`: the Verilog
//!   lowering passes `$readmemh`'s memory as a string naming it, and the
//!   simulator only accepts a memory read (`@mem[0]`), so every
//!   `$readmemh` in Verilog fails with "needs a memory". `reticle sim`
//!   also gives the simulator no file provider, so it could not read the
//!   file even then.
//! - `synthesis_drops_the_contents_readmemh_loads`: synthesis drops the
//!   call with a note, so the ROM of a netlist is empty.
//! - `displaying_a_memory_word_prints_the_memory_name`: `$display("%h",
//!   mem[1])` prints the memory's name rather than the word.
//! - `the_hx8k_database_lacks_the_breakout_boards_uart_pins`: the HX8K's
//!   device database lists only the breakout board's LEDs and clock, so
//!   the constraints for its serial port are refused.
//! - `a_rom_with_two_read_ports_is_not_duplicated_across_block_rams`: a
//!   read-only memory read from both buses stays a generic cell on
//!   iCE40, where a written one is duplicated per read port.
//! - `block_ram_loses_the_contents_it_is_initialised_with`: a memory
//!   mapped to SB_RAM40_4K gets no INIT_* parameters, so the ROM in the
//!   exported netlist is blank even when the IR holds the program.
//! - `reticle_fpga_does_not_flatten_a_design_with_instances`: the
//!   binary's FPGA flow maps the top module alone, so a design with any
//!   instance is refused; the flow here flattens first, as the library
//!   documents.
//!
//! Until the second and third are fixed, [`preload_rom`] puts the program
//! into the ROM's initial contents in the IR, which the simulator and
//! generic synthesis honour. It is the one step here that a user of the
//! binary cannot take, and it stands exactly where `$readmemh` should have
//! done the work: the ROM holds `sw/hello.hex`, word for word, before the
//! first clock edge. The seventh defect then drops those contents again
//! on the way into block RAM, which is the main reason the exported
//! netlist would not yet print anything on a board.
//!
//! `examples/` is not in the published crate, so every test that needs the
//! example skips with a message when it is absent.

#![cfg(all(
    feature = "ip",
    feature = "verilog",
    feature = "sim",
    feature = "synth",
    feature = "fpga"
))]

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use reticle::diag::{Diagnostics, Severity};
use reticle::fpga::{self, Constraints, FpgaOptions};
use reticle::ip::{self, Elaboration, PathProvider, Project, SourceEntry};
use reticle::ir::hier::FlattenOptions;
use reticle::ir::{CellKind, Delay, Design, TimeUnit};
use reticle::logic::Logic;
use reticle::sim::{MemoryFiles, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run as synth_run};
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

#[path = "rv32i_asm/mod.rs"]
mod asm;

/// The line `sw/hello.s` prints.
const HELLO: &str = "Hello from Reticle\n";

/// The testbench's clock: half a period, in nanoseconds.
const HALF_NS: u64 = 42;

/// Clock cycles per UART bit, as `soc_top` and the testbench configure it.
const CLK_DIV: u64 = 104;

/// The part the project targets.
const DEVICE: &str = "ice40-hx8k-ct256";

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

/// `examples/soc`, when this copy of the crate has it.
fn example() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/soc");
    if dir.join("reticle.proj").is_file() {
        Some(dir)
    } else {
        println!("skipping: examples/soc is not in this copy of the crate");
        None
    }
}

/// A file of the example, as text.
fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("examples/soc/{rel}: {e}"))
}

/// The ROM image as `$readmemh` reads it: a header, then one word per
/// line.
fn render_hex(words: &[u32]) -> String {
    let mut out = String::from(
        "// hello.hex: sw/hello.s assembled, one little-endian word per line,\n\
         // the first at address 0. Generated by tests/soc.rs; do not edit.\n",
    );
    for word in words {
        out.push_str(&format!("{word:08x}\n"));
    }
    out
}

/// The words of `sw/hello.hex`, read the way `$readmemh` would.
fn rom_image(dir: &Path) -> Vec<u32> {
    read(dir, "sw/hello.hex")
        .lines()
        .map(|line| line.split("//").next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(|word| u32::from_str_radix(word, 16).expect("a hex word"))
        .collect()
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

/// The design `reticle build` produces: `soc_top` and the library.
fn soc_design(dir: &Path) -> Design {
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
        project.top = Some("soc_tb".to_owned());
    });
    built.elaboration.design.expect("the testbench elaborates")
}

/// Puts the program into `soc_top.rom`'s initial contents.
///
/// This stands in for `$readmemh`, which Reticle does not yet honour from
/// Verilog in either the simulator or synthesis (see the module docs and
/// the tests at the end). The initial contents are what both of them
/// read. With any other synthesis tool they would end up in the block
/// RAMs' `INIT_*` parameters; Reticle's block RAM mapper does not write
/// those yet (`block_ram_loses_the_contents_it_is_initialised_with`).
fn preload_rom(design: &mut Design, words: &[u32]) {
    let module = design
        .modules
        .iter_mut()
        .map(|(_, m)| m)
        // A testbench that overrides a parameter gets a specialised copy,
        // `soc_top$CLK_DIV_104`; there is only ever one.
        .find(|m| m.name.as_str().split('$').next() == Some("soc_top"))
        .expect("soc_top is in the design");
    let rom = module
        .memories
        .iter_mut()
        .map(|(_, m)| m)
        .find(|m| m.name.as_str() == "rom")
        .expect("soc_top has a memory called rom");
    let size = usize::try_from(rom.size).expect("a small ROM");
    assert!(words.len() <= size, "the program does not fit the ROM");
    let mut init: Vec<_> = words
        .iter()
        .map(|w| Logic::from_u64(u64::from(*w), 32))
        .collect();
    init.resize(size, Logic::from_u64(0, 32));
    rom.init = Some(init);
}

// ---------------------------------------------------------------------------
// The program
// ---------------------------------------------------------------------------

#[test]
fn hello_hex_is_the_assembled_source() {
    let Some(dir) = example() else { return };
    let source = read(&dir, "sw/hello.s");
    let words = asm::assemble(&source).unwrap_or_else(|e| panic!("sw/hello.s: {e}"));
    let expected = render_hex(&words);
    let path = dir.join("sw/hello.hex");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        fs::write(&path, &expected).expect("write sw/hello.hex");
    }
    let actual = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "sw/hello.hex is not sw/hello.s assembled; rerun with UPDATE_EXPECT=1 and commit both"
    );
    // The string is where the program says it is, so a reader of the hex
    // can find it: `addi a0, zero, message` is the fourth instruction.
    let message = (words[3] >> 20) as usize;
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    assert_eq!(&bytes[message..message + HELLO.len()], HELLO.as_bytes());
    assert_eq!(bytes[message + HELLO.len()], 0, "the string is terminated");
}

#[test]
fn the_assembler_matches_hand_encoded_instructions() {
    // Encodings worked out by hand from the specification's field
    // layout, so the text front end is tested apart from the program.
    let words = asm::assemble(
        "start:\n\
         addi x1, x0, -1      # 0xfff00093\n\
         lui  a0, 0x12345     # 0x12345537\n\
         sw   ra, 8(sp)       # 0x00112423\n\
         beq  a0, zero, start # back 12 bytes\n\
         j    start\n\
         ret\n\
         .string \"a\\n\"\n",
    )
    .expect("assembles");
    assert_eq!(words[0], 0xfff0_0093);
    assert_eq!(words[1], 0x1234_5537);
    assert_eq!(words[2], 0x0011_2423);
    assert_eq!(words[3], 0xfe05_0ae3);
    assert_eq!(words[4], 0xff1f_f06f);
    assert_eq!(words[5], 0x0000_8067);
    assert_eq!(words[6], 0x0000_0a61);
    assert!(
        asm::assemble("addi x1, x0, 4096")
            .unwrap_err()
            .contains("line 1")
    );
    assert!(asm::assemble("bogus x1").is_err());
}

// ---------------------------------------------------------------------------
// Building and synthesis
// ---------------------------------------------------------------------------

#[test]
fn the_project_resolves_and_elaborates() {
    let Some(dir) = example() else { return };
    let built = build(&dir, |_| {});
    assert_eq!(built.project.top.as_deref(), Some("soc_top"));
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
            ("rv32i", "rtl/rv32i.v"),
            ("uart", "rtl/uart_tx.v"),
            ("uart", "rtl/uart_rx.v"),
            ("uart", "rtl/uart.v"),
            ("soc", "rtl/soc_top.v"),
        ],
        "the user's only HDL is rtl/soc_top.v; everything else is the library"
    );
    assert!(built.elaboration.blackboxes.is_empty());
    assert!(built.elaboration.skipped.is_empty());
    let design = built.elaboration.design.expect("a design");
    let top = design.top_module().expect("a top");
    assert_eq!(top.name.as_str(), "soc_top");
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
fn the_soc_synthesises_without_errors_or_latches() {
    let Some(dir) = example() else { return };
    let mut design = soc_design(&dir);
    preload_rom(&mut design, &rom_image(&dir));
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
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
}

// ---------------------------------------------------------------------------
// Simulation
// ---------------------------------------------------------------------------

/// A wire's changes, as `(time, level)` in the order they happened, with
/// `None` for a level that is not 0 or 1.
type Waveform = Vec<(u64, Option<bool>)>;

/// Decodes 8N1 frames from the transitions of a serial line.
///
/// A frame starts at a falling edge from idle; each bit is sampled in its
/// middle, `bit` ticks apart, and the start bit must still be low there
/// and the stop bit high.
fn decode_uart(edges: &[(u64, Option<bool>)], bit: u64) -> Result<Vec<u8>, String> {
    let level_at = |t: u64| -> Option<bool> {
        edges
            .iter()
            .take_while(|(when, _)| *when <= t)
            .last()
            .and_then(|(_, level)| *level)
    };
    let mut out = Vec::new();
    let mut from = 0u64;
    loop {
        let start = edges
            .windows(2)
            .find(|w| w[1].0 >= from && w[0].1 == Some(true) && w[1].1 == Some(false))
            .map(|w| w[1].0);
        let Some(start) = start else { break };
        let sample = |k: u64| level_at(start + k * bit + bit / 2);
        if sample(0) != Some(false) {
            return Err(format!("a start bit at {start} is not low in its middle"));
        }
        let mut byte = 0u8;
        for k in 0..8 {
            match sample(k + 1) {
                Some(true) => byte |= 1 << k,
                Some(false) => {}
                None => return Err(format!("data bit {k} of the frame at {start} is x")),
            }
        }
        if sample(9) != Some(true) {
            return Err(format!("the frame at {start} has no stop bit"));
        }
        out.push(byte);
        from = start + 9 * bit + bit / 2;
    }
    Ok(out)
}

#[test]
fn the_line_comes_out_of_the_serial_wire() {
    let Some(dir) = example() else { return };
    let bench = read(&dir, "tb/soc_tb.v");
    assert!(bench.contains(&format!("localparam CLK_DIV  = {CLK_DIV};")));
    assert!(bench.contains(&format!("localparam HALF     = {HALF_NS};")));

    let mut design = testbench_design(&dir);
    preload_rom(&mut design, &rom_image(&dir));
    let mut files = MemoryFiles::new();
    files.insert("sw/hello.hex", read(&dir, "sw/hello.hex"));
    let options = SimOptions {
        files: Some(Box::new(files)),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).expect("the testbench simulates");

    // The waveform of the pin, and nothing else: every change of
    // `uart_tx` with the time it happened.
    let tx = sim
        .net("soc_tb.uart_tx")
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

    // The only message is the `$readmemh` that Reticle cannot yet run
    // from Verilog; `preload_rom` did its work.
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert!(
        messages
            .iter()
            .all(|m| m.contains("$readmem needs a memory")),
        "unexpected simulator messages: {messages:?}"
    );
}

// ---------------------------------------------------------------------------
// The iCE40 flow
// ---------------------------------------------------------------------------

#[test]
fn the_soc_maps_onto_the_hx8k_and_exports_for_nextpnr() {
    let Some(dir) = example() else { return };
    let mut design = soc_design(&dir);
    preload_rom(&mut design, &rom_image(&dir));
    let top = design.top.expect("a top");
    design
        .flatten(top, &FlattenOptions::default())
        .unwrap_or_else(|d| panic!("soc_top does not flatten: {} problem(s)", d.len()));
    design.remove_unused_modules(top);
    let top = design.top.expect("the top survives flattening");

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
    // The only complaint is the database gap that
    // `the_hx8k_database_lacks_the_breakout_boards_uart_pins` pins: the
    // pins are right for the board, and nextpnr, which has the real
    // package, takes them.
    let complaints: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
    assert_eq!(
        complaints,
        [
            "`ice40-hx8k-ct256` has no pin named B12",
            "`ice40-hx8k-ct256` has no pin named B10",
        ],
        "{}",
        diags.render(&map)
    );

    let mut diags = Diagnostics::new();
    let flow = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
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

    // The ROM, the four RAM lanes and the register file (two copies, one
    // per read port) all went into block RAM. The ROM's contents did
    // not: see `block_ram_loses_the_contents_it_is_initialised_with`.
    let luts = flow.count("SB_LUT4");
    let brams = flow.count("SB_RAM40_4K");
    assert!(luts > 1000, "{luts} LUTs is too few for a processor");
    assert!(luts <= 7680, "{luts} LUTs does not fit the HX8K's 7680");
    assert!(
        brams >= 1,
        "nothing went into block RAM, so the ROM or RAM became logic"
    );
    assert!(brams <= 32, "{brams} block RAMs does not fit the HX8K's 32");
    assert!(
        flow.netlist.iter().all(|(cell, _)| !cell.starts_with('$')),
        "a generic cell survived: {:?}",
        flow.netlist
    );

    // Every cell is a primitive the device has; the two problems left
    // are the same two pins again.
    let problems: Vec<String> = fpga::check_nextpnr_json(&design, top, device, &constraints)
        .into_iter()
        .map(|p| format!("{}: {}", p.object, p.message))
        .collect();
    assert_eq!(
        problems,
        [
            "uart_tx: `ice40-hx8k-ct256` has no package pin `B12`",
            "uart_rx: `ice40-hx8k-ct256` has no package pin `B10`",
        ]
    );
    let inputs = fpga::export_nextpnr(&design, top, device, &constraints).expect("exports");
    assert_eq!(inputs.constraints_name, "soc_top.pcf");
    assert_eq!(
        inputs.args.join(" "),
        "nextpnr-ice40 --hx8k --package ct256 --json soc_top.json --pcf soc_top.pcf --asc soc_top.asc"
    );
    for pin in ["set_io clk J3", "set_io uart_tx B12", "set_io uart_rx B10"] {
        assert!(inputs.pcf_or_lpf.contains(pin), "the PCF lacks `{pin}`");
    }
    assert!(inputs.json.contains("\"SB_RAM40_4K\""));

    // The files a board needs, where a reader can pick them up.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("soc");
    fs::create_dir_all(&out).expect("a scratch directory");
    fs::write(out.join("soc_top.json"), &inputs.json).expect("write the netlist");
    fs::write(out.join(&inputs.constraints_name), &inputs.pcf_or_lpf).expect("write the PCF");
    println!("wrote {}/soc_top.json and soc_top.pcf", out.display());
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

#[cfg(feature = "cli")]
#[test]
fn reticle_build_builds_the_project() {
    let Some(dir) = example() else { return };
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("soc-build");
    fs::create_dir_all(&scratch).expect("a scratch directory");
    let lock = scratch.join("reticle.lock");
    let lock_arg = lock.to_string_lossy().into_owned();
    let (code, _, err) = reticle(
        &dir,
        &["build", "--synth", "--lock", &lock_arg, "reticle.proj"],
    );
    assert_eq!(code, 0, "reticle build failed:\n{err}");
    assert!(
        err.contains("note: built `soc`: 5 module(s) from 5 source(s)"),
        "{err}"
    );
    // The one note synthesis gives is the `$readmemh` it drops; see
    // `synthesis_drops_the_contents_readmemh_loads`.
    assert!(err.contains("simulation-only statement dropped"), "{err}");
    assert!(!err.contains("warning"), "{err}");
    let lock = fs::read_to_string(&lock).expect("a lock file");
    assert!(lock.contains("rv32i") && lock.contains("uart"), "{lock}");
}

#[cfg(feature = "cli")]
#[test]
fn reticle_sim_cannot_run_the_testbench_yet() {
    // How a user would run the testbench from the command line: the
    // testbench, their top-level and the library sources the build report
    // lists. Today the ROM cannot be loaded (see
    // `readmemh_in_verilog_never_reaches_the_simulator`), so the core runs
    // nothing and the testbench times out. When this starts printing the
    // line, flip it to assert `HELLO` and delete `preload_rom`.
    let Some(dir) = example() else { return };
    let (code, out, err) = reticle(
        &dir,
        &[
            "sim",
            "--quiet",
            "tb/soc_tb.v",
            "rtl/soc_top.v",
            "../../ip/rv32i/rtl/rv32i.v",
            "../../ip/uart/rtl/uart_tx.v",
            "../../ip/uart/rtl/uart_rx.v",
            "../../ip/uart/rtl/uart.v",
        ],
    );
    assert_ne!(code, 0);
    assert!(err.contains("$readmem needs a memory"), "{err}");
    assert_eq!(
        out, "soc_tb: timed out with nothing but \"\" received\n",
        "reticle sim now does something different; see the comment above"
    );
}

// ---------------------------------------------------------------------------
// Gaps in Reticle this example found, each pinned in a few lines
// ---------------------------------------------------------------------------

/// Elaborates one Verilog module.
fn verilog(text: &str) -> Design {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let id = map.add("gap.v", text).expect("fits");
    let file = parse_source(
        &mut map,
        id,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    let design = elaborate(
        &[&file],
        &ElabOptions::new(Dialect::Verilog2005),
        &mut diags,
    );
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    design.expect("a design")
}

#[test]
fn a_system_task_without_parentheses_is_dropped() {
    // IEEE 1364-2005 §17.4.1: `$finish;` and `$finish(n);` are the same
    // task. The Verilog lowering sends a bare system identifier down the
    // expression path (`expr_stmt` in src/verilog/elab/lower/stmt.rs
    // handles a bare `Ident` as a task call but not a `SystemIdent`), so
    // the statement vanishes without a diagnostic and the simulation runs
    // on. Fixed, the first process stops the run at 10 and nothing is
    // printed; flip the assertions and let the testbench say `$finish;`.
    let design = verilog(
        "module t;\n\
         initial begin #10 $finish; end\n\
         initial begin #20 $display(\"still running\"); end\n\
         endmodule\n",
    );
    let mut sim = Simulator::new(&design, SimOptions::default()).expect("simulates");
    sim.run();
    assert!(!sim.finished(), "`$finish;` works now; see the comment");
    assert_eq!(sim.output(), "still running\n");

    // With parentheses it works.
    let design = verilog(
        "module t;\n\
         initial begin #10 $finish(0); end\n\
         initial begin #20 $display(\"still running\"); end\n\
         endmodule\n",
    );
    let mut sim = Simulator::new(&design, SimOptions::default()).expect("simulates");
    sim.run();
    assert!(sim.finished());
    assert_eq!(sim.output(), "");
}

/// A memory loaded from a file and read back, as every ROM is written.
const READMEMH: &str = "module t (input wire clk, input wire [1:0] a, output reg [7:0] q);\n\
     reg [7:0] rom [0:3];\n\
     initial $readmemh(\"rom.hex\", rom);\n\
     always @(posedge clk) q <= rom[a];\n\
     endmodule\n";

#[test]
fn readmemh_in_verilog_never_reaches_the_simulator() {
    // The Verilog lowering passes the memory to `$readmemh` as a string
    // holding its name (src/verilog/elab/lower/stmt.rs, the `readmemh`
    // arm of `system_task`), while the simulator only accepts a memory
    // read, `@rom[0]` in the IR (`mem_arg` in src/sim/sys.rs). So no
    // `$readmemh` written in Verilog loads anything, whatever the file
    // provider holds. Fixed, `rom[1]` reads 8'h22.
    let design = verilog(READMEMH);
    let mut files = MemoryFiles::new();
    files.insert("rom.hex", "11 22 33 44");
    let options = SimOptions {
        files: Some(Box::new(files)),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).expect("simulates");
    sim.run_for(1);
    let rom = sim.memory("t.rom").expect("the memory");
    assert!(
        sim.get_mem(rom, 1).and_then(|v| v.to_u64()).is_none(),
        "`$readmemh` works from Verilog now; see the comment"
    );
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert_eq!(
        messages,
        ["$readmem needs a memory (`@mem[0]`) as second argument"]
    );
}

#[test]
fn synthesis_drops_the_contents_readmemh_loads() {
    // Synthesis treats `$readmemh` like `$display`: a simulation-only
    // statement, dropped with note S0014 (src/synth/proc/exec.rs). The
    // memory keeps no initial contents, so the ROM of every netlist built
    // from this source is empty, and on an iCE40 its block RAMs would be
    // configured blank. Every other synthesis tool reads the file here;
    // Reticle would need a file provider in `SynthOptions` to, since the
    // library does no I/O. Fixed, `rom` carries four initial words.
    let mut design = verilog(READMEMH);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some("S0014") && d.severity == Severity::Note),
        "the `$readmemh` is no longer dropped; see the comment"
    );
    let top = design.top_module().expect("a top");
    let init = top
        .memories
        .iter()
        .find(|(_, m)| m.name.as_str() == "rom")
        .map(|(_, m)| m.init.clone());
    assert!(
        matches!(init, Some(None)),
        "the ROM has contents now; see the comment"
    );
}

#[test]
fn displaying_a_memory_word_prints_the_memory_name() {
    // `syscall_args` asks `scope_argument` whether each argument names a
    // scope or an array, and `resolve_path` resolves `mem[1]` to the
    // memory itself, ignoring the index. So the word is replaced by the
    // memory's name, and `%h` prints "mem" in hex. Fixed, this prints
    // "42 42".
    let design = verilog(
        "module t;\n\
         reg [7:0] mem [0:3];\n\
         reg [7:0] v;\n\
         initial begin\n\
         mem[1] = 8'h42;\n\
         v = mem[1];\n\
         $display(\"%h %h\", mem[1], v);\n\
         end\n\
         endmodule\n",
    );
    let mut sim = Simulator::new(&design, SimOptions::default()).expect("simulates");
    sim.run();
    assert_eq!(sim.output(), "6d656d 42\n", "fixed? see the comment");
}

#[test]
fn the_hx8k_database_lacks_the_breakout_boards_uart_pins() {
    // src/fpga/devices/ice40.dev lists nine pins for `ice40-hx8k-ct256`:
    // the eight LEDs of the iCE40-HX8K breakout board and its clock, J3.
    // The CT256 package has over two hundred, and the board's own serial
    // port is on B12 and B10, so constraining it is an error (F0202) and
    // `check_nextpnr_json` refuses the netlist, although nextpnr would
    // take it. Fixed, no F0202 is reported.
    let design = verilog("module t (input wire a, output wire y);\nassign y = a;\nendmodule\n");
    let device = fpga::target(DEVICE).expect("the HX8K");
    let mut map = SourceMap::new();
    let text = "set_io y B12\nset_io a B10\n";
    let file = map.add("pins.rcf", text).expect("fits");
    let mut diags = Diagnostics::new();
    let constraints = Constraints::parse(text, file, &mut diags);
    constraints.check(&design, device, &mut diags);
    assert_eq!(
        diags.iter().filter(|d| d.code == Some("F0202")).count(),
        2,
        "the HX8K knows B12 and B10 now; see the comment"
    );
}

#[test]
fn a_rom_with_two_read_ports_is_not_duplicated_across_block_rams() {
    // A memory with two read ports and one write port becomes one block
    // RAM copy per read port (`Mapper::block_ram` in
    // src/fpga/primitives.rs). A ROM — two read ports, no write port,
    // initial contents — is the easier case, since there is nothing to
    // keep the copies in step, but duplication only happens when there
    // is exactly one writer, so the ROM is left as a generic `$memrd`
    // that no device has and nextpnr cannot place. The reason given
    // also reads as though it fits: "2 port(s) ... needs 2". soc_top
    // shares one ROM port between the buses, which this core allows; a
    // design that cannot hits this. Fixed, the ROM maps to block RAM.
    let mut design = verilog(
        "module t (input wire clk, input wire [7:0] a, input wire [7:0] b,\n\
         output reg [31:0] qa, output reg [31:0] qb);\n\
         reg [31:0] rom [0:255];\n\
         always @(posedge clk) begin qa <= rom[a]; qb <= rom[b]; end\n\
         endmodule\n",
    );
    let top = design.top.expect("a top");
    let rom = design.modules[top]
        .memories
        .iter_mut()
        .map(|(_, m)| m)
        .find(|m| m.name.as_str() == "rom")
        .expect("the ROM");
    rom.init = Some((0..256).map(|i| Logic::from_u64(i, 32)).collect());
    let device = fpga::target(DEVICE).expect("the HX8K");
    let mut diags = Diagnostics::new();
    let flow = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow runs");
    let report = flow.to_text();
    assert_eq!(
        flow.count("$memrd"),
        2,
        "the ROM maps now; see the comment:\n{report}"
    );
    assert_eq!(flow.count("SB_RAM40_4K"), 0, "{report}");
    assert!(
        report.contains(
            "has 2 port(s) and each serves either a read or a write, \
             but the memory needs 2 (2 read, 0 write)"
        ),
        "{report}"
    );
}

#[test]
fn block_ram_loses_the_contents_it_is_initialised_with() {
    // The iCE40 database says SB_RAM40_4K is initialisable through
    // INIT_0..INIT_F (`flags dual_port init` in ice40.dev), but the block
    // RAM mapper never writes them: a memory with initial contents maps
    // to block RAM without a word of them, silently. `preload_rom` gives
    // soc_top's ROM its contents in the IR, and they reach neither the
    // netlist nor the JSON nextpnr reads, so a board configured from the
    // export would fetch zeros. Only a ROM small enough to become logic
    // keeps its contents (`emit_registers` builds its rows as
    // constants). Fixed, the JSON carries INIT_0 with 0x1234 in it.
    let mut design = verilog(
        "module t (input wire clk, input wire [7:0] a, output reg [15:0] q);\n\
         reg [15:0] rom [0:255];\n\
         always @(posedge clk) q <= rom[a];\n\
         endmodule\n",
    );
    let top = design.top.expect("a top");
    let rom = design.modules[top]
        .memories
        .iter_mut()
        .map(|(_, m)| m)
        .find(|m| m.name.as_str() == "rom")
        .expect("the ROM");
    rom.init = Some((0..256).map(|_| Logic::from_u64(0x1234, 16)).collect());
    let device = fpga::target(DEVICE).expect("the HX8K");
    let mut diags = Diagnostics::new();
    let flow = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow runs");
    assert_eq!(flow.count("SB_RAM40_4K"), 1, "{}", flow.to_text());
    let json = fpga::export_nextpnr(&design, top, device, &Constraints::new())
        .expect("exports")
        .json;
    assert!(
        !json.contains("INIT_"),
        "block RAM carries its contents now; see the comment"
    );
    assert!(
        !diags.iter().any(|d| d.message.contains("initial")),
        "the loss is reported now; see the comment"
    );
}

#[cfg(feature = "cli")]
#[test]
fn reticle_fpga_does_not_flatten_a_design_with_instances() {
    // `fpga::synthesize_for` maps one module and says a hierarchical
    // design should be flattened first; `reticle fpga` (src/bin/reticle/
    // main.rs) hands it the top as elaborated, so any design with an
    // instance in it — soc_top has two — is refused with "an instance is
    // left". The test's own flow flattens, which is what the binary should
    // do. Fixed, this writes top.json and exits 0.
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("soc-fpga-hier");
    fs::create_dir_all(&scratch).expect("a scratch directory");
    fs::write(
        scratch.join("h.v"),
        "module inv (input wire a, output wire y);\n  assign y = ~a;\nendmodule\n\
         module top (input wire a, output wire y);\n  inv u (.a(a), .y(y));\nendmodule\n",
    )
    .expect("write the design");
    let (code, _, err) = reticle(
        &scratch,
        &[
            "fpga",
            "--device",
            "ice40-hx1k-tq144",
            "--top",
            "top",
            "--quiet",
            "h.v",
        ],
    );
    assert_ne!(code, 0, "reticle fpga flattens now; see the comment");
    assert!(
        err.contains("an instance is left: nextpnr wants one flat netlist"),
        "{err}"
    );
}
