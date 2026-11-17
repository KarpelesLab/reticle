//! Configuring a Lattice iCE40 over the SPI bus it shares with its
//! configuration flash, through an FT2232H's MPSSE.
//!
//! An iCE40 has no JTAG worth the name. It configures itself from a SPI
//! flash at power-up (*master* mode), or takes a bitstream from a host
//! that holds its `SPI_SS_B` low while `CRESET_B` rises (*slave* mode).
//! The usual board — Lattice's own breakouts, the iCEstick, and the
//! iCE40UP5K-SG48 board with an FT2232H this was written against — puts
//! the FT2232H's channel A on that bus the way `iceprog` expects:
//!
//! | `ADBUS` | MPSSE role | Board net |
//! |---|---|---|
//! | 0 | `SK` | `SPI_SCK` (FPGA pin 15 on an SG48) and the flash's `CLK` |
//! | 1 | `DO` | the flash's `DI`, which is also the FPGA's `SPI_SO` (pin 14) |
//! | 2 | `DI` | the flash's `DO`, which is also the FPGA's `SPI_SI` (pin 17) |
//! | 4 | GPIO | `SPI_SS_B` (pin 16) and the flash's `/CS` |
//! | 6 | GPIO, input | `CDONE` |
//! | 7 | GPIO | `CRESET_B` |
//!
//! [`ICEPROG_WIRING`] is that table.
//!
//! # Two things share one bus, and only one of them is the flash
//!
//! **The flash** is reached the obvious way: hold `CRESET_B` low so the
//! FPGA lets go of the bus, then talk SPI mode 0 to it with the MPSSE.
//!
//! **The FPGA's configuration memory** is the subtle part. In slave mode
//! the FPGA takes its data on `SPI_SI` — and on this wiring `SPI_SI` is
//! the *flash's output*, which reaches the FT2232H on `ADBUS2`, the MPSSE's
//! **input**. `iceprog -S` shifts the bitstream out of `ADBUS1` instead,
//! which on this wiring lands on the FPGA's `SPI_SO`, an output; the board
//! has to be rewired (Lattice's breakout boards have jumpers for exactly
//! this) before `iceprog -S` can work. [`SramPath`] names both routes.
//! [`SramPath::FlashOutput`] is the one this wiring needs: the MPSSE turns
//! `ADBUS2` into an output and toggles `SK` and it from GPIO commands, one
//! bit at a time — slow by MPSSE standards and still well under a second
//! for an UP5K's 104 KB. `ADBUS1` is left an **input** throughout, so the
//! FT2232H drives nothing onto the FPGA's `SPI_SO`.
//!
//! The flash sees all of this, because its `/CS` is the FPGA's
//! `SPI_SS_B`. What it sees is harmless by construction: with `ADBUS1`
//! undriven its `DI` sits at the FT2232H's internal pull-up, so the one
//! command byte it decodes in each select is `0xFF`, which a W25Q-series
//! part ignores, and a flash never drives `DO` unless a read command asked
//! it to. Which pull-up, and that the level really is high, is something
//! [`Programmer::bus_levels`] reads back rather than assumes.
//!
//! # Sans-I/O, again
//!
//! Every sequence here is a [`super::ftdi::Mpsse`] buffer. What moves the
//! bytes is a [`Link`], which [`super::usb::Cable`] implements and the
//! tests implement with a model of a W25Q flash and an iCE40's slave port
//! (the `tests` module), so that the order of `CRESET_B` and `SPI_SS_B`, the
//! write-enable before every erase, and which sectors a write touches are
//! all checked with no board attached.
//!
//! # Requests that are never sent
//!
//! [`NOT_SENT`] is the list, in the spirit of `docs/apollo-protocol.md`
//! §7: flash opcodes that exist, that some tool somewhere sends, and that
//! this one does not — status-register writes, the security and OTP
//! registers, block locks, deep power-down, and chip erase. The only
//! opcodes this module puts on the bus are [`SENT`]; the flash model in
//! the tests records every opcode it is sent and the tests assert that set
//! never leaves [`SENT`].

use std::time::Duration;

use super::ProgramError;
use super::ftdi::{Chip, Mpsse};

/// What carries MPSSE bytes to an adapter and back, and how to wait.
///
/// [`super::usb::Cable`] is the real one. The tests implement it with a
/// simulated board, which is what makes everything in this module
/// checkable without one.
pub trait Link {
    /// Writes `commands` and returns exactly `read_len` bytes of what they
    /// captured.
    ///
    /// # Errors
    ///
    /// Whatever the transport reports.
    fn exchange(&mut self, commands: &[u8], read_len: usize) -> Result<Vec<u8>, ProgramError>;

    /// Waits. A sequence that needs the board to settle — the FPGA
    /// clearing its configuration memory after `CRESET_B` rises, a flash
    /// finishing an erase — says so through this rather than sleeping
    /// itself.
    fn pause(&mut self, how_long: Duration);
}

/// Which `ADBUS` bit carries each net, as masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wiring {
    /// The SPI clock (`ADBUS0` for the MPSSE to drive it).
    pub sck: u8,
    /// Host-to-flash data (`ADBUS1`, the MPSSE's `DO`).
    pub mosi: u8,
    /// Flash-to-host data (`ADBUS2`, the MPSSE's `DI`), which is also the
    /// FPGA's `SPI_SI`.
    pub miso: u8,
    /// The flash's `/CS` and the FPGA's `SPI_SS_B`.
    pub cs: u8,
    /// The FPGA's `CDONE`, an input.
    pub cdone: u8,
    /// The FPGA's `CRESET_B`.
    pub creset: u8,
}

impl Wiring {
    /// The output value to write when letting go of every pin.
    ///
    /// The pins become inputs, so the value should not matter, and it
    /// does: the FT2232H applies a `SET_BITS_LOW`'s value **before** its
    /// direction, so for an instant every pin still driven takes the new
    /// value. Releasing with a value of zero pulsed `CRESET_B` low for
    /// that instant — long enough to reset the part — and a design just
    /// loaded into configuration memory was replaced, on release, by the
    /// flash's. With `CRESET_B` and `/CS` written high, the instant changes
    /// nothing: they were high or about to float high on their pull-ups.
    #[must_use]
    pub fn released(&self) -> u8 {
        self.creset | self.cs
    }
}

/// The FT2232H channel A wiring every `iceprog`-compatible board uses.
///
/// `iceprog.c` drives the low byte with direction `0x93` — `SK`, `DO`,
/// `ADBUS4` and `ADBUS7` — and reads `CDONE` as bit 6, which is where
/// these come from. QUOTED from `iceprog` and the board's schematic notes,
/// then CHECKED on an iCE40UP5K-SG48 board: the flash answers its JEDEC
/// identifier over these pins, `CDONE` reads high while a design runs and
/// low with `CRESET_B` held, and a bitstream shifted through them
/// configures the part. `docs/programming.md` has the record.
pub const ICEPROG_WIRING: Wiring = Wiring {
    sck: 0x01,
    mosi: 0x02,
    miso: 0x04,
    cs: 0x10,
    cdone: 0x40,
    creset: 0x80,
};

/// How a bitstream reaches the FPGA's `SPI_SI` in slave mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SramPath {
    /// Out of the MPSSE's `DO` (`ADBUS1`) at full shift-engine speed, as
    /// `iceprog -S` does. Right for a board whose `ADBUS1` is wired to the
    /// FPGA's `SPI_SI` — a Lattice breakout with its jumpers in the
    /// configuration-memory position.
    HostOutput,
    /// Out of `ADBUS2`, the net the flash's `DO` and the FPGA's `SPI_SI`
    /// share, toggled by GPIO commands. Right for a board wired for flash
    /// programming, which is every board with no jumpers.
    FlashOutput,
}

/// `READ DATA`: three address bytes, then data for as long as the
/// clock runs.
pub const OP_READ: u8 = 0x03;
/// `PAGE PROGRAM`: three address bytes and up to 256 data bytes.
pub const OP_PAGE_PROGRAM: u8 = 0x02;
/// `READ STATUS REGISTER-1`. Bit 0 is `BUSY`, bit 1 `WEL`.
pub const OP_READ_STATUS: u8 = 0x05;
/// `WRITE ENABLE`: sets `WEL`, which every erase and program needs and
/// clears.
pub const OP_WRITE_ENABLE: u8 = 0x06;
/// `WRITE DISABLE`: clears `WEL`. Sent after a failed write so the part is
/// not left armed.
pub const OP_WRITE_DISABLE: u8 = 0x04;
/// `SECTOR ERASE`: one 4 KiB sector, three address bytes.
pub const OP_SECTOR_ERASE: u8 = 0x20;
/// `READ JEDEC ID`: manufacturer, memory type, capacity.
pub const OP_JEDEC_ID: u8 = 0x9F;
/// `RELEASE POWER-DOWN`, which also wakes a part another tool left
/// asleep (`iceprog` powers the flash down when it finishes).
pub const OP_RELEASE_POWER_DOWN: u8 = 0xAB;
/// `READ UNIQUE ID`: four dummy bytes, then a 64-bit serial number. A
/// read, and a way to tell two boards apart that have no USB serial.
pub const OP_UNIQUE_ID: u8 = 0x4B;

/// Every flash opcode this module sends. Nothing else is ever put on the
/// bus as a command byte; the tests' flash model records every opcode it
/// decodes and asserts this.
pub const SENT: &[u8] = &[
    OP_READ,
    OP_PAGE_PROGRAM,
    OP_READ_STATUS,
    OP_WRITE_ENABLE,
    OP_WRITE_DISABLE,
    OP_SECTOR_ERASE,
    OP_JEDEC_ID,
    OP_RELEASE_POWER_DOWN,
    OP_UNIQUE_ID,
];

/// Flash opcodes that exist on a W25Q-series part and are **never sent**,
/// with why. This is the list `docs/programming.md` reproduces.
///
/// Most of them change something a power cycle does not change back and
/// that no bitstream needs changed: the status registers hold the block
/// protection and the quad-enable bit that the FPGA's own boot depends on
/// being clear, and the security registers and their lock bits are
/// one-time programmable. Chip erase is here because a whole-chip erase is
/// never what writing one image needs — [`plan_write`] erases only the
/// sectors whose contents must change — and a 16 MiB flash may hold more
/// than one image. Deep power-down is here because a flash left asleep is
/// one the FPGA cannot boot from until something wakes it.
pub const NOT_SENT: &[(u8, &str, &str)] = &[
    (
        0x01,
        "WRITE STATUS REGISTER-1",
        "non-volatile block protection; the FPGA's boot needs none",
    ),
    (
        0x31,
        "WRITE STATUS REGISTER-2",
        "holds QE and the OTP lock bits; QE set breaks single-SPI boot",
    ),
    (
        0x11,
        "WRITE STATUS REGISTER-3",
        "drive strength and the protection scheme",
    ),
    (
        0x50,
        "VOLATILE SR WRITE ENABLE",
        "only exists to precede a status-register write",
    ),
    (
        0x42,
        "PROGRAM SECURITY REGISTER",
        "one-time programmable once locked",
    ),
    (
        0x44,
        "ERASE SECURITY REGISTER",
        "one-time programmable once locked",
    ),
    (
        0x36,
        "INDIVIDUAL BLOCK LOCK",
        "a write protection the next tool has to know to undo",
    ),
    (0x7E, "GLOBAL BLOCK LOCK", "as individual block lock"),
    (0x98, "GLOBAL BLOCK UNLOCK", "the other half of the lock"),
    (
        0xC7,
        "CHIP ERASE",
        "never what writing one image needs; sectors are erased instead",
    ),
    (0x60, "CHIP ERASE", "the same operation's other opcode"),
    (
        0xB9,
        "POWER-DOWN",
        "a sleeping flash does not answer until woken",
    ),
];

/// The FT2232H's SPI clock this module uses unless told otherwise: 6 MHz.
///
/// The W25Q128 reads at up to 50 MHz with `READ DATA`; the limit is the
/// board. Six megahertz is the rate the board's own flashing script was
/// run at, and a 16 MiB backup takes about 25 seconds at it.
pub const DEFAULT_SPI_HZ: u32 = 6_000_000;

/// One page: the most `PAGE PROGRAM` takes, and its alignment.
pub const PAGE: usize = 256;
/// One sector: the smallest erase.
pub const SECTOR: usize = 4096;

/// Bytes per read job. The command is three bytes whatever the length, so
/// the limit is only how often progress is reported.
const READ_CHUNK: usize = 64 * 1024;

/// How long `CRESET_B` is held low before it is released: Lattice gives a
/// 200 ns minimum (iCE40 family data sheet, `tCRESET_N`); a millisecond
/// is generous without being slow.
const RESET_PULSE: Duration = Duration::from_millis(1);

/// How long the FPGA needs after `CRESET_B` rises before it can take a
/// bitstream: 1 200 µs for the UP5K, 800 µs for the LP/HX parts (TN1248,
/// *iCE40 Programming and Configuration*, slave SPI timing). Two
/// milliseconds covers both, and is what `iceprog` waits.
const CLEAR_TIME: Duration = Duration::from_millis(2);

/// How long after `CRESET_B` rises a load waits, sending nothing, to see
/// that the part did not boot from its flash instead. An UP5K booting from
/// a flash at its default rate raises `CDONE` in about 70 ms.
const MASTER_CHECK: Duration = Duration::from_millis(200);

/// Dummy clocks after the last bitstream byte. TN1248 asks for at least
/// 49 to finish configuration and wake the I/O; this sends more, which
/// costs nothing.
const TRAILING_CLOCKS: usize = 160;

/// How long between `BUSY` polls, and how many to try before giving up.
/// A W25Q128 sector erase is 45 ms typical and 400 ms worst case; a page
/// program 0.7 ms typical and 3 ms worst. Two seconds bounds both with a
/// wide margin and still fails quickly on a bus that answers nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(1);
/// See [`POLL_INTERVAL`].
const POLL_LIMIT: u32 = 2_000;

/// How long between `CDONE` polls after a boot, and how many: one second
/// in all. An UP5K boots from a 6 MHz-default flash in about 70 ms.
const BOOT_POLL: Duration = Duration::from_millis(1);
/// See [`BOOT_POLL`].
const BOOT_POLLS: u32 = 1_000;

/// What the flash says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlashId {
    /// The three bytes of `READ JEDEC ID`.
    pub jedec: [u8; 3],
    /// The 64-bit serial of `READ UNIQUE ID`.
    pub unique: [u8; 8],
}

impl FlashId {
    /// The capacity the identifier's third byte encodes, as `2^n` bytes,
    /// or `None` for an identifier that is not a SPI NOR flash's — all
    /// ones or all zeros, which is what a bus with nothing on it reads.
    ///
    /// The encoding is JEDEC's and every SPI NOR vendor follows it for
    /// the parts an iCE40 boots from: `0x18` is 16 MiB (a W25Q128), `0x15`
    /// 2 MiB, and so on. Below 64 KiB or above 64 MiB is refused, because
    /// a part that small holds no iCE40 bitstream and one that large needs
    /// four-byte addresses that nothing here sends.
    #[must_use]
    pub fn capacity(&self) -> Option<usize> {
        let [maker, _, size] = self.jedec;
        if maker == 0x00 || maker == 0xFF || !(0x10..=0x19).contains(&size) {
            return None;
        }
        Some(1usize << size)
    }

    /// The manufacturer, for the ones a board is likely to carry.
    #[must_use]
    pub fn maker(&self) -> &'static str {
        match self.jedec[0] {
            0xEF => "Winbond",
            0xC2 => "Macronix",
            0x20 => "Micron",
            0x1F => "Adesto",
            0x9D => "ISSI",
            0xC8 => "GigaDevice",
            0x01 => "Spansion",
            _ => "unknown maker",
        }
    }
}

/// What a write did, for the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// Sectors that were already right and were not touched.
    pub sectors_unchanged: usize,
    /// Sectors erased.
    pub sectors_erased: usize,
    /// Pages programmed.
    pub pages_programmed: usize,
}

/// Which sectors a write must erase and which pages it must program.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WritePlan {
    /// The address of each 4 KiB sector to erase, ascending.
    pub erase: Vec<u32>,
    /// Each page to program, as its address and its 256 bytes, ascending.
    pub program: Vec<(u32, Vec<u8>)>,
    /// Sectors the write covers whose contents are already right.
    pub unchanged: usize,
}

/// Works out the least a write of `data` at `base` has to do, given the
/// flash's present contents.
///
/// `current` is what the flash holds from the start of the sector holding
/// `base` to the end of the sector holding the last byte — whole sectors,
/// because a sector is the unit of erase. Bytes of those sectors outside
/// `data` keep their present value, so a write that does not start or end
/// on a sector boundary does not lose what is beside it.
///
/// A sector is **left alone** when it already holds the right bytes. It is
/// **programmed without an erase** when every change is a one becoming a
/// zero, which is all a page program can do. Otherwise it is **erased** and
/// every page of it that is not all ones is programmed.
///
/// # Panics
///
/// When `current` is not the whole sectors described above: that is the
/// caller reading the wrong range, not a user's input.
#[must_use]
pub fn plan_write(base: u32, data: &[u8], current: &[u8]) -> WritePlan {
    let base = base as usize;
    let first = base / SECTOR * SECTOR;
    let end = (base + data.len()).div_ceil(SECTOR) * SECTOR;
    assert_eq!(
        current.len(),
        end - first,
        "plan_write needs the whole sectors the write covers"
    );
    let mut want = current.to_vec();
    want[base - first..base - first + data.len()].copy_from_slice(data);

    let mut plan = WritePlan::default();
    for (index, (now, new)) in current.chunks(SECTOR).zip(want.chunks(SECTOR)).enumerate() {
        let sector = first + index * SECTOR;
        if now == new {
            plan.unchanged += 1;
            continue;
        }
        let needs_erase = now.iter().zip(new).any(|(n, w)| !n & w != 0);
        if needs_erase {
            plan.erase.push(address(sector));
        }
        for (p, (page_now, page_new)) in now.chunks(PAGE).zip(new.chunks(PAGE)).enumerate() {
            let program = if needs_erase {
                page_new.iter().any(|&b| b != 0xFF)
            } else {
                page_now != page_new
            };
            if program {
                plan.program
                    .push((address(sector + p * PAGE), page_new.to_vec()));
            }
        }
    }
    plan
}

/// An address that is known to fit: every caller has checked it against
/// a flash of at most 64 MiB.
fn address(at: usize) -> u32 {
    u32::try_from(at).expect("a flash address fits in 32 bits")
}

/// Whether `bytes` looks like an iCE40 bitstream: the 32-bit
/// synchronisation word `7E AA 99 7E` within its first few hundred bytes,
/// after the `FF 00` comment header `icepack` writes.
///
/// It is a sanity check, not a parse: it catches an `.asc` (text), an ECP5
/// or Xilinx `.bit`, or the wrong file entirely, before the part is reset.
#[must_use]
pub fn looks_like_bitstream(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(1024)]
        .windows(4)
        .any(|w| w == [0x7E, 0xAA, 0x99, 0x7E])
}

/// The levels on the bus with nothing driving it but the board, which
/// [`Programmer::bus_levels`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusLevels {
    /// The whole `ADBUS` byte.
    pub raw: u8,
    /// `CDONE`.
    pub cdone: bool,
    /// `CRESET_B`.
    pub creset: bool,
    /// The flash's `DI` with the FT2232H not driving it, which is the
    /// level the flash decodes during a slave-mode load.
    pub mosi: bool,
}

/// An iCE40 board on the end of an MPSSE channel.
pub struct Programmer<L: Link> {
    link: L,
    wiring: Wiring,
}

impl<L: Link> Programmer<L> {
    /// Takes the channel, sets the SPI clock, and **drives nothing**: every
    /// pin is left an input until an operation needs it, so opening the
    /// programmer neither resets the FPGA nor selects the flash.
    ///
    /// # Errors
    ///
    /// Whatever the link reports.
    pub fn new(
        mut link: L,
        chip: Chip,
        divisor: u16,
        wiring: Wiring,
    ) -> Result<Programmer<L>, ProgramError> {
        let mut m = Mpsse::new();
        m.configure_chip(chip, divisor);
        m.set_pins_low(wiring.released(), 0);
        link.exchange(m.commands(), 0)?;
        Ok(Programmer { link, wiring })
    }

    /// The link, for a caller that wants it back.
    pub fn into_link(self) -> L {
        self.link
    }

    fn run(&mut self, m: &Mpsse) -> Result<Vec<u8>, ProgramError> {
        self.link.exchange(m.commands(), m.read_len())
    }

    /// The pins driven while the flash is being talked to: `CRESET_B` low
    /// so the FPGA has let go of the bus, the clock idle low, `/CS` as
    /// given.
    fn flash_pins(&self, selected: bool) -> (u8, u8) {
        let w = self.wiring;
        let value = if selected { 0 } else { w.cs };
        (value, w.sck | w.mosi | w.cs | w.creset)
    }

    /// One flash transaction: select, send `command`, read `reply_len`
    /// bytes, deselect.
    fn transaction(&self, m: &mut Mpsse, command: &[u8], reply_len: usize) {
        let (v, d) = self.flash_pins(true);
        m.set_pins_low(v, d);
        m.spi_write(command);
        if reply_len > 0 {
            m.spi_read(reply_len);
        }
        let (v, d) = self.flash_pins(false);
        m.set_pins_low(v, d);
    }

    /// Reads every `ADBUS` level with the FT2232H driving nothing — or,
    /// with `hold_reset`, nothing but `CRESET_B`, held low.
    ///
    /// This is what shows, rather than assumes, that `CDONE` is high on a
    /// configured part and what level the flash's `DI` floats to when
    /// nobody drives it, which is what the flash decodes during a
    /// slave-mode load.
    ///
    /// # Errors
    ///
    /// Whatever the link reports.
    pub fn bus_levels(&mut self, hold_reset: bool) -> Result<BusLevels, ProgramError> {
        let mut m = Mpsse::new();
        if hold_reset {
            m.set_pins_low(0, self.wiring.creset);
        } else {
            m.set_pins_low(self.wiring.released(), 0);
        }
        m.get_pins_low();
        m.send_immediate();
        let raw = self.run(&m)?[0];
        let w = self.wiring;
        Ok(BusLevels {
            raw,
            cdone: raw & w.cdone != 0,
            creset: raw & w.creset != 0,
            mosi: raw & w.mosi != 0,
        })
    }

    /// Holds the FPGA in reset, so that it lets go of the SPI bus, and
    /// wakes the flash.
    ///
    /// **This stops whatever the FPGA was running.** It comes back when
    /// [`boot`](Self::boot) releases it, configured from the flash.
    ///
    /// # Errors
    ///
    /// Whatever the link reports.
    pub fn take_flash(&mut self) -> Result<(), ProgramError> {
        let mut m = Mpsse::new();
        let (v, d) = self.flash_pins(false);
        m.set_pins_low(v, d);
        self.run(&m)?;
        // The FPGA tri-states its SPI pins as soon as CRESET_B is low; the
        // pause is for a part that was mid-boot and had the flash selected.
        self.link.pause(Duration::from_millis(10));
        let mut m = Mpsse::new();
        self.transaction(&mut m, &[OP_RELEASE_POWER_DOWN], 0);
        self.run(&m)?;
        // tRES1 is 3 µs.
        self.link.pause(Duration::from_millis(1));
        Ok(())
    }

    /// Reads the flash's identifiers. [`take_flash`](Self::take_flash)
    /// first.
    ///
    /// # Errors
    ///
    /// Whatever the link reports.
    pub fn flash_id(&mut self) -> Result<FlashId, ProgramError> {
        let mut m = Mpsse::new();
        self.transaction(&mut m, &[OP_JEDEC_ID], 3);
        self.transaction(&mut m, &[OP_UNIQUE_ID, 0, 0, 0, 0], 8);
        m.send_immediate();
        let reply = self.run(&m)?;
        let mut id = FlashId {
            jedec: [0; 3],
            unique: [0; 8],
        };
        id.jedec.copy_from_slice(&reply[..3]);
        id.unique.copy_from_slice(&reply[3..11]);
        Ok(id)
    }

    /// The identifier and capacity of a flash that is safe to write, or
    /// why it is not.
    fn checked_capacity(&mut self) -> Result<(FlashId, usize), ProgramError> {
        let id = self.flash_id()?;
        let capacity = id.capacity().ok_or_else(|| {
            ProgramError::Flash(format!(
                "the flash answered JEDEC ID {:02X} {:02X} {:02X}, which is not a SPI NOR \
                 flash this knows the size of; is the FPGA holding the bus, or is nothing \
                 there?",
                id.jedec[0], id.jedec[1], id.jedec[2]
            ))
        })?;
        Ok((id, capacity))
    }

    /// Reads status register 1.
    ///
    /// # Errors
    ///
    /// Whatever the link reports.
    pub fn status(&mut self) -> Result<u8, ProgramError> {
        let mut m = Mpsse::new();
        self.transaction(&mut m, &[OP_READ_STATUS], 1);
        m.send_immediate();
        Ok(self.run(&m)?[0])
    }

    /// Polls `BUSY` until it clears.
    fn wait_ready(&mut self, what: &str) -> Result<(), ProgramError> {
        for _ in 0..POLL_LIMIT {
            if self.status()? & 1 == 0 {
                return Ok(());
            }
            self.link.pause(POLL_INTERVAL);
        }
        // Leave the part disarmed rather than with WEL set.
        let mut m = Mpsse::new();
        self.transaction(&mut m, &[OP_WRITE_DISABLE], 0);
        self.run(&m)?;
        Err(ProgramError::Flash(format!(
            "the flash still reported BUSY {} ms after {what}",
            POLL_LIMIT
        )))
    }

    /// Reads `len` bytes from `at`, reporting `(done, total)` as it goes.
    /// [`take_flash`](Self::take_flash) first.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Flash`] for a range past the end of the flash, and
    /// whatever the link reports.
    pub fn read(
        &mut self,
        at: u32,
        len: usize,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Vec<u8>, ProgramError> {
        let (_, capacity) = self.checked_capacity()?;
        if at as usize + len > capacity {
            return Err(ProgramError::Flash(format!(
                "{len} bytes from {at:#x} runs past the end of a {capacity}-byte flash"
            )));
        }
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let take = (len - out.len()).min(READ_CHUNK);
            let addr = at as usize + out.len();
            let [_, a2, a1, a0] = address(addr).to_be_bytes();
            let mut m = Mpsse::new();
            self.transaction(&mut m, &[OP_READ, a2, a1, a0], take);
            m.send_immediate();
            out.extend(self.run(&m)?);
            progress(out.len(), len);
        }
        Ok(out)
    }

    /// Writes `data` at `at`, erasing and programming only what has to
    /// change ([`plan_write`]), then reads the whole range back and
    /// compares it. [`take_flash`](Self::take_flash) first.
    ///
    /// `progress` is told `(done, total)` over the erase-and-program steps.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Flash`] for a flash this does not know the size of,
    /// a write past its end, a write that never finishes, or a read-back
    /// that differs — with the first differing address.
    pub fn write(
        &mut self,
        at: u32,
        data: &[u8],
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<WriteReport, ProgramError> {
        let (_, capacity) = self.checked_capacity()?;
        let start = at as usize;
        if data.is_empty() || start + data.len() > capacity {
            return Err(ProgramError::Flash(format!(
                "{} bytes at {at:#x} does not fit a {capacity}-byte flash",
                data.len()
            )));
        }
        let first = start / SECTOR * SECTOR;
        let end = (start + data.len()).div_ceil(SECTOR) * SECTOR;
        let current = self.read(address(first), end - first, &mut |_, _| {})?;
        let plan = plan_write(at, data, &current);

        let total = plan.erase.len() + plan.program.len();
        let mut done = 0;
        for &sector in &plan.erase {
            let [_, a2, a1, a0] = sector.to_be_bytes();
            let mut m = Mpsse::new();
            self.transaction(&mut m, &[OP_WRITE_ENABLE], 0);
            self.transaction(&mut m, &[OP_SECTOR_ERASE, a2, a1, a0], 0);
            self.run(&m)?;
            self.wait_ready(&format!("erasing the sector at {sector:#x}"))?;
            done += 1;
            progress(done, total);
        }
        for (page, bytes) in &plan.program {
            let [_, a2, a1, a0] = page.to_be_bytes();
            let mut command = vec![OP_PAGE_PROGRAM, a2, a1, a0];
            command.extend_from_slice(bytes);
            let mut m = Mpsse::new();
            self.transaction(&mut m, &[OP_WRITE_ENABLE], 0);
            self.transaction(&mut m, &command, 0);
            self.run(&m)?;
            self.wait_ready(&format!("programming the page at {page:#x}"))?;
            done += 1;
            progress(done, total);
        }

        let back = self.read(at, data.len(), &mut |_, _| {})?;
        if let Some(i) = back.iter().zip(data).position(|(a, b)| a != b) {
            return Err(ProgramError::Flash(format!(
                "verify failed: the flash holds {:#04x} at {:#x} where {:#04x} was written",
                back[i],
                start + i,
                data[i]
            )));
        }
        Ok(WriteReport {
            sectors_unchanged: plan.unchanged,
            sectors_erased: plan.erase.len(),
            pages_programmed: plan.program.len(),
        })
    }

    /// Releases `CRESET_B` with `SPI_SS_B` high, so the FPGA configures
    /// itself from the flash as it does at power-up, lets go of the bus,
    /// and waits up to a second for `CDONE`.
    ///
    /// Returns the number of millisecond polls `CDONE` took to rise, or
    /// `None` if it never did. A pin the FT2232H drove while the FPGA
    /// boots would fight the FPGA for its own flash, so `SK`, `DO` and
    /// `/CS` are released in the same breath as `CRESET_B`: the FPGA reads
    /// `SPI_SS_B` high as it leaves reset, and then owns the bus.
    ///
    /// # Errors
    ///
    /// Whatever the link reports.
    pub fn boot(&mut self) -> Result<Option<u32>, ProgramError> {
        let w = self.wiring;
        let mut m = Mpsse::new();
        m.set_pins_low(w.cs, w.cs | w.creset);
        self.run(&m)?;
        self.link.pause(RESET_PULSE);
        let mut m = Mpsse::new();
        m.set_pins_low(w.cs | w.creset, w.cs | w.creset);
        m.set_pins_low(w.released(), 0);
        self.run(&m)?;
        self.poll_cdone()
    }

    fn poll_cdone(&mut self) -> Result<Option<u32>, ProgramError> {
        for poll in 0..BOOT_POLLS {
            if self.bus_levels(false)?.cdone {
                return Ok(Some(poll));
            }
            self.link.pause(BOOT_POLL);
        }
        Ok(None)
    }

    /// Loads `bitstream` into the FPGA's configuration memory as a SPI
    /// slave, over `path`, and reports whether `CDONE` rose.
    ///
    /// The sequence is TN1248's: `SPI_SS_B` low while `CRESET_B` rises
    /// selects slave mode; wait for the part to clear itself; eight clocks
    /// with `SPI_SS_B` high; the bitstream with it low; then trailing
    /// clocks with it high. Nothing is written to the flash, and every pin
    /// is released afterwards, so the next power cycle or
    /// [`boot`](Self::boot) brings back whatever the flash holds.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Bit`] for bytes that are not an iCE40 bitstream —
    /// checked before the part is reset — and whatever the link reports.
    pub fn load_sram(&mut self, bitstream: &[u8], path: SramPath) -> Result<bool, ProgramError> {
        if !looks_like_bitstream(bitstream) {
            return Err(ProgramError::Bit(
                "this is not an iCE40 bitstream: there is no 7E AA 99 7E synchronisation word \
                 near its start (an .asc has to be packed into a .bin first)"
                    .into(),
            ));
        }
        let w = self.wiring;
        // The data line: ADBUS2 for the flash-output route, the MPSSE's DO
        // for the host-output one. The other data line is an input either
        // way, so nothing drives the FPGA's SPI_SO.
        let data = match path {
            SramPath::FlashOutput => w.miso,
            SramPath::HostOutput => w.mosi,
        };
        let dir = w.sck | data | w.cs | w.creset;

        // Reset, selected.
        let mut m = Mpsse::new();
        m.set_pins_low(0, dir);
        self.run(&m)?;
        self.link.pause(RESET_PULSE);
        // Out of reset with SPI_SS_B low: slave mode.
        let mut m = Mpsse::new();
        m.set_pins_low(w.creset, dir);
        self.run(&m)?;
        self.link.pause(CLEAR_TIME);
        // A part that missed SPI_SS_B and went into master mode boots from
        // its flash on its own, and its CDONE then says nothing about the
        // bitstream sent here. It is caught before any data: a slave waits,
        // and a master that found an image raises CDONE within this time.
        self.link.pause(MASTER_CHECK);
        let mut m = Mpsse::new();
        m.get_pins_low();
        m.send_immediate();
        if self.run(&m)?[0] & w.cdone != 0 {
            self.release()?;
            return Err(ProgramError::Flash(
                "CDONE rose with no bitstream sent: the FPGA did not take SPI_SS_B low as \
                 slave mode and configured itself from its flash instead"
                    .into(),
            ));
        }

        let mut m = Mpsse::new();
        // Eight clocks with SPI_SS_B high.
        clock_idle(&mut m, w.creset | w.cs, dir, w.sck, 8);
        match path {
            SramPath::FlashOutput => {
                for &byte in bitstream {
                    for bit in (0..8).rev() {
                        let level = if byte >> bit & 1 != 0 { data } else { 0 };
                        m.set_pins_low(w.creset | level, dir);
                        m.set_pins_low(w.creset | level | w.sck, dir);
                    }
                }
                m.set_pins_low(w.creset, dir);
            }
            SramPath::HostOutput => {
                m.set_pins_low(w.creset, dir);
                m.spi_write(bitstream);
            }
        }
        clock_idle(&mut m, w.creset | w.cs, dir, w.sck, TRAILING_CLOCKS);
        self.run(&m)?;
        // CDONE is read with the pins still as they were: releasing them
        // first would make a part that rejected the bitstream and one
        // that took it look alike if anything about the release reset it.
        let mut done = false;
        for _ in 0..10 {
            let mut m = Mpsse::new();
            m.get_pins_low();
            m.send_immediate();
            if self.run(&m)?[0] & w.cdone != 0 {
                done = true;
                break;
            }
            self.link.pause(BOOT_POLL);
        }
        // Hand everything back. CRESET_B stays high on its pull-up, so
        // the configured part keeps running.
        self.release()?;
        Ok(done)
    }

    /// Stops driving every pin.
    ///
    /// # Errors
    ///
    /// Whatever the link reports.
    pub fn release(&mut self) -> Result<(), ProgramError> {
        let mut m = Mpsse::new();
        m.set_pins_low(self.wiring.released(), 0);
        self.run(&m)?;
        Ok(())
    }
}

/// `count` clock pulses with the other pins at `value`.
fn clock_idle(m: &mut Mpsse, value: u8, dir: u8, sck: u8, count: usize) {
    for _ in 0..count {
        m.set_pins_low(value | sck, dir);
        m.set_pins_low(value, dir);
    }
}

#[cfg(test)]
mod tests;
