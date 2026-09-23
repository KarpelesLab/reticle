//! The FTDI MPSSE command encoding.
//!
//! Every opcode, every field and every control request in this file comes
//! from FTDI's public documentation: application note **AN_108**, *Command
//! Processor for MPSSE and MCU Host Bus Emulation Modes*, and **AN_135**,
//! *MPSSE Basics*. Nothing here is specific to JTAG — the MPSSE is a
//! general shift engine with three output pins and one input pin — and
//! nothing here performs I/O. A [`Mpsse`] is a byte buffer being built;
//! what sends it is [`super::usb`].
//!
//! # The opcode is a bit field
//!
//! The data-shifting opcodes are not an arbitrary table. AN_108 §3 gives
//! them as six independent flags, and spelling them out that way is the
//! only way to keep them straight:
//!
//! | Bit | Meaning when set |
//! |-----|------------------|
//! | 0 | TDI changes on the **falling** edge of TCK |
//! | 1 | the length field counts **bits**, not bytes |
//! | 2 | TDO is sampled on the **falling** edge of TCK |
//! | 3 | shift **LSB first** |
//! | 4 | drive TDI from the command's data |
//! | 5 | capture TDO into the reply |
//! | 6 | this is a TMS command |
//!
//! JTAG wants bit 0 and not bit 2 (drive on the falling edge, sample on
//! the rising one) and bit 3 (IEEE 1149.1 shifts least-significant bit
//! first), which is where [`CMD_BYTES_OUT`] and its neighbours come from.
//!
//! # Lengths are off by one
//!
//! A byte command carries a 16-bit little-endian length of *N-1*, so one
//! command moves 1 to 65 536 bytes. A bit command carries a one-byte
//! length of *N-1*, so 1 to 8 bits. Encoding N instead of N-1 is a
//! one-character mistake that shifts one bit too many through the TAP and
//! desynchronises everything after it, so the constructors here take N and
//! do the subtraction once.
//!
//! # Reads come back wrapped in modem status
//!
//! Every bulk-IN packet from an FTDI part begins with two status bytes,
//! whether or not the MPSSE produced any data. [`strip_status`] removes
//! them; forgetting to is how a reply looks plausible and decodes to
//! nonsense.

/// TDI/TDO change on the falling edge of TCK.
const WRITE_NEG: u8 = 0x01;
/// The length field counts bits rather than bytes.
const BIT_MODE: u8 = 0x02;
/// Shift least-significant bit first, as IEEE 1149.1 requires.
const LSB_FIRST: u8 = 0x08;
/// Drive TDI from the bytes that follow the command.
const DO_WRITE: u8 = 0x10;
/// Capture TDO into the reply stream.
const DO_READ: u8 = 0x20;
/// Drive TMS from the command's data byte instead of TDI.
const WRITE_TMS: u8 = 0x40;

/// Clock whole bytes out of TDI, LSB first, no capture (`0x19`).
pub const CMD_BYTES_OUT: u8 = WRITE_NEG | LSB_FIRST | DO_WRITE;
/// Clock whole bytes out of TDI and capture TDO (`0x39`).
pub const CMD_BYTES_INOUT: u8 = CMD_BYTES_OUT | DO_READ;
/// Clock 1..8 bits out of TDI, LSB first, no capture (`0x1B`).
pub const CMD_BITS_OUT: u8 = CMD_BYTES_OUT | BIT_MODE;
/// Clock 1..8 bits out of TDI and capture TDO (`0x3B`).
pub const CMD_BITS_INOUT: u8 = CMD_BITS_OUT | DO_READ;
/// Clock 1..8 bits out of TMS, holding TDI, no capture (`0x4B`).
pub const CMD_TMS_OUT: u8 = WRITE_TMS | BIT_MODE | LSB_FIRST | WRITE_NEG;
/// Clock 1..8 bits out of TMS, holding TDI, and capture TDO (`0x6B`).
pub const CMD_TMS_INOUT: u8 = CMD_TMS_OUT | DO_READ;

/// Set the low byte of the data bus (`ADBUS`): value, then direction.
pub const CMD_SET_BITS_LOW: u8 = 0x80;
/// Set the high byte of the data bus (`ACBUS`): value, then direction.
pub const CMD_SET_BITS_HIGH: u8 = 0x82;
/// Turn the internal TDI-to-TDO loopback off.
pub const CMD_LOOPBACK_OFF: u8 = 0x85;
/// Set the clock divisor: low byte, then high byte.
pub const CMD_SET_DIVISOR: u8 = 0x86;
/// Flush whatever has been captured back to the host at once.
pub const CMD_SEND_IMMEDIATE: u8 = 0x87;
/// Turn the divide-by-5 clock prescaler off (H-series parts only), which
/// is what makes the master clock 60 MHz rather than 12 MHz.
pub const CMD_DISABLE_DIV5: u8 = 0x8A;
/// Turn three-phase data clocking off (it is for I2C).
pub const CMD_DISABLE_3PHASE: u8 = 0x8D;
/// Turn adaptive clocking off (it is for ARM's RTCK).
pub const CMD_DISABLE_ADAPTIVE: u8 = 0x97;

/// The first byte of the MPSSE's "I did not understand that" reply. The
/// second is the offending opcode.
pub const BAD_COMMAND: u8 = 0xFA;

/// The name of an opcode this module emits, or `None` for a byte that
/// is not one of them.
///
/// It exists to tell a genuine "bad command" reply — `0xFA` followed by
/// the opcode the MPSSE did not understand — from captured TDO that
/// happens to start with `0xFA`, which one reply in 256 will.
#[must_use]
pub fn opcode_name(opcode: u8) -> Option<&'static str> {
    Some(match opcode {
        CMD_BYTES_OUT => "clock bytes out",
        CMD_BYTES_INOUT => "clock bytes in and out",
        CMD_BITS_OUT => "clock bits out",
        CMD_BITS_INOUT => "clock bits in and out",
        CMD_TMS_OUT => "clock TMS out",
        CMD_TMS_INOUT => "clock TMS out and TDO in",
        CMD_SET_BITS_LOW => "set the low data bus",
        CMD_SET_BITS_HIGH => "set the high data bus",
        CMD_LOOPBACK_OFF => "loopback off",
        CMD_SET_DIVISOR => "set the clock divisor",
        CMD_SEND_IMMEDIATE => "send immediate",
        CMD_DISABLE_DIV5 => "disable the divide-by-5 prescaler",
        CMD_DISABLE_3PHASE => "disable three-phase clocking",
        CMD_DISABLE_ADAPTIVE => "disable adaptive clocking",
        _ => return None,
    })
}

/// The MPSSE master clock on an H-series part once [`CMD_DISABLE_DIV5`]
/// has been sent (AN_108 §3.8.2).
pub const MASTER_CLOCK_HZ: u32 = 60_000_000;

/// Most bytes one byte-shift command can carry: the length field holds
/// *N-1* in 16 bits.
pub const MAX_BYTES_PER_COMMAND: usize = 65_536;

/// The divisor that gets closest to `hz` without exceeding it.
///
/// AN_108 §3.8.2: `TCK = 60 MHz / ((1 + divisor) * 2)`. Rounding is
/// upwards so the result is never *faster* than asked for; programming a
/// board is not a place to overshoot a clock.
#[must_use]
pub fn divisor_for(hz: u32) -> u16 {
    if hz == 0 {
        return u16::MAX;
    }
    let half = u64::from(MASTER_CLOCK_HZ) / 2;
    let div = half.div_ceil(u64::from(hz)).saturating_sub(1);
    u16::try_from(div.min(u64::from(u16::MAX))).unwrap_or(u16::MAX)
}

/// The clock a divisor actually produces, for reporting it back.
#[must_use]
pub fn clock_hz(divisor: u16) -> u32 {
    MASTER_CLOCK_HZ / ((u32::from(divisor) + 1) * 2)
}

/// A buffer of MPSSE commands under construction, and how many bytes of
/// captured TDO the device will send back when it runs them.
///
/// This is the whole of the FTDI side: a `Vec<u8>` with typed pushes. It
/// never touches a device, so every encoding decision above is a unit
/// test away.
#[derive(Clone, Debug, Default)]
pub struct Mpsse {
    bytes: Vec<u8>,
    read_len: usize,
}

impl Mpsse {
    /// An empty command buffer.
    #[must_use]
    pub fn new() -> Mpsse {
        Mpsse::default()
    }

    /// The encoded commands.
    #[must_use]
    pub fn commands(&self) -> &[u8] {
        &self.bytes
    }

    /// Takes the encoded commands.
    #[must_use]
    pub fn into_commands(self) -> Vec<u8> {
        self.bytes
    }

    /// How many bytes of captured TDO running these commands will return,
    /// before the modem-status bytes [`strip_status`] removes.
    #[must_use]
    pub fn read_len(&self) -> usize {
        self.read_len
    }

    /// True when nothing has been encoded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// How many command bytes have been encoded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Appends a raw opcode with no operands.
    pub fn opcode(&mut self, opcode: u8) {
        self.bytes.push(opcode);
    }

    /// The fixed opening sequence for synchronous JTAG-style clocking:
    /// 60 MHz master clock, no adaptive or three-phase clocking, the
    /// requested divisor, and no loopback.
    ///
    /// AN_135 §4.2 gives this same list in this same order.
    pub fn configure(&mut self, divisor: u16) {
        self.opcode(CMD_DISABLE_DIV5);
        self.opcode(CMD_DISABLE_ADAPTIVE);
        self.opcode(CMD_DISABLE_3PHASE);
        let [lo, hi] = divisor.to_le_bytes();
        self.bytes.extend_from_slice(&[CMD_SET_DIVISOR, lo, hi]);
        self.opcode(CMD_LOOPBACK_OFF);
    }

    /// Drives the low byte of the data bus: `value` on the pins marked as
    /// outputs by `direction` (1 = output).
    pub fn set_pins_low(&mut self, value: u8, direction: u8) {
        self.bytes
            .extend_from_slice(&[CMD_SET_BITS_LOW, value, direction]);
    }

    /// Drives the high byte of the data bus.
    pub fn set_pins_high(&mut self, value: u8, direction: u8) {
        self.bytes
            .extend_from_slice(&[CMD_SET_BITS_HIGH, value, direction]);
    }

    /// Asks the device to flush its capture buffer to the host now
    /// instead of waiting for its latency timer.
    pub fn send_immediate(&mut self) {
        self.opcode(CMD_SEND_IMMEDIATE);
    }

    /// Clocks `data` out of TDI, LSB of each byte first, capturing
    /// nothing. Splits into as many commands as the 16-bit length field
    /// needs; `data` may be megabytes long.
    pub fn write_bytes(&mut self, data: &[u8]) {
        for chunk in data.chunks(MAX_BYTES_PER_COMMAND) {
            self.byte_command(CMD_BYTES_OUT, chunk.len());
            self.bytes.extend_from_slice(chunk);
        }
    }

    /// Clocks `data` out of TDI and captures the same number of bytes of
    /// TDO.
    pub fn exchange_bytes(&mut self, data: &[u8]) {
        for chunk in data.chunks(MAX_BYTES_PER_COMMAND) {
            self.byte_command(CMD_BYTES_INOUT, chunk.len());
            self.bytes.extend_from_slice(chunk);
            self.read_len += chunk.len();
        }
    }

    /// Clocks `count` bits (1..=8) out of TDI, taking them from the low
    /// `count` bits of `data`, LSB first.
    ///
    /// # Panics
    ///
    /// When `count` is 0 or above 8. Callers in this crate compute it
    /// from a remainder modulo 8, so the range is a bug, not input.
    pub fn write_bits(&mut self, data: u8, count: u8) {
        assert!((1..=8).contains(&count), "a bit command moves 1..=8 bits");
        self.bytes
            .extend_from_slice(&[CMD_BITS_OUT, count - 1, data]);
    }

    /// Clocks `count` bits (1..=8) out of TDI and captures the same
    /// number of TDO bits, which arrive as **one byte, left-aligned**:
    /// the MPSSE shifts each captured bit in at bit 7 and moves the byte
    /// right, so after `count` bits the first one sits at bit
    /// `8 - count`. [`super::jtag::unpack_capture`] is what undoes that.
    ///
    /// # Panics
    ///
    /// When `count` is 0 or above 8.
    pub fn exchange_bits(&mut self, data: u8, count: u8) {
        assert!((1..=8).contains(&count), "a bit command moves 1..=8 bits");
        self.bytes
            .extend_from_slice(&[CMD_BITS_INOUT, count - 1, data]);
        self.read_len += 1;
    }

    /// Clocks `count` bits (1..=7) out of TMS, taken from the low
    /// `count` bits of `tms`, LSB first, while TDI is held at `tdi` for
    /// every one of those clocks.
    ///
    /// Holding TDI is what lets the last bit of a shift leave the same
    /// cycle the TAP moves to `Exit1`, which is the only way to end a
    /// scan in one clock.
    ///
    /// # Panics
    ///
    /// When `count` is 0 or above 7, or when `tms` has bits set at or
    /// above `count` (bit 7 belongs to TDI and the rest must be zero, so
    /// a stray bit would clock the TAP somewhere unintended).
    pub fn write_tms(&mut self, tms: u8, count: u8, tdi: bool) {
        self.bytes
            .extend_from_slice(&[CMD_TMS_OUT, count - 1, tms_byte(tms, count, tdi)]);
    }

    /// [`write_tms`](Self::write_tms), capturing TDO as well. The reply
    /// is one byte, left-aligned exactly as
    /// [`exchange_bits`](Self::exchange_bits) describes.
    ///
    /// # Panics
    ///
    /// As [`write_tms`](Self::write_tms).
    pub fn exchange_tms(&mut self, tms: u8, count: u8, tdi: bool) {
        self.bytes
            .extend_from_slice(&[CMD_TMS_INOUT, count - 1, tms_byte(tms, count, tdi)]);
        self.read_len += 1;
    }

    /// Pushes a byte-command header with its off-by-one 16-bit length.
    fn byte_command(&mut self, opcode: u8, len: usize) {
        debug_assert!((1..=MAX_BYTES_PER_COMMAND).contains(&len));
        let [lo, hi] = u16::try_from(len - 1).unwrap_or(u16::MAX).to_le_bytes();
        self.bytes.extend_from_slice(&[opcode, lo, hi]);
    }
}

/// The data byte of a TMS command: TMS levels in bits 0..`count`, the
/// held TDI level in bit 7.
fn tms_byte(tms: u8, count: u8, tdi: bool) -> u8 {
    assert!((1..=7).contains(&count), "a TMS command moves 1..=7 bits");
    assert!(
        tms >> count == 0,
        "a TMS command's spare bits must be zero: bit 7 is TDI"
    );
    if tdi { tms | 0x80 } else { tms }
}

/// The bulk-IN packet size of an FT2232H at high speed. The two
/// modem-status bytes are repeated at the head of every packet of this
/// size, not once per transfer.
pub const PACKET_SIZE: usize = 512;

/// Removes the two modem-status bytes that head every bulk-IN packet.
///
/// `raw` is a whole bulk read, which the host controller has concatenated
/// out of packets of `packet_size` bytes; each of those begins with the
/// two status bytes. A trailing short packet is handled the same way, and
/// a packet with nothing but its status bytes contributes nothing.
#[must_use]
pub fn strip_status(raw: &[u8], packet_size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    for packet in raw.chunks(packet_size.max(3)) {
        if packet.len() > 2 {
            out.extend_from_slice(&packet[2..]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The opcodes assembled from AN_108's flag bits are the numbers
    /// every other MPSSE JTAG driver uses. If a flag is ever renamed,
    /// this is what notices.
    #[test]
    fn the_opcodes_are_the_documented_numbers() {
        assert_eq!(CMD_BYTES_OUT, 0x19);
        assert_eq!(CMD_BYTES_INOUT, 0x39);
        assert_eq!(CMD_BITS_OUT, 0x1B);
        assert_eq!(CMD_BITS_INOUT, 0x3B);
        assert_eq!(CMD_TMS_OUT, 0x4B);
        assert_eq!(CMD_TMS_INOUT, 0x6B);
    }

    /// Byte commands encode N-1 in little-endian, and split at 65 536.
    #[test]
    fn a_byte_command_encodes_one_less_than_the_length() {
        let mut m = Mpsse::new();
        m.write_bytes(&[0xAA]);
        assert_eq!(m.commands(), [CMD_BYTES_OUT, 0x00, 0x00, 0xAA]);
        assert_eq!(m.read_len(), 0);

        let mut m = Mpsse::new();
        m.write_bytes(&[0; 258]);
        assert_eq!(&m.commands()[..3], [CMD_BYTES_OUT, 0x01, 0x01]);
        assert_eq!(m.commands().len(), 3 + 258);

        let mut m = Mpsse::new();
        m.exchange_bytes(&[0; MAX_BYTES_PER_COMMAND + 5]);
        // Two commands: a full one and a five-byte one.
        assert_eq!(&m.commands()[..3], [CMD_BYTES_INOUT, 0xFF, 0xFF]);
        let second = MAX_BYTES_PER_COMMAND + 3;
        assert_eq!(&m.commands()[second..second + 3], [CMD_BYTES_INOUT, 4, 0]);
        assert_eq!(m.read_len(), MAX_BYTES_PER_COMMAND + 5);
    }

    /// A TMS command puts TMS in the low bits and TDI in bit 7, and the
    /// length is again one less.
    #[test]
    fn a_tms_command_carries_tdi_in_bit_seven() {
        let mut m = Mpsse::new();
        m.write_tms(0b011, 3, false);
        assert_eq!(m.commands(), [CMD_TMS_OUT, 2, 0b0000_0011]);

        let mut m = Mpsse::new();
        m.exchange_tms(0b1, 1, true);
        assert_eq!(m.commands(), [CMD_TMS_INOUT, 0, 0b1000_0001]);
        assert_eq!(m.read_len(), 1);
    }

    /// A TMS command with a bit set outside its count would clock the TAP
    /// somewhere nobody asked for; it is rejected rather than truncated.
    #[test]
    #[should_panic(expected = "spare bits must be zero")]
    fn a_tms_command_rejects_stray_bits() {
        Mpsse::new().write_tms(0b100, 2, false);
    }

    /// The divisor rounds so the clock never comes out faster than asked.
    #[test]
    fn the_divisor_never_overshoots() {
        assert_eq!(divisor_for(1_000_000), 29);
        assert_eq!(clock_hz(29), 1_000_000);
        assert_eq!(divisor_for(2_000_000), 14);
        assert_eq!(clock_hz(14), 2_000_000);
        // 7 MHz does not divide evenly; the next slower clock is taken.
        let d = divisor_for(7_000_000);
        assert!(clock_hz(d) <= 7_000_000);
        assert!(clock_hz(d.saturating_sub(1)) > 7_000_000);
        // The extremes stay in range rather than wrapping.
        assert_eq!(divisor_for(0), u16::MAX);
        assert_eq!(divisor_for(u32::MAX), 0);
        assert_eq!(clock_hz(0), 30_000_000);
    }

    /// `configure` is the documented opening sequence, divisor included.
    #[test]
    fn configure_emits_the_documented_opening_sequence() {
        let mut m = Mpsse::new();
        m.configure(0x001D);
        assert_eq!(
            m.commands(),
            [
                CMD_DISABLE_DIV5,
                CMD_DISABLE_ADAPTIVE,
                CMD_DISABLE_3PHASE,
                CMD_SET_DIVISOR,
                0x1D,
                0x00,
                CMD_LOOPBACK_OFF,
            ]
        );
    }

    /// Only the opcodes this module emits are named, so a stray `0xFA`
    /// in captured TDO is not mistaken for a rejected command.
    #[test]
    fn only_emitted_opcodes_are_named() {
        assert_eq!(opcode_name(CMD_BYTES_OUT), Some("clock bytes out"));
        assert_eq!(opcode_name(CMD_TMS_INOUT), Some("clock TMS out and TDO in"));
        assert_eq!(opcode_name(BAD_COMMAND), None);
        assert_eq!(opcode_name(0x00), None);
        assert_eq!(opcode_name(0xFF), None);
        // Fewer than a tenth of all byte values are ours, so the test in
        // `usb::Cable` that uses this is a narrow filter rather than a
        // broad one.
        let named = (0..=u8::MAX).filter(|b| opcode_name(*b).is_some()).count();
        assert_eq!(named, 14);
    }

    /// Two status bytes come off the head of every packet, not just the
    /// first, and a packet that is nothing but status contributes none.
    #[test]
    fn modem_status_is_stripped_per_packet() {
        let mut raw = vec![0x32, 0x60];
        raw.extend(std::iter::repeat_n(0xAB, 510));
        raw.extend_from_slice(&[0x32, 0x60, 0x01, 0x02]);
        let data = strip_status(&raw, PACKET_SIZE);
        assert_eq!(data.len(), 512);
        assert_eq!(data[509], 0xAB);
        assert_eq!(&data[510..], [0x01, 0x02]);

        assert!(strip_status(&[0x32, 0x60], PACKET_SIZE).is_empty());
        assert!(strip_status(&[], PACKET_SIZE).is_empty());
    }
}
