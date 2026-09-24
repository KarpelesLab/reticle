//! Configuring a Xilinx 7-series part over JTAG, as a sequence of
//! [`Job`]s.
//!
//! Every number here comes from Xilinx **UG470**, *7 Series FPGAs
//! Configuration User Guide* — the same document `fpga::xc7` writes the
//! bitstream container from, and the two were written against the same
//! chapter: the instruction opcodes from UG470 table 6-3 ("Boundary-Scan
//! Instructions"), the configuration flow from UG470 table 6-5
//! ("Configuration Sequence"), the type 1 packet header from chapter 5,
//! and the status register's fields from the *Status Register
//! Description* table in the same chapter (see [`Status`]).
//!
//! Nothing in this file performs I/O. Each function returns a [`Job`]:
//! bytes to send and a description of what comes back. The driver
//! ([`super::usb`], or the `reticle program` command) sends them in
//! order and decides what to do between them, which is the point —
//! `IDCODE` has to be *checked* before `JPROGRAM` erases anything, and a
//! sans-I/O sequencer cannot make that decision for the caller.
//!
//! # SRAM only
//!
//! This module configures the part's volatile configuration memory and
//! nothing else. There is no code here that writes the board's QSPI
//! flash, and there deliberately is not: a JTAG configuration is undone
//! by a power cycle, so every experiment is reversible, and a flash write
//! is not. Nothing here touches the mode pins either; JTAG configuration
//! works whatever the mode jumper is set to.
//!
//! # Bit order, which is the whole difficulty
//!
//! Two opposite conventions meet in this file.
//!
//! - **JTAG shifts least-significant bit first.** Bit 0 of a register is
//!   the first bit through TDI, and the first bit out of TDO is bit 0 of
//!   what was captured. [`super::jtag`] is LSB-first throughout.
//! - **A configuration bitstream is most-significant bit first.** The
//!   `.bit` file's payload is a byte stream whose *bit 7 of byte 0* is
//!   the first bit the configuration engine must see.
//!
//! So each payload byte is **bit-reversed in place** before it is
//! shifted ([`reverse_bits`]), and the byte order is left alone. Reverse
//! the bytes instead of the bits, or neither, and the container still
//! looks structurally fine while the part ignores it — which is exactly
//! the failure mode that is impossible to diagnose from the outside.
//! [`word_msb_first`] is the same correction on the way back, for the
//! configuration registers read through `CFG_OUT`, and `IDCODE` — which
//! really is LSB-first, being a JTAG register rather than a
//! configuration one — is read with [`Job::capture_u32`] and no
//! correction at all. The contrast is the subject of
//! `tests/program_jtag.rs`.

use super::jtag::{Job, Scan};

/// The instruction register of every 7-series part is six bits wide
/// (UG470 table 6-3).
pub const IR_LENGTH: usize = 6;

/// The boundary-scan instructions this crate uses (UG470 table 6-3).
///
/// Only what configuration needs is listed; the boundary-scan
/// instructions proper (`EXTEST`, `SAMPLE`, `INTEST`) are not, because
/// nothing here does boundary scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Instruction {
    /// Shift the part's 32-bit `IDCODE` out of DR.
    Idcode = 0b001001,
    /// Shift the 32-bit `USERCODE` out of DR.
    Usercode = 0b001000,
    /// Pulse `PROGRAM_B`: clear the configuration memory and restart.
    JProgram = 0b001011,
    /// Run the startup sequence.
    JStart = 0b001100,
    /// Run the shutdown sequence.
    JShutdown = 0b001101,
    /// Route DR into the configuration engine's input.
    CfgIn = 0b000101,
    /// Route the configuration engine's output into DR.
    CfgOut = 0b000100,
    /// The in-system-configuration no-op, used to idle the engine.
    IscNoop = 0b010100,
    /// The mandatory one-bit bypass register.
    Bypass = 0b111111,
}

impl Instruction {
    /// The opcode, as the six bits that go into the instruction
    /// register least-significant first.
    #[must_use]
    pub fn bits(self) -> u8 {
        self as u8
    }
}

/// The `IDCODE` of the XC7A35T on a Digilent Basys 3, as Project X-Ray's
/// `artix7/xc7a35tcpg236-1/part.json` records it.
///
/// The top four bits are the silicon revision and differ between steppings
/// of the same part, so [`idcode_matches`] compares the other 28.
pub const IDCODE_XC7A35T: u32 = 0x0362_D093;

/// True when a read-back `IDCODE` identifies the same part as
/// `expected`, ignoring the revision nibble in bits 31..28.
///
/// UG470 table 6-2 splits `IDCODE` into a revision, a 21-bit device
/// identifier, a manufacturer identifier and a constant `1`. A part that
/// has been re-spun reads back a different revision and is still the same
/// part, so refusing on the full 32 bits would reject good hardware.
#[must_use]
pub fn idcode_matches(read: u32, expected: u32) -> bool {
    read & 0x0FFF_FFFF == expected & 0x0FFF_FFFF
}

/// The JEDEC manufacturer identifier Xilinx uses, in `IDCODE` bits 11..1
/// with the mandatory `1` in bit 0 — so `0x093` as a twelve-bit field.
pub const MANUFACTURER_XILINX: u32 = 0x093;

/// True when an `IDCODE` names a Xilinx part.
///
/// Everything else in this module — the status register, the instruction
/// codes, the six-bit instruction register — is Xilinx's. Shifting any of
/// it at another vendor's part is not a harmless misread: the instruction
/// register is a different width, so the bits land on whatever
/// instruction happens to be at that position. A Gowin GW2A's is eight
/// bits wide, for one, and this project has a board with one on it.
///
/// So the caller checks this before doing anything beyond reading the
/// identifier, which [`super::jtag::idcode_after_reset`] does without
/// asking the vendor anything.
#[must_use]
pub fn is_xilinx(idcode: u32) -> bool {
    idcode & 0xFFF == MANUFACTURER_XILINX
}

/// The revision nibble of an `IDCODE` (bits 31..28).
#[must_use]
pub fn idcode_revision(idcode: u32) -> u8 {
    u8::try_from(idcode >> 28).unwrap_or(0)
}

/// Reverses the bits of one byte, most-significant to least.
///
/// This is the whole of the bit-order correction: a configuration
/// stream's byte has its first-transmitted bit at bit 7, and JTAG sends
/// bit 0 first.
#[must_use]
pub fn reverse_bits(byte: u8) -> u8 {
    byte.reverse_bits()
}

/// [`reverse_bits`] over a whole buffer. The **byte order is not
/// touched**: only the bits inside each byte move.
#[must_use]
pub fn reverse_all(data: &[u8]) -> Vec<u8> {
    data.iter().map(|b| b.reverse_bits()).collect()
}

/// Reads a 32-bit configuration register out of a capture whose bits
/// arrived most-significant first.
///
/// The configuration engine shifts a register out through `CFG_OUT` with
/// its bit 31 first, the opposite of a JTAG register like `IDCODE`.
/// `packed` is what [`Job::capture`](super::jtag::Job::capture) returned:
/// four bytes, LSB-first, bit *i* being the *i*-th bit out of TDO.
///
/// # Panics
///
/// When `packed` is not four bytes long.
#[must_use]
pub fn word_msb_first(packed: &[u8]) -> u32 {
    assert_eq!(packed.len(), 4, "a configuration register is 32 bits");
    u32::from_be_bytes([
        packed[0].reverse_bits(),
        packed[1].reverse_bits(),
        packed[2].reverse_bits(),
        packed[3].reverse_bits(),
    ])
}

// ---------------------------------------------------------------------
// The configuration register file (UG470 chapter 5)
// ---------------------------------------------------------------------

/// A type 1 packet header (UG470 table 5-20): `001`, a two-bit opcode, a
/// 14-bit register address, two reserved bits and an 11-bit word count.
#[must_use]
pub fn type1_header(opcode: u32, register: u32, words: u32) -> u32 {
    0x2000_0000 | ((opcode & 0x3) << 27) | ((register & 0x3FFF) << 13) | (words & 0x7FF)
}

/// A type 1 no-operation packet, which is also the stream's padding.
pub const NOOP: u32 = 0x2000_0000;

/// The synchronisation word that tells the configuration engine where
/// the packet stream starts (UG470, "Bitstream Composition").
pub const SYNC_WORD: u32 = 0xAA99_5566;

/// The status register's address in the configuration register file
/// (UG470 table 5-22).
pub const REG_STAT: u32 = 7;

/// The 7-series status register.
///
/// The bit indices are UG470's *Status Register Description* table —
/// numbered 5-25 in some revisions and 5-29 in others (v1.10, page 110),
/// the text being identical:
///
/// | Bits | Name | Bits | Name |
/// |---|---|---|---|
/// | 31-27 | *reserved* | 10-8 | `MODE` (the M pins) |
/// | 26-25 | `BUS_WIDTH` | 7 | `GHIGH_B` |
/// | 24-21 | *reserved* | 6 | `GWE` |
/// | 20-18 | `STARTUP_STATE` | 5 | `GTS_CFG_B` |
/// | 17 | `XADC_OVER_TEMP` | 4 | `EOS` |
/// | 16 | `DEC_ERROR` | 3 | `DCI_MATCH` |
/// | 15 | `ID_ERROR` | 2 | `MMCM_LOCK` |
/// | 14 | **`DONE`** (the pin) | 1 | `PART_SECURED` |
/// | 13 | `RELEASE_DONE` | 0 | `CRC_ERROR` |
/// | 12 | `INIT_B` (the pin) | | |
/// | 11 | `INIT_COMPLETE` | | |
///
/// The two reserved ranges are reserved in the specification and are not
/// always zero on a real part. After configuring an XC7A35T this crate
/// has seen bits 29 and 30 set every time, and **bit 28 set by a
/// Vivado-built bitstream and clear by one of Reticle's own**, with every
/// named field identical. So [`Display`](std::fmt::Display) names what
/// the table names and then lists any other set bit rather than dropping
/// it: an unexplained difference is worth seeing, and a decoder that
/// hides one costs more than it saves.
///
/// The whole word is kept, and [`Display`](std::fmt::Display) prints it
/// beside the decoded names: a field this crate placed wrongly would be
/// worse than the raw number, while the raw number can be checked
/// against any other tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status(pub u32);

impl Status {
    /// `CRC_ERROR` (bit 0): the engine rejected a packet's CRC.
    #[must_use]
    pub fn crc_error(self) -> bool {
        self.0 & (1 << 0) != 0
    }

    /// `MMCM_LOCK` (bit 2): every MMCM in the design has locked. It
    /// reads high on a part that has none, the bit being an AND over
    /// all of them.
    #[must_use]
    pub fn mmcm_lock(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    /// `EOS` (bit 4): the startup sequence has finished, which is the
    /// part saying it is running the design rather than still coming up.
    #[must_use]
    pub fn end_of_startup(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    /// `MODE` (bits 10..8): the level on the part's three mode pins,
    /// `0b101` being JTAG. This crate reads it and never changes it —
    /// JTAG configuration works whatever the board's mode jumper is set
    /// to, and the jumper is the user's business.
    #[must_use]
    pub fn mode(self) -> u8 {
        u8::try_from(self.0 >> 8 & 0b111).unwrap_or(0)
    }

    /// `INIT_COMPLETE` (bit 11): initialisation has finished.
    #[must_use]
    pub fn init_complete(self) -> bool {
        self.0 & (1 << 11) != 0
    }

    /// `INIT_B` (bit 12): the level on the `INIT_B` pin, which goes high
    /// once the configuration memory is clear and the part will take
    /// data.
    #[must_use]
    pub fn init_b(self) -> bool {
        self.0 & (1 << 12) != 0
    }

    /// `RELEASE_DONE` (bit 13): the internal `DONE` signal has been
    /// released, whatever the pin is being held at externally.
    #[must_use]
    pub fn release_done(self) -> bool {
        self.0 & (1 << 13) != 0
    }

    /// `DONE` (bit 14): the level on the `DONE` pin.
    ///
    /// **This is the bit that says a bitstream was accepted.** It is low
    /// on an unconfigured part and after `JPROGRAM`, and goes high
    /// during the startup sequence that `JSTART` runs.
    #[must_use]
    pub fn done(self) -> bool {
        self.0 & (1 << 14) != 0
    }

    /// `ID_ERROR` (bit 15): a write to `FDRI` was attempted without a
    /// successful device-ID check, which is what a bitstream for another
    /// part looks like from inside.
    #[must_use]
    pub fn id_error(self) -> bool {
        self.0 & (1 << 15) != 0
    }

    /// `DEC_ERROR` (bit 16): an `FDRI` write around a decrypt operation.
    #[must_use]
    pub fn dec_error(self) -> bool {
        self.0 & (1 << 16) != 0
    }

    /// `PART_SECURED` (bit 1): the part's security level is set.
    #[must_use]
    pub fn part_secured(self) -> bool {
        self.0 >> 1 & 1 == 1
    }

    /// `DCI_MATCH` (bit 3): every digitally controlled impedance block
    /// has matched.
    #[must_use]
    pub fn dci_match(self) -> bool {
        self.0 >> 3 & 1 == 1
    }

    /// `GTS_CFG_B` (bit 5): the global tristate has been released, so IO
    /// drivers are active.
    #[must_use]
    pub fn gts_cfg_b(self) -> bool {
        self.0 >> 5 & 1 == 1
    }

    /// `GWE` (bit 6): the global write enable is on, so flip-flops and
    /// memories may change.
    #[must_use]
    pub fn gwe(self) -> bool {
        self.0 >> 6 & 1 == 1
    }

    /// `GHIGH_B` (bit 7): the global high signal has been released.
    #[must_use]
    pub fn ghigh_b(self) -> bool {
        self.0 >> 7 & 1 == 1
    }

    /// `XADC_OVER_TEMP` (bit 17): the analogue-to-digital converter has
    /// reported an over-temperature shutdown.
    #[must_use]
    pub fn xadc_over_temp(self) -> bool {
        self.0 >> 17 & 1 == 1
    }

    /// `STARTUP_STATE` (bits 20-18): which of the eight startup phases
    /// the sequencer is in.
    #[must_use]
    pub fn startup_state(self) -> u8 {
        u8::try_from(self.0 >> 18 & 0b111).unwrap_or(0)
    }

    /// `BUS_WIDTH` (bits 26-25): 0 is x1, 1 is x8, 2 is x16, 3 is x32.
    #[must_use]
    pub fn bus_width(self) -> u8 {
        u8::try_from(self.0 >> 25 & 0b11).unwrap_or(0)
    }

    /// Every bit set that no named field in the table accounts for, as
    /// bit positions.
    ///
    /// This is what makes a reserved bit visible. It is how the
    /// difference between a Vivado bitstream and one of Reticle's own was
    /// noticed on an XC7A35T: bit 28.
    #[must_use]
    pub fn unnamed_bits(self) -> Vec<u32> {
        // Bits 0 to 20 are all named or part of a named field, as are 25
        // and 26 (`BUS_WIDTH`). Bits 21 to 24 and 27 to 31 are the two
        // reserved ranges.
        const NAMED: u32 = 0x001F_FFFF | 0x0600_0000;
        (0..32)
            .filter(|i| self.0 & !NAMED & (1 << i) != 0)
            .collect()
    }

    /// The bits worth naming in a report, in a fixed order.
    #[must_use]
    pub fn flags(self) -> Vec<(&'static str, bool)> {
        vec![
            ("CRC_ERROR", self.crc_error()),
            ("DEC_ERROR", self.dec_error()),
            ("ID_ERROR", self.id_error()),
            ("DONE", self.done()),
            ("RELEASE_DONE", self.release_done()),
            ("INIT_B", self.init_b()),
            ("INIT_COMPLETE", self.init_complete()),
            ("EOS", self.end_of_startup()),
            ("MMCM_LOCK", self.mmcm_lock()),
            ("PART_SECURED", self.part_secured()),
            ("DCI_MATCH", self.dci_match()),
            ("GTS_CFG_B", self.gts_cfg_b()),
            ("GWE", self.gwe()),
            ("GHIGH_B", self.ghigh_b()),
            ("XADC_OVER_TEMP", self.xadc_over_temp()),
        ]
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#010x} [", self.0)?;
        let set: Vec<&str> = self
            .flags()
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name)
            .collect();
        if set.is_empty() {
            write!(f, "no flags")?;
        } else {
            write!(f, "{}", set.join(" "))?;
        }
        write!(
            f,
            ", MODE {:03b}, STARTUP {}, BUS x{}",
            self.mode(),
            self.startup_state(),
            1u32 << self.bus_width()
        )?;
        // Anything the table does not account for is named by its bit
        // position rather than dropped. A reserved bit that differs
        // between two bitstreams is exactly the thing worth seeing.
        let unnamed = self.unnamed_bits();
        if !unnamed.is_empty() {
            let list: Vec<String> = unnamed.iter().map(|b| format!("bit{b}")).collect();
            write!(f, ", reserved {}", list.join("+"))?;
        }
        write!(f, "]")
    }
}

// ---------------------------------------------------------------------
// The jobs
// ---------------------------------------------------------------------

/// How many TCK to rest in `Run-Test/Idle` after `JPROGRAM`, while the
/// part clears its configuration memory. UG470 says to wait for `INIT_B`;
/// the status read that follows is what actually confirms it, and this is
/// only so the wait does not start by asking too early.
pub const ERASE_IDLE_CYCLES: usize = 10_000;

/// How many TCK the startup sequence needs after `JSTART`. UG470 table
/// 6-5 asks for at least the eight startup phases; a wide margin costs
/// microseconds.
pub const STARTUP_IDLE_CYCLES: usize = 2_000;

/// Puts the MPSSE into a known state and resets the TAP.
///
/// `divisor` comes from [`super::ftdi::divisor_for`]. `pins` and `dirs`
/// are the low-byte data-bus value and direction; see
/// [`super::BASYS3_PINS`].
#[must_use]
pub fn init_job(divisor: u16, pins: u8, dirs: u8) -> Job {
    init_job_for(super::ftdi::Chip::HighSpeed, divisor, pins, dirs)
}

/// The same, for a named FTDI part.
///
/// A C or D part does not have the three opcodes an H part opens with
/// and answers each of them "bad command", after which it obeys nothing.
#[must_use]
pub fn init_job_for(chip: super::ftdi::Chip, divisor: u16, pins: u8, dirs: u8) -> Job {
    let mut mpsse = super::ftdi::Mpsse::new();
    mpsse.configure_chip(chip, divisor);
    mpsse.set_pins_low(pins, dirs);
    mpsse.set_pins_high(0x00, 0x00);
    let mut scan = Scan::from_mpsse(mpsse);
    scan.reset();
    scan.finish()
}

/// Shifts `IDCODE` into IR and reads the 32-bit DR back.
///
/// Capture 0 is the `IDCODE`, read with
/// [`Job::capture_u32`](super::jtag::Job::capture_u32): it is a JTAG
/// register and really does come out least-significant bit first.
#[must_use]
pub fn idcode_job() -> Job {
    let mut scan = Scan::new();
    scan.reset();
    scan.shift_ir(&[Instruction::Idcode.bits()], IR_LENGTH);
    let _ = scan.read_dr(32);
    scan.finish()
}

/// The `CFG_IN` payload that asks the configuration engine for one
/// register, as 32-bit words in configuration (most-significant bit
/// first) order.
///
/// Dummy word, bus-width-safe sync, a no-op, the type 1 read, then two
/// no-ops to clock the answer into the output register — the padding
/// UG470's read-back sequence (chapter 6, "Readback") calls for.
#[must_use]
pub fn register_read_words(register: u32) -> [u32; 6] {
    [
        0xFFFF_FFFF,
        SYNC_WORD,
        NOOP,
        type1_header(0b01, register, 1),
        NOOP,
        NOOP,
    ]
}

/// The same words as a byte stream in configuration order (each word
/// big-endian), before the JTAG bit reversal.
#[must_use]
pub fn register_read_payload(register: u32) -> Vec<u8> {
    register_read_words(register)
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect()
}

/// Reads one configuration register: `CFG_IN` with a read packet, then
/// `CFG_OUT` and a 32-bit DR capture.
///
/// Capture 0 is the register, and it arrives most-significant bit first,
/// so it is decoded with [`word_msb_first`] and **not**
/// [`Job::capture_u32`](super::jtag::Job::capture_u32).
#[must_use]
pub fn register_job(register: u32) -> Job {
    let payload = reverse_all(&register_read_payload(register));
    let bits = payload.len() * 8;
    let mut scan = Scan::new();
    scan.shift_ir(&[Instruction::CfgIn.bits()], IR_LENGTH);
    scan.shift_dr(&payload, bits);
    scan.shift_ir(&[Instruction::CfgOut.bits()], IR_LENGTH);
    let _ = scan.read_dr(32);
    scan.finish()
}

/// Reads the status register.
#[must_use]
pub fn status_job() -> Job {
    register_job(REG_STAT)
}

/// Decodes the reply to a [`status_job`] or [`register_job`].
///
/// # Errors
///
/// [`JobError`](super::jtag::JobError) when the reply is too short for
/// the job that produced it.
pub fn read_status(job: &Job, reply: &[u8]) -> Result<Status, super::jtag::JobError> {
    let packed = job.capture(0, reply)?;
    Ok(Status(word_msb_first(&packed)))
}

/// `JPROGRAM`: clears the configuration memory, then rests in
/// `Run-Test/Idle` while the part erases.
///
/// This is what makes the operation reversible and what makes it
/// destructive-of-nothing-permanent: it clears *SRAM*. The part reloads
/// from flash on the next power cycle exactly as it did before.
#[must_use]
pub fn erase_job() -> Job {
    let mut scan = Scan::new();
    scan.shift_ir(&[Instruction::JProgram.bits()], IR_LENGTH);
    // UG470's flow parks the TAP on ISC_NOOP while the erase runs, so
    // the instruction register is not left holding JPROGRAM.
    scan.shift_ir(&[Instruction::IscNoop.bits()], IR_LENGTH);
    scan.idle(ERASE_IDLE_CYCLES);
    scan.finish()
}

/// `CFG_IN` followed by the whole configuration payload shifted through
/// DR.
///
/// `payload` is the `.bit` file's configuration data **as it appears in
/// the file**, most-significant bit first; the reversal to JTAG order
/// happens here, once, so no caller can forget it. Nothing is captured,
/// so this job writes and never waits on a reply.
#[must_use]
pub fn configure_job(payload: &[u8]) -> Job {
    let reversed = reverse_all(payload);
    let bits = reversed.len() * 8;
    let mut scan = Scan::new();
    scan.shift_ir(&[Instruction::CfgIn.bits()], IR_LENGTH);
    scan.shift_dr(&reversed, bits);
    scan.finish()
}

/// `JSTART` and the startup clocks, then `BYPASS` so the TAP is left
/// holding nothing that does anything.
#[must_use]
pub fn start_job() -> Job {
    let mut scan = Scan::new();
    scan.shift_ir(&[Instruction::JStart.bits()], IR_LENGTH);
    scan.idle(STARTUP_IDLE_CYCLES);
    scan.shift_ir(&[Instruction::Bypass.bits()], IR_LENGTH);
    scan.idle(64);
    scan.finish()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_status_word_accounts_for_every_bit_it_reads() {
        // The two readings this project has taken from an XC7A35T after
        // configuring it: the first with Project X-Ray's Vivado-built
        // harness bitstream, the second with one Reticle wrote itself.
        // Every named field is identical; they differ in bit 28 alone,
        // which UG470 reserves.
        let vivado = Status(0x7010_7dfc);
        let ours = Status(0x6010_7dfc);

        for s in [vivado, ours] {
            assert!(s.done(), "both had finished configuring");
            assert!(s.release_done() && s.end_of_startup());
            assert!(s.init_b() && s.init_complete());
            assert!(s.mmcm_lock());
            assert!(!s.crc_error() && !s.id_error() && !s.dec_error());
            assert!(!s.xadc_over_temp());
            assert_eq!(s.mode(), 0b101, "the M pins select JTAG");
            assert_eq!(s.startup_state(), 4);
            assert_eq!(s.bus_width(), 0);
        }

        // The difference, and the whole point of reporting it: a decoder
        // that listed only the flags it knows showed these two as
        // identical, which is how a real difference stayed invisible.
        assert_eq!(vivado.unnamed_bits(), vec![28, 29, 30]);
        assert_eq!(ours.unnamed_bits(), vec![29, 30]);
        assert_eq!(vivado.0 ^ ours.0, 1 << 28);

        // And it reaches the text a person reads.
        let shown = format!("{vivado}");
        assert!(shown.contains("reserved bit28+bit29+bit30"), "{shown}");
        assert!(format!("{ours}").contains("reserved bit29+bit30"));
        assert!(
            shown.contains("DONE") && shown.contains("MODE 101"),
            "{shown}"
        );
    }

    #[test]
    fn only_a_xilinx_idcode_is_treated_as_one() {
        // The two parts this project has read over JTAG.
        assert!(is_xilinx(IDCODE_XC7A35T));
        assert!(is_xilinx(0x0362_D093));
        // A Gowin GW2A-18, which answers on a board plugged in here. Its
        // instruction register is eight bits, not six, so shifting this
        // module's instructions at it executes whatever lands on those
        // bits rather than misreading a register.
        assert!(!is_xilinx(0x0000_081B));
        // The manufacturer field is bits 11..1 with the mandatory 1 in
        // bit 0, so the revision nibble must not affect the answer.
        for revision in 0..16u32 {
            assert!(is_xilinx((revision << 28) | 0x0362_D093));
        }
    }

    use super::*;
    use crate::program::jtag::unpack_capture;

    /// The opcodes are UG470 table 6-3's, six bits each.
    #[test]
    fn the_instruction_opcodes_are_the_documented_ones() {
        assert_eq!(Instruction::Idcode.bits(), 0x09);
        assert_eq!(Instruction::Usercode.bits(), 0x08);
        assert_eq!(Instruction::JProgram.bits(), 0x0B);
        assert_eq!(Instruction::JStart.bits(), 0x0C);
        assert_eq!(Instruction::JShutdown.bits(), 0x0D);
        assert_eq!(Instruction::CfgOut.bits(), 0x04);
        assert_eq!(Instruction::CfgIn.bits(), 0x05);
        assert_eq!(Instruction::IscNoop.bits(), 0x14);
        assert_eq!(Instruction::Bypass.bits(), 0x3F);
        for i in [
            Instruction::Idcode,
            Instruction::JProgram,
            Instruction::JStart,
            Instruction::CfgIn,
            Instruction::CfgOut,
            Instruction::Bypass,
        ] {
            assert!(u32::from(i.bits()) < (1 << IR_LENGTH), "{i:?} is too wide");
        }
    }

    /// A re-spun part reads back a different revision nibble and is
    /// still the same part; anything else is a mismatch.
    #[test]
    fn idcode_matching_ignores_only_the_revision() {
        assert!(idcode_matches(IDCODE_XC7A35T, IDCODE_XC7A35T));
        assert!(idcode_matches(0x1362_D093, IDCODE_XC7A35T));
        assert!(idcode_matches(0xF362_D093, IDCODE_XC7A35T));
        assert_eq!(idcode_revision(0x1362_D093), 1);
        // The XC7A50T, one part along, must not pass.
        assert!(!idcode_matches(0x0362_C093, IDCODE_XC7A35T));
        // A dead cable reads all ones or all zeros; neither is the part.
        assert!(!idcode_matches(0xFFFF_FFFF, IDCODE_XC7A35T));
        assert!(!idcode_matches(0x0000_0000, IDCODE_XC7A35T));
    }

    /// The type 1 read header for the status register is the value every
    /// other 7-series tool sends, which is the check that the field
    /// positions are right.
    #[test]
    fn the_status_read_packet_is_the_documented_one() {
        assert_eq!(type1_header(0b01, REG_STAT, 1), 0x2800_E001);
        // A write of one word to the same register, for the field split.
        assert_eq!(type1_header(0b10, REG_STAT, 1), 0x3000_E001);
        assert_eq!(type1_header(0b00, 0, 0), NOOP);
        let words = register_read_words(REG_STAT);
        assert_eq!(words[1], SYNC_WORD);
        assert_eq!(words[3], 0x2800_E001);
        let payload = register_read_payload(REG_STAT);
        assert_eq!(payload.len(), 24);
        assert_eq!(&payload[4..8], [0xAA, 0x99, 0x55, 0x66]);
    }

    /// Bit reversal moves bits inside bytes and leaves the byte order
    /// alone. Reversing the bytes instead is the classic mistake, so the
    /// two are distinguished here explicitly.
    #[test]
    fn the_payload_is_bit_reversed_and_not_byte_reversed() {
        assert_eq!(reverse_bits(0x01), 0x80);
        assert_eq!(reverse_bits(0x80), 0x01);
        assert_eq!(reverse_bits(0xAA), 0x55);
        assert_eq!(reverse_bits(0x00), 0x00);
        assert_eq!(reverse_bits(0xFF), 0xFF);

        let sync = SYNC_WORD.to_be_bytes();
        assert_eq!(sync, [0xAA, 0x99, 0x55, 0x66]);
        let shifted = reverse_all(&sync);
        assert_eq!(shifted, [0x55, 0x99, 0xAA, 0x66]);
        // Not the byte reversal, which would be [0x66, 0x55, 0x99, 0xAA].
        assert_ne!(shifted, [0x66, 0x55, 0x99, 0xAA]);
        // And it is an involution, so a double application is a no-op.
        assert_eq!(reverse_all(&shifted), sync);
    }

    /// The first bit the configuration engine must see is bit 7 of the
    /// payload's first byte, and after the reversal it is bit 0 of the
    /// first byte shifted — which is the first bit JTAG sends. Written
    /// out bit by bit, because this is the claim the whole thing rests
    /// on.
    #[test]
    fn the_first_bit_sent_is_the_payloads_most_significant_bit() {
        let payload = [0b1000_0000u8, 0b0000_0001];
        let shifted = reverse_all(&payload);
        // JTAG sends bit 0 of byte 0 first.
        assert_eq!(shifted[0] & 1, 1, "payload bit 7 must go out first");
        // ...and the payload's very last bit (bit 0 of the last byte)
        // must be the last one out, i.e. bit 7 of the last shifted byte.
        assert_eq!(shifted[1] >> 7, 1, "payload bit 0 must go out last");
    }

    /// A configuration register read is most-significant bit first, and
    /// `IDCODE` is least-significant bit first. Decoding either one with
    /// the other's rule gives a plausible but wrong number, so both are
    /// pinned here against the same captured bits.
    #[test]
    fn the_two_read_back_orders_are_opposites() {
        // TDO produced, in order, the 32 bits of 0x2000_0001 MSB first:
        // 0,0,1,0, 0,0,0,0, ... , 0,0,0,1.
        let value = 0x2000_0001u32;
        let mut packed = [0u8; 4];
        for i in 0..32 {
            if value >> (31 - i) & 1 == 1 {
                packed[i / 8] |= 1 << (i % 8);
            }
        }
        assert_eq!(word_msb_first(&packed), value);
        // Read LSB-first instead and it is a different, wrong number.
        assert_ne!(u32::from_le_bytes(packed), value);

        // And the mirror: an IDCODE arrives LSB first.
        let idcode = IDCODE_XC7A35T;
        let lsb = idcode.to_le_bytes();
        assert_eq!(u32::from_le_bytes(lsb), idcode);
        assert_ne!(word_msb_first(&lsb), idcode);
    }

    /// Every field is where UG470's status-register table puts it, and
    /// nowhere else: each one is checked against a word with that bit
    /// alone set, so a field that moved would land on a neighbour and
    /// be caught.
    #[test]
    fn the_status_bits_are_where_ug470_puts_them() {
        /// One field: its name, its bit index, and how `Status` reads it.
        type Field = (&'static str, u32, fn(Status) -> bool);
        let fields: [Field; 9] = [
            ("CRC_ERROR", 0, Status::crc_error),
            ("MMCM_LOCK", 2, Status::mmcm_lock),
            ("EOS", 4, Status::end_of_startup),
            ("INIT_COMPLETE", 11, Status::init_complete),
            ("INIT_B", 12, Status::init_b),
            ("RELEASE_DONE", 13, Status::release_done),
            ("DONE", 14, Status::done),
            ("ID_ERROR", 15, Status::id_error),
            ("DEC_ERROR", 16, Status::dec_error),
        ];
        for (name, bit, read) in fields {
            assert!(read(Status(1 << bit)), "{name} should be bit {bit}");
            assert!(!read(Status(!(1 << bit))), "{name} reads a neighbour");
        }
        // The mode pins are a three-bit field, and 0b101 is JTAG.
        assert_eq!(Status(0b101 << 8).mode(), 0b101);
        assert_eq!(Status(u32::MAX).mode(), 0b111);
        assert_eq!(Status(!(0b111 << 8)).mode(), 0);

        // The startup phase and the bus width are fields, so they are
        // always shown; a reserved bit is shown only when it is set.
        assert_eq!(
            Status(0).to_string(),
            "0x00000000 [no flags, MODE 000, STARTUP 0, BUS x1]"
        );
        // What an XC7A35T reads back once it is running a design, with
        // its mode pins set to JTAG.
        let running = (1 << 14) | (1 << 13) | (1 << 12) | (1 << 11) | (1 << 4) | (0b101 << 8);
        assert_eq!(
            Status(running).to_string(),
            "0x00007d10 [DONE RELEASE_DONE INIT_B INIT_COMPLETE EOS, \
             MODE 101, STARTUP 0, BUS x1]"
        );
    }

    /// `idcode_job` reads back exactly one 32-bit capture, and a reply
    /// built by the rules of the encoding decodes to the part's code.
    /// This is the read path end to end with no hardware.
    #[test]
    fn an_idcode_job_round_trips_a_synthetic_reply() {
        let job = idcode_job();
        assert_eq!(job.capture_count(), 1);
        assert_eq!(job.read_len(), 5);
        // The commands must contain the IDCODE opcode's five low bits as
        // a bit command: 0x09 & 0x1F = 0b01001.
        assert!(
            job.commands()
                .windows(3)
                .any(|w| w == [super::super::ftdi::CMD_BITS_OUT, 4, 0b0_1001])
        );

        let lsb = IDCODE_XC7A35T.to_le_bytes();
        let reply = [
            lsb[0],
            lsb[1],
            lsb[2],
            (lsb[3] & 0x7F) << 1,
            u8::from(lsb[3] & 0x80 != 0) << 7,
        ];
        let read = job.capture_u32(0, &reply).unwrap();
        assert_eq!(read, IDCODE_XC7A35T);
        assert!(idcode_matches(read, IDCODE_XC7A35T));
    }

    /// A status job carries the read packet bit-reversed, captures one
    /// 32-bit word, and `read_status` puts it back together the
    /// most-significant-bit-first way.
    #[test]
    fn a_status_job_carries_the_reversed_packet_and_decodes_msb_first() {
        let job = status_job();
        assert_eq!(job.capture_count(), 1);
        assert_eq!(job.read_len(), 5);
        // The reversed sync word must be somewhere in the commands.
        let reversed_sync = reverse_all(&SYNC_WORD.to_be_bytes());
        assert!(
            job.commands()
                .windows(4)
                .any(|w| w == reversed_sync.as_slice()),
            "the sync word must go out bit-reversed"
        );
        // ...and the unreversed one must not.
        assert!(
            !job.commands()
                .windows(4)
                .any(|w| w == SYNC_WORD.to_be_bytes()),
            "the sync word must not go out in file order"
        );

        // A part reporting DONE, EOS and INIT_COMPLETE.
        let value = (1 << 14) | (1 << 4) | (1 << 11);
        let mut packed = [0u8; 4];
        for i in 0..32u32 {
            if value >> (31 - i) & 1 == 1 {
                packed[i as usize / 8] |= 1 << (i % 8);
            }
        }
        let reply = [
            packed[0],
            packed[1],
            packed[2],
            (packed[3] & 0x7F) << 1,
            u8::from(packed[3] & 0x80 != 0) << 7,
        ];
        assert_eq!(unpack_capture(&reply, 32), packed);
        let status = read_status(&job, &reply).unwrap();
        assert_eq!(status.0, value);
        assert!(status.done());
        assert!(status.end_of_startup());
    }

    /// The configuration job writes and never reads, and its length is
    /// the payload plus the per-command overhead — so a two-megabyte
    /// bitstream produces a two-megabyte transfer rather than something
    /// quadratic.
    #[test]
    fn the_configuration_job_is_write_only_and_linear() {
        let payload = vec![0x5Au8; 300_000];
        let job = configure_job(&payload);
        assert_eq!(job.read_len(), 0);
        assert_eq!(job.capture_count(), 0);
        assert!(
            job.commands().len() < payload.len() + 1024,
            "the encoding must not grow the payload"
        );
        assert!(job.commands().len() > payload.len());
        // The payload's bytes appear reversed, never as they are in the
        // file: 0x5A reverses to 0x5A's mirror, 0b0101_1010 -> 0b0101_1010
        // is a palindrome, so use a byte that is not.
        let payload = vec![0x01u8; 64];
        let job = configure_job(&payload);
        assert!(job.commands().windows(8).any(|w| w == [0x80; 8]));
        assert!(!job.commands().windows(8).any(|w| w == [0x01; 8]));
    }

    /// `erase_job` leaves the instruction register on `ISC_NOOP` and
    /// clocks the idle the erase needs; `start_job` ends on `BYPASS`.
    /// Neither reads anything back.
    #[test]
    fn the_erase_and_start_jobs_are_write_only() {
        let erase = erase_job();
        assert_eq!(erase.read_len(), 0);
        // ERASE_IDLE_CYCLES/8 bytes of idle have to be in there.
        assert!(erase.commands().len() > ERASE_IDLE_CYCLES / 8);

        let start = start_job();
        assert_eq!(start.read_len(), 0);
        assert!(start.commands().len() > STARTUP_IDLE_CYCLES / 8);
    }

    /// `init_job` opens with the MPSSE setup before it touches the TAP:
    /// clocking a TAP at an unset divisor is how a first attempt fails
    /// for no visible reason.
    #[test]
    fn the_init_job_configures_the_mpsse_first() {
        let job = init_job(29, 0x88, 0x8B);
        let c = job.commands();
        assert_eq!(c[0], super::super::ftdi::CMD_DISABLE_DIV5);
        assert!(
            c.windows(3)
                .any(|w| w == [super::super::ftdi::CMD_SET_BITS_LOW, 0x88, 0x8B])
        );
        assert_eq!(job.read_len(), 0);
    }
}
