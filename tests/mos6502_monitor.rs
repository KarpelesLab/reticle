//! `examples/mos6502_monitor`: a 6502 with a monitor you can type at.
//!
//! `examples/mos6502_computer` prints one line down a wire and stops.
//! This is the same processor with a keyboard: 256 bytes of ROM, an
//! ACIA, and a parser that examines memory, deposits bytes and runs
//! code. The interesting part of testing it is that **the monitor is
//! software before it is gateware**, so most of what is below drives the
//! machine a character at a time and compares what comes back against a
//! string this file computed — the way one would test a parser, because
//! that is what it is.
//!
//! | Test | What it shows |
//! |------|----------------|
//! | `monitor_rom_is_the_assembled_source` | `rtl/monitor_rom.v` is `sw/monitor.s` assembled by the opcode matrix in `tests/mos6502_asm`, inside one page, with the three vectors pointing at the labels the source names |
//! | `the_monitor_uses_no_instruction_the_core_has_not_got` | every opcode in the ROM is one of the 151 documented NMOS encodings, so nothing 65C02-only is in it |
//! | `the_monitor_prompts_when_it_is_reset` | a backslash and a carriage return, out of the machine, from a cold start |
//! | `the_monitor_examines_one_location` | `FF00` answers with the byte the ROM holds there |
//! | `the_monitor_examines_a_range` | `FFFA.FFFF` answers with the six vector bytes, on one line |
//! | `the_monitor_breaks_a_range_every_eight_bytes` | a range across `$xx00` starts a new labelled line there and nowhere else |
//! | `the_monitor_deposits_bytes_and_reads_them_back` | `0300: ...` writes, and a range reads back what was written |
//! | `a_bare_colon_and_a_bare_dot_carry_on_from_the_last_one` | the two continuations |
//! | `the_monitor_runs_what_was_deposited` | `0300R` transfers control, and a deposited program prints |
//! | `the_monitor_rejects_a_line_it_cannot_parse` | a fresh `\` and nothing else |
//! | `the_monitor_edits_a_line_with_backspace_and_cancels_it_with_escape` | the two editing keys |
//! | `the_whole_session_matches_the_one_a_real_monitor_gives` | **the oracle**: a transcript recorded from a running original, reproduced here |
//! | `the_acia_reports_the_rate_the_host_set` | `SET_LINE_CODING`'s rate reaches CONTROL's baud bits and the ACIA's reported rate |
//! | `the_processor_runs_at_one_cycle_in_fifty_nine` | the divider the design is built with |
//! | `a_partly_filled_packet_goes_when_the_machine_falls_silent` | `in_commit` after a silence, so one keystroke does not wait for sixty-three more |
//! | `the_project_resolves_and_elaborates` | the manifest builds from three library packages and four sources |
//! | `the_machine_synthesises_without_errors_or_latches` | generic synthesis: no error, no warning, no latch |
//! | `the_machine_maps_onto_the_ecp5_and_fits_the_part` | the ECP5 flow fits `monitor_cynthion` on an LFE5U-12F, with the footprint printed |
//! | `reticle_sim_runs_the_testbench` | the session comes out of the binary too |
//!
//! **What none of it proves is the board.** The USB stack is not in any
//! of these runs: `monitor_bench` is the machine with its byte interface
//! bare, so what is tested is the computer and not the path to it.
//! `a_6502_monitor_answers_through_the_transceiver_that_is_on_the_board`
//! in `tests/ip_library.rs` is the same machine through the whole stack
//! and through a model of the transceiver that is on this board,
//! including its late LineState; README.md says which of the two halves
//! each run covers and what was done on the part instead.
//!
//! **What the behaviour tests as a family would not catch.** Every one of
//! them types at the monitor and compares what comes back, so between
//! them they would catch a wrong answer, a missing answer, a byte in the
//! wrong order and an answer that never ends. What none of them can see
//! is anything that is not a *byte at the interface*: how long the
//! monitor took, how many bus cycles it spent, which instructions it
//! executed, or whether it wrote somewhere it should not have on the way.
//! A monitor that answered every one of these correctly and also
//! scribbled over page three would pass all of them —
//! `the_monitor_deposits_bytes_and_reads_them_back` would only notice if
//! it scribbled over *that* byte. What covers the instructions rather
//! than the bytes is `the_monitor_uses_no_instruction_the_core_has_not_got`,
//! which decodes the ROM rather than running it, and `ip/mos6502`'s own
//! thirty-odd tests, which is why nothing here re-tests the processor.
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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::{Diagnostics, Severity};
use reticle::fpga::{self, Constraints, FpgaOptions};
use reticle::ip::{self, Elaboration, PathProvider, Project, SourceEntry};
use reticle::ir::{CellKind, Design};
use reticle::logic::Logic;
use reticle::sim::{NetHandle, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run as synth_run};

/// The 6502 assembler built from the documented opcode matrix, shared
/// with `tests/ip_library.rs` and the other two 6502 examples.
#[path = "mos6502_asm/mod.rs"]
mod asm;

/// The part the project targets: the Cynthion's ECP5.
const DEVICE: &str = "ecp5-12f-CABGA256";

/// The two pages the ROM answers to, and their size.
const ROM_BASE: u32 = 0xFE00;
const ROM_BYTES: u32 = 512;

/// The three vectors, where the part has always had them.
const VEC_NMI: u16 = 0xFFFA;
const VEC_RES: u16 = 0xFFFC;
const VEC_IRQ: u16 = 0xFFFE;

/// Clocks per 6502 bus cycle in the design that goes on the board.
const CPU_DIV: u64 = 59;

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

/// `examples/mos6502_monitor`, when this copy of the crate has it.
fn example() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/mos6502_monitor");
    if dir.join("reticle.proj").is_file() {
        Some(dir)
    } else {
        println!("skipping: examples/mos6502_monitor is not in this copy of the crate");
        None
    }
}

/// A file of the example, as text.
fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel))
        .unwrap_or_else(|e| panic!("examples/mos6502_monitor/{rel}: {e}"))
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

/// The design with `which` — one of the two testbenches — as its top.
fn bench_design(dir: &Path, which: &str) -> Design {
    let want = format!("tb/{which}.v");
    let built = build(dir, |project| {
        let bench = project
            .testbenches
            .iter()
            .find(|path| **path == want)
            .unwrap_or_else(|| panic!("the manifest has no `testbench {want}`"))
            .clone();
        project.sources.push(SourceEntry {
            path: bench,
            language: None,
            encrypted: false,
            span: project.span,
        });
        project.top = Some(which.to_owned());
    });
    built.elaboration.design.expect("the testbench elaborates")
}

// ---------------------------------------------------------------------------
// The monitor as software
// ---------------------------------------------------------------------------

/// `sw/monitor.s` assembled.
fn monitor(dir: &Path) -> asm::Image {
    asm::assemble(&read(dir, "sw/monitor.s")).unwrap_or_else(|e| panic!("sw/monitor.s: {e}"))
}

/// The ROM's 256 bytes, by offset from `$FF00`, with anything the
/// program did not place left as zero — which is what
/// `rtl/monitor_rom.v` has to answer, because a `case` needs an entry
/// for every address a reader can present.
fn rom_bytes(image: &asm::Image) -> [u8; 512] {
    let mut out = [0u8; 512];
    for (address, byte) in &image.bytes {
        let at = u32::from(*address);
        assert!(
            (ROM_BASE..ROM_BASE + ROM_BYTES).contains(&at),
            "{at:#06x} is outside the two pages this machine's ROM answers to"
        );
        out[(at - ROM_BASE) as usize] = *byte;
    }
    out
}

/// A little-endian word of the image.
fn word_at(image: &asm::Image, address: u16) -> u16 {
    let low = image.bytes.get(&address).copied().unwrap_or_default();
    let high = image.bytes.get(&(address + 1)).copied().unwrap_or_default();
    u16::from(low) | (u16::from(high) << 8)
}

/// `rtl/monitor_rom.v`, rendered from the assembled program.
///
/// A `case` over all 256 addresses and not a `reg` array with
/// `$readmemh`: a memory with initial contents is what `fpga::primitives`
/// turns into a block RAM, and there is no block RAM site on this
/// fabric, so the design would map and then fail to place. A `case` is a
/// combinational process, which `synth::proc` lowers to a multiplexer
/// tree and the technology mapper covers with lookup tables, and no step
/// of that has an opinion about memories at all.
fn render_rom(bytes: &[u8; 512], labels: &BTreeMap<String, u16>) -> String {
    let mut out = String::from(
        "// monitor_rom.v: sw/monitor.s assembled, as 256 lookup tables' worth\n\
         // of constant.\n\
         //\n\
         // Generated by tests/mos6502_monitor.rs; do not edit. Change\n\
         // sw/monitor.s and run\n\
         //\n\
         //     UPDATE_EXPECT=1 cargo test --all-features \\\n\
         //         --test mos6502_monitor monitor_rom\n\
         //\n\
         // It is a `case` and not a `reg [7:0] rom [0:255]` with a\n\
         // `$readmemh`, and the difference is not style. An array with\n\
         // initial contents is a memory, `fpga::primitives` maps a memory\n\
         // of this size onto a `DP16KD`, and `src/fpga/trellis` builds no\n\
         // `bram` site at all — so that spelling compiles, maps, and then\n\
         // fails to place with \"the design needs 1 `bram` site(s) and the\n\
         // part has 0\". A combinational `case` is never a memory: it is a\n\
         // multiplexer tree, which is what a ROM on this part has to be.\n\
         //\n\
         // The address is nine bits because the ROM is two pages. Every\n\
         // one of the 512 is written out, including the zeros between the\n\
         // program and the vectors, because a `case` that does not cover\n\
         // its selector is a latch.\n\
         module monitor_rom (\n\
         \x20   input  wire [8:0] addr,\n\
         \x20   output reg  [7:0] data\n\
         );\n\
         \x20   always @(*) begin\n\
         \x20       case (addr)\n",
    );
    // Where each label landed, so the listing can be read.
    let mut at: BTreeMap<u16, Vec<&str>> = BTreeMap::new();
    for (name, address) in labels {
        if (ROM_BASE..ROM_BASE + ROM_BYTES).contains(&u32::from(*address)) {
            at.entry(*address).or_default().push(name.as_str());
        }
    }
    for (offset, byte) in bytes.iter().enumerate() {
        let address = u16::try_from(ROM_BASE).expect("the ROM is inside sixteen bits")
            + u16::try_from(offset).expect("an offset inside the ROM");
        if let Some(names) = at.get(&address) {
            let mut names = names.clone();
            names.sort_unstable();
            out.push_str(&format!(
                "            // {address:04X}  {}\n",
                names.join(", ")
            ));
        }
        out.push_str(&format!(
            "            9'h{offset:03X}: data = 8'h{byte:02X};\n"
        ));
    }
    out.push_str(
        "            default: data = 8'h00;\n\
         \x20       endcase\n\
         \x20   end\n\
         endmodule\n",
    );
    out
}

#[test]
fn monitor_rom_is_the_assembled_source() {
    let Some(dir) = example() else { return };
    let image = monitor(&dir);

    let lowest = u32::from(*image.bytes.keys().next().expect("a program"));
    let highest = u32::from(*image.bytes.keys().next_back().expect("a program"));
    assert_eq!(lowest, ROM_BASE, "the monitor does not start at the ROM");
    assert_eq!(
        highest,
        ROM_BASE + ROM_BYTES - 1,
        "the last byte of the ROM is $FFFF, and it is the top of the RES vector"
    );

    // All three point at the reset entry. There is no handler and no
    // room for one, so an interrupt that cannot happen restarts the
    // machine rather than running whatever $00 is; the source says so
    // where it places them.
    for vector in [VEC_NMI, VEC_RES, VEC_IRQ] {
        assert_eq!(
            word_at(&image, vector),
            image.label("reset"),
            "the vector at {vector:#06x}"
        );
    }

    // How much of the ROM the program actually uses, which is the one
    // number that decided how big this ROM had to be.
    let last_code = *image
        .bytes
        .keys()
        .rfind(|a| u32::from(**a) < u32::from(VEC_NMI))
        .expect("code below the vectors");
    let used = u32::from(last_code) - ROM_BASE + 1;
    println!(
        "the monitor is {used} bytes of code, plus six of vectors, in a \
         {ROM_BYTES}-byte ROM"
    );
    assert!(
        used <= ROM_BYTES - 6,
        "{used} bytes of code does not leave room for the vectors"
    );

    let bytes = rom_bytes(&image);
    let expected = render_rom(&bytes, &image.labels);
    expect_file(
        &dir.join("rtl/monitor_rom.v"),
        &expected,
        "sw/monitor.s assembled",
    );
}

/// Nothing in the ROM is an instruction `ip/mos6502` does not have.
///
/// The core is an NMOS 6502 and says so; the published 65C02 ports of
/// this interface use `DEC A` and `PLX`, which decode on this core as
/// one-byte no-operations and would hang the delay loops they are in.
/// So a monitor written for this machine has to stay inside the 151
/// documented encodings, and this walks the ROM to prove it did.
///
/// The walk is by *length*, not a scan for byte values: an operand that
/// happened to be `$3A` would be a false alarm, and the only way to know
/// which bytes are opcodes is to decode from the entry point forward.
/// Every instruction's length comes from the assembler's own table,
/// which was typed from the documentation and not from the core.
///
/// What it would not catch: an instruction only ever reached by a
/// computed jump into the middle of another one. There is no such thing
/// in this ROM, and the walk covers every byte below the vectors, so a
/// gap would show as an unreachable byte rather than as silence.
#[test]
fn the_monitor_uses_no_instruction_the_core_has_not_got() {
    let Some(dir) = example() else { return };
    let image = monitor(&dir);
    let bytes = rom_bytes(&image);

    let mut lengths = [0usize; 256];
    for insn in asm::TABLE {
        lengths[usize::from(insn.code)] = insn.mode.len() as usize;
    }

    // Every label is an entry point; that covers the fall-through
    // routines as well as the ones that are jumped to.
    let mut seen = [false; ROM_BYTES as usize];
    let mut queue: Vec<u16> = image
        .labels
        .values()
        .copied()
        .filter(|a| (ROM_BASE..u32::from(VEC_NMI)).contains(&u32::from(*a)))
        .collect();
    queue.push(image.label("reset"));
    let mut undecodable: Vec<String> = Vec::new();
    while let Some(start) = queue.pop() {
        let mut pc = start;
        while u32::from(pc) < u32::from(VEC_NMI) {
            let offset = (u32::from(pc) - ROM_BASE) as usize;
            if seen[offset] {
                break;
            }
            seen[offset] = true;
            let code = bytes[offset];
            let len = lengths[usize::from(code)];
            if len == 0 {
                undecodable.push(format!("{pc:04X}: {code:02X}"));
                break;
            }
            for step in 1..len {
                seen[offset + step] = true;
            }
            pc =
                pc.wrapping_add(u16::try_from(len).expect("an instruction is at most three bytes"));
        }
    }
    assert!(
        undecodable.is_empty(),
        "these bytes are not documented NMOS encodings: {undecodable:?}"
    );

    let last_code = u32::from(
        *image
            .bytes
            .keys()
            .rfind(|a| u32::from(**a) < u32::from(VEC_NMI))
            .expect("code"),
    );
    let unreached: Vec<String> = (0..=(last_code - ROM_BASE) as usize)
        .filter(|offset| !seen[*offset])
        .map(|offset| format!("{:04X}", ROM_BASE as usize + offset))
        .collect();
    assert!(
        unreached.is_empty(),
        "the walk did not reach every byte of the program, so it did not decode all of it: {unreached:?}"
    );
}

// ---------------------------------------------------------------------------
// Driving the machine
// ---------------------------------------------------------------------------

/// Half a clock period, in simulator ticks.
const HALF: u64 = 500;

/// How many clocks of silence end a command's answer.
///
/// The monitor prints a character about every twenty processor cycles,
/// and `monitor_bench` runs one cycle per clock, so a few hundred clocks
/// with nothing printed means it has stopped and is waiting for a key.
/// It is a count of clocks and never a number of seconds, so a slower
/// machine running this test measures the same thing.
const SILENCE: u64 = 4000;

/// The machine, typed at a character at a time.
struct Machine<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    key_data: NetHandle,
    key_valid: NetHandle,
    key_ready: NetHandle,
    print_data: NetHandle,
    print_valid: NetHandle,
    print_commit: NetHandle,
    acia_rate: NetHandle,
    acia_control: NetHandle,
    /// Everything it has printed since the last time it was taken.
    out: Vec<u8>,
    /// Clocks spent, so a test can say how long something took without
    /// ever mentioning a second.
    clocks: u64,
}

fn net(sim: &Simulator<'_>, name: &str) -> NetHandle {
    let path = format!("{}.{name}", sim.top_name());
    sim.net(&path)
        .unwrap_or_else(|| panic!("the bench has no net `{path}`"))
}

fn high(sim: &Simulator<'_>, handle: NetHandle) -> bool {
    sim.get(handle).to_u64() == Some(1)
}

impl<'d> Machine<'d> {
    fn boot(design: &'d Design, rate: u64) -> Machine<'d> {
        let mut sim = Simulator::new(design, SimOptions::default())
            .unwrap_or_else(|d| panic!("the bench does not simulate: {}", d.len()));
        let clk = net(&sim, "clk");
        let rst_n = net(&sim, "rst_n");
        let key_data = net(&sim, "key_data");
        let key_valid = net(&sim, "key_valid");
        let key_ready = net(&sim, "key_ready");
        let print_data = net(&sim, "print_data");
        let print_valid = net(&sim, "print_valid");
        let print_commit = net(&sim, "print_commit");
        let print_ready = net(&sim, "print_ready");
        let host_rate = net(&sim, "host_rate");
        let acia_rate = net(&sim, "acia_rate");
        let acia_control = net(&sim, "acia_control");

        sim.set(key_valid, Logic::from_bool(false));
        sim.set(key_data, Logic::from_u64(0, 8));
        sim.set(print_ready, Logic::from_bool(true));
        sim.set(host_rate, Logic::from_u64(rate, 32));
        sim.set(rst_n, Logic::from_bool(false));
        sim.set(clk, Logic::from_bool(false));

        let mut machine = Machine {
            sim,
            clk,
            key_data,
            key_valid,
            key_ready,
            print_data,
            print_valid,
            print_commit,
            acia_rate,
            acia_control,
            out: Vec::new(),
            clocks: 0,
        };
        machine.clear_ram();
        for _ in 0..4 {
            machine.tick();
        }
        machine.sim.set(rst_n, Logic::from_bool(true));
        machine
    }

    /// Fills the RAM with zeros.
    ///
    /// A distributed RAM has no initial contents — `fpga::primitives`
    /// refuses to lower a memory that has any, and the array in
    /// `monitor_machine` therefore has none — so in simulation every
    /// byte of it is `x` until something writes it. A real part comes up
    /// with whatever its cells held, which is not `x` and is usually
    /// zero; the emulator the transcripts were recorded from starts at
    /// zero too. So the test writes the zeros, and says so, rather than
    /// asserting against `x` or pretending the hardware does it.
    fn clear_ram(&mut self) {
        let ram = self
            .sim
            .memory(&format!("{}.dut.ram", self.sim.top_name()))
            .expect("the machine has a RAM");
        for index in 0..self.sim.mem_len(ram) as u64 {
            self.sim.set_mem(ram, index, Logic::from_u64(0, 8));
        }
    }

    /// What the host says the line rate is, from now on.
    fn set_rate(&mut self, rate: u64) {
        let host_rate = net(&self.sim, "host_rate");
        self.sim.set(host_rate, Logic::from_u64(rate, 32));
    }

    /// Some clocks.
    fn run(&mut self, clocks: u64) {
        for _ in 0..clocks {
            self.tick();
        }
    }

    /// One clock, collecting whatever the machine printed in it.
    fn tick(&mut self) {
        self.sim.run_for(HALF);
        // The byte is taken on the rising edge, so it is read here, in
        // the half-period the design has settled in and before the edge
        // that lets it move on.
        let moved = high(&self.sim, self.print_valid);
        let byte = self.sim.get(self.print_data).to_u64();
        self.sim.set(self.clk, Logic::from_bool(true));
        if moved {
            // An `x` byte becomes zero and a wide one is masked: what is
            // on an eight-bit net is a byte however the simulator holds it.
            self.out
                .push(u8::try_from(byte.unwrap_or(0) & 0xFF).expect("a byte"));
        }
        self.sim.run_for(HALF);
        self.sim.set(self.clk, Logic::from_bool(false));
        self.clocks += 1;
    }

    /// Runs until the machine has printed nothing for SILENCE clocks.
    fn settle(&mut self) {
        let mut quiet = 0u64;
        let mut spent = 0u64;
        let mut seen = self.out.len();
        while quiet < SILENCE && spent < 2_000_000 {
            self.tick();
            spent += 1;
            if self.out.len() == seen {
                quiet += 1;
            } else {
                seen = self.out.len();
                quiet = 0;
            }
        }
        assert!(
            quiet >= SILENCE,
            "the machine never stopped printing: {:?}",
            self.flat()
        );
    }

    /// Types one byte, waiting for the ACIA to have room for it.
    ///
    /// `key_ready` is the ACIA's `rx_ready`, which is low while a byte
    /// is waiting to be read — so this is throttled by the monitor
    /// consuming characters and cannot type faster than a 6502 reads.
    fn key(&mut self, byte: u8) {
        self.sim
            .set(self.key_data, Logic::from_u64(u64::from(byte), 8));
        self.sim.set(self.key_valid, Logic::from_bool(true));
        let mut spent = 0;
        while !high(&self.sim, self.key_ready) && spent < 200_000 {
            self.tick();
            spent += 1;
        }
        assert!(
            high(&self.sim, self.key_ready),
            "the machine never had room for the byte {byte:#04x}"
        );
        self.tick();
        self.sim.set(self.key_valid, Logic::from_bool(false));
    }

    /// Types a line and returns everything printed in answer, with
    /// carriage returns written `|` so a failure can be read.
    ///
    /// The answer includes the carriage return the monitor prints at the
    /// *start* of the next line, because that is what it does: there is
    /// no prompt character between commands, only a fresh line. Waiting
    /// for silence rather than for a byte count is what makes that
    /// boundary unambiguous.
    fn command(&mut self, line: &str) -> String {
        self.out.clear();
        for byte in line.bytes() {
            self.key(byte);
        }
        self.key(b'\r');
        self.settle();
        self.flat()
    }

    /// What it has printed, with carriage returns as `|`.
    fn flat(&self) -> String {
        String::from_utf8_lossy(&self.out).replace('\r', "|")
    }

    fn take(&mut self) -> String {
        let text = self.flat();
        self.out.clear();
        text
    }
}

/// A machine at the prompt, with the banner already taken.
fn at_prompt(design: &Design, rate: u64) -> Machine<'_> {
    let mut machine = Machine::boot(design, rate);
    machine.settle();
    assert_eq!(
        machine.take(),
        "\\|",
        "the prompt is a backslash and a return"
    );
    machine
}

#[test]
fn the_monitor_prompts_when_it_is_reset() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = Machine::boot(&design, 115_200);
    machine.settle();
    assert_eq!(
        machine.flat(),
        "\\|",
        "a backslash and a carriage return, and nothing else"
    );
    println!(
        "the prompt took {} clocks from reset, at one bus cycle per clock",
        machine.clocks
    );
}

#[test]
fn the_monitor_examines_one_location() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let image = monitor(&dir);
    let bytes = rom_bytes(&image);
    let mut machine = at_prompt(&design, 115_200);

    assert_eq!(
        machine.command("FE00"),
        format!("FE00||FE00: {:02X}|", bytes[0]),
        "the echo, the monitor's own return, the ROM's first byte, and the next line"
    );
}

#[test]
fn the_monitor_examines_a_range() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let image = monitor(&dir);
    let bytes = rom_bytes(&image);
    let mut machine = at_prompt(&design, 115_200);

    // The six vector bytes are $FFFA..$FFFF and therefore all on one
    // row: a row breaks at an address whose low three bits are zero, and
    // $FFF8 is the last of those.
    let want: String = (0x1FA..=0x1FFusize)
        .map(|offset| format!(" {:02X}", bytes[offset]))
        .collect();
    assert_eq!(
        machine.command("FFFA.FFFF"),
        format!("FFFA.FFFF||FFFA:{want}|"),
        "six bytes on one line, with one address label"
    );

    // And they are the vectors the ROM was assembled with: all three
    // point at the reset entry, because this machine cannot raise an
    // interrupt and has no room for a handler that would prove it.
    let vectors: Vec<u16> = (0..3)
        .map(|i| u16::from(bytes[0x1FA + 2 * i]) | (u16::from(bytes[0x1FB + 2 * i]) << 8))
        .collect();
    assert_eq!(
        vectors,
        vec![image.label("reset"); 3],
        "the bytes the monitor printed are NMI, RES and IRQ"
    );
}

#[test]
fn the_monitor_breaks_a_range_every_eight_bytes() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = at_prompt(&design, 115_200);

    // RAM, so the contents are this session's own: five bytes from
    // $02FE, then a range that crosses $0300.
    machine.command("02FE: 11 22 33 44 55");
    assert_eq!(
        machine.command("02FE.0302"),
        "02FE.0302||02FE: 11 22|0300: 33 44 55|",
        "the row breaks at $0300 and nowhere else"
    );
}

#[test]
fn the_monitor_deposits_bytes_and_reads_them_back() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = at_prompt(&design, 115_200);

    // The deposit prints the byte that *was* there first, which is a
    // consequence of the parser's shape rather than a decision: the
    // address item ends while the mode is still EXAMINE and the `:`
    // changes it only afterwards. Confirmed against a running original.
    assert_eq!(machine.command("0300: AA BB"), "0300: AA BB||0300: 00|");
    assert_eq!(machine.command("0300.0301"), "0300.0301||0300: AA BB|");
}

#[test]
fn a_bare_colon_and_a_bare_dot_carry_on_from_the_last_one() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = at_prompt(&design, 115_200);

    machine.command("0300: AA BB");
    // A bare colon carries on from $0302, and a store prints nothing.
    assert_eq!(machine.command(": CC DD"), ": CC DD||");
    assert_eq!(machine.command("0300.0301"), "0300.0301||0300: AA BB|");
    // A bare dot carries on from $0302, with no label and no return of
    // its own, because $0302 does not start a row.
    assert_eq!(machine.command(".0303"), ".0303| CC DD|");
}

#[test]
fn the_monitor_runs_what_was_deposited() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = at_prompt(&design, 115_200);

    // A program that prints `*` through the ACIA and then stops in a
    // loop of its own. Assembled here, from the same documented table
    // the ROM was, so the test does not have to know any encodings.
    let program = asm::assemble(
        "        .org $0300\n\
         start:  lda $5001\n\
                 and #$10\n\
                 beq start\n\
                 lda #$2a\n\
                 sta $5000\n\
         halt:   jmp halt\n",
    )
    .expect("the program assembles");
    let bytes: Vec<u8> = program.bytes.values().copied().collect();
    let deposit: String = bytes.iter().map(|b| format!(" {b:02X}")).collect();

    machine.command(&format!("0300:{deposit}"));
    assert_eq!(
        machine.command("0300R"),
        format!("0300R||0300: {:02X}*", bytes[0]),
        "the byte at the run address, then what the program printed, and \
         then nothing at all — the monitor is gone"
    );
}

#[test]
fn the_monitor_rejects_a_line_it_cannot_parse() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = at_prompt(&design, 115_200);

    // A letter that is not a hexadecimal digit and not `R`; two of them;
    // lower case, which is rejected rather than folded; and a good
    // address followed by a bad character.
    for line in ["HELLO", "ZZ", "ff00"] {
        assert_eq!(
            machine.command(line),
            format!("{line}|\\|"),
            "`{line}` should be echoed and then refused with a fresh prompt"
        );
    }

    // A good address and then a bad character: the address is examined
    // first, because the item ended before the bad character was read,
    // and *then* the line is refused. Confirmed against a running
    // original, which does the same for the same reason.
    assert_eq!(machine.command("0300G"), "0300G||0300: 00\\|");

    // A line of separators is not an error, it is empty.
    assert_eq!(
        machine.command("  "),
        "  ||",
        "an empty line is just a new line"
    );
}

#[test]
fn the_monitor_edits_a_line_with_backspace_and_cancels_it_with_escape() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let image = monitor(&dir);
    let bytes = rom_bytes(&image);
    let mut machine = at_prompt(&design, 115_200);

    // `FF09` backspaced to `FF0` and then `0`: the answer is $FF00's.
    assert_eq!(
        machine.command("FE09\u{8}0"),
        format!("FE09\u{8}0||FE00: {:02X}|", bytes[0]),
        "the backspace is echoed and the digit it removed is gone"
    );

    // Backspace at the left margin does nothing at all.
    assert_eq!(
        machine.command("\u{8}\u{8}FE00"),
        format!("\u{8}\u{8}FE00||FE00: {:02X}|", bytes[0])
    );

    // Escape throws the line away and prompts again. It is echoed
    // first, which is what a monitor that echoes before it looks does.
    machine.out.clear();
    for byte in b"FFFF" {
        machine.key(*byte);
    }
    machine.key(0x1B);
    machine.settle();
    assert_eq!(machine.take(), "FFFF\u{1b}\\|", "escape cancels the line");
}

/// The whole session, against one recorded from a running original.
///
/// This is the oracle, and it is the reason this monitor can be said to
/// reproduce an interface rather than to resemble one. Each expected
/// string below was **printed by a published 65C02 build of the same
/// interface, running in a third-party emulator**, driven with the same
/// input — README.md says exactly what was run. Nothing of that build is
/// in this repository and none of it was read; what was compared is what
/// came out of a terminal.
///
/// The addresses are all RAM, so the bytes are this session's own and
/// not the other machine's ROM. That is the trick that lets two
/// different machines with different memory maps be compared at all.
///
/// What it would not catch: anything the original also gets wrong, and
/// anything neither was asked. It is a check against a second
/// implementation, not against a specification.
#[test]
fn the_whole_session_matches_the_one_a_real_monitor_gives() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = at_prompt(&design, 115_200);

    // Written out rather than computed, so that what is asserted is the
    // recorded transcript and not the same idea expressed twice.
    //
    // **Every command form the monitor has is in here**, and the list is
    // the point rather than the length: an examine, a range, a range that
    // runs backwards, a range crossing a row boundary, two examines on one
    // line, an over-long number, a one-byte deposit, a multi-byte deposit,
    // a deposit over a byte that was deposited, a bare `:` continuing one
    // and a bare `.` continuing a range. What is *not* here is `R`, and
    // that is not laziness: the two machines have different memory maps, so
    // the program a `R` would run would have to be a different program, and
    // comparing two different programs' output proves nothing about either
    // monitor. `the_monitor_runs_what_was_deposited` covers `R` against
    // this machine instead.
    let script: [(&str, &str); 15] = [
        // A one-byte deposit, and the thing about it that looks like a
        // bug and is not: it prints `<addr>: <what was there>` first.
        // That is the address item being *examined* -- the mode is still
        // EXAMINE when the item ends, and the `:` changes it only
        // afterwards -- and the original does exactly the same, which is
        // what these three lines are here to hold.
        ("0310: 11", "0310: 11||0310: 00|"),
        ("0310: 22", "0310: 22||0310: 11|"),
        ("0310", "0310||0310: 22|"),
        ("0310: 33", "0310: 33||0310: 22|"),
        ("0310", "0310||0310: 33|"),
        // A multi-byte deposit prints one such line and not one a byte,
        // because there is one address item in the line.
        ("0300: AA BB", "0300: AA BB||0300: 00|"),
        (": CC DD", ": CC DD||"),
        ("0300.0303", "0300.0303||0300: AA BB CC DD|"),
        (".0307", ".0307| 00 00 00 00|"),
        ("0305.0300", "0305.0300||0305: 00|"),
        (
            "02FE.0310",
            "02FE.0310||02FE: 00 00|0300: AA BB CC DD 00 00 00 00|\
                       0308: 00 00 00 00 00 00 00 00|0310: 33|",
        ),
        ("0300 0400", "0300 0400||0300: AA|0400: 00|"),
        ("12345", "12345||2345: 00|"),
        // And a deposit of more bytes than the row it starts in, so the
        // single label is checked against a case where a *range* would
        // have printed two.
        (
            "0400: 01 02 03 04 05 06 07 08 09",
            "0400: 01 02 03 04 05 06 07 08 09||0400: 00|",
        ),
        (
            "0400.0408",
            "0400.0408||0400: 01 02 03 04 05 06 07 08|0408: 09|",
        ),
    ];
    for (line, want) in script {
        assert_eq!(machine.command(line), want, "`{line}`");
    }
}

// ---------------------------------------------------------------------------
// The ACIA and the machine around it
// ---------------------------------------------------------------------------

#[test]
fn the_acia_reports_the_rate_the_host_set() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");

    // The machine boots first and the host speaks afterwards, which is
    // the order it happens in: the monitor writes $1F to CONTROL within
    // a few milliseconds of reset, and a host opens the port and sends
    // SET_LINE_CODING whenever a person runs a terminal program.
    //
    // 9600 is what `usb_cdc_req` reports before any host has spoken, so
    // that is what the machine boots against.
    let mut machine = at_prompt(&design, 9600);
    let before = machine
        .sim
        .get(machine.acia_control)
        .to_u64()
        .expect("CONTROL is driven");
    assert_eq!(
        before, 0x1F,
        "the monitor programmes CONTROL before any host has spoken, and wins"
    );

    // Each of these is a change from the one before it, because that is
    // what the load is an edge on. 115200 and 230400 are both code 0 —
    // the 65C51's four baud bits cannot name either — and the pair is
    // here to show that two rates the table flattens together are still
    // two changes.
    // `named` is bit 4, the receiver's clock source: the generator for a
    // rate the table names and the 16x external clock for one it cannot,
    // because the generator selected with a rate field of `0000` is a
    // state a W65C51N data sheet has no meaning for.
    for (rate, code, named, reported) in [
        (115_200u64, 0x00u64, false, 115_200u64),
        (9600, 0x0E, true, 9600),
        (19_200, 0x0F, true, 19_200),
        (1200, 0x08, true, 1200),
        (230_400, 0x00, false, 230_400),
        (115_200, 0x00, false, 115_200),
    ] {
        machine.set_rate(rate);
        machine.run(64);
        let control = machine
            .sim
            .get(machine.acia_control)
            .to_u64()
            .expect("CONTROL is driven");
        assert_eq!(
            control & 0x0F,
            code,
            "rate {rate}: CONTROL's baud bits, {control:#04x} in all"
        );
        assert_eq!(
            control & 0x10 != 0,
            named,
            "rate {rate}: the receiver's clock source, {control:#04x} in all"
        );
        assert_eq!(
            control & 0xE0,
            0x00,
            "rate {rate}: the word length and stop bits the monitor wrote are untouched"
        );
        assert_eq!(
            machine.sim.get(machine.acia_rate).to_u64(),
            Some(reported),
            "rate {rate}: what the ACIA says it is programmed to"
        );

        // And the processor reads it back over its own bus, which is the
        // whole point of putting the host's rate in a register rather
        // than in a wire nothing inside the machine can see.
        assert_eq!(
            machine.command("5003"),
            format!("5003||5003: {control:02X}|"),
            "rate {rate}: the processor reads CONTROL back"
        );
    }

    // A rate the host never sends, and the answer to it. Zero is not a
    // rate, so it gets the same answer as a rate the table cannot name:
    // the whole external configuration, which is the one thing that is
    // true about a bit clock nobody has described.
    machine.set_rate(0);
    machine.run(64);
    let control = machine
        .sim
        .get(machine.acia_control)
        .to_u64()
        .expect("CONTROL is driven");
    assert_eq!(
        control, 0x00,
        "a rate of zero is no generator and no rate, not a generator with no rate"
    );
}

#[test]
fn the_processor_runs_at_one_cycle_in_fifty_nine() {
    let Some(dir) = example() else { return };
    // The design's own divider, which `monitor_bench` sets to one so
    // that every other test is fifty-nine times shorter. Here it is what
    // is being measured, so it is the board's value.
    let design = bench_design(&dir, "monitor_bench");
    let mut fast = Machine::boot(&design, 115_200);
    fast.settle();
    let quick = fast.clocks;
    println!("the prompt takes {quick} clocks with one clock per bus cycle");
    println!(
        "on the board that is about {} clocks, {CPU_DIV} times more, which at \
         60 MHz is {} ms",
        quick * CPU_DIV,
        (quick * CPU_DIV) / 60_000
    );
    assert!(quick > 0);
}

#[test]
fn a_partly_filled_packet_goes_when_the_machine_falls_silent() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_bench");
    let mut machine = Machine::boot(&design, 115_200);

    // The prompt is two bytes, which is not a packet. `in_commit` has to
    // go high after the silence, or the backslash would sit in the
    // endpoint until sixty-two more characters joined it — which for a
    // machine waiting for a keystroke is for ever.
    let mut committed = false;
    for _ in 0..100_000 {
        machine.tick();
        if high(&machine.sim, machine.print_commit) {
            committed = true;
            break;
        }
    }
    assert!(
        committed,
        "nothing committed the prompt's two bytes, so the host would never see them;          printed {:?}",
        machine.flat()
    );
    assert_eq!(
        machine.flat(),
        "\\|",
        "and it committed after the prompt rather than in the middle of it"
    );
}

// ---------------------------------------------------------------------------
// The project, and the part
// ---------------------------------------------------------------------------

#[test]
fn the_project_resolves_and_elaborates() {
    let Some(dir) = example() else { return };
    let built = build(&dir, |_| {});
    assert_eq!(built.project.name, "mos6502_monitor");
    assert_eq!(built.project.top.as_deref(), Some("monitor_cynthion"));
    assert_eq!(built.project.device.as_deref(), Some(DEVICE));
    assert_eq!(built.project.sources.len(), 5, "five files in rtl/");
    assert_eq!(
        built.project.depends.len(),
        5,
        "three library packages, and where the two they depend on live"
    );
    assert!(
        built.elaboration.blackboxes.is_empty(),
        "a module nothing declares: {:?}",
        built.elaboration.blackboxes
    );
    let design = built.elaboration.design.expect("it elaborates");
    assert!(design.top.is_some());
}

/// Everything synthesis said at warning level or above.
fn complaints(diags: &Diagnostics) -> Vec<String> {
    diags
        .iter()
        .filter(|d| d.severity >= Severity::Warning)
        .map(|d| format!("{}: {}", d.severity, d.message))
        .collect()
}

/// Synthesis has no error, no warning and no latch.
#[test]
fn the_machine_synthesises_without_errors_or_latches() {
    let Some(dir) = example() else { return };
    let built = build(&dir, |_| {});
    let mut design = built.elaboration.design.expect("it elaborates");
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

/// The ECP5 flow fits `monitor_cynthion` on the part the board has.
///
/// The numbers are printed rather than pinned, because a footprint that
/// is asserted to the unit is a test that fails on every improvement.
/// What *is* asserted is what would make the design not exist: that it
/// fits, that nothing generic survived, and that no memory was left as a
/// memory — the last because a memory the flow declined to lower is a
/// design that maps and then cannot be placed.
#[test]
fn the_machine_maps_onto_the_ecp5_and_fits_the_part() {
    let Some(dir) = example() else { return };
    let built = build(&dir, |_| {});
    let mut design = built.elaboration.design.expect("it elaborates");
    let top = design.top.expect("a top");
    let device = fpga::target(DEVICE).expect("the Cynthion's ECP5 is a built-in device");
    let rcf = read(&dir, "board/cynthion.rcf");
    let mut map = SourceMap::new();
    let file = map
        .add("board/cynthion.rcf", rcf.clone())
        .expect("the constraints fit");
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(&rcf, file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    constraints.check(&design, device, &mut diags);
    // Warnings and not errors: Reticle's pin list for this package is
    // partial, so it says so about every ULPI ball rather than pretending
    // to have checked them. The ball map that matters is the fabric
    // database's, and `reticle fpga --bitstream` uses that.
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let mut diags = Diagnostics::new();

    let flow = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap_or_else(|e| panic!("the ECP5 flow failed: {e:?}"));

    println!("{DEVICE}:");
    for (cell, count) in &flow.netlist {
        println!("  {count:>5} x {cell}");
    }
    println!("  {} LUTs, depth {}", flow.luts, flow.lut_depth);
    for fallback in &flow.primitives.bram_fallbacks {
        println!(
            "  memory {} -> {} ({} cell(s)), built {}",
            fallback.memory, fallback.style, fallback.cells, fallback.built
        );
    }

    let unbuilt: Vec<&str> = flow
        .primitives
        .bram_fallbacks
        .iter()
        .filter(|f| !f.built)
        .map(|f| f.memory.as_str())
        .collect();
    assert!(
        unbuilt.is_empty(),
        "a memory was left as a memory, so this cannot be placed: {unbuilt:?}"
    );
    assert_eq!(
        flow.count("DP16KD"),
        0,
        "a block RAM was chosen, and this fabric has no site for one"
    );

    let luts = flow.count("LUT4");
    assert!(luts > 3000, "{luts} LUTs is too few for a whole computer");
    assert!(
        luts <= 12_144,
        "{luts} LUT4 does not fit the LFE5U-12F's 12 144"
    );
    let rams = flow.count("TRELLIS_DPR16X4");
    assert!(
        rams <= 3036,
        "{rams} distributed RAMs does not fit the part's 3036"
    );
    assert!(
        flow.netlist.iter().all(|(cell, _)| !cell.starts_with('$')),
        "a generic cell survived: {:?}",
        flow.netlist
    );
}

/// What the ROM costs, in the only currency this part has.
///
/// The monitor is 266 bytes of code and six of vectors, which did not fit
/// the one page the interface it reproduces manages, so the ROM is two.
/// This is the measurement that makes that a decision rather than a
/// drift: it maps `monitor_rom` on its own and prints the lookup tables
/// it takes, and asserts only that it is built from logic and not from
/// storage — because a ROM that became a memory on this fabric could not
/// be placed at all.
///
/// What it would not catch: whether the *rest* of the design still fits
/// once the ROM has grown. `the_machine_maps_onto_the_ecp5_and_fits_the_part`
/// is that, and it asserts the total.
#[test]
fn the_rom_is_lookup_tables_and_this_is_what_they_cost() {
    let Some(dir) = example() else { return };
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let text = read(&dir, "rtl/monitor_rom.v");
    let id = map.add("rtl/monitor_rom.v", text).expect("the ROM fits");
    let file = reticle::verilog::parse_source(
        &mut map,
        id,
        reticle::verilog::Dialect::Verilog2005,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let options = reticle::verilog::ElabOptions::new(reticle::verilog::Dialect::Verilog2005)
        .with_top("monitor_rom");
    let mut design =
        reticle::verilog::elaborate(&[&file], &options, &mut diags).expect("the ROM elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let top = design.top.expect("a top");

    let device = fpga::target(DEVICE).expect("the Cynthion's ECP5 is a built-in device");
    let constraints = Constraints::default();
    let flow = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap_or_else(|e| panic!("the ECP5 flow failed: {e:?}"));

    println!(
        "monitor_rom: {} LUT4, depth {}, for {ROM_BYTES} bytes",
        flow.count("LUT4"),
        flow.lut_depth
    );
    println!(
        "  which is {:.1} lookup tables a byte",
        flow.count("LUT4") as f64 / f64::from(ROM_BYTES)
    );
    assert_eq!(
        flow.count("TRELLIS_DPR16X4"),
        0,
        "the ROM became storage, which cannot hold a constant on this flow"
    );
    assert_eq!(
        flow.count("DP16KD"),
        0,
        "the ROM became a block RAM, and this fabric has no site for one"
    );
    assert_eq!(flow.count("TRELLIS_FF"), 0, "a ROM has nothing to remember");
    assert!(flow.count("LUT4") > 0, "a ROM of constants is still logic");
}

/// The session in `tb/monitor_tb.v`, through the simulator.
///
/// The Verilog testbench types the same commands the Rust ones do and
/// prints the whole transcript with `$display`, so this is the same
/// machine driven by a second thing — and the second thing is the one
/// `reticle sim` runs, which is what a person reaching for this example
/// will type first.
///
/// What it would not catch: anything about the USB stack, which is not
/// in this run either.
#[test]
fn the_testbench_session_comes_out_of_the_simulator() {
    let Some(dir) = example() else { return };
    let design = bench_design(&dir, "monitor_tb");
    let mut sim = Simulator::new(&design, SimOptions::default())
        .unwrap_or_else(|d| panic!("the testbench does not simulate: {}", d.len()));
    sim.run();
    assert!(sim.finished(), "the testbench did not reach $finish");
    let out = sim.output();
    println!("{out}");

    // The prompt, the echo, and the ROM's own first instruction.
    let image = monitor(&dir);
    let bytes = rom_bytes(&image);
    let want = format!(
        "\\|FE00||FE00: {:02X}|FFFA.FFFF||FFFA:{}|HELLO|\\|",
        bytes[0],
        (0x1FA..=0x1FFusize)
            .map(|offset| format!(" {:02X}", bytes[offset]))
            .collect::<String>()
    );
    assert!(
        out.contains(&want),
        "the session is not what the monitor should have said\n  want {want:?}\n  got  {out:?}"
    );

    // 115200 is not a rate the 65C51's four baud bits can name, so the
    // ACIA reports the whole external configuration — no generator
    // selected and no rate — with the word length and stop bits the
    // monitor wrote left alone, and the host's own number on `acia_rate`.
    assert!(
        out.contains("acia_control=00 acia_rate=115200"),
        "the ACIA did not report the host's rate in {out:?}"
    );

    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert!(messages.is_empty(), "simulator messages: {messages:?}");
}
