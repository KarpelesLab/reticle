//! Naming a Lattice part from its `IDCODE`, and the ECP5's volatile
//! configuration sequence.
//!
//! Two halves, and the first is much the older. Naming a part is a table
//! and four bit fields; configuring one is a walk through the ECP5's
//! in-system configuration instructions with a bitstream shifted through
//! the data register. Both are **sans-I/O**: what is here builds
//! [`jtag::Plan`]s and decodes replies, and a transport — an FTDI cable
//! or a Cynthion's Apollo microcontroller — is what puts them on a wire.
//!
//! # What has been run on a part
//!
//! The naming half has been exercised against a real part many times: a
//! Great Scott Gadgets Cynthion's LFE5U-12F answers `0x21111043` and
//! `tests/program_apollo.rs` reads it.
//!
//! What the configuration half has been run against is recorded in
//! `docs/programming.md`, under "Configuring an ECP5", and nowhere else:
//! this file describes the sequence and claims nothing about where it has
//! been. Nothing here writes a flash and nothing here can. Every
//! instruction in [`isc`] acts on the configuration **SRAM**, a power
//! cycle undoes all of it, and the instruction that turns the part into a
//! pass-through to the board's SPI flash is named in [`NOT_SHIFTED`] and
//! is never sent.
//!
//! # An `IDCODE`
//!
//! An `IDCODE` is the same shape for every vendor — IEEE 1149.1 clause
//! 12 — so the only vendor-specific part is what the device field means:
//!
//! | Bits | Field |
//! |---|---|
//! | 31..28 | version |
//! | 27..12 | part number |
//! | 11..1 | JEDEC manufacturer |
//! | 0 | always 1 |
//!
//! Lattice parts are matched on the **whole 32-bit value** rather than
//! on the low 28 bits the way [`super::xilinx::idcode_matches`] matches
//! a Xilinx part. That is not an oversight. On an ECP5 the top nibble is
//! not a silicon revision: `0x2…` is the LFE5U-12F, `0x4…` the LFE5U
//! parts above it, `0x8…` the LFE5UM5G and `0x0…` the LFE5UM, and three
//! of those share the device field `0x1111`. Masking the nibble off
//! would merge four different parts.

use std::fmt;

use super::jtag::{self, Plan, TapState};

/// The 11-bit JEDEC manufacturer identity of Lattice Semiconductor:
/// JEP106 bank 1, code `0x21`, which lands in `IDCODE` bits 11..1.
pub const MANUFACTURER_LATTICE: u32 = 0x021;

/// The low twelve bits of every Lattice `IDCODE`: the manufacturer
/// field with IEEE 1149.1's mandatory `1` under it.
pub const LATTICE_IDCODE_TAIL: u32 = (MANUFACTURER_LATTICE << 1) | 1;

/// The 11-bit JEDEC manufacturer field of an `IDCODE`.
#[must_use]
pub fn manufacturer(idcode: u32) -> u32 {
    (idcode >> 1) & 0x7FF
}

/// The 16-bit part number field of an `IDCODE` (bits 27..12).
#[must_use]
pub fn part_number(idcode: u32) -> u32 {
    (idcode >> 12) & 0xFFFF
}

/// The version field of an `IDCODE` (bits 31..28).
#[must_use]
pub fn version(idcode: u32) -> u32 {
    idcode >> 28
}

/// True when an `IDCODE` names a Lattice part.
///
/// This is only the manufacturer field and the mandatory bit, so it says
/// who made the part and nothing about which one it is. It is also the
/// check that tells a real answer from a dead chain: an undriven TDO
/// reads as all ones or all zeros, and neither of those is a Lattice
/// identifier.
#[must_use]
pub fn is_lattice(idcode: u32) -> bool {
    idcode & 0xFFF == LATTICE_IDCODE_TAIL
}

/// Every ECP5 `IDCODE` this crate can name, with the part it belongs
/// to.
///
/// These are the values published for the family: they appear in
/// Lattice's own BSDL files for each part and, identically, in every
/// open tool that speaks to an ECP5 — OpenOCD's `src/flash/nor/lattice.c`
/// device table and Project Trellis' device database, which were the two
/// independent sources checked against each other while writing this.
/// Where a part has an automotive sibling with the same identifier, both
/// names are given, because the identifier genuinely cannot tell them
/// apart.
///
/// HIGH confidence in the numbers. One of these parts **has** been
/// configured by Reticle — the LFE5U-12F of a Cynthion r1.4, which
/// `docs/programming.md` records — and none of the other nine has; this
/// table claims nothing either way, it only names them.
pub const ECP5_PARTS: [(u32, &str); 10] = [
    (0x2111_1043, "LFE5U-12F (or LAE5U-12F)"),
    (0x4111_1043, "LFE5U-25F"),
    (0x4111_2043, "LFE5U-45F"),
    (0x4111_3043, "LFE5U-85F"),
    (0x0111_1043, "LFE5UM-25F (or LAE5UM-25F)"),
    (0x0111_2043, "LFE5UM-45F (or LAE5UM-45F)"),
    (0x0111_3043, "LFE5UM-85F (or LAE5UM-85F)"),
    (0x8111_1043, "LFE5UM5G-25F"),
    (0x8111_2043, "LFE5UM5G-45F"),
    (0x8111_3043, "LFE5UM5G-85F"),
];

/// The ECP5 an `IDCODE` names, or `None`.
#[must_use]
pub fn ecp5_part(idcode: u32) -> Option<&'static str> {
    ECP5_PARTS
        .iter()
        .find(|(id, _)| *id == idcode)
        .map(|(_, name)| *name)
}

/// A sentence describing an `IDCODE`, for a report.
///
/// It always names the raw value first and the reading second, so a
/// wrong reading cannot hide the number it was read from.
#[must_use]
pub fn describe(idcode: u32) -> String {
    let mut text = format!("IDCODE {idcode:#010x}");
    if !is_lattice(idcode) {
        text.push_str(&format!(
            " (manufacturer field {:#05x}, which is not Lattice's {MANUFACTURER_LATTICE:#05x})",
            manufacturer(idcode)
        ));
        return text;
    }
    text.push_str(&format!(
        ": manufacturer {:#05x} (Lattice), part number {:#06x}, version {}",
        manufacturer(idcode),
        part_number(idcode),
        version(idcode)
    ));
    match ecp5_part(idcode) {
        Some(part) => text.push_str(&format!(" — {part}")),
        None => text.push_str(" — a Lattice part this crate has no name for"),
    }
    text
}

// ---------------------------------------------------------------------
// Recognising an ECP5 .bit, without decoding it
// ---------------------------------------------------------------------

/// What an ECP5 `.bit` file says about itself.
///
/// This is deliberately **not** [`crate::fpga::ecp5::Ecp5Stream`], which
/// decodes every frame and needs to be told the part's frame geometry
/// from Project Trellis' `devices.json`. A programmer needs two things
/// from a file — is it one, and which part is it for — and neither needs
/// the geometry. Keeping it that way is what lets `reticle program` load
/// an ECP5 bitstream in a build with no fabric database anywhere near it,
/// exactly as [`super::gowin::fs_idcode`] reads a `.fs` without knowing
/// anything about Gowin's fabric.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitHeader {
    /// The NUL-terminated strings of the metadata header, in order.
    /// `ecppack` writes one, `Part: <part>-<speed><package>`.
    pub metadata: Vec<String>,
    /// The operand of the `VERIFY_ID` command, when the stream has one.
    ///
    /// `None` is possible and is not an error: `VERIFY_ID` is optional,
    /// and a stream without it asks the part to accept whatever it is
    /// given. A caller that wants the check has to make it itself.
    pub idcode: Option<u32>,
    /// Whether the payload is the compressed form (`LSC_PROG_INCR_CMP`)
    /// rather than the plain one (`LSC_PROG_INCR_RTI`).
    pub compressed: bool,
}

/// The four bytes that mark the start of an ECP5 command stream.
///
/// Duplicated from [`crate::fpga::ecp5::PREAMBLE`] rather than shared,
/// because `src/program` must compile with the `fpga` feature off. A test
/// asserts the two agree when both features are on, which is the only
/// thing a shared constant would have bought.
pub const BIT_PREAMBLE: [u8; 4] = [0xff, 0xff, 0xbd, 0xb3];

/// Whether `bytes` looks like an ECP5 `.bit` rather than some other
/// vendor's file of the same extension.
///
/// It is the metadata header and then the preamble, which a Xilinx `.bit`
/// does not have: that one opens with a big-endian length and a `0x0f
/// 0xf0` run. The whole check is "find the preamble after the strings",
/// because a lone `0xff 0x00` is not distinctive enough to act on.
#[must_use]
pub fn is_ecp5_bit(bytes: &[u8]) -> bool {
    read_bit_header(bytes).is_ok()
}

/// Why an ECP5 `.bit` could not be recognised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BitError {
    /// The file does not open with `0xff 0x00` (or `LSCC` and then that).
    NotABitFile,
    /// The metadata header runs off the end of the file.
    UnterminatedMetadata,
    /// The preamble is not where the metadata header says it should be.
    NoPreamble,
    /// The command stream ends in the middle of a command.
    Truncated,
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitError::NotABitFile => write!(
                f,
                "not an ECP5 bitstream: it does not start with 0xff 0x00 and a metadata header"
            ),
            BitError::UnterminatedMetadata => write!(
                f,
                "the metadata header is unterminated: no 0xff closes it before the file ends"
            ),
            BitError::NoPreamble => write!(
                f,
                "no ECP5 preamble (ff ff bd b3) follows the metadata header, so this is some \
                 other vendor's file"
            ),
            BitError::Truncated => write!(f, "the command stream ends in the middle of a command"),
        }
    }
}

impl std::error::Error for BitError {}

/// Reads what an ECP5 `.bit` says about itself, without decoding a single
/// frame.
///
/// # Errors
///
/// The variants of [`BitError`]. Note that the stream's *contents* are
/// not checked at all: this walks commands only far enough to find
/// `VERIFY_ID` and the payload command, then stops. A malformed payload
/// is the part's complaint to make, and `src/fpga/ecp5.rs` is what checks
/// one properly.
pub fn read_bit_header(bytes: &[u8]) -> Result<BitHeader, BitError> {
    let mut at = 0usize;
    // Some tools prefix `LSCC`; Lattice's own and `ecppack` do not.
    if bytes.len() >= 4 && &bytes[0..4] == b"LSCC" {
        at = 4;
    }
    if bytes.len() < at + 2 || bytes[at] != 0xff || bytes[at + 1] != 0x00 {
        return Err(BitError::NotABitFile);
    }
    at += 2;
    let mut metadata = Vec::new();
    let mut current = Vec::new();
    loop {
        let Some(byte) = bytes.get(at).copied() else {
            return Err(BitError::UnterminatedMetadata);
        };
        at += 1;
        if byte == 0xff {
            break;
        }
        if byte == 0x00 {
            metadata.push(String::from_utf8_lossy(&current).into_owned());
            current.clear();
            continue;
        }
        current.push(byte);
    }
    // The 0xff just consumed closes the metadata header and is *not* part
    // of the preamble: `ecppack` writes one terminator and then four more
    // bytes. Counting it as the preamble's first byte is a mistake that
    // reads a real file as "not a .bit", which is how it was found.
    if bytes.len() < at + 4 || bytes[at..at + 4] != BIT_PREAMBLE {
        return Err(BitError::NoPreamble);
    }
    at += 4;

    // Walk commands until the payload starts. Only two are read; the rest
    // are skipped by their operand widths.
    let mut header = BitHeader {
        metadata,
        idcode: None,
        compressed: false,
    };
    loop {
        let Some(opcode) = bytes.get(at).copied() else {
            return Ok(header);
        };
        at += 1;
        match opcode {
            // A dummy byte: no operands.
            0xff => {}
            // The payload. Two operand bytes and then the frames, which
            // is where this walk stops.
            0x82 | 0xb8 => {
                header.compressed = opcode == 0xb8;
                return Ok(header);
            }
            // `VERIFY_ID`: three reserved bytes and a big-endian IDCODE.
            0xe2 => {
                let Some(operand) = bytes.get(at + 3..at + 7) else {
                    return Err(BitError::Truncated);
                };
                header.idcode = Some(u32::from_be_bytes([
                    operand[0], operand[1], operand[2], operand[3],
                ]));
                at += 7;
            }
            // `LSC_WRITE_COMP_DIC`: three reserved bytes and eight of
            // dictionary.
            0x02 => at += 11,
            // Everything else in a header is one opcode and three
            // operand bytes, optionally followed by a four-byte word.
            // The ones that take a word: control registers, addresses,
            // the USERCODE, the SED check word, `JUMP`.
            0x22 | 0x23 | 0x46 | 0xa2 | 0xb4 | 0xc2 | 0xf6 | 0x7e => at += 7,
            // `SPI_MODE` carries its byte in the first operand slot.
            0x79 => at += 3,
            // `LSC_RESET_CRC`, `ISC_PROGRAM_DONE` and the rest: three
            // reserved bytes.
            _ => at += 3,
        }
        if at > bytes.len() {
            return Err(BitError::Truncated);
        }
    }
}

// ---------------------------------------------------------------------
// The ECP5 configuration instruction set
// ---------------------------------------------------------------------

/// How wide an ECP5's JTAG instruction register is.
///
/// Eight bits, which is why every constant in [`isc`] is a `u8` and why
/// [`Plan::shift_ir`] is always given `IR_BITS`. A wrong width does not
/// misread: the bits land on whatever instruction they land on.
pub const IR_BITS: usize = 8;

/// The instructions this module shifts into an ECP5's instruction
/// register.
///
/// *Provenance*: Apollo's own host package, `apollo_fpga/ecp5.py`'s
/// `ECP5Programmer.Opcode` enumeration (BSD-3-Clause), read from a
/// checkout and cross-checked against `ecpprog`'s `ecp5_cmds`, which
/// agrees on every value below. HIGH for the numbers; what the
/// *sequence* has been run against is `docs/programming.md`'s business,
/// not this list's.
pub mod isc {
    /// Do nothing. Shifted to park the instruction register while the
    /// part works, and once at the end.
    pub const ISC_NOOP: u8 = 0xFF;
    /// Present the 32-bit `IDCODE` in the data register.
    ///
    /// Reticle does not need this to read an identifier — after
    /// `Test-Logic-Reset` every 1149.1 part presents one with **no
    /// instruction shifted at all**, which is
    /// [`super::super::jtag::idcode_plan`] — but the configuration
    /// sequence reads the identifier again through it, after
    /// `LSC_REFRESH`, to confirm the part still answers.
    pub const READ_ID: u8 = 0xE0;
    /// Present the 32-bit configuration status register.
    pub const LSC_READ_STATUS: u8 = 0x3C;
    /// Present one byte whose bit 0 says the part is still working.
    pub const LSC_CHECK_BUSY: u8 = 0xF0;
    /// Restart configuration, as strobing `PROGRAMN` would.
    pub const LSC_REFRESH: u8 = 0x79;
    /// Enter configuration mode.
    pub const ISC_ENABLE: u8 = 0xC6;
    /// Leave configuration mode, which is what starts the part.
    pub const ISC_DISABLE: u8 = 0x26;
    /// Clear the configuration SRAM.
    pub const ISC_ERASE: u8 = 0x0E;
    /// Set the address the next write starts at.
    pub const LSC_SET_WORKING_ADDRESS: u8 = 0x46;
    /// Accept the data that follows as one contiguous bitstream.
    pub const LSC_BITSTREAM_BURST: u8 = 0x7A;
    /// The instruction Lattice's own flow shifts a long run of ones
    /// through before `ISC_ENABLE`.
    ///
    /// Apollo's source comments it `# ???` and shifts 510 bits of
    /// `0x3f` then sixty-three `0xff` through it; `ecpprog` does not send
    /// it at all. It is kept because the sequence that reproduces
    /// Apollo's is the one with the fewest unknowns, and this was in it.
    pub const UNDOCUMENTED_PREAMBLE: u8 = 0x1C;
}

/// Instructions that reach something a power cycle does not undo, named
/// so that "this is never shifted" is a checkable claim rather than a
/// promise in prose.
///
/// `docs/apollo-protocol.md` §7 is the same idea one layer up, for USB
/// requests; this is the layer below, for JTAG instructions, and
/// [`instructions_of`] plus a test assert that no plan this module builds
/// carries one. The first entry is the important one: an ECP5 in
/// **background SPI** mode is a pass-through to the board's configuration
/// flash, and everything destructive on a Cynthion is on the far side of
/// it.
pub const NOT_SHIFTED: &[(u8, &str)] = &[
    (
        0x3A,
        "LSC_ENTER_BACKGROUND_SPI: turns the part into a pass-through to the board's \
         configuration flash, where a write is not undone by a power cycle",
    ),
    (
        0xC2,
        "ISC_PROGRAM_USERCODE: writes the part's USERCODE, which is not part of \
         loading a design and is not this project's board to stamp",
    ),
    (
        0xCE,
        "ISC_PROGRAM_SECURITY: sets the security bits, which cannot be cleared",
    ),
];

/// The 32-bit configuration status register.
///
/// *Provenance*: the flag positions are `apollo_fpga/ecp5.py`'s
/// `STATUS_FLAG_*` constants, which agree with `ecpprog`'s status
/// printer. HIGH for `DONE`, `BUSY`, `FAIL` and `ISC_ENABLE`, which are
/// the four this crate acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status(pub u32);

impl Status {
    /// The part has a complete configuration and is running it.
    #[must_use]
    pub fn done(self) -> bool {
        self.0 & (1 << 8) != 0
    }

    /// The part is in configuration mode, which `ISC_ENABLE` puts it in
    /// and `ISC_DISABLE` takes it out of.
    #[must_use]
    pub fn isc_enabled(self) -> bool {
        self.0 & (1 << 9) != 0
    }

    /// The part is still working on the last instruction.
    #[must_use]
    pub fn busy(self) -> bool {
        self.0 & (1 << 12) != 0
    }

    /// The part rejected something.
    #[must_use]
    pub fn fail(self) -> bool {
        self.0 & (1 << 13) != 0
    }

    /// The bitstream's `VERIFY_ID` did not match the part.
    #[must_use]
    pub fn id_error(self) -> bool {
        self.0 & (1 << 27) != 0
    }

    /// A command in the bitstream was not one the part knows.
    #[must_use]
    pub fn invalid_command(self) -> bool {
        self.0 & (1 << 28) != 0
    }

    /// A command in the bitstream failed, which is what a bad check word
    /// reports as.
    #[must_use]
    pub fn execution_failed(self) -> bool {
        self.0 & (1 << 26) != 0
    }

    /// Every flag that is a complaint, for a message.
    #[must_use]
    pub fn faults(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.fail() {
            out.push("FAIL");
        }
        if self.execution_failed() {
            out.push("execution failed");
        }
        if self.id_error() {
            out.push("ID error");
        }
        if self.invalid_command() {
            out.push("invalid command");
        }
        out
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#010x}", self.0)?;
        let mut flags: Vec<&str> = Vec::new();
        if self.done() {
            flags.push("DONE");
        }
        if self.isc_enabled() {
            flags.push("ISC_ENABLE");
        }
        if self.busy() {
            flags.push("BUSY");
        }
        flags.extend(self.faults());
        if flags.is_empty() {
            return write!(f, " (no flags set)");
        }
        write!(f, " ({})", flags.join(", "))
    }
}

/// The bytes of `bitstream` in the order the data register wants them.
///
/// The configuration logic takes the **most significant bit of the first
/// byte first**, and a JTAG data register shifts the least significant
/// bit of the first byte first ([`Plan`]'s packing, and
/// `docs/apollo-protocol.md` §4's "Bit order", which was measured
/// against this board). So each byte is reversed and **the byte order is
/// left alone**.
///
/// That is one of two arrangements that look plausible, and the other one
/// mirrors the whole file, so it is worth saying why this is the right
/// one. Apollo's host package reverses each byte *and* the byte order,
/// then shifts the result as a big-endian integer, which starts at the
/// last byte — so the first bit it puts on the wire is bit 7 of
/// `bitstream[0]`, exactly as here.
#[must_use]
pub fn burst_order(bitstream: &[u8]) -> Vec<u8> {
    bitstream.iter().map(|b| b.reverse_bits()).collect()
}

/// Read the identifier the way that needs an instruction, so it can be
/// re-read in the middle of a sequence.
///
/// Capture 0 is the 32-bit `IDCODE`.
#[must_use]
pub fn read_id_plan() -> Plan {
    let mut plan = Plan::new();
    plan.shift_ir(&[isc::READ_ID], IR_BITS);
    let _ = plan.read_dr(32);
    plan
}

/// Read the configuration status register. Capture 0 is a [`Status`].
#[must_use]
pub fn status_plan() -> Plan {
    let mut plan = Plan::new();
    plan.shift_ir(&[isc::LSC_READ_STATUS], IR_BITS);
    let _ = plan.read_dr(32);
    plan
}

/// Read the busy flag. Capture 0 is one byte whose bit 0 is set while
/// the part is still working on an instruction.
#[must_use]
pub fn busy_plan() -> Plan {
    let mut plan = Plan::new();
    plan.shift_ir(&[isc::LSC_CHECK_BUSY], IR_BITS);
    let _ = plan.read_dr(8);
    plan
}

/// How many `Run-Test/Idle` clocks follow an instruction that acts.
///
/// Two, which is what Apollo's host package holds after `ISC_ENABLE`,
/// `ISC_ERASE` and `ISC_DISABLE`. The waiting that actually matters is
/// not done in TCK at all — it is done by polling [`busy_plan`] and
/// [`status_plan`], because a plan cannot be told how fast TCK is and a
/// Cynthion is never told either.
pub const SETTLE_CLOCKS: usize = 2;

/// How many `Run-Test/Idle` clocks are held after the payload, before
/// the status register is believed.
///
/// A hundred. Lattice's flow calls this "allow configuration time"; it
/// is the part finishing the last frame it was handed.
pub const START_CLOCKS: usize = 100;

/// The five plans of a volatile SRAM configuration, in order.
///
/// They are separate because **between them the host has to wait**, and a
/// plan has no way to wait: it cannot be told a clock rate, so a number
/// of TCK cycles is not a duration. The caller runs each in turn and
/// polls [`busy_plan`] or [`status_plan`] in between, the way
/// `reticle program` polls a Gowin part rather than sleeping.
///
/// The order, and why each step is there:
///
/// 1. [`SramPlans::refresh`] — `LSC_REFRESH`, then `READ_ID`. Refresh
///    restarts configuration, which is what strobing `PROGRAMN` would do;
///    reading the identifier back is the check that the part is still
///    there and still the one that was asked for, **before** anything is
///    erased.
/// 2. [`SramPlans::enable`] — the `0x1C` preamble, then `ISC_ENABLE`. The
///    status register afterwards must show `ISC_ENABLE`.
/// 3. [`SramPlans::erase`] — `ISC_ERASE`. This is the first step that
///    destroys anything, and what it destroys is the configuration
///    **SRAM**: a power cycle reloads the part from its flash, which is
///    untouched.
/// 4. [`SramPlans::burst`] — `LSC_SET_WORKING_ADDRESS`, then
///    `LSC_BITSTREAM_BURST` with the whole file, then `ISC_NOOP` and a
///    hundred idle clocks. This is the big one and the only step whose
///    size depends on the bitstream.
/// 5. [`SramPlans::start`] — `ISC_DISABLE`, then `ISC_NOOP`. Leaving
///    configuration mode is what lets the part run.
///
/// Steps 2, 3 and 5 each end with their own status read, so a failure is
/// attributed to a step rather than to the whole sequence.
///
/// *Provenance*: `ECP5CommandBasedProgrammer.configure` in Apollo's
/// `apollo_fpga/ecp5.py` (BSD-3-Clause), which is the sequence a Cynthion
/// is configured with by its own vendor's tool. It agrees with
/// `ecpprog`'s `ecp5_program` except for the `0x1C` preamble, which
/// `ecpprog` omits.
#[derive(Clone, Debug)]
pub struct SramPlans {
    /// `LSC_REFRESH`, then `READ_ID`. Capture 0 is the `IDCODE`.
    pub refresh: Plan,
    /// The `0x1C` preamble and `ISC_ENABLE`. Capture 0 is a [`Status`].
    pub enable: Plan,
    /// `ISC_ERASE`. Capture 0 is a [`Status`].
    pub erase: Plan,
    /// The payload. No captures: it is one long write.
    pub burst: Plan,
    /// `ISC_DISABLE`. Capture 0 is a [`Status`].
    pub start: Plan,
}

/// The bits the `0x1C` preamble shifts: `0x3f` and sixty-three `0xff`,
/// of which 510 are shifted.
const PREAMBLE_BITS: usize = 510;

/// The plans that load `bitstream` into an ECP5's configuration SRAM.
///
/// `bitstream` is the file as it is on disk; [`burst_order`] is applied
/// here, so a caller never has to know about the bit order.
#[must_use]
pub fn sram_plans(bitstream: &[u8]) -> SramPlans {
    let mut refresh = Plan::new();
    refresh.reset();
    refresh.shift_ir(&[isc::LSC_REFRESH], IR_BITS);
    refresh.idle(SETTLE_CLOCKS);
    refresh.shift_ir(&[isc::READ_ID], IR_BITS);
    let _ = refresh.read_dr(32);

    let mut preamble = vec![0x3f_u8];
    preamble.extend(std::iter::repeat_n(0xff_u8, 63));
    let mut enable = Plan::new();
    enable.shift_ir(&[isc::UNDOCUMENTED_PREAMBLE], IR_BITS);
    enable.shift_dr(&preamble, PREAMBLE_BITS);
    enable.shift_ir(&[isc::ISC_ENABLE], IR_BITS);
    enable.shift_dr(&[0x00], 8);
    enable.idle(SETTLE_CLOCKS);
    enable.shift_ir(&[isc::LSC_READ_STATUS], IR_BITS);
    let _ = enable.read_dr(32);

    let mut erase = Plan::new();
    erase.shift_ir(&[isc::ISC_ERASE], IR_BITS);
    erase.shift_dr(&[0x01], 8);
    erase.idle(SETTLE_CLOCKS);
    erase.shift_ir(&[isc::LSC_READ_STATUS], IR_BITS);
    let _ = erase.read_dr(32);

    let payload = burst_order(bitstream);
    let mut burst = Plan::new();
    burst.shift_ir(&[isc::LSC_SET_WORKING_ADDRESS], IR_BITS);
    burst.shift_dr(&[0x01], 8);
    burst.shift_ir(&[isc::LSC_BITSTREAM_BURST], IR_BITS);
    burst.shift_dr(&payload, payload.len() * 8);
    burst.shift_ir(&[isc::ISC_NOOP], IR_BITS);
    burst.idle(START_CLOCKS);

    let mut start = Plan::new();
    start.shift_ir(&[isc::ISC_DISABLE], IR_BITS);
    start.idle(SETTLE_CLOCKS);
    start.shift_ir(&[isc::ISC_NOOP], IR_BITS);
    start.idle(START_CLOCKS);
    start.shift_ir(&[isc::LSC_READ_STATUS], IR_BITS);
    let _ = start.read_dr(32);

    SramPlans {
        refresh,
        enable,
        erase,
        burst,
        start,
    }
}

impl SramPlans {
    /// The five plans in the order they are run, so a caller — or a test
    /// checking them against [`NOT_SHIFTED`] — can walk them without
    /// naming each field.
    #[must_use]
    pub fn in_order(&self) -> [&Plan; 5] {
        [
            &self.refresh,
            &self.enable,
            &self.erase,
            &self.burst,
            &self.start,
        ]
    }
}

/// Every instruction a plan shifts into the instruction register, in the
/// order it shifts them.
///
/// It exists so a test can check a plan against [`NOT_SHIFTED`] rather
/// than a reader checking it by eye.
#[must_use]
pub fn instructions_of(plan: &Plan) -> Vec<u8> {
    plan.ops()
        .iter()
        .filter_map(|op| match op {
            jtag::Op::Shift {
                state: TapState::ShiftIr,
                data,
                ..
            } => data.first().copied(),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields of an `IDCODE` are IEEE 1149.1 clause 12's, and every
    /// entry in the table is a Lattice one with bit 0 set.
    #[test]
    fn every_listed_part_is_a_well_formed_lattice_idcode() {
        for (idcode, name) in ECP5_PARTS {
            assert_eq!(idcode & 1, 1, "{name} must have bit 0 set");
            assert!(is_lattice(idcode), "{name} must be a Lattice identifier");
            assert_eq!(manufacturer(idcode), MANUFACTURER_LATTICE, "{name}");
            assert_eq!(ecp5_part(idcode), Some(name));
        }
    }

    /// Every identifier in the table is distinct, which is the whole
    /// reason the version nibble is not masked off.
    #[test]
    fn the_table_has_no_duplicates_and_needs_the_version_nibble() {
        let mut seen = Vec::new();
        for (idcode, name) in ECP5_PARTS {
            assert!(!seen.contains(&idcode), "{name} is listed twice");
            seen.push(idcode);
        }
        // Four parts share the device field 0x1111 and differ only in
        // the top nibble; masking it would merge them.
        let sharing: Vec<&str> = ECP5_PARTS
            .iter()
            .filter(|(id, _)| part_number(*id) == 0x1111)
            .map(|(_, name)| *name)
            .collect();
        assert_eq!(sharing.len(), 4, "got {sharing:?}");
    }

    /// A dead chain does not look like a part. This is the check that
    /// stops "the board is not answering" from being reported as an
    /// exotic Lattice device.
    #[test]
    fn an_undriven_chain_is_not_a_lattice_part() {
        assert!(!is_lattice(0x0000_0000));
        assert!(!is_lattice(0xFFFF_FFFF));
        // A Xilinx part is not one either.
        assert!(!is_lattice(super::super::xilinx::IDCODE_XC7A35T));
        assert!(describe(0xFFFF_FFFF).contains("not Lattice"));
        assert!(describe(0x0000_0000).contains("0x00000000"));
    }

    /// The description leads with the raw value, whatever it decodes
    /// to, and names the part when it knows one.
    #[test]
    fn a_description_leads_with_the_number() {
        let text = describe(0x4111_1043);
        assert!(text.starts_with("IDCODE 0x41111043"), "{text}");
        assert!(text.contains("LFE5U-25F"), "{text}");
        assert!(text.contains("Lattice"), "{text}");

        // A Lattice identifier that is not in the table is still
        // reported, as a Lattice part with no name.
        let text = describe(0x0127_0043);
        assert!(text.starts_with("IDCODE 0x01270043"), "{text}");
        assert!(text.contains("no name for"), "{text}");
    }

    /// The configuration sequence is the one Apollo's own tool performs,
    /// in that order. This pins the whole walk, because the order is the
    /// part that cannot be checked by reading one instruction.
    #[test]
    fn the_sram_sequence_is_the_documented_walk() {
        let plans = sram_plans(&[0xff, 0x00, 0xbd]);
        assert_eq!(
            instructions_of(&plans.refresh),
            vec![isc::LSC_REFRESH, isc::READ_ID]
        );
        assert_eq!(
            instructions_of(&plans.enable),
            vec![
                isc::UNDOCUMENTED_PREAMBLE,
                isc::ISC_ENABLE,
                isc::LSC_READ_STATUS
            ]
        );
        assert_eq!(
            instructions_of(&plans.erase),
            vec![isc::ISC_ERASE, isc::LSC_READ_STATUS]
        );
        assert_eq!(
            instructions_of(&plans.burst),
            vec![
                isc::LSC_SET_WORKING_ADDRESS,
                isc::LSC_BITSTREAM_BURST,
                isc::ISC_NOOP
            ]
        );
        assert_eq!(
            instructions_of(&plans.start),
            vec![isc::ISC_DISABLE, isc::ISC_NOOP, isc::LSC_READ_STATUS]
        );

        // The first step resets the TAP, because the part has to be found
        // in a known state, and nothing else does: a reset in the middle
        // would abandon configuration mode.
        assert!(matches!(plans.refresh.ops()[0], jtag::Op::Reset));
        for plan in [&plans.enable, &plans.erase, &plans.burst, &plans.start] {
            assert!(
                !plan.ops().iter().any(|op| matches!(op, jtag::Op::Reset)),
                "only the first step may reset the TAP"
            );
        }

        // Each step that can fail reads the status back itself, so a
        // failure is attributed to a step.
        for plan in [&plans.enable, &plans.erase, &plans.start] {
            assert_eq!(plan.capture_count(), 1);
        }
        // The payload captures nothing: it is one long write.
        assert_eq!(plans.burst.capture_count(), 0);
        // And the identifier is read before anything is erased.
        assert_eq!(plans.refresh.capture_count(), 1);
    }

    /// Not one plan shifts an instruction that reaches something a power
    /// cycle does not undo. This is the assertion behind
    /// [`NOT_SHIFTED`]'s prose.
    #[test]
    fn no_plan_shifts_an_instruction_that_outlives_a_power_cycle() {
        let mut all: Vec<u8> = Vec::new();
        let plans = sram_plans(&[0x00; 64]);
        for plan in plans.in_order() {
            all.extend(instructions_of(plan));
        }
        for plan in [read_id_plan(), status_plan(), busy_plan()] {
            all.extend(instructions_of(&plan));
        }
        for (opcode, why) in NOT_SHIFTED {
            assert!(
                !all.contains(opcode),
                "{opcode:#04x} must never be shifted: {why}"
            );
        }
        // Every instruction that *is* shifted is one of the named ones,
        // so a stray byte cannot hide in a plan.
        let known = [
            isc::ISC_NOOP,
            isc::READ_ID,
            isc::LSC_READ_STATUS,
            isc::LSC_CHECK_BUSY,
            isc::LSC_REFRESH,
            isc::ISC_ENABLE,
            isc::ISC_DISABLE,
            isc::ISC_ERASE,
            isc::LSC_SET_WORKING_ADDRESS,
            isc::LSC_BITSTREAM_BURST,
            isc::UNDOCUMENTED_PREAMBLE,
        ];
        for opcode in &all {
            assert!(known.contains(opcode), "{opcode:#04x} is not in `isc`");
        }
        // The list is not empty, which is the failure mode this test
        // would otherwise pass with.
        assert_eq!(NOT_SHIFTED.len(), 3);
        for (_, why) in NOT_SHIFTED {
            assert!(!why.is_empty());
        }
    }

    /// The payload reaches the data register with the most significant
    /// bit of the first byte first. Getting this backwards produces a
    /// stream of exactly the right length that configures nothing.
    #[test]
    fn the_payload_is_bit_reversed_and_not_byte_reversed() {
        // `0x80` is bit 7 alone; reversed it is bit 0 alone, which is the
        // first bit a data register shifts.
        assert_eq!(burst_order(&[0x80]), vec![0x01]);
        assert_eq!(burst_order(&[0x01]), vec![0x80]);
        // The byte order is untouched, which is the half that is easy to
        // get wrong: the preamble's first byte stays first.
        assert_eq!(
            burst_order(&[0xff, 0xff, 0xbd, 0xb3]),
            vec![0xff, 0xff, 0xbd, 0xcd]
        );
        assert_eq!(burst_order(&[]), Vec::<u8>::new());
        // It is its own inverse, so the round trip is checkable.
        let file: Vec<u8> = (0..=255u8).collect();
        assert_eq!(burst_order(&burst_order(&file)), file);

        // And the plan shifts exactly that, bit for bit.
        let plans = sram_plans(&[0x80, 0x01]);
        let shifted: Vec<(Vec<u8>, usize)> = plans
            .burst
            .ops()
            .iter()
            .filter_map(|op| match op {
                jtag::Op::Shift {
                    state: TapState::ShiftDr,
                    data,
                    bits,
                    ..
                } => Some((data.clone(), *bits)),
                _ => None,
            })
            .collect();
        // The working address, then the payload.
        assert_eq!(shifted[0], (vec![0x01], 8));
        assert_eq!(shifted[1], (vec![0x01, 0x80], 16));
    }

    /// The status register's flags are the ones a run reads, and the
    /// display names every flag it finds.
    #[test]
    fn the_status_register_names_its_flags() {
        let idle = Status(0);
        assert!(!idle.done());
        assert!(!idle.isc_enabled());
        assert!(!idle.busy());
        assert!(idle.faults().is_empty());
        assert!(idle.to_string().contains("no flags set"), "{idle}");
        assert!(idle.to_string().starts_with("0x00000000"), "{idle}");

        let configured = Status(1 << 8);
        assert!(configured.done());
        assert!(configured.to_string().contains("DONE"), "{configured}");

        let in_isc = Status(1 << 9);
        assert!(in_isc.isc_enabled());
        assert!(!in_isc.done());
        assert!(in_isc.to_string().contains("ISC_ENABLE"), "{in_isc}");

        let working = Status(1 << 12);
        assert!(working.busy());

        // Every complaint is reported, and a part can assert several.
        let broken = Status((1 << 13) | (1 << 26) | (1 << 27) | (1 << 28));
        assert_eq!(
            broken.faults(),
            vec!["FAIL", "execution failed", "ID error", "invalid command"]
        );
        assert!(!broken.done(), "a part that failed has not finished");
        let text = broken.to_string();
        for flag in broken.faults() {
            assert!(text.contains(flag), "{text}");
        }

        // A dead chain reads as all ones, and that is not a configured
        // part even though the DONE bit is in it: every complaint is set
        // too, which is what the caller checks.
        let dead = Status(0xFFFF_FFFF);
        assert!(dead.done());
        assert_eq!(dead.faults().len(), 4);
    }

    /// A minimal ECP5 `.bit`: the metadata header, its terminator, the
    /// preamble, padding, `LSC_RESET_CRC`, `VERIFY_ID` and a payload
    /// command. The terminator and the preamble's first byte are both
    /// `0xff` and they are **different bytes**, which is the mistake this
    /// pins: counting the terminator as part of the preamble reads a real
    /// file as "not a .bit".
    fn minimal_bit(idcode: u32, compressed: bool) -> Vec<u8> {
        let mut out = vec![0xff, 0x00];
        out.extend_from_slice(b"Part: LFE5U-12F-8CABGA256");
        out.push(0x00);
        out.push(0xff); // the metadata terminator
        out.extend_from_slice(&BIT_PREAMBLE);
        out.extend_from_slice(&[0xff; 4]);
        out.extend_from_slice(&[0x3b, 0, 0, 0]);
        out.push(0xe2);
        out.extend_from_slice(&[0, 0, 0]);
        out.extend_from_slice(&idcode.to_be_bytes());
        out.push(if compressed { 0xb8 } else { 0x82 });
        out.extend_from_slice(&[0x91, 0x1d, 0x8a]);
        out
    }

    /// A file says which part it is for and whether it is compressed,
    /// without a frame of it being decoded.
    #[test]
    fn a_bit_file_names_its_part_without_being_decoded() {
        let bytes = minimal_bit(0x2111_1043, true);
        assert!(is_ecp5_bit(&bytes));
        let header = read_bit_header(&bytes).unwrap();
        assert_eq!(header.metadata, vec!["Part: LFE5U-12F-8CABGA256"]);
        assert_eq!(header.idcode, Some(0x2111_1043));
        assert!(header.compressed);
        // The uncompressed payload command says so.
        let plain = read_bit_header(&minimal_bit(0x4111_1043, false)).unwrap();
        assert!(!plain.compressed);
        assert_eq!(plain.idcode, Some(0x4111_1043));

        // An `LSCC` prefix is accepted, because some tools write one.
        let mut prefixed = b"LSCC".to_vec();
        prefixed.extend_from_slice(&bytes);
        assert_eq!(read_bit_header(&prefixed).unwrap(), header);
    }

    /// Every way a file can fail to be one says which way.
    #[test]
    fn a_file_that_is_not_one_is_refused_by_name() {
        // A Xilinx `.bit` opens with a big-endian length and a run of
        // `0x0f 0xf0`, and it is *not* one of these.
        let xilinx = [0x00, 0x09, 0x0f, 0xf0, 0x0f, 0xf0, 0x0f, 0xf0];
        assert!(!is_ecp5_bit(&xilinx));
        assert_eq!(read_bit_header(&xilinx), Err(BitError::NotABitFile));
        assert_eq!(read_bit_header(&[]), Err(BitError::NotABitFile));
        assert!(
            read_bit_header(&xilinx)
                .unwrap_err()
                .to_string()
                .contains("0xff 0x00")
        );

        // A header that never ends.
        let open = [0xff, 0x00, b'P', b'a', b'r', b't'];
        assert_eq!(read_bit_header(&open), Err(BitError::UnterminatedMetadata));

        // A header that ends and is followed by something else. This is
        // the one that catches the off-by-one: `0xff 0x00 0x00 0xff` has a
        // terminator and no preamble after it.
        let no_preamble = [0xff, 0x00, 0x00, 0xff, 0x01, 0x02, 0x03, 0x04];
        assert_eq!(read_bit_header(&no_preamble), Err(BitError::NoPreamble));
        assert!(
            read_bit_header(&no_preamble)
                .unwrap_err()
                .to_string()
                .contains("ff ff bd b3")
        );

        // A `VERIFY_ID` whose operand runs off the end.
        let mut short = minimal_bit(0x2111_1043, true);
        short.truncate(short.len() - 6);
        assert_eq!(read_bit_header(&short), Err(BitError::Truncated));

        // A stream that simply ends before a payload command is not an
        // error: everything asked of it was found.
        let mut headerless = vec![0xff, 0x00, 0x00, 0xff];
        headerless.extend_from_slice(&BIT_PREAMBLE);
        let read = read_bit_header(&headerless).unwrap();
        assert_eq!(read.idcode, None);
        assert!(!read.compressed);
        assert_eq!(read.metadata, vec![String::new()]);
    }

    /// The preamble is duplicated from `fpga::ecp5` because `src/program`
    /// has to compile without the `fpga` feature. This is what a shared
    /// constant would have bought.
    #[test]
    #[cfg(feature = "fpga")]
    fn the_preamble_agrees_with_the_container_modules() {
        assert_eq!(BIT_PREAMBLE, crate::fpga::ecp5::PREAMBLE);
        // And so do the instruction opcodes that both modules name, where
        // they name the same thing. The in-stream command numbers and the
        // JTAG instruction numbers are *different* namespaces that
        // partly collide — `0x79` is `SPI_MODE` in a stream and
        // `LSC_REFRESH` over JTAG — so only the ones that really are the
        // same operation are compared.
        assert_eq!(isc::ISC_NOOP, crate::fpga::ecp5::CMD_DUMMY);
        assert_eq!(
            isc::LSC_SET_WORKING_ADDRESS,
            crate::fpga::ecp5::CMD_LSC_INIT_ADDRESS
        );
    }

    /// The instruction register is eight bits, and every instruction is
    /// shifted as eight.
    #[test]
    fn every_instruction_is_shifted_as_eight_bits() {
        assert_eq!(IR_BITS, 8);
        let plans = sram_plans(&[0u8; 4]);
        for plan in plans.in_order() {
            for op in plan.ops() {
                if let jtag::Op::Shift {
                    state: TapState::ShiftIr,
                    bits,
                    data,
                    ..
                } = op
                {
                    assert_eq!(*bits, IR_BITS);
                    assert_eq!(data.len(), 1);
                }
            }
        }
    }
}
