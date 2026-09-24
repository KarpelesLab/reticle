//! Configuring a Gowin GW1N/GW2A part's SRAM over JTAG, as a sequence of
//! [`Job`]s.
//!
//! The sequence is Gowin's own, from **UG290** (*Gowin FPGA Products
//! Programming and Configuration Guide*), section *JTAG Configuration*,
//! and **TN653** (*Gowin FPGA JTAG Programming*): reset the TAP, check the
//! identifier, erase the SRAM if it holds a design, then
//!
//! ```text
//! CONFIG_ENABLE (0x15)  ADDRESS_INITIALIZE (0x12)  TRANSFER (0x17)
//! <the whole .fs file through DR, first character first>
//! 0x0a  <the file's checksum, 32 bits>  0x08
//! CONFIG_DISABLE (0x3a)  NOOP (0x02)
//! ```
//!
//! and read the status register. Where UG290 leaves a detail open this
//! follows openFPGALoader's `src/gowin.cpp`, which loads these parts in
//! daily use: six idle clocks after every instruction, and polling the
//! status register for *edit mode* and *memory erase* instead of trusting
//! a fixed wait.
//!
//! **The checksum step is openFPGALoader's and not Gowin's documents'**:
//! neither UG290 nor TN653 names instructions `0x0a` or `0x08`. It is not
//! optional. On a GW2A-18 the documented sequence without it erased the
//! part, shifted the file, and left `DONE` clear with no error bit set;
//! openFPGALoader sends it on every load. The two codes are the `.fs`
//! footer's own command bytes — `0x0a` carries the configuration data's
//! checksum and `0x08` ends the stream — and the value is the one that
//! footer line holds ([`fs_checksum`]), which openFPGALoader's
//! `FsParser::checksum` recomputes to the same number: the configuration
//! rows as written, joined, summed as sixteen-bit words.
//!
//! # SRAM only, by construction
//!
//! [`Instruction`] lists every opcode this module can shift, and it holds
//! none that reaches flash. The ones that must never be sent are named in
//! [`FORBIDDEN`] with what they do, and a test checks the two lists never
//! meet. On a GW2A that matters doubly: it has no flash of its own, and
//! `0x16` is a pass-through to the *board's* SPI flash.
//!
//! Nothing here performs I/O; see [`super::xilinx`] for why.
//!
//! # Bit order
//!
//! A `.fs` file is text: one line per command or configuration row, one
//! `0`/`1` character per bit. Gowin loads it "from the MSB, bit by bit",
//! which is to say the **first character is the first bit on TDI**. JTAG
//! shifts least-significant bit first, so [`fs_payload`] packs eight
//! characters into a byte first-character-highest, as the file reads,
//! and [`write_job`] bit-reverses each byte before shifting — the same
//! correction [`super::xilinx::configure_job`] makes for a `.bit`.
//!
//! The status register, like `IDCODE`, is an ordinary JTAG register and
//! is read least-significant bit first with no correction.

use super::jtag::{Job, Scan};
use super::xilinx::reverse_all;

/// A GW1N or GW2A instruction register is eight bits wide (UG290,
/// *JTAG Configuration*; TN653 §2.2.4).
pub const IR_LENGTH: usize = 8;

/// Clocks to rest in `Run-Test/Idle` after each instruction. UG290 asks
/// for at least three; openFPGALoader settled on six after five proved
/// marginal.
pub const INSTRUCTION_IDLE_CYCLES: usize = 6;

/// The instructions this module shifts (UG290, *JTAG Configuration
/// Instructions*). Every one reads, or acts on SRAM only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Instruction {
    /// Ends a command sequence.
    Noop = 0x02,
    /// Clears the SRAM configuration.
    EraseSram = 0x05,
    /// Marks the end of an SRAM erase.
    XferDone = 0x09,
    /// Selects the 32-bit identifier register.
    ReadIdcode = 0x11,
    /// Resets the configuration address counter.
    InitAddress = 0x12,
    /// Enters configuration (edit) mode.
    ConfigEnable = 0x15,
    /// Selects the configuration data register, for the bitstream.
    Transfer = 0x17,
    /// Leaves configuration mode, which starts the loaded design.
    ConfigDisable = 0x3a,
    /// Selects the 32-bit status register.
    Status = 0x41,
    /// Takes the configuration data's checksum through a 32-bit DR.
    /// Undocumented; see the module documentation.
    Checksum = 0x0a,
    /// Follows the checksum. Undocumented; see the module documentation.
    ChecksumDone = 0x08,
}

impl Instruction {
    /// The opcode.
    pub fn bits(self) -> u8 {
        self as u8
    }

    /// Every instruction this module can shift.
    pub const ALL: [Instruction; 11] = [
        Instruction::Noop,
        Instruction::EraseSram,
        Instruction::XferDone,
        Instruction::ReadIdcode,
        Instruction::InitAddress,
        Instruction::ConfigEnable,
        Instruction::Transfer,
        Instruction::ConfigDisable,
        Instruction::Status,
        Instruction::Checksum,
        Instruction::ChecksumDone,
    ];
}

/// Opcodes this module must never shift, and why. None is in
/// [`Instruction`]; a test holds that true.
pub const FORBIDDEN: &[(u8, &str)] = &[
    (
        0x16,
        "GW2A: pass-through to the board's SPI flash, which writes it",
    ),
    (
        0x3c,
        "reload the configuration from flash, replacing what was loaded",
    ),
    (0x3d, "GW1N: SPI flash access mode"),
    (0x3f, "GW5A: SPI and boot configuration"),
    (0x71, "program the internal flash"),
    (0x75, "erase the internal flash"),
    (0x78, "enable the second flash"),
    (0x7a, "hand JTAG to the on-chip MCU"),
];

/// The identifiers of the parts this module will configure: `(IDCODE,
/// name)`. Only one has been read from a board. Apicula notes that a
/// GW2A-18, a GW2A-18C and a GW2AR-18C all answer it.
pub const KNOWN_PARTS: &[(u32, &str)] = &[(0x0000_081b, "GW2A(R)-18(C)")];

/// The name of a part this module knows, by identifier.
pub fn part_name(idcode: u32) -> Option<&'static str> {
    KNOWN_PARTS
        .iter()
        .find(|(id, _)| *id == idcode)
        .map(|(_, name)| *name)
}

/// The status register, as a GW2A reports it (UG290 table 7-13, *Status
/// Register*; TN653 table 2-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status(pub u32);

impl Status {
    fn bit(self, n: u32) -> bool {
        self.0 >> n & 1 == 1
    }

    /// Bit 0: the configuration data failed its CRC.
    pub fn crc_error(self) -> bool {
        self.bit(0)
    }

    /// Bit 1: a command the part did not accept.
    pub fn bad_command(self) -> bool {
        self.bit(1)
    }

    /// Bit 2: the bitstream's identifier is not this part's.
    pub fn id_verify_failed(self) -> bool {
        self.bit(2)
    }

    /// Bit 3: the configuration timed out.
    pub fn timeout(self) -> bool {
        self.bit(3)
    }

    /// Bit 5: the SRAM has been erased.
    pub fn memory_erase(self) -> bool {
        self.bit(5)
    }

    /// Bit 6: a preamble was seen.
    pub fn preamble(self) -> bool {
        self.bit(6)
    }

    /// Bit 7: configuration (edit) mode.
    pub fn edit_mode(self) -> bool {
        self.bit(7)
    }

    /// Bit 13: configuration finished and the design is running.
    pub fn done(self) -> bool {
        self.bit(13)
    }

    /// Bit 14: the security bit is set.
    pub fn security(self) -> bool {
        self.bit(14)
    }

    /// Any of the four error bits.
    pub fn error(self) -> bool {
        self.0 & 0xf != 0
    }

    /// The fields that are set, by name, for a report.
    pub fn flags(self) -> Vec<&'static str> {
        [
            (0, "CRC_ERROR"),
            (1, "BAD_COMMAND"),
            (2, "ID_VERIFY_FAILED"),
            (3, "TIMEOUT"),
            (5, "MEMORY_ERASE"),
            (6, "PREAMBLE"),
            (7, "EDIT_MODE"),
            (8, "PROGRAM_SPI_FLASH"),
            (10, "NON_JTAG_CONFIG"),
            (11, "BYPASS"),
            (13, "DONE"),
            (14, "SECURITY"),
            (15, "ENCRYPTION_FORMAT"),
            (16, "ENCRYPTION_KEY_MATCH"),
        ]
        .into_iter()
        .filter(|(n, _)| self.bit(*n))
        .map(|(_, name)| name)
        .collect()
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#010x} [{}]", self.0, self.flags().join(" "))
    }
}

/// Why a `.fs` file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsError {
    /// A line held something other than `0` and `1`.
    NotBinary {
        /// The line, counting from one.
        line: usize,
    },
    /// A line's length is not a whole number of bytes.
    Ragged {
        /// The line, counting from one.
        line: usize,
        /// Its length in characters.
        length: usize,
    },
    /// There was nothing to load.
    Empty,
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::NotBinary { line } => {
                write!(f, "line {line} is not made of `0` and `1` characters")
            }
            FsError::Ragged { line, length } => write!(
                f,
                "line {line} is {length} characters, not a whole number of bytes"
            ),
            FsError::Empty => write!(f, "the file holds no configuration data"),
        }
    }
}

impl std::error::Error for FsError {}

/// A `.fs` file as the bytes to shift, packed first character highest.
///
/// Every line is sent: the header's commands, the configuration rows and
/// the footer, in file order. Lines starting with `/` are comments and
/// are skipped, as are blank lines and trailing `\r`.
///
/// # Errors
///
/// [`FsError`] for a line that is not binary or not whole bytes, and for
/// a file with nothing in it.
pub fn fs_payload(text: &str) -> Result<Vec<u8>, FsError> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('/') {
            continue;
        }
        if line.len() % 8 != 0 {
            return Err(FsError::Ragged {
                line: index + 1,
                length: line.len(),
            });
        }
        for chunk in line.as_bytes().chunks(8) {
            let mut byte = 0u8;
            for &c in chunk {
                byte = (byte << 1)
                    | match c {
                        b'0' => 0,
                        b'1' => 1,
                        _ => return Err(FsError::NotBinary { line: index + 1 }),
                    };
            }
            out.push(byte);
        }
    }
    if out.is_empty() {
        return Err(FsError::Empty);
    }
    Ok(out)
}

/// The IDCODE a `.fs` file's `0x06` command checks, if it has one: an
/// eight-byte line `06 00 00 00 <idcode, big-endian>`.
pub fn fs_idcode(text: &str) -> Option<u32> {
    text.lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| l.len() == 64 && l.bytes().all(|c| c == b'0' || c == b'1'))
        .find(|l| &l[..8] == "00000110")
        .and_then(|l| u32::from_str_radix(&l[32..], 2).ok())
}

/// The checksum a `.fs` file's footer carries in its `0x0a` command: an
/// eight-byte line `0a 00 00 00 <value, big-endian>`.
pub fn fs_checksum(text: &str) -> Option<u32> {
    text.lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| l.len() == 64 && l.bytes().all(|c| c == b'0' || c == b'1'))
        .find(|l| &l[..8] == "00001010")
        .and_then(|l| u32::from_str_radix(&l[32..], 2).ok())
}

/// Shifts one instruction and rests.
fn instruction(scan: &mut Scan, op: Instruction) {
    scan.shift_ir(&[op.bits()], IR_LENGTH);
    scan.idle(INSTRUCTION_IDLE_CYCLES);
}

/// Reads the status register. Capture 0 is the word; decode it with
/// [`read_status`].
#[must_use]
pub fn status_job() -> Job {
    let mut scan = Scan::new();
    instruction(&mut scan, Instruction::Status);
    let _ = scan.read_dr(32);
    scan.finish()
}

/// Decodes the reply to a [`status_job`].
///
/// # Errors
///
/// [`JobError`](super::jtag::JobError) when the reply is too short.
pub fn read_status(job: &Job, reply: &[u8]) -> Result<Status, super::jtag::JobError> {
    job.capture_u32(0, reply).map(Status)
}

/// `CONFIG_ENABLE`: into edit mode. Poll [`Status::edit_mode`] after.
#[must_use]
pub fn enable_job() -> Job {
    let mut scan = Scan::new();
    instruction(&mut scan, Instruction::ConfigEnable);
    scan.finish()
}

/// `ERASE_SRAM` and `NOOP`. Poll [`Status::memory_erase`] after, then
/// send [`erase_done_job`]. UG290 gives 6 ms as a GW2A-18's reference
/// erase time.
#[must_use]
pub fn erase_job() -> Job {
    let mut scan = Scan::new();
    instruction(&mut scan, Instruction::EraseSram);
    instruction(&mut scan, Instruction::Noop);
    scan.finish()
}

/// `XFER_DONE`, `NOOP`, then `CONFIG_DISABLE`, `NOOP`: the erase is over
/// and edit mode is left. Poll for [`Status::edit_mode`] clear after.
#[must_use]
pub fn erase_done_job() -> Job {
    let mut scan = Scan::new();
    instruction(&mut scan, Instruction::XferDone);
    instruction(&mut scan, Instruction::Noop);
    instruction(&mut scan, Instruction::ConfigDisable);
    instruction(&mut scan, Instruction::Noop);
    scan.finish()
}

/// The whole load: `CONFIG_ENABLE`, `ADDRESS_INITIALIZE`, `TRANSFER`, the
/// payload through DR in one scan, the checksum, then `CONFIG_DISABLE`
/// and `NOOP`.
///
/// `payload` is [`fs_payload`]'s output, first character highest; the
/// reversal to JTAG order happens here, once. `checksum` is
/// [`fs_checksum`]'s, and goes through DR least-significant bit first,
/// like any JTAG register.
#[must_use]
pub fn write_job(payload: &[u8], checksum: u32) -> Job {
    let reversed = reverse_all(payload);
    let mut scan = Scan::new();
    instruction(&mut scan, Instruction::ConfigEnable);
    instruction(&mut scan, Instruction::InitAddress);
    instruction(&mut scan, Instruction::Transfer);
    scan.shift_dr(&reversed, reversed.len() * 8);
    instruction(&mut scan, Instruction::Checksum);
    scan.shift_dr(&checksum.to_le_bytes(), 32);
    instruction(&mut scan, Instruction::ChecksumDone);
    instruction(&mut scan, Instruction::ConfigDisable);
    instruction(&mut scan, Instruction::Noop);
    scan.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_instruction_this_can_shift_reaches_flash() {
        for op in Instruction::ALL {
            assert!(
                FORBIDDEN.iter().all(|(code, _)| *code != op.bits()),
                "{op:?} is {:#04x}, which is forbidden",
                op.bits()
            );
        }
    }

    /// UG290's two success values (table 7-13's notes), and the value
    /// reported after a CRC failure on a Tang Primer 20K in
    /// openFPGALoader's issue tracker.
    #[test]
    fn the_status_word_decodes_as_ug290_lays_it_out() {
        let loaded = Status(0x0000_2020);
        assert!(loaded.done() && loaded.memory_erase() && !loaded.error());
        assert!(!loaded.edit_mode());
        let secured = Status(0x0000_6020);
        assert!(secured.done() && secured.security());
        let crc = Status(0xa1);
        assert!(crc.crc_error() && crc.edit_mode() && crc.memory_erase() && crc.error());
        assert_eq!(
            crc.to_string(),
            "0x000000a1 [CRC_ERROR MEMORY_ERASE EDIT_MODE]"
        );
    }

    #[test]
    fn a_fs_file_packs_first_character_highest() {
        let text = "//comment\n1000000000000001\r\n\n11111111\n";
        assert_eq!(fs_payload(text).unwrap(), vec![0x80, 0x01, 0xff]);
        assert_eq!(
            fs_payload("1010"),
            Err(FsError::Ragged { line: 1, length: 4 })
        );
        assert_eq!(fs_payload("1010201a"), Err(FsError::NotBinary { line: 1 }));
        assert_eq!(fs_payload("//only a comment\n"), Err(FsError::Empty));
    }

    #[test]
    fn the_idcode_comes_from_the_0x06_command() {
        let line = format!("{:08b}{:024b}{:032b}", 0x06, 0, 0x0000_081bu32);
        let text = format!("1111111111111111\n{line}\n");
        assert_eq!(fs_idcode(&text), Some(0x0000_081b));
        assert_eq!(fs_idcode("1111111111111111\n"), None);
        let sum = format!("{:08b}{:024b}{:032b}", 0x0a, 0, 0xbf45u32);
        assert_eq!(fs_checksum(&format!("{line}\n{sum}\n")), Some(0xbf45));
        assert_eq!(fs_checksum(&line), None);
        assert_eq!(part_name(0x0000_081b), Some("GW2A(R)-18(C)"));
        assert_eq!(part_name(0x0362_d093), None);
    }

    /// The write job reverses each byte, and only once: the first `.fs`
    /// character must leave first, which JTAG's LSB-first shift sends
    /// from bit 0.
    #[test]
    fn the_write_job_holds_the_payload_reversed() {
        let job = write_job(&[0x80, 0x01, 0xc0], 0xbf45);
        let commands = job.commands();
        // The two whole bytes of the scan go out as one MPSSE write of
        // the reversed bytes: 0x80 -> 0x01, 0x01 -> 0x80.
        assert!(
            commands.windows(2).any(|w| w == [0x01, 0x80]),
            "the reversed payload is not in {commands:02x?}"
        );
        assert_eq!(job.capture_count(), 0);
        assert_eq!(status_job().capture_count(), 1);
    }
}
