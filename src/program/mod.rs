//! Configuring a real FPGA over USB: FTDI MPSSE, JTAG, and the Xilinx
//! 7-series configuration sequence.
//!
//! This is the one part of Reticle that talks to hardware, and it is
//! arranged so that almost none of it does.
//!
//! | Module | What it is | Touches a device |
//! |---|---|---|
//! | [`ftdi`] | the MPSSE command encoding (FTDI AN_108) | no |
//! | [`jtag`] | the IEEE 1149.1 TAP state machine and scans | no |
//! | [`xilinx`] | the UG470 configuration sequence | no |
//! | [`usb`] | opening, claiming and bulk transfers, over `rawusb` | yes |
//!
//! The first three are *sequencers*: pure functions from a request to a
//! [`Job`](jtag::Job) — a buffer of bytes to send and a description of
//! what comes back — and from a reply to a decoded value. They are the
//! same sans-I/O discipline the rest of the crate follows, for the same
//! reason: the tricky part (bit order, off-by-one lengths, the TAP walk)
//! becomes unit-testable with no board on the bench, and the part that
//! cannot be tested that way shrinks to "write these bytes, read those".
//!
//! `unsafe` appears nowhere in this module. Every ioctl lives in
//! `rawusb`'s own `src/sys/`, which is where it belongs.
//!
//! # The flow
//!
//! ```no_run
//! # #[cfg(feature = "program")]
//! # fn go() -> Result<(), reticle::program::ProgramError> {
//! use reticle::program::{self, xilinx};
//!
//! let bytes = std::fs::read("design.bit").unwrap();
//! let bit = program::read_bit_container(&bytes)?;
//!
//! let cable = program::usb::Cable::open(None)?;
//! let (pins, dirs) = program::BASYS3_PINS;
//! cable.run(&xilinx::init_job(program::default_divisor(), pins, dirs))?;
//!
//! let job = xilinx::idcode_job();
//! let idcode = job.capture_u32(0, &cable.run(&job)?)?;
//! assert!(xilinx::idcode_matches(idcode, xilinx::IDCODE_XC7A35T));
//!
//! cable.run(&xilinx::erase_job())?;
//! cable.run(&xilinx::configure_job(bit.payload(&bytes)))?;
//! cable.run(&xilinx::start_job())?;
//!
//! let job = xilinx::status_job();
//! let status = xilinx::read_status(&job, &cable.run(&job)?)?;
//! assert!(status.done());
//! # Ok(())
//! # }
//! ```
//!
//! # What this does and does not do
//!
//! It writes the part's **volatile configuration memory** and nothing
//! else. There is no flash programming here and none is planned in this
//! module: a JTAG configuration is undone by a power cycle, so every
//! attempt is reversible, and a QSPI write is not. It does not touch the
//! board's mode pins either — JTAG configuration works whatever the mode
//! jumper is set to, which is what makes this the safe way in.

pub mod ftdi;
pub mod jtag;
pub mod usb;
pub mod xilinx;

use std::fmt;

/// The TCK this crate clocks a board at unless told otherwise.
///
/// One megahertz. The FT2232H will go to 30 MHz and a two-megabyte
/// bitstream would then take under a second instead of about twenty, but
/// speed buys nothing here: the operation happens once, the cable and
/// the board's series termination are unknown, and a flaky first attempt
/// costs far more to diagnose than the twenty seconds it saved.
pub const DEFAULT_CLOCK_HZ: u32 = 1_000_000;

/// The divisor for [`DEFAULT_CLOCK_HZ`].
#[must_use]
pub fn default_divisor() -> u16 {
    ftdi::divisor_for(DEFAULT_CLOCK_HZ)
}

/// USB vendor identifier of Future Technology Devices International.
pub const FTDI_VENDOR_ID: u16 = 0x0403;

/// USB product identifier of the FT2232H, the dual-channel part on a
/// Digilent Basys 3 and on most low-cost JTAG cables.
pub const FT2232H_PRODUCT_ID: u16 = 0x6010;

/// The interface that carries the MPSSE on an FT2232H. Channel A is the
/// one with the shift engine; channel B is a plain serial port, and on a
/// Basys 3 it is the USB-UART bridge.
pub const MPSSE_INTERFACE: u8 = 0;

/// The low byte of the FT2232H's data bus on a Digilent board, as
/// (value, direction).
///
/// `ADBUS0` is TCK, `ADBUS1` TDI, `ADBUS2` TDO and `ADBUS3` TMS, which
/// is the MPSSE's fixed assignment. `ADBUS7` is an output held high: on
/// Digilent's JTAG-SMT2 circuit it enables the buffer between the FTDI
/// part and the FPGA's JTAG pins, and leaving it low leaves the chain
/// disconnected. The same pair of numbers appears in every published
/// OpenOCD configuration for a Basys 3 (`ftdi layout_init 0x0088
/// 0x008b`), which is where they were taken from — they describe the
/// board, not the FPGA, and no data sheet states them.
///
/// TMS starts high, which is the idle level for a TAP.
pub const BASYS3_PINS: (u8, u8) = (0x88, 0x8B);

/// Why programming failed.
#[derive(Debug)]
pub enum ProgramError {
    /// The `.bit` file is not one, or is truncated.
    Bit(String),
    /// No adapter matched.
    NoDevice {
        /// The serial number that was asked for, if any.
        wanted: Option<String>,
        /// The serial numbers that were found.
        found: Vec<String>,
    },
    /// More than one adapter is attached and none was named.
    Ambiguous(Vec<String>),
    /// The part on the other end of the cable is not the expected one.
    IdcodeMismatch {
        /// What was read back.
        read: u32,
        /// What the part should have answered.
        expected: u32,
    },
    /// Configuration ran to the end but the part did not assert `DONE`.
    NotDone(xilinx::Status),
    /// A reply could not be read back.
    Job(jtag::JobError),
    /// The USB layer refused or failed.
    Usb(String),
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgramError::Bit(why) => write!(f, "{why}"),
            ProgramError::NoDevice { wanted, found } => {
                match wanted {
                    Some(serial) => write!(f, "no adapter with serial number `{serial}`")?,
                    None => write!(
                        f,
                        "no FTDI adapter found (looking for USB {FTDI_VENDOR_ID:#06x}:{FT2232H_PRODUCT_ID:#06x})"
                    )?,
                }
                if found.is_empty() {
                    Ok(())
                } else {
                    write!(f, "; attached: {}", found.join(", "))
                }
            }
            ProgramError::Ambiguous(serials) => write!(
                f,
                "{} adapters are attached; name one with --device: {}",
                serials.len(),
                serials.join(", ")
            ),
            ProgramError::IdcodeMismatch { read, expected } => write!(
                f,
                "the part answered IDCODE {read:#010x}, not {expected:#010x}: \
                 this is not the device this bitstream is for, and nothing was written"
            ),
            ProgramError::NotDone(status) => write!(
                f,
                "the configuration sequence completed but DONE did not assert; status {status}"
            ),
            ProgramError::Job(err) => write!(f, "{err}"),
            ProgramError::Usb(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ProgramError {}

impl From<jtag::JobError> for ProgramError {
    fn from(err: jtag::JobError) -> ProgramError {
        ProgramError::Job(err)
    }
}

/// The `.bit` wrapper's fields and where its configuration data sits.
///
/// A `.bit` file is a short keyed header — `a` the design name, `b` the
/// part, `c` the date, `d` the time — followed by `e` and a 32-bit
/// big-endian length, then the configuration data itself. That is the
/// same container `fpga::xc7` writes, described in the same place.
///
/// This reader exists rather than calling `fpga::xc7::read_bit` because
/// programming a board should not drag in the synthesis feature stack:
/// all a programmer needs is where the payload starts. When the `fpga`
/// feature is on, `fpga::xc7::read_bit` is the reader that also replays
/// the packet stream and checks its CRCs, and the two agree on the
/// payload by construction — both take everything after the `e` field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitContainer {
    /// The `a` field: the design name and the tool that wrote it.
    pub design: String,
    /// The `b` field: the part, as `7a35tcpg236`.
    pub part: String,
    /// The `c` field: the date.
    pub date: String,
    /// The `d` field: the time.
    pub time: String,
    /// Offset of the configuration data in the file.
    pub offset: usize,
    /// Length of the configuration data.
    pub length: usize,
}

impl BitContainer {
    /// The configuration data, borrowed out of the bytes this container
    /// was read from.
    ///
    /// # Panics
    ///
    /// When `bytes` is not the buffer [`read_bit_container`] was given.
    #[must_use]
    pub fn payload<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.offset..self.offset + self.length]
    }
}

/// Reads a `.bit` file's header and finds its configuration data.
///
/// The payload is returned as a range rather than a copy: it is
/// megabytes, and [`xilinx::configure_job`] is going to make one
/// bit-reversed copy of it anyway.
///
/// # Errors
///
/// [`ProgramError::Bit`] when the file is not a `.bit`, is truncated, or
/// carries no configuration data that starts with the sync word.
pub fn read_bit_container(bytes: &[u8]) -> Result<BitContainer, ProgramError> {
    let bad = |why: String| ProgramError::Bit(why);
    let at = |pos: usize, n: usize| -> Result<&[u8], ProgramError> {
        bytes
            .get(pos..pos + n)
            .ok_or_else(|| bad(format!("the file ends after {} byte(s)", bytes.len())))
    };
    let u16_at = |pos: usize| -> Result<usize, ProgramError> {
        let b = at(pos, 2)?;
        Ok(usize::from(u16::from_be_bytes([b[0], b[1]])))
    };

    let magic_len = u16_at(0)?;
    let mut pos = 2 + magic_len;
    let separator = u16_at(pos)?;
    if separator != 1 {
        return Err(bad(format!(
            "the header's separator is {separator}, not 1: this is not a .bit file"
        )));
    }
    pos += 2;

    let mut container = BitContainer {
        design: String::new(),
        part: String::new(),
        date: String::new(),
        time: String::new(),
        offset: 0,
        length: 0,
    };
    loop {
        let key = at(pos, 1)?[0];
        pos += 1;
        if key == b'e' {
            let b = at(pos, 4)?;
            let length = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            pos += 4;
            let length = usize::try_from(length).unwrap_or(usize::MAX);
            // Checked, because the length is four bytes out of a file
            // that may be anything at all: on a 32-bit host `pos +
            // length` is free to wrap, and a wrapped comparison would
            // accept a range that is not there.
            if pos.checked_add(length).is_none_or(|end| bytes.len() < end) {
                return Err(bad(format!(
                    "the header claims {length} bytes of configuration data \
                     but only {} follow",
                    bytes.len() - pos
                )));
            }
            container.offset = pos;
            container.length = length;
            break;
        }
        let length = u16_at(pos)?;
        pos += 2;
        let value = String::from_utf8_lossy(at(pos, length)?)
            .trim_end_matches('\0')
            .to_owned();
        pos += length;
        match key {
            b'a' => container.design = value,
            b'b' => container.part = value,
            b'c' => container.date = value,
            b'd' => container.time = value,
            other => {
                return Err(bad(format!(
                    "the header carries an unknown field `{}`",
                    char::from(other)
                )));
            }
        }
    }

    let payload = container.payload(bytes);
    if !payload
        .windows(4)
        .take(256)
        .any(|w| w == xilinx::SYNC_WORD.to_be_bytes())
    {
        return Err(bad(format!(
            "no sync word {:#010x} near the start of {} byte(s) of configuration data",
            xilinx::SYNC_WORD,
            payload.len()
        )));
    }
    Ok(container)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal but well-formed `.bit` file.
    fn sample(part: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x00, 0x09];
        out.extend_from_slice(&[0x0F, 0xF0, 0x0F, 0xF0, 0x0F, 0xF0, 0x0F, 0xF0, 0x00]);
        out.extend_from_slice(&[0x00, 0x01]);
        for (key, value) in [
            (b'a', "top"),
            (b'b', part),
            (b'c', "2026/01/01"),
            (b'd', "00:00:00"),
        ] {
            out.push(key);
            let bytes = value.as_bytes();
            let len = u16::try_from(bytes.len() + 1).expect("short field");
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(bytes);
            out.push(0);
        }
        out.push(b'e');
        out.extend_from_slice(
            &u32::try_from(payload.len())
                .expect("short payload")
                .to_be_bytes(),
        );
        out.extend_from_slice(payload);
        out
    }

    /// A payload that starts the way a real one does.
    fn payload(len: usize) -> Vec<u8> {
        let mut p = vec![0xFFu8; 16];
        p.extend_from_slice(&xilinx::SYNC_WORD.to_be_bytes());
        p.resize(len, 0x20);
        p
    }

    /// The header fields come back and the payload is exactly the bytes
    /// after the `e` field.
    #[test]
    fn a_bit_container_reads_back() {
        let body = payload(64);
        let file = sample("7a35tcpg236", &body);
        let bit = read_bit_container(&file).expect("reads");
        assert_eq!(bit.design, "top");
        assert_eq!(bit.part, "7a35tcpg236");
        assert_eq!(bit.date, "2026/01/01");
        assert_eq!(bit.time, "00:00:00");
        assert_eq!(bit.length, 64);
        assert_eq!(bit.payload(&file), body.as_slice());
        assert_eq!(bit.offset + bit.length, file.len());
    }

    /// Anything that is not a `.bit` is refused with a reason, and a
    /// truncated one is not read past its end.
    #[test]
    fn a_broken_container_is_refused() {
        assert!(matches!(
            read_bit_container(b"not a bitstream at all"),
            Err(ProgramError::Bit(_))
        ));
        assert!(matches!(read_bit_container(&[]), Err(ProgramError::Bit(_))));

        let file = sample("7a35tcpg236", &payload(64));
        for cut in [0, 4, 20, 40, file.len() - 1] {
            assert!(
                matches!(read_bit_container(&file[..cut]), Err(ProgramError::Bit(_))),
                "a file cut to {cut} bytes must be refused"
            );
        }

        // A payload with no sync word is not configuration data.
        let file = sample("7a35tcpg236", &[0x00; 64]);
        assert!(matches!(
            read_bit_container(&file),
            Err(ProgramError::Bit(_))
        ));

        // A length field big enough to wrap a `usize` on a 32-bit host
        // is refused, not trusted into a slice.
        let mut file = sample("7a35tcpg236", &payload(64));
        // The `e` key, its four-byte length and then 64 bytes of payload
        // are the tail of the file.
        let e = file.len() - 64 - 5;
        assert_eq!(file[e], b'e');
        file[e + 1..e + 5].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            read_bit_container(&file),
            Err(ProgramError::Bit(_))
        ));
    }

    /// The default clock is the slow one, and it is exact.
    #[test]
    fn the_default_clock_is_one_megahertz() {
        assert_eq!(ftdi::clock_hz(default_divisor()), DEFAULT_CLOCK_HZ);
    }

    /// The pin setup drives TCK, TDI, TMS and the board's buffer enable,
    /// and leaves TDO an input. Getting TDO's direction wrong is a
    /// silent failure: the read would come back as whatever the FTDI
    /// part is driving.
    #[test]
    fn the_pin_setup_leaves_tdo_an_input() {
        let (value, dirs) = BASYS3_PINS;
        assert_eq!(dirs & 0b0000_0001, 0b0000_0001, "TCK is an output");
        assert_eq!(dirs & 0b0000_0010, 0b0000_0010, "TDI is an output");
        assert_eq!(dirs & 0b0000_0100, 0, "TDO must be an input");
        assert_eq!(dirs & 0b0000_1000, 0b0000_1000, "TMS is an output");
        assert_eq!(dirs & 0b1000_0000, 0b1000_0000, "the buffer enable drives");
        assert_eq!(value & 0b0000_1000, 0b0000_1000, "TMS idles high");
        assert_eq!(value & 0b1000_0000, 0b1000_0000, "the buffer is enabled");
        assert_eq!(value & 0b0000_0001, 0, "TCK idles low");
    }

    /// The messages say what happened and what was not done, because
    /// they are what a user sees when a board does not answer.
    #[test]
    fn the_errors_explain_themselves() {
        let e = ProgramError::IdcodeMismatch {
            read: 0xFFFF_FFFF,
            expected: xilinx::IDCODE_XC7A35T,
        };
        let text = e.to_string();
        assert!(text.contains("0xffffffff"));
        assert!(text.contains("nothing was written"));

        let e = ProgramError::NoDevice {
            wanted: Some("210183BD4B37".to_owned()),
            found: vec!["OTHER".to_owned()],
        };
        assert!(e.to_string().contains("210183BD4B37"));
        assert!(e.to_string().contains("OTHER"));

        let e = ProgramError::Ambiguous(vec!["A".to_owned(), "B".to_owned()]);
        assert!(e.to_string().contains("--device"));

        let e = ProgramError::NotDone(xilinx::Status(0));
        assert!(e.to_string().contains("DONE did not assert"));
    }
}
