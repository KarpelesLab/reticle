//! The JTAG programmer, checked against a model of a TAP rather than
//! against a board.
//!
//! The unit tests next to the code check each encoding on its own. What
//! is here is the other half: a small, independent **interpreter** for
//! the MPSSE command stream driving a model IEEE 1149.1 TAP, so that
//! `reticle::program`'s output is judged by what a part would do with
//! it, not by what bytes it happens to contain.
//!
//! That matters because the two mistakes this code can make are exactly
//! the two a byte-level test cannot see:
//!
//! - an off-by-one in a shift length, which leaves the TAP one bit out
//!   of step and every later scan reading plausible nonsense;
//! - a bit order backwards, which produces a stream that is the right
//!   length and the right shape and that the part ignores.
//!
//! The model is written from the standard, not from
//! `reticle::program::jtag`: it decodes the opcodes by their documented
//! bit fields and walks the state machine from its own table. It shares
//! [`TapState`] with the crate, since re-declaring sixteen state names
//! would test nothing.
//!
//! One test needs the board and is `#[ignore]`d; `cargo test -- --ignored
//! program_a_real_part` runs it. Everything else runs anywhere.

#![cfg(feature = "program")]

use reticle::program::jtag::TapState;
use reticle::program::{ftdi, xilinx};

// ---------------------------------------------------------------------
// A model TAP, and an interpreter for the MPSSE command stream
// ---------------------------------------------------------------------

/// A device on the far end of the cable: a TAP with an instruction
/// register, a data register and something to shift out of TDO.
struct Part {
    state: TapState,
    /// The instruction currently latched, as the six bits `Update-IR`
    /// last captured.
    instruction: u8,
    /// Bits shifted into IR since `Capture-IR`, first bit first.
    ir_shifted: Vec<bool>,
    /// Bits shifted into DR since `Capture-DR`, first bit first.
    dr_shifted: Vec<bool>,
    /// Every DR value latched at `Update-DR`, in order.
    dr_updates: Vec<Vec<bool>>,
    /// Every instruction latched at `Update-IR`, in order.
    ir_updates: Vec<u8>,
    /// What TDO presents while shifting DR, first bit first.
    dr_out: Vec<bool>,
    /// How far through `dr_out` the shift has got.
    dr_out_pos: usize,
    /// The 32-bit `IDCODE` this part answers with.
    idcode: u32,
    /// The value `CFG_OUT` shifts out, most-significant bit first.
    cfg_out: u32,
    /// How many TCK have been clocked in `Run-Test/Idle`.
    idle_clocks: usize,
}

impl Part {
    fn new(idcode: u32, cfg_out: u32) -> Part {
        Part {
            // A real TAP powers up in Test-Logic-Reset. Starting
            // somewhere else would be a fairer test of `reset`, and the
            // dedicated test below does exactly that.
            state: TapState::TestLogicReset,
            instruction: xilinx::Instruction::Bypass.bits(),
            ir_shifted: Vec::new(),
            dr_shifted: Vec::new(),
            dr_updates: Vec::new(),
            ir_updates: Vec::new(),
            dr_out: Vec::new(),
            dr_out_pos: 0,
            idcode,
            cfg_out,
            idle_clocks: 0,
        }
    }

    /// One TCK. Returns what TDO presented during it.
    fn clock(&mut self, tms: bool, tdi: bool) -> bool {
        let tdo = match self.state {
            TapState::ShiftDr => {
                let bit = self.dr_out.get(self.dr_out_pos).copied().unwrap_or(false);
                self.dr_out_pos += 1;
                self.dr_shifted.push(tdi);
                bit
            }
            TapState::ShiftIr => {
                self.ir_shifted.push(tdi);
                // A 7-series TAP captures `01` into the low two bits of
                // IR; nothing here reads it, so zero will do.
                false
            }
            _ => false,
        };
        if self.state == TapState::RunTestIdle {
            self.idle_clocks += 1;
        }

        let next = self.state.step(tms);
        match next {
            TapState::CaptureIr => self.ir_shifted.clear(),
            TapState::CaptureDr => {
                self.dr_shifted.clear();
                self.dr_out = self.capture_dr();
                self.dr_out_pos = 0;
            }
            TapState::UpdateIr => {
                let mut value = 0u8;
                for (i, bit) in self.ir_shifted.iter().take(8).enumerate() {
                    if *bit {
                        value |= 1 << i;
                    }
                }
                self.instruction = value;
                self.ir_updates.push(value);
            }
            TapState::UpdateDr => self.dr_updates.push(self.dr_shifted.clone()),
            TapState::TestLogicReset => {
                // IEEE 1149.1: the reset state loads IDCODE (or BYPASS
                // on a part with no IDCODE register).
                self.instruction = xilinx::Instruction::Idcode.bits();
            }
            _ => {}
        }
        self.state = next;
        tdo
    }

    /// What this part presents on TDO for the latched instruction.
    fn capture_dr(&self) -> Vec<bool> {
        if self.instruction == xilinx::Instruction::Idcode.bits() {
            // A JTAG IDCODE shifts out least-significant bit first.
            (0..32).map(|i| self.idcode >> i & 1 == 1).collect()
        } else if self.instruction == xilinx::Instruction::CfgOut.bits() {
            // A configuration register shifts out most-significant bit
            // first. The opposite convention, on purpose.
            (0..32).map(|i| self.cfg_out >> (31 - i) & 1 == 1).collect()
        } else {
            Vec::new()
        }
    }

    /// The `i`-th DR value latched at `Update-DR`, as bytes packed
    /// least-significant bit first.
    fn dr_update_bytes(&self, i: usize) -> Vec<u8> {
        let bits = &self.dr_updates[i];
        let mut out = vec![0u8; bits.len().div_ceil(8)];
        for (i, bit) in bits.iter().enumerate() {
            if *bit {
                out[i / 8] |= 1 << (i % 8);
            }
        }
        out
    }
}

/// Runs an MPSSE command stream against a [`Part`] and returns the bytes
/// the adapter would have sent back, decoded from the opcodes' documented
/// bit fields.
///
/// `tms` starts high, the level [`reticle::program::BASYS3_PINS`] leaves
/// the pin at; a data command does not touch TMS, so whatever the last
/// TMS command left is what the TAP sees.
fn interpret(commands: &[u8], part: &mut Part) -> Vec<u8> {
    let mut reply = Vec::new();
    let mut tms_level = true;
    let mut i = 0;
    while i < commands.len() {
        let opcode = commands[i];
        i += 1;
        match opcode {
            // No operands.
            ftdi::CMD_LOOPBACK_OFF
            | ftdi::CMD_SEND_IMMEDIATE
            | ftdi::CMD_DISABLE_DIV5
            | ftdi::CMD_DISABLE_3PHASE
            | ftdi::CMD_DISABLE_ADAPTIVE => {}
            // Two operands.
            ftdi::CMD_SET_BITS_LOW | ftdi::CMD_SET_BITS_HIGH => {
                if opcode == ftdi::CMD_SET_BITS_LOW {
                    tms_level = commands[i] & 0b1000 != 0;
                }
                i += 2;
            }
            ftdi::CMD_SET_DIVISOR => i += 2,
            // Byte shifts: a 16-bit length of N-1, then N bytes.
            ftdi::CMD_BYTES_OUT | ftdi::CMD_BYTES_INOUT => {
                let n = usize::from(u16::from_le_bytes([commands[i], commands[i + 1]])) + 1;
                i += 2;
                for k in 0..n {
                    let byte = commands[i + k];
                    let mut got = 0u8;
                    for b in 0..8 {
                        let tdo = part.clock(tms_level, byte >> b & 1 == 1);
                        // The MPSSE shifts a captured bit in at bit 7
                        // and moves the byte right.
                        got >>= 1;
                        if tdo {
                            got |= 0x80;
                        }
                    }
                    if opcode == ftdi::CMD_BYTES_INOUT {
                        reply.push(got);
                    }
                }
                i += n;
            }
            // Bit shifts: a one-byte length of N-1, then one data byte.
            ftdi::CMD_BITS_OUT | ftdi::CMD_BITS_INOUT => {
                let n = usize::from(commands[i]) + 1;
                let byte = commands[i + 1];
                i += 2;
                let mut got = 0u8;
                for b in 0..n {
                    let tdo = part.clock(tms_level, byte >> b & 1 == 1);
                    got >>= 1;
                    if tdo {
                        got |= 0x80;
                    }
                }
                if opcode == ftdi::CMD_BITS_INOUT {
                    reply.push(got);
                }
            }
            // TMS shifts: TMS in the low bits, TDI held in bit 7.
            ftdi::CMD_TMS_OUT | ftdi::CMD_TMS_INOUT => {
                let n = usize::from(commands[i]) + 1;
                let byte = commands[i + 1];
                i += 2;
                let tdi = byte & 0x80 != 0;
                let mut got = 0u8;
                for b in 0..n {
                    tms_level = byte >> b & 1 == 1;
                    let tdo = part.clock(tms_level, tdi);
                    got >>= 1;
                    if tdo {
                        got |= 0x80;
                    }
                }
                if opcode == ftdi::CMD_TMS_INOUT {
                    reply.push(got);
                }
            }
            other => panic!("the model does not know MPSSE opcode {other:#04x}"),
        }
    }
    reply
}

// ---------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------

/// The initialisation job drives a TAP that started anywhere at all into
/// `Test-Logic-Reset` and leaves it resting in `Run-Test/Idle`.
#[test]
fn the_reset_walk_works_from_every_state() {
    for start in [
        TapState::TestLogicReset,
        TapState::RunTestIdle,
        TapState::ShiftDr,
        TapState::ShiftIr,
        TapState::PauseDr,
        TapState::Exit2Ir,
        TapState::UpdateDr,
        TapState::SelectIr,
    ] {
        let mut part = Part::new(xilinx::IDCODE_XC7A35T, 0);
        part.state = start;
        let (pins, dirs) = reticle::program::BASYS3_PINS;
        let job = xilinx::init_job(reticle::program::default_divisor(), pins, dirs);
        let reply = interpret(job.commands(), &mut part);
        assert!(reply.is_empty(), "the init job captures nothing");
        assert_eq!(
            part.state,
            TapState::RunTestIdle,
            "reset from {start:?} must end in Run-Test/Idle"
        );
    }
}

/// An `IDCODE` read shifts the right instruction and decodes the part's
/// answer. The model shifts the register out least-significant bit
/// first, as IEEE 1149.1 says; decoding it the other way round would
/// give `0x93D06203`, which is the bug this pins down.
#[test]
fn an_idcode_read_decodes_the_parts_answer() {
    let mut part = Part::new(xilinx::IDCODE_XC7A35T, 0);
    let job = xilinx::idcode_job();
    let reply = interpret(job.commands(), &mut part);

    assert_eq!(reply.len(), job.read_len());
    assert_eq!(
        part.ir_updates,
        [xilinx::Instruction::Idcode.bits()],
        "one instruction, and it is IDCODE"
    );
    let read = job.capture_u32(0, &reply).expect("decodes");
    assert_eq!(read, xilinx::IDCODE_XC7A35T);
    assert!(xilinx::idcode_matches(read, xilinx::IDCODE_XC7A35T));
    assert_ne!(read, xilinx::IDCODE_XC7A35T.swap_bytes());
    assert_ne!(read, xilinx::IDCODE_XC7A35T.reverse_bits());

    // A part with a revision nibble still matches, and a different part
    // does not.
    let mut other = Part::new(0x2362_D093, 0);
    let reply = interpret(job.commands(), &mut other);
    let read = job.capture_u32(0, &reply).expect("decodes");
    assert_eq!(xilinx::idcode_revision(read), 2);
    assert!(xilinx::idcode_matches(read, xilinx::IDCODE_XC7A35T));

    let mut wrong = Part::new(0x0362_C093, 0);
    let reply = interpret(job.commands(), &mut wrong);
    let read = job.capture_u32(0, &reply).expect("decodes");
    assert!(!xilinx::idcode_matches(read, xilinx::IDCODE_XC7A35T));
}

/// A status read sends `CFG_IN` with the read packet, then `CFG_OUT`,
/// and decodes a register that comes back **most**-significant bit
/// first — the opposite of `IDCODE`, through the same cable.
#[test]
fn a_status_read_decodes_the_opposite_bit_order() {
    // DONE, EOS, INIT_COMPLETE and INIT_B, with the mode pins on JTAG.
    let stat = (1 << 14) | (1 << 4) | (1 << 11) | (1 << 12) | (0b101 << 8);
    let mut part = Part::new(xilinx::IDCODE_XC7A35T, stat);
    let job = xilinx::status_job();
    let reply = interpret(job.commands(), &mut part);

    assert_eq!(
        part.ir_updates,
        [
            xilinx::Instruction::CfgIn.bits(),
            xilinx::Instruction::CfgOut.bits()
        ]
    );
    let status = xilinx::read_status(&job, &reply).expect("decodes");
    assert_eq!(status.0, stat);
    assert!(status.done());
    assert!(status.end_of_startup());
    assert!(status.init_b());
    assert!(!status.crc_error());

    // The `CFG_IN` payload that reached the part must be the read
    // packet in configuration order: the model latched the DR bits, so
    // turning them back into words is the round trip.
    let packed = part.dr_update_bytes(0);
    let words: Vec<u32> = packed
        .chunks(4)
        .map(|c| {
            u32::from_be_bytes([
                c[0].reverse_bits(),
                c[1].reverse_bits(),
                c[2].reverse_bits(),
                c[3].reverse_bits(),
            ])
        })
        .collect();
    assert_eq!(words, xilinx::register_read_words(xilinx::REG_STAT));
    assert_eq!(words[1], xilinx::SYNC_WORD);
}

/// The configuration payload arrives at the part in file order,
/// most-significant bit first, bit for bit.
///
/// This is the test the milestone rests on: everything else can be
/// right and this one thing backwards, and the part will accept the
/// stream's shape and configure nothing.
#[test]
fn the_configuration_payload_arrives_most_significant_bit_first() {
    // A payload whose every byte is asymmetric, so a reversal anywhere
    // is visible.
    let payload: Vec<u8> = (0..64u8)
        .map(|i| i.wrapping_mul(7).wrapping_add(1))
        .collect();
    let mut part = Part::new(xilinx::IDCODE_XC7A35T, 0);
    let job = xilinx::configure_job(&payload);
    let reply = interpret(job.commands(), &mut part);

    assert!(reply.is_empty(), "configuring captures nothing");
    assert_eq!(part.ir_updates, [xilinx::Instruction::CfgIn.bits()]);
    assert_eq!(part.dr_updates.len(), 1);

    let arrived = &part.dr_updates[0];
    assert_eq!(arrived.len(), payload.len() * 8);
    for (i, bit) in arrived.iter().enumerate() {
        let byte = payload[i / 8];
        // The first bit into the part is bit 7 of byte 0, then bit 6,
        // and so on.
        let expected = byte >> (7 - i % 8) & 1 == 1;
        assert_eq!(*bit, expected, "bit {i} of the payload arrived wrong");
    }

    // And the two ways of getting it wrong really are different, so the
    // check above has teeth.
    let reversed_bytes: Vec<bool> = payload
        .iter()
        .rev()
        .flat_map(|b| (0..8).map(move |k| b >> (7 - k) & 1 == 1))
        .collect();
    assert_ne!(*arrived, reversed_bytes);
    let lsb_first: Vec<bool> = payload
        .iter()
        .flat_map(|b| (0..8).map(move |k| b >> k & 1 == 1))
        .collect();
    assert_ne!(*arrived, lsb_first);
}

/// A real bitstream's first words reach the part unchanged: the dummy
/// pad, the sync word, and the first packet after it.
#[test]
fn a_bitstream_payload_keeps_its_sync_word() {
    let mut payload = vec![0xFFu8; 16];
    payload.extend_from_slice(&xilinx::SYNC_WORD.to_be_bytes());
    payload.extend_from_slice(&0x2000_0000u32.to_be_bytes());
    payload.extend_from_slice(&0x3000_8001u32.to_be_bytes());
    payload.extend_from_slice(&0x0362_D093u32.to_be_bytes());

    let mut part = Part::new(xilinx::IDCODE_XC7A35T, 0);
    let job = xilinx::configure_job(&payload);
    interpret(job.commands(), &mut part);

    // Rebuild the words the part saw, most-significant bit first.
    let bits = &part.dr_updates[0];
    let words: Vec<u32> = bits
        .chunks(32)
        .map(|c| {
            c.iter()
                .enumerate()
                .fold(0u32, |w, (i, b)| if *b { w | 1 << (31 - i) } else { w })
        })
        .collect();
    assert_eq!(words[4], xilinx::SYNC_WORD);
    assert_eq!(words[5], 0x2000_0000);
    assert_eq!(words[6], 0x3000_8001);
    assert_eq!(words[7], 0x0362_D093);
}

/// `JPROGRAM` is shifted, the instruction register is left on
/// `ISC_NOOP`, and the part is clocked in `Run-Test/Idle` while it
/// erases. Too few idle clocks is a configuration that starts before the
/// memory is clear.
#[test]
fn the_erase_job_shifts_jprogram_and_then_waits() {
    let mut part = Part::new(xilinx::IDCODE_XC7A35T, 0);
    let job = xilinx::erase_job();
    let reply = interpret(job.commands(), &mut part);
    assert!(reply.is_empty());
    assert_eq!(
        part.ir_updates,
        [
            xilinx::Instruction::JProgram.bits(),
            xilinx::Instruction::IscNoop.bits()
        ]
    );
    assert!(
        part.idle_clocks >= xilinx::ERASE_IDLE_CYCLES,
        "only {} idle clocks",
        part.idle_clocks
    );
}

/// `JSTART` is shifted and the startup clocks follow it, then `BYPASS`
/// so nothing is left selected.
#[test]
fn the_start_job_shifts_jstart_and_clocks_the_startup() {
    let mut part = Part::new(xilinx::IDCODE_XC7A35T, 0);
    let job = xilinx::start_job();
    interpret(job.commands(), &mut part);
    assert_eq!(
        part.ir_updates,
        [
            xilinx::Instruction::JStart.bits(),
            xilinx::Instruction::Bypass.bits()
        ]
    );
    assert!(part.idle_clocks >= xilinx::STARTUP_IDLE_CYCLES);
    assert_eq!(part.instruction, xilinx::Instruction::Bypass.bits());
}

/// The whole sequence, in the order `reticle program` runs it, against
/// one model part: reset, IDCODE, status, erase, configure, start. Every
/// instruction the part sees is accounted for.
#[test]
fn the_whole_sequence_reaches_the_part_in_order() {
    let mut part = Part::new(xilinx::IDCODE_XC7A35T, 0);
    let (pins, dirs) = reticle::program::BASYS3_PINS;
    let payload = vec![0x5Au8; 128];

    let init = xilinx::init_job(reticle::program::default_divisor(), pins, dirs);
    interpret(init.commands(), &mut part);

    let idcode = xilinx::idcode_job();
    let reply = interpret(idcode.commands(), &mut part);
    assert_eq!(
        idcode.capture_u32(0, &reply).unwrap(),
        xilinx::IDCODE_XC7A35T
    );

    let status = xilinx::status_job();
    let _ = interpret(status.commands(), &mut part);
    interpret(xilinx::erase_job().commands(), &mut part);
    interpret(xilinx::configure_job(&payload).commands(), &mut part);
    interpret(xilinx::start_job().commands(), &mut part);

    use xilinx::Instruction as I;
    assert_eq!(
        part.ir_updates,
        [
            I::Idcode.bits(),
            I::CfgIn.bits(),
            I::CfgOut.bits(),
            I::JProgram.bits(),
            I::IscNoop.bits(),
            I::CfgIn.bits(),
            I::JStart.bits(),
            I::Bypass.bits(),
        ]
    );
    assert_eq!(part.state, TapState::RunTestIdle);
    // The configuration payload is the second CFG_IN scan's DR.
    assert_eq!(part.dr_updates.len(), 4);
    assert_eq!(part.dr_updates[3].len(), payload.len() * 8);
}

/// The container reader agrees with `fpga::xc7`'s, which parses the same
/// file and checks its CRCs as well. Two independent readers landing on
/// the same payload is what says the payload boundary is right.
#[test]
#[cfg(feature = "fpga")]
fn the_two_container_readers_agree() {
    let Some(path) = std::env::var_os("RETICLE_BITSTREAM") else {
        println!("RETICLE_BITSTREAM is not set, skipping");
        return;
    };
    let bytes = std::fs::read(&path).expect("the bitstream named by RETICLE_BITSTREAM");
    let mine = reticle::program::read_bit_container(&bytes).expect("reads");
    let theirs = reticle::fpga::xc7::read_bit(&bytes).expect("reads");
    assert_eq!(mine.part, theirs.header.part);
    assert_eq!(mine.design, theirs.header.design);
    assert_eq!(mine.date, theirs.header.date);
    assert_eq!(mine.time, theirs.header.time);
    // Every FDRI word of the stream must be inside what this reader
    // hands the part.
    assert!(mine.length > theirs.frames.len() * 4);
    assert_eq!(mine.offset + mine.length, bytes.len());
}

/// Loads a bitstream into a board that is actually attached.
///
/// Set `RETICLE_BITSTREAM` to a `.bit` for the part, and
/// `RETICLE_ADAPTER` to a serial number if more than one adapter is
/// plugged in. Everything this does is undone by a power cycle: it
/// writes the part's volatile configuration memory and never its flash.
#[test]
#[ignore = "needs an FPGA board on USB; set RETICLE_BITSTREAM"]
fn program_a_real_part() {
    use reticle::program::usb;

    let Some(path) = std::env::var_os("RETICLE_BITSTREAM") else {
        println!("RETICLE_BITSTREAM is not set, skipping");
        return;
    };
    let bytes = std::fs::read(&path).expect("the bitstream named by RETICLE_BITSTREAM");
    let bit = reticle::program::read_bit_container(&bytes).expect("a .bit file");
    println!(
        "{}: {} for {}",
        path.to_string_lossy(),
        bit.design,
        bit.part
    );

    let serial = std::env::var("RETICLE_ADAPTER").ok();
    let cable = match usb::Cable::open(serial.as_deref()) {
        Ok(cable) => cable,
        Err(err) => {
            println!("no adapter to program: {err}, skipping");
            return;
        }
    };
    let (pins, dirs) = reticle::program::BASYS3_PINS;
    cable
        .run(&xilinx::init_job(
            reticle::program::default_divisor(),
            pins,
            dirs,
        ))
        .expect("the MPSSE accepts its setup");

    let job = xilinx::idcode_job();
    let idcode = job
        .capture_u32(0, &cable.run(&job).expect("the IDCODE scan"))
        .expect("decodes");
    println!("IDCODE {idcode:#010x}");
    assert!(
        xilinx::idcode_matches(idcode, xilinx::IDCODE_XC7A35T),
        "the part answered {idcode:#010x}, not an XC7A35T"
    );

    let read_status = || {
        let job = xilinx::status_job();
        let reply = cable.run(&job).expect("the status scan");
        xilinx::read_status(&job, &reply).expect("decodes")
    };
    println!("status before: {}", read_status());

    cable.run(&xilinx::erase_job()).expect("JPROGRAM");
    let erased = read_status();
    println!("status after JPROGRAM: {erased}");
    assert!(!erased.done(), "DONE should be low after JPROGRAM");

    cable
        .run(&xilinx::configure_job(bit.payload(&bytes)))
        .expect("the configuration shift");
    cable.run(&xilinx::start_job()).expect("JSTART");

    let after = read_status();
    println!("status after JSTART: {after}");
    assert!(after.done(), "DONE did not assert; status {after}");
    assert!(!after.crc_error(), "the part reported a CRC error");
    assert!(!after.id_error(), "the part reported an ID error");
}
