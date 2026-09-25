//! Configuring a real FPGA over USB: FTDI MPSSE, JTAG, and the Xilinx
//! 7-series configuration sequence.
//!
//! This is the one part of Reticle that talks to hardware, and it is
//! arranged so that almost none of it does.
//!
//! | Module | What it is | Touches a device |
//! |---|---|---|
//! | [`ftdi`] | the MPSSE command encoding (FTDI AN_108) | no |
//! | [`apollo`] | the Cynthion debugger's request set (`docs/apollo-protocol.md`) | no |
//! | [`jtag`] | the IEEE 1149.1 TAP state machine and scans | no |
//! | [`xilinx`] | the UG470 configuration sequence | no |
//! | [`gowin`] | the Gowin `.fs` container | no |
//! | [`lattice`] | naming a Lattice part from its `IDCODE` | no |
//! | [`choose_adapter`] | which transport a serial number belongs to | no |
//! | [`usb`] | opening, claiming and transfers, over `rawusb` | yes |
//!
//! Everything but [`usb`] is a *sequencer* or a decision: pure functions
//! from a request to a buffer of bytes to send and a description of what
//! comes back, from a reply to a decoded value, and from what is attached
//! to which transport to use. They are the same sans-I/O discipline the
//! rest of the crate follows, for the same reason: the tricky parts (bit
//! order, off-by-one lengths, the TAP walk, and which of two boards a
//! serial number belongs to) become unit-testable with no board on the
//! bench, and the part that cannot be tested that way shrinks to "write
//! these bytes, read those".
//!
//! # Two transports
//!
//! There are two ways out of this module to a scan chain, and they have
//! almost nothing in common:
//!
//! - an **FTDI MPSSE** ([`ftdi`], [`usb::Cable`]) — a shift engine, told
//!   about TMS bits, driven by bulk transfers;
//! - a **Cynthion's Apollo microcontroller** ([`apollo`],
//!   [`usb::Debugger`]) — a TAP controller of its own, told about state
//!   *numbers*, driven by control requests, and with no way to be sent a
//!   TMS sequence at all.
//!
//! What they share is one level above both encodings: [`jtag::Plan`], a
//! list of named JTAG operations. [`jtag::Scan::apply`] compiles a plan
//! to MPSSE bytes and [`apollo::compile`] compiles the same plan to
//! control requests, so [`jtag::idcode_plan`] is written once and both
//! transports perform it.
//!
//! Which of the two a serial number belongs to is a **decision made
//! before anything is opened**: [`usb::adapters`] enumerates the bus once
//! and [`choose_adapter`] — a pure function, testable with nothing
//! attached — says which transport owns that serial. It is not a retry
//! loop for a reason. The two protocols have nothing in common, so a
//! fallback would land on whatever device happened to answer; and
//! reaching Apollo may take a Cynthion's USB port away from its FPGA,
//! which is not undone in software, so a request the Apollo transport
//! cannot serve has to be refused before that rather than after it.
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

pub mod apollo;
pub mod ftdi;
pub mod gowin;
pub mod jtag;
pub mod lattice;
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

/// The same byte for a Sipeed Tang board's on-board JTAG adapter (a Tang
/// Primer 20K dock has one): TCK, TDI and TMS as outputs, TMS high, and
/// nothing else driven. `ADBUS7` means nothing there, so it is left an
/// input. These are openFPGALoader's numbers for its `ft2232` cable,
/// which its board table gives the Tang Primer 20K (`cable.hpp`,
/// `board.hpp`).
pub const SIPEED_PINS: (u8, u8) = (0x08, 0x0B);

// ---------------------------------------------------------------------
// Which adapter a serial number names
// ---------------------------------------------------------------------

/// What kind of adapter a serial number belongs to.
///
/// This exists because `reticle program --device <serial>` has to decide
/// **which transport owns that serial before it opens anything**. The two
/// transports are not interchangeable and neither is a fallback for the
/// other: an FTDI cable sent Apollo's control requests answers nothing,
/// and a Cynthion has no bulk endpoints to write MPSSE bytes to. With
/// several boards attached, "try one and fall back to the other" is also
/// how the wrong board gets opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    /// An FTDI FT2232, driven as an MPSSE shift engine by [`usb::Cable`].
    Ftdi,
    /// A Cynthion whose FPGA owns the USB port. Reaching Apollo means
    /// asking the gateware to give the port up, which
    /// `docs/apollo-protocol.md` §2 describes and [`usb::Debugger::open`]
    /// performs.
    CynthionGateware,
    /// A Cynthion already in Apollo debugger mode, driven by
    /// [`usb::Debugger`].
    CynthionDebugger,
}

impl AdapterKind {
    /// Whether this adapter is reached through [`usb::Debugger`] rather
    /// than [`usb::Cable`]. This is the dispatch.
    #[must_use]
    pub fn is_cynthion(self) -> bool {
        matches!(
            self,
            AdapterKind::CynthionGateware | AdapterKind::CynthionDebugger
        )
    }

    /// A short phrase naming what this is, for a listing or an error.
    ///
    /// One phrase per kind, in one place, so `--list`, the line the
    /// command prints when it has chosen, and the message that lists what
    /// *is* attached cannot describe the same board differently.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            AdapterKind::Ftdi => "FTDI cable",
            AdapterKind::CynthionGateware => "Cynthion, running gateware (hands over on demand)",
            AdapterKind::CynthionDebugger => "Cynthion, Apollo debugger",
        }
    }

    /// What *kind of name* the serial number beside this adapter is.
    ///
    /// `--device` takes a string, and on a Cynthion there are two strings
    /// in play that are not the same kind of thing. A board in gateware
    /// mode advertises its **flash UID** — sixteen hex digits, the board's
    /// stable identity, the same before and after a handover — as its USB
    /// serial string. A board in debugger mode reports the
    /// microcontroller's own serial string, which is not the flash UID and
    /// is not derived from it.
    ///
    /// So a listing that prints a bare column of strings invites someone
    /// to copy one without knowing which they copied, and the two behave
    /// differently: a flash UID names a board, and a debugger's serial
    /// names a board *in one mode*. This is the label that says which.
    ///
    /// See `docs/apollo-protocol.md` §1 and §9, and note what is **not**
    /// here: a debugger's flash UID cannot be read, because Apollo reads
    /// it by forcing the FPGA offline and driving the board's
    /// configuration flash over JTAG — §7's line, which this project does
    /// not cross.
    #[must_use]
    pub fn identifier(self) -> &'static str {
        match self {
            AdapterKind::Ftdi => "USB serial number",
            AdapterKind::CynthionGateware => "USB serial number, which is this board's flash UID",
            AdapterKind::CynthionDebugger => "the microcontroller's own USB serial number",
        }
    }
}

/// How many hex digits a Cynthion's flash UID has.
///
/// The board here reports `2a5a4adf30c460de`: sixteen digits, sixty-four
/// bits, which is the width of a JEDEC `READ_UID` reply. HIGH for this
/// board, from the flash UID its owner's run of Apollo's own `info`
/// command printed; MEDIUM as a property of every Cynthion, since one
/// board is one board.
const FLASH_UID_DIGITS: usize = 16;

/// Whether a `--device` value is spelled like a Cynthion's flash UID.
///
/// This only decides **which sentence an error gets**, never which device
/// is opened: a value that matches an attached adapter's serial number is
/// used whatever it looks like. The point is that a flash UID which
/// matches nothing has a specific reason — the board wearing it is in
/// debugger mode, where its USB serial is something else and its flash
/// UID cannot be read without crossing `docs/apollo-protocol.md` §7 —
/// and a user who typed one deserves that sentence rather than a bare
/// "no such adapter".
fn looks_like_flash_uid(value: &str) -> bool {
    value.len() == FLASH_UID_DIGITS && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// One attached programming adapter: a serial number and what carries it.
///
/// A Cynthion contributes **one** of these, never two, because a board is
/// only ever in one mode at a time and the serial number it reports is
/// that mode's: its flash UID in gateware mode and the
/// microcontroller's own string as a debugger (`docs/apollo-protocol.md`
/// §1). So this is a serial number and the adapter wearing it, not a
/// board identity — the board identity is the flash UID, and only one of
/// the two modes puts it on the bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    /// The serial number the device reports, or a description of where it
    /// is when that could not be read.
    pub serial: String,
    /// Which transport reaches it.
    pub kind: AdapterKind,
}

impl fmt::Display for Adapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.serial, self.kind.describe())
    }
}

/// Picks the adapter a `--device` serial names out of everything
/// attached.
///
/// This is a pure function over the list [`usb::adapters`] produces, and
/// it runs **before any device is opened**, which is the point: the
/// transport is chosen from the descriptors of what is on the bus, not by
/// trying one transport and catching its failure.
///
/// A Cynthion has two names and this matches **whichever one is on the
/// bus**: in gateware mode the USB serial string is the board's flash
/// UID, and in debugger mode it is the microcontroller's own string. So
/// either spelling selects a board, as long as it is the spelling of the
/// mode the board is in now. Nothing here tries to follow a serial across
/// a handover; that cannot work, and [`usb::Debugger::open`] does not
/// attempt it either.
///
/// # Errors
///
/// [`ProgramError::NoAdapter`] when nothing is attached,
/// [`ProgramError::NoDevice`] when a named serial matches nothing —
/// listing what *is* there, since that list is the answer to "then what
/// should I have typed" — [`ProgramError::UnreadableFlashUid`] for the one
/// case that has a better explanation than "no such adapter", and
/// [`ProgramError::Ambiguous`] when several are attached and none was
/// named. Guessing is not an option: with an FTDI cable and a Cynthion on
/// the same machine a guess reaches the wrong board.
pub fn choose_adapter<'a>(
    serial: Option<&str>,
    attached: &'a [Adapter],
) -> Result<&'a Adapter, ProgramError> {
    if attached.is_empty() {
        return Err(ProgramError::NoAdapter);
    }
    let Some(wanted) = serial else {
        if attached.len() > 1 {
            return Err(ProgramError::Ambiguous(
                attached.iter().map(Adapter::to_string).collect(),
            ));
        }
        return Ok(&attached[0]);
    };
    if let Some(adapter) = attached.iter().find(|adapter| adapter.serial == wanted) {
        return Ok(adapter);
    }
    // A flash UID that matched nothing, with a debugger attached, is the
    // one miss with a real explanation: the board wearing that UID is
    // very likely the debugger in front of us, and its USB serial is a
    // different string. Reticle will not read a debugger's flash UID to
    // check, because Apollo reads it by forcing the FPGA offline and
    // driving the configuration flash over JTAG.
    let debuggers: Vec<String> = attached
        .iter()
        .filter(|adapter| adapter.kind == AdapterKind::CynthionDebugger)
        .map(|adapter| adapter.serial.clone())
        .collect();
    if looks_like_flash_uid(wanted) && !debuggers.is_empty() {
        return Err(ProgramError::UnreadableFlashUid {
            wanted: wanted.to_owned(),
            debuggers,
        });
    }
    Err(ProgramError::NoDevice {
        wanted: Some(wanted.to_owned()),
        found: attached.iter().map(Adapter::to_string).collect(),
    })
}

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
    /// No Cynthion matched.
    ///
    /// Separate from [`ProgramError::NoDevice`] because the two look for
    /// different things and the message has to say which: a user who has
    /// no FTDI cable attached and a user whose Cynthion is unplugged are
    /// not helped by the same sentence.
    NoCynthion {
        /// The serial number that was asked for, if any.
        wanted: Option<String>,
        /// The Cynthions that were found, and whether each is already in
        /// debugger mode.
        found: Vec<(String, bool)>,
    },
    /// Nothing that can reach a scan chain is attached at all.
    ///
    /// Separate from [`ProgramError::NoDevice`] and
    /// [`ProgramError::NoCynthion`], which are each one transport
    /// reporting that *its* kind of adapter is missing. This one is the
    /// answer to "program something", before a transport has been
    /// chosen, so it has to name every kind that was looked for.
    NoAdapter,
    /// A `--device` value spelled like a Cynthion's flash UID matched no
    /// attached serial number, and at least one Apollo debugger is on the
    /// bus whose flash UID this project will not read.
    ///
    /// This is a miss with a specific cause, so it gets a specific
    /// sentence. A board in gateware mode advertises its flash UID as its
    /// USB serial string and is found by it; a board in debugger mode
    /// reports the microcontroller's string instead, and the only way to
    /// its flash UID is Apollo's own: force the FPGA offline (vendor
    /// request `0xC1`) and drive the board's configuration flash over
    /// JTAG. `docs/apollo-protocol.md` §7 is why that is not done here.
    UnreadableFlashUid {
        /// The flash UID that was asked for.
        wanted: String,
        /// The serial numbers of the attached debuggers, which are what
        /// can be typed instead.
        debuggers: Vec<String>,
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
            ProgramError::NoCynthion { wanted, found } => {
                match wanted {
                    Some(serial) => write!(f, "no Cynthion with serial number `{serial}`")?,
                    None => write!(
                        f,
                        "no Cynthion found (looking for USB {:#06x}:{:#06x} in gateware mode \
                         or {:#06x}:{:#06x} in debugger mode)",
                        apollo::VENDOR_ID,
                        apollo::GATEWARE_PRODUCT_ID,
                        apollo::VENDOR_ID,
                        apollo::DEBUGGER_PRODUCT_ID
                    )?,
                }
                if found.is_empty() {
                    return Ok(());
                }
                let listed: Vec<String> = found
                    .iter()
                    .map(|(serial, debugger)| {
                        let mode = if *debugger { "debugger" } else { "gateware" };
                        format!("{serial} ({mode} mode)")
                    })
                    .collect();
                write!(f, "; attached: {}", listed.join(", "))
            }
            ProgramError::NoAdapter => write!(
                f,
                "no programming adapter is attached: no FTDI FT2232 \
                 ({FTDI_VENDOR_ID:#06x}:{FT2232H_PRODUCT_ID:#06x}) and no Cynthion \
                 ({:#06x}:{:#06x} in gateware mode or {:#06x}:{:#06x} as an Apollo debugger)",
                apollo::VENDOR_ID,
                apollo::GATEWARE_PRODUCT_ID,
                apollo::VENDOR_ID,
                apollo::DEBUGGER_PRODUCT_ID
            ),
            ProgramError::UnreadableFlashUid { wanted, debuggers } => write!(
                f,
                "no attached adapter reports the serial number `{wanted}`. It is spelled like a \
                 Cynthion's flash UID, which is what a board advertises as its USB serial \
                 number *in gateware mode* — a board already in Apollo debugger mode reports \
                 the microcontroller's serial number instead, and Reticle will not read a \
                 debugger's flash UID, because Apollo reads it by forcing the FPGA offline \
                 (vendor request {:#04x}) and driving the board's configuration flash over \
                 JTAG, which this project does not do. Name the debugger instead: {}",
                apollo::REQUEST_FORCE_FPGA_OFFLINE,
                debuggers.join(", ")
            ),
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

    /// The board attached to the machine this was written on, in each of
    /// its two modes, plus a cable to get the dispatch wrong with.
    ///
    /// These are the real strings, from `docs/apollo-protocol.md` §1 and
    /// §9. The two Cynthion ones are the point of the whole selection
    /// layer: the gateware one is the board's flash UID and the debugger
    /// one is the microcontroller's own serial number, they are not the
    /// same length and neither can be derived from the other, and a user
    /// who reads one off a listing has no way to know which.
    const CABLE: &str = "210183BD4B37";
    const FLASH_UID: &str = "2a5a4adf30c460de";
    const MICROCONTROLLER: &str = "35L6H2CMGJJVCIBAEA3GCLAN74";

    fn adapter(serial: &str, kind: AdapterKind) -> Adapter {
        Adapter {
            serial: serial.to_owned(),
            kind,
        }
    }

    /// Either of a Cynthion's two names selects the Apollo transport, and
    /// an FTDI serial selects the FTDI one — whichever order they are
    /// attached in, and with the other kind on the bus at the same time.
    ///
    /// This is the property the command line rests on. The two Cynthion
    /// strings have nothing in common to pattern-match on, so the only
    /// thing that can decide the transport is the descriptor of the device
    /// wearing the string, which is what `AdapterKind` carries.
    #[test]
    fn either_of_a_cynthions_two_names_chooses_the_apollo_transport() {
        let gateware = [
            adapter(CABLE, AdapterKind::Ftdi),
            adapter(FLASH_UID, AdapterKind::CynthionGateware),
        ];
        assert_eq!(
            choose_adapter(Some(FLASH_UID), &gateware).unwrap().kind,
            AdapterKind::CynthionGateware
        );
        assert_eq!(
            choose_adapter(Some(CABLE), &gateware).unwrap().kind,
            AdapterKind::Ftdi
        );

        // The same board, in the other mode, under its other name.
        let debugger = [
            adapter(MICROCONTROLLER, AdapterKind::CynthionDebugger),
            adapter(CABLE, AdapterKind::Ftdi),
        ];
        assert_eq!(
            choose_adapter(Some(MICROCONTROLLER), &debugger)
                .unwrap()
                .kind,
            AdapterKind::CynthionDebugger
        );
        assert_eq!(
            choose_adapter(Some(CABLE), &debugger).unwrap().kind,
            AdapterKind::Ftdi
        );

        // Only the Cynthion kinds route to Apollo, and both of them do.
        assert!(AdapterKind::CynthionGateware.is_cynthion());
        assert!(AdapterKind::CynthionDebugger.is_cynthion());
        assert!(!AdapterKind::Ftdi.is_cynthion());
    }

    /// A serial number is a name for a *mode*, not for a board, so the
    /// name of the mode a board is not in selects nothing — and the error
    /// lists what is attached, since that list is the answer.
    #[test]
    fn the_name_of_the_other_mode_selects_nothing() {
        let debugger = [adapter(MICROCONTROLLER, AdapterKind::CynthionDebugger)];
        // The microcontroller's serial while the board runs gateware.
        let gateware = [adapter(FLASH_UID, AdapterKind::CynthionGateware)];
        let err = choose_adapter(Some(MICROCONTROLLER), &gateware).unwrap_err();
        let text = err.to_string();
        assert!(matches!(err, ProgramError::NoDevice { .. }), "{text}");
        assert!(text.contains(MICROCONTROLLER), "{text}");
        assert!(text.contains(FLASH_UID), "{text}");
        assert!(text.contains("gateware"), "{text}");

        // And the other way round: a flash UID against a board that is
        // already a debugger. This one has a better answer than "no such
        // adapter", because the reason is that Reticle will not read a
        // debugger's flash UID.
        let err = choose_adapter(Some(FLASH_UID), &debugger).unwrap_err();
        let text = err.to_string();
        assert!(
            matches!(err, ProgramError::UnreadableFlashUid { .. }),
            "{text}"
        );
        assert!(text.contains(FLASH_UID), "{text}");
        assert!(text.contains(MICROCONTROLLER), "{text}");
        assert!(text.contains("flash UID"), "{text}");
        // The forbidden request is named so the refusal can be checked
        // against `docs/apollo-protocol.md` §7 rather than believed.
        assert!(text.contains("0xc1"), "{text}");

        // A string that is not spelled like a flash UID gets the plain
        // message even with a debugger attached: the special sentence is
        // an explanation, not a catch-all.
        let err = choose_adapter(Some("NOSUCHTHING"), &debugger).unwrap_err();
        assert!(matches!(err, ProgramError::NoDevice { .. }));
        assert!(looks_like_flash_uid(FLASH_UID));
        assert!(!looks_like_flash_uid(MICROCONTROLLER));
        assert!(!looks_like_flash_uid(CABLE), "twelve digits, not sixteen");
        assert!(!looks_like_flash_uid("2a5a4adf30c460dz"), "not hex");
    }

    /// With nothing named, one adapter is taken and two are refused. A
    /// guess between an FTDI cable and a Cynthion is a guess about which
    /// board to touch.
    #[test]
    fn a_choice_between_two_adapters_is_never_guessed() {
        let one = [adapter(MICROCONTROLLER, AdapterKind::CynthionDebugger)];
        assert_eq!(
            choose_adapter(None, &one).unwrap().kind,
            AdapterKind::CynthionDebugger
        );

        let two = [
            adapter(CABLE, AdapterKind::Ftdi),
            adapter(MICROCONTROLLER, AdapterKind::CynthionDebugger),
        ];
        let err = choose_adapter(None, &two).unwrap_err();
        let text = err.to_string();
        assert!(matches!(err, ProgramError::Ambiguous(_)), "{text}");
        assert!(text.contains("--device"), "{text}");
        // Both, with what each is: a serial number on its own does not
        // tell anyone which board it belongs to.
        assert!(text.contains(CABLE), "{text}");
        assert!(text.contains("FTDI cable"), "{text}");
        assert!(text.contains(MICROCONTROLLER), "{text}");
        assert!(text.contains("Apollo debugger"), "{text}");

        // Nothing attached is its own error, naming every kind that was
        // looked for rather than only one transport's.
        let err = choose_adapter(None, &[]).unwrap_err();
        let text = err.to_string();
        assert!(matches!(err, ProgramError::NoAdapter), "{text}");
        assert!(text.contains("0x0403"), "{text}");
        assert!(text.contains("0x1d50"), "{text}");
        // A named serial with nothing attached is still a missing serial.
        assert!(matches!(
            choose_adapter(Some(CABLE), &[]).unwrap_err(),
            ProgramError::NoAdapter
        ));
    }

    /// Every kind describes itself and says what kind of name its serial
    /// number is, because `--list` prints both and a person copies one of
    /// them into `--device`.
    #[test]
    fn every_adapter_kind_says_what_it_is_and_what_its_name_is() {
        for kind in [
            AdapterKind::Ftdi,
            AdapterKind::CynthionGateware,
            AdapterKind::CynthionDebugger,
        ] {
            assert!(!kind.describe().is_empty());
            assert!(!kind.identifier().is_empty());
        }
        // The two Cynthion modes must not describe themselves the same
        // way: which mode a board is in is what decides which of its two
        // names is on the bus.
        assert_ne!(
            AdapterKind::CynthionGateware.describe(),
            AdapterKind::CynthionDebugger.describe()
        );
        assert!(
            AdapterKind::CynthionGateware
                .identifier()
                .contains("flash UID"),
            "a gateware advertises the board's flash UID as its serial"
        );
        assert!(
            AdapterKind::CynthionDebugger
                .identifier()
                .contains("microcontroller"),
            "a debugger reports the microcontroller's own serial"
        );
        // The listing line and the dispatch line share this, so it has to
        // carry the serial and the kind together.
        let shown = adapter(MICROCONTROLLER, AdapterKind::CynthionDebugger).to_string();
        assert!(shown.contains(MICROCONTROLLER), "{shown}");
        assert!(shown.contains("Apollo debugger"), "{shown}");
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

        // A missing Cynthion must not be reported as a missing FTDI
        // cable: they are different boards and different advice.
        let e = ProgramError::NoCynthion {
            wanted: None,
            found: Vec::new(),
        };
        let text = e.to_string();
        assert!(text.contains("no Cynthion found"), "{text}");
        assert!(text.contains("0x1d50"), "{text}");
        assert!(!text.contains("FTDI"), "{text}");

        let e = ProgramError::NoCynthion {
            wanted: Some("2a5a4adf30c460de".to_owned()),
            found: vec![("35L6H2CMGJJVCIBAEA3GCLAN74".to_owned(), true)],
        };
        let text = e.to_string();
        assert!(text.contains("2a5a4adf30c460de"), "{text}");
        assert!(text.contains("35L6H2CMGJJVCIBAEA3GCLAN74"), "{text}");
        // Which mode each attached board is in is the thing that
        // explains why the one asked for was not found: a board puts its
        // flash UID on the bus in gateware mode and the
        // microcontroller's own serial number as a debugger, so the two
        // modes of one board answer to different names.
        assert!(text.contains("debugger mode"), "{text}");

        let e = ProgramError::NotDone(xilinx::Status(0));
        assert!(e.to_string().contains("DONE did not assert"));
    }
}
