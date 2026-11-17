//! The iCE40 programmer against a simulated board.
//!
//! [`Board`] is a bit-level model of the bus `iceprog`-wired boards have:
//! an FT2232H's `ADBUS`, a W25Q-series flash and an iCE40's configuration
//! port, all on the same four wires. It interprets the MPSSE byte stream
//! the programmer produces — GPIO writes, MSB-first byte shifts, pin reads
//! — edge by edge, so what it checks is what a board would see:
//!
//! - the flash decodes only the opcodes it is clocked, executes an erase
//!   or a program **only after a write enable**, and records every opcode,
//!   so a test can assert nothing outside [`SENT`] was ever sent;
//! - the FPGA enters slave mode only if `SPI_SS_B` is low as `CRESET_B`
//!   rises, ignores clocks that come before it has cleared itself, and
//!   raises `CDONE` only on the exact bitstream it was meant to get;
//! - two drivers on one net are recorded as contention.
//!
//! What it does **not** model: real timing beyond the pauses it is told
//! about, the FPGA's actual bitstream checks (it compares bytes against
//! the test's expectation), and any flash quirk outside the W25Q's basic
//! command set. Those are what the board itself checks.

use super::*;
use crate::program::ftdi::{self, Chip};

const W: Wiring = ICEPROG_WIRING;

/// A W25Q-series flash at bit level.
struct Flash {
    mem: Vec<u8>,
    jedec: [u8; 3],
    unique: [u8; 8],
    wel: bool,
    busy_polls: u32,
    /// Every opcode decoded, in order.
    opcodes: Vec<u8>,
    /// The current transaction's whole bytes, while `/CS` is low.
    taken: Vec<u8>,
    /// The bits of the byte being shifted in, and how many bits in all.
    partial: u8,
    count: usize,
    /// A bit position that always reads back as zero, to make a verify
    /// fail.
    stuck_low: Option<usize>,
}

impl Flash {
    fn new(size_log2: u8) -> Flash {
        Flash {
            mem: vec![0xFF; 1 << size_log2],
            jedec: [0xEF, 0x40, size_log2],
            unique: [1, 2, 3, 4, 5, 6, 7, 8],
            wel: false,
            busy_polls: 0,
            opcodes: Vec::new(),
            taken: Vec::new(),
            partial: 0,
            count: 0,
            stuck_low: None,
        }
    }

    fn push_bit(&mut self, bit: bool) {
        self.partial = self.partial << 1 | u8::from(bit);
        self.count += 1;
        if self.count.is_multiple_of(8) {
            self.taken.push(self.partial);
        }
    }

    fn select(&mut self) {
        self.taken.clear();
        self.count = 0;
    }

    /// The level the flash drives on `DO` while bit `index` of the
    /// transaction is clocked, or `None` when its output is off.
    fn output(&self, index: usize) -> Option<bool> {
        let bytes = &self.taken;
        let &op = bytes.first()?;
        let (from, data): (usize, Box<dyn Fn(usize) -> u8 + '_>) = match op {
            OP_JEDEC_ID => (1, Box::new(|n| self.jedec.get(n).copied().unwrap_or(0))),
            OP_READ_STATUS => {
                let status = u8::from(self.busy_polls > 0) | u8::from(self.wel) << 1;
                (1, Box::new(move |_| status))
            }
            OP_UNIQUE_ID => (5, Box::new(|n| self.unique.get(n).copied().unwrap_or(0))),
            OP_READ if bytes.len() >= 4 => {
                let addr = usize::from(bytes[1]) << 16
                    | usize::from(bytes[2]) << 8
                    | usize::from(bytes[3]);
                (
                    4,
                    Box::new(move |n| {
                        let at = (addr + n) % self.mem.len();
                        let mut byte = self.mem[at];
                        if let Some(stuck) = self.stuck_low
                            && stuck / 8 == at
                        {
                            byte &= !(0x80 >> (stuck % 8));
                        }
                        byte
                    }),
                )
            }
            _ => return None,
        };
        let byte = index / 8;
        if byte < from {
            return None;
        }
        Some(data(byte - from) >> (7 - index % 8) & 1 != 0)
    }

    fn deselect(&mut self) {
        let bytes = std::mem::take(&mut self.taken);
        self.count = 0;
        let Some(&op) = bytes.first() else { return };
        self.opcodes.push(op);
        match op {
            OP_WRITE_ENABLE if bytes.len() == 1 => self.wel = true,
            OP_WRITE_DISABLE => self.wel = false,
            OP_READ_STATUS => self.busy_polls = self.busy_polls.saturating_sub(1),
            OP_SECTOR_ERASE if bytes.len() == 4 && self.wel && self.busy_polls == 0 => {
                let addr =
                    (usize::from(bytes[1]) << 16 | usize::from(bytes[2]) << 8) & !(SECTOR - 1);
                self.mem[addr..addr + SECTOR].fill(0xFF);
                self.wel = false;
                self.busy_polls = 3;
            }
            OP_PAGE_PROGRAM if bytes.len() > 4 && self.wel && self.busy_polls == 0 => {
                let page = usize::from(bytes[1]) << 16 | usize::from(bytes[2]) << 8;
                for (i, &b) in bytes[4..].iter().enumerate() {
                    let at = page + (usize::from(bytes[3]) + i) % PAGE;
                    self.mem[at] &= b;
                }
                self.wel = false;
                self.busy_polls = 1;
            }
            _ => {}
        }
    }
}

/// What the FPGA's configuration port is doing.
#[derive(Debug, PartialEq)]
enum Fpga {
    Reset,
    /// Slave mode, with the bits taken while selected, and the trailing
    /// clocks seen since.
    Slave {
        bits: Vec<bool>,
        trailing: usize,
    },
    /// Configured, from the flash or from a host.
    Running,
    /// Out of reset in master mode but with nothing to boot from.
    Unconfigured,
}

struct Board {
    value: u8,
    dir: u8,
    flash: Flash,
    fpga: Fpga,
    /// The bitstream the FPGA accepts, standing in for its CRC check.
    expected: Vec<u8>,
    /// Microseconds of pause since `CRESET_B` last rose.
    since_release: u64,
    clocked_too_early: bool,
    contention: Vec<&'static str>,
    /// Flash transactions while the FPGA was out of reset.
    flash_while_running: usize,
    pauses: Vec<Duration>,
    pin_writes: usize,
    /// Just released into master mode: the host must let go of the bus
    /// before anything else happens.
    booting: bool,
    /// Times the FPGA configured itself from the flash.
    flash_boots: usize,
}

impl Board {
    fn new(size_log2: u8) -> Board {
        Board {
            value: 0,
            dir: 0,
            flash: Flash::new(size_log2),
            // As found: a configured part, its pins released.
            fpga: Fpga::Running,
            expected: Vec::new(),
            since_release: 1_000_000,
            clocked_too_early: false,
            contention: Vec::new(),
            flash_while_running: 0,
            pauses: Vec::new(),
            pin_writes: 0,
            booting: false,
            flash_boots: 0,
        }
    }

    fn driven(&self, mask: u8) -> Option<bool> {
        (self.dir & mask != 0).then_some(self.value & mask != 0)
    }
    fn cs(&self) -> bool {
        self.driven(W.cs).unwrap_or(true)
    }
    fn creset(&self) -> bool {
        self.driven(W.creset).unwrap_or(true)
    }
    fn sck(&self) -> bool {
        self.driven(W.sck).unwrap_or(false)
    }
    /// The flash's `DI`: the host's `DO` when driven, else the pull-up.
    fn mosi(&self) -> bool {
        self.driven(W.mosi).unwrap_or(true)
    }
    fn fpga_in_reset(&self) -> bool {
        self.fpga == Fpga::Reset
    }
    /// The net the flash's `DO` and the FPGA's `SPI_SI` share.
    fn miso(&mut self) -> bool {
        let flash = if self.cs() {
            None
        } else {
            self.flash.output(self.flash.count)
        };
        match (self.driven(W.miso), flash) {
            (Some(host), Some(_)) => {
                self.contention
                    .push("host and flash both drive the flash's DO");
                host
            }
            (Some(host), None) => host,
            (None, Some(f)) => f,
            (None, None) => true,
        }
    }
    /// Called before anything that takes time: a host still driving the
    /// bus by then is fighting a booting FPGA for its flash.
    fn settle(&mut self) {
        if self.booting && self.dir & (W.sck | W.mosi | W.cs) != 0 {
            self.contention
                .push("the host still drives the bus as the FPGA boots from flash");
        }
        self.booting = false;
    }

    fn levels(&mut self) -> u8 {
        self.settle();
        let mut b = 0;
        let miso = self.miso();
        for (mask, level) in [
            (W.sck, self.sck()),
            (W.mosi, self.mosi()),
            (W.miso, miso),
            (W.cs, self.cs()),
            (W.cdone, self.fpga == Fpga::Running),
            (W.creset, self.creset()),
        ] {
            if level {
                b |= mask;
            }
        }
        b
    }

    /// One `SET_BITS_LOW`. The FT2232H applies the value **before** the
    /// direction, so the pins still driven take the new value for an
    /// instant before the new direction lets them go — measured on a
    /// board, where a release written as value zero reset the FPGA.
    fn set_bits_low(&mut self, value: u8, dir: u8) {
        let old_dir = self.dir;
        self.set(value, old_dir);
        self.set(value, dir);
    }

    /// Moves the pins to `value`/`dir`, acting on every edge that makes.
    fn set(&mut self, value: u8, dir: u8) {
        let (cs0, creset0, sck0) = (self.cs(), self.creset(), self.sck());
        self.value = value;
        self.dir = dir;
        self.pin_writes += 1;
        let (cs1, creset1, sck1) = (self.cs(), self.creset(), self.sck());
        if creset0 && !creset1 {
            self.fpga = Fpga::Reset;
        }
        if !creset0 && creset1 {
            self.since_release = 0;
            if !cs1 {
                self.fpga = Fpga::Slave {
                    bits: Vec::new(),
                    trailing: 0,
                };
            } else {
                // The FPGA samples SPI_SS_B now and starts driving the bus
                // once it has cleared itself; the host has until then to
                // let go, which the next pause or pin read checks.
                self.booting = true;
                self.flash_boots += 1;
                self.fpga = if looks_like_bitstream(&self.flash.mem) {
                    Fpga::Running
                } else {
                    Fpga::Unconfigured
                };
            }
        }
        if cs0 && !cs1 {
            self.flash.select();
            if !self.fpga_in_reset() && !matches!(self.fpga, Fpga::Slave { .. }) {
                self.flash_while_running += 1;
            }
        }
        if !cs0 && cs1 {
            self.flash.deselect();
        }
        if !sck0 && sck1 {
            self.rising();
        }
    }

    /// A rising clock: the flash and a slave-mode FPGA both sample.
    fn rising(&mut self) {
        let miso = self.miso();
        let cs = self.cs();
        if !cs {
            let mosi = self.mosi();
            self.flash.push_bit(mosi);
        }
        let early = self.since_release < 1_200;
        if let Fpga::Slave { bits, trailing } = &mut self.fpga {
            if !cs {
                if early {
                    self.clocked_too_early = true;
                }
                bits.push(miso);
            } else if !bits.is_empty() {
                *trailing += 1;
            }
            let done = *trailing >= 49 && {
                let got: Vec<u8> = bits
                    .chunks(8)
                    .map(|b| b.iter().fold(0u8, |acc, &bit| acc << 1 | u8::from(bit)))
                    .collect();
                bits.len().is_multiple_of(8) && got == self.expected
            };
            if done {
                self.fpga = Fpga::Running;
            }
        }
    }

    /// Clocks one byte through an MPSSE shift, MSB first, returning what
    /// `DI` read.
    fn shift(&mut self, out: Option<u8>) -> u8 {
        assert!(self.dir & W.sck != 0, "a shift with SK not driven");
        let mut got = 0u8;
        for bit in (0..8).rev() {
            if let Some(byte) = out {
                assert!(self.dir & W.mosi != 0, "a write with DO not driven");
                let level = byte >> bit & 1 != 0;
                let v = (self.value & !W.mosi) | if level { W.mosi } else { 0 };
                self.set(v, self.dir);
            }
            let v = self.value;
            self.set(v | W.sck, self.dir);
            // Sampled on the rising edge, which `set` just made: what the
            // flash drives for the bit it is now on.
            got = got << 1 | u8::from(self.sample_after_rise());
            self.set(v & !W.sck, self.dir);
        }
        got
    }

    /// `DI` as the MPSSE samples it at the rising edge: the flash's output
    /// for the bit that edge clocked in.
    fn sample_after_rise(&mut self) -> bool {
        if self.cs() {
            return true;
        }
        let index = self.flash.count.saturating_sub(1);
        match (self.driven(W.miso), self.flash.output(index)) {
            (Some(h), _) => h,
            (None, Some(f)) => f,
            (None, None) => true,
        }
    }
}

impl Link for Board {
    fn exchange(&mut self, commands: &[u8], read_len: usize) -> Result<Vec<u8>, ProgramError> {
        let mut out = Vec::new();
        let mut i = 0;
        let len16 = |i: usize| usize::from(commands[i + 1]) | usize::from(commands[i + 2]) << 8;
        while i < commands.len() {
            match commands[i] {
                ftdi::CMD_DISABLE_DIV5
                | ftdi::CMD_DISABLE_ADAPTIVE
                | ftdi::CMD_DISABLE_3PHASE
                | ftdi::CMD_LOOPBACK_OFF
                | ftdi::CMD_SEND_IMMEDIATE => i += 1,
                ftdi::CMD_SET_DIVISOR => i += 3,
                ftdi::CMD_SET_BITS_LOW => {
                    self.set_bits_low(commands[i + 1], commands[i + 2]);
                    i += 3;
                }
                ftdi::CMD_GET_BITS_LOW => {
                    out.push(self.levels());
                    i += 1;
                }
                ftdi::CMD_SPI_BYTES_OUT | ftdi::CMD_SPI_BYTES_INOUT => {
                    let n = len16(i) + 1;
                    let capture = commands[i] == ftdi::CMD_SPI_BYTES_INOUT;
                    for k in 0..n {
                        let got = self.shift(Some(commands[i + 3 + k]));
                        if capture {
                            out.push(got);
                        }
                    }
                    i += 3 + n;
                }
                ftdi::CMD_SPI_BYTES_IN => {
                    let n = len16(i) + 1;
                    for _ in 0..n {
                        let got = self.shift(None);
                        out.push(got);
                    }
                    i += 3;
                }
                other => panic!("the model does not know MPSSE opcode {other:#04x}"),
            }
        }
        assert_eq!(out.len(), read_len, "a job's reply length is what it said");
        Ok(out)
    }

    fn pause(&mut self, how_long: Duration) {
        self.settle();
        self.pauses.push(how_long);
        self.since_release += u64::try_from(how_long.as_micros()).unwrap_or(u64::MAX);
    }
}

fn programmer(board: Board) -> Programmer<Board> {
    Programmer::new(board, Chip::HighSpeed, 4, W).expect("the model does not fail")
}

/// A fake bitstream: the comment header, the synchronisation word, and
/// some bytes.
fn bitstream(len: usize) -> Vec<u8> {
    let mut b = vec![0xFF, 0x00, 0x00, 0xFF, 0x7E, 0xAA, 0x99, 0x7E];
    b.extend((0..len).map(|i| u8::try_from(i * 7 % 251).unwrap()));
    b
}

fn assert_only_sent(flash: &Flash) {
    for op in &flash.opcodes {
        assert!(
            SENT.contains(op) || *op == 0xFF,
            "opcode {op:#04x} reached the flash and is not in SENT"
        );
    }
}

/// The two lists cannot overlap: an opcode is sent or it is not.
#[test]
fn nothing_is_both_sent_and_never_sent() {
    for (op, name, _) in NOT_SENT {
        assert!(!SENT.contains(op), "{name} ({op:#04x}) is in both lists");
    }
}

/// Opening drives nothing: a programmer that reset the part or selected
/// the flash just by being created would stop a running design.
#[test]
fn opening_the_programmer_drives_no_pin() {
    let p = programmer(Board::new(18));
    let board = p.into_link();
    assert_eq!(board.dir, 0);
    assert_eq!(board.fpga, Fpga::Running);
}

/// The identifiers come back in the right byte order, and the flash is
/// only ever talked to with the FPGA held in reset.
#[test]
fn identifying_the_flash_holds_the_fpga_in_reset() {
    let mut p = programmer(Board::new(24));
    p.take_flash().unwrap();
    let id = p.flash_id().unwrap();
    assert_eq!(id.jedec, [0xEF, 0x40, 0x18]);
    assert_eq!(id.unique, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(id.capacity(), Some(16 << 20));
    assert_eq!(id.maker(), "Winbond");
    let board = p.into_link();
    assert_eq!(board.flash_while_running, 0);
    assert!(board.fpga_in_reset());
    assert_only_sent(&board.flash);
}

/// An empty bus reads all ones (or all zeros) and is refused rather than
/// taken for a flash of some size.
#[test]
fn an_empty_bus_is_not_a_flash() {
    for jedec in [[0xFF; 3], [0; 3], [0xEF, 0x40, 0x30]] {
        let id = FlashId {
            jedec,
            unique: [0; 8],
        };
        assert_eq!(id.capacity(), None, "{jedec:02X?}");
    }
}

/// A read returns the flash's bytes from the right address, across the
/// chunk boundary.
#[test]
fn a_read_returns_the_flash_contents() {
    let mut board = Board::new(18);
    for (i, b) in board.flash.mem.iter_mut().enumerate() {
        *b = u8::try_from(i % 253).unwrap();
    }
    let expected = board.flash.mem[1000..1000 + 70_000].to_vec();
    let mut p = programmer(board);
    p.take_flash().unwrap();
    let got = p.read(1000, 70_000, &mut |_, _| {}).unwrap();
    assert_eq!(got, expected);
    // Past the end is refused, not wrapped.
    assert!(p.read(1 << 18, 1, &mut |_, _| {}).is_err());
}

/// The plan touches only what changes: a sector already right is left
/// alone, a one-to-zero change programs without erasing, and bytes of a
/// partly covered sector outside the write keep their value.
#[test]
fn the_write_plan_erases_only_what_it_must() {
    let mut current = vec![0xFF; 3 * SECTOR];
    current[SECTOR..2 * SECTOR].fill(0x00);
    current[2 * SECTOR + 10] = 0x5A;

    // Sector 0: 0xFF -> 0x0F, ones to zeros only: no erase, one page.
    // Sector 1: already zero: unchanged.
    // Sector 2: a zero becomes one at byte 0: erase, and the page holding
    // byte 10 (outside the write) is reprogrammed with its 0x5A.
    let mut data = vec![0xFF; 2 * SECTOR + 1];
    data[5] = 0x0F;
    data[SECTOR..2 * SECTOR].fill(0x00);
    data[2 * SECTOR] = 0xFF;
    let mut cur = current.clone();
    cur[2 * SECTOR] = 0x00;
    let plan = plan_write(0, &data, &cur);
    assert_eq!(plan.unchanged, 1);
    assert_eq!(plan.erase, vec![address(2 * SECTOR)]);
    let pages: Vec<u32> = plan.program.iter().map(|(a, _)| *a).collect();
    assert_eq!(pages, vec![0, address(2 * SECTOR)]);
    assert_eq!(plan.program[1].1[10], 0x5A);
    assert_eq!(plan.program[1].1[0], 0xFF);
}

/// A write lands, verifies, and leaves the rest of the flash alone; a
/// second identical write does nothing at all.
#[test]
fn a_write_lands_and_is_verified() {
    let mut board = Board::new(18);
    board.flash.mem[..SECTOR].fill(0x00);
    board.flash.mem[3 * SECTOR + 100] = 0x42;
    let mut p = programmer(board);
    p.take_flash().unwrap();
    let image = bitstream(9000);
    let report = p.write(0, &image, &mut |_, _| {}).unwrap();
    assert_eq!(report.sectors_erased, 1, "{report:?}");
    assert_eq!(report.sectors_unchanged, 0);
    let again = p.write(0, &image, &mut |_, _| {}).unwrap();
    assert_eq!(again.sectors_erased + again.pages_programmed, 0);
    assert_eq!(again.sectors_unchanged, 3);

    let board = p.into_link();
    assert_eq!(&board.flash.mem[..image.len()], &image[..]);
    assert_eq!(board.flash.mem[3 * SECTOR + 100], 0x42, "outside the write");
    assert_eq!(board.flash_while_running, 0);
    assert_only_sent(&board.flash);
    assert!(!board.flash.wel, "left with the write latch set");
}

/// A flash that does not hold what was written is reported, with the
/// address.
#[test]
fn a_failed_verify_names_the_address() {
    let mut board = Board::new(18);
    board.flash.stuck_low = Some(8 * 300 + 1);
    let mut p = programmer(board);
    p.take_flash().unwrap();
    let image = vec![0xFF; 600];
    let mut data = image.clone();
    data[0] = 0;
    let err = p.write(0, &data, &mut |_, _| {}).unwrap_err().to_string();
    assert!(err.contains("0x12c"), "{err}");
}

/// The slave-mode load on this wiring: the FPGA gets exactly the
/// bitstream on its `SPI_SI`, only after it has cleared itself, and
/// raises `CDONE`; the flash, selected throughout, decodes nothing but
/// `0xFF` and is left as it was; the host never fights the flash for
/// its `DO`; and every pin is released at the end.
#[test]
fn a_slave_load_over_the_flash_output_configures_the_part() {
    let mut board = Board::new(18);
    let image = bitstream(500);
    board.expected = image.clone();
    // The flash holds an image of its own, so that a part which booted
    // from it instead would also come up configured: only `flash_boots`
    // tells the two apart, which is how the release glitch was missed.
    let other = bitstream(300);
    board.flash.mem[..other.len()].copy_from_slice(&other);
    let before = board.flash.mem.clone();
    let mut p = programmer(board);
    assert!(p.load_sram(&image, SramPath::FlashOutput).unwrap());
    let board = p.into_link();
    assert_eq!(board.fpga, Fpga::Running);
    assert_eq!(
        board.flash_boots, 0,
        "the part was reset and booted from its flash"
    );
    assert!(!board.clocked_too_early);
    assert!(board.contention.is_empty(), "{:?}", board.contention);
    assert!(
        board.flash.opcodes.iter().all(|&op| op == 0xFF),
        "{:02x?}",
        board.flash.opcodes
    );
    assert_eq!(board.flash.mem, before);
    assert_eq!(board.dir, 0);
}

/// The `iceprog -S` route, on a board wired for flash programming, does
/// not configure the part: what it shifts goes to the flash and to the
/// FPGA's `SPI_SO`, and its `SPI_SI` sees only the pull-up. This is why
/// [`SramPath::FlashOutput`] exists.
#[test]
fn the_iceprog_route_does_not_reach_spi_si_on_this_wiring() {
    let mut board = Board::new(18);
    let image = bitstream(64);
    board.expected = image.clone();
    let mut p = programmer(board);
    assert!(!p.load_sram(&image, SramPath::HostOutput).unwrap());
}

/// Releasing to boot lets go of the bus as `CRESET_B` rises, so the FPGA
/// configures itself from the flash with nothing fighting it.
#[test]
fn booting_releases_the_bus_and_waits_for_cdone() {
    let mut board = Board::new(18);
    let image = bitstream(100);
    board.flash.mem[..image.len()].copy_from_slice(&image);
    let mut p = programmer(board);
    p.take_flash().unwrap();
    assert!(p.boot().unwrap().is_some());
    let board = p.into_link();
    assert!(board.contention.is_empty(), "{:?}", board.contention);
    assert_eq!(board.dir, 0);

    // A blank flash boots nothing, and that is reported, not hidden.
    let mut p = programmer(Board::new(18));
    assert_eq!(p.boot().unwrap(), None);
}

/// A file that is not a bitstream is refused before any pin moves.
#[test]
fn a_non_bitstream_is_refused_before_the_part_is_reset() {
    let mut p = programmer(Board::new(18));
    let writes = 2; // `new` released the pins once, in two steps.
    assert!(
        p.load_sram(b"not a bitstream", SramPath::FlashOutput)
            .is_err()
    );
    let board = p.into_link();
    assert_eq!(board.pin_writes, writes);
    assert_eq!(board.fpga, Fpga::Running);
}
