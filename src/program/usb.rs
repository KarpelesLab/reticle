//! The USB transport: opening an adapter and moving bytes across it.
//!
//! This is the only file in Reticle that touches a device, and it is
//! deliberately the shallowest: it opens, detaches, claims, writes and
//! reads. Every decision about *what* to write is made by [`super::ftdi`],
//! [`super::jtag`], [`super::apollo`] and [`super::xilinx`], which know
//! nothing about USB.
//!
//! There are two devices here, not one:
//!
//! | Type | Part | Protocol |
//! |---|---|---|
//! | [`Cable`] | an FTDI FT2232H or FT2232D | MPSSE bytes over bulk endpoints |
//! | [`Debugger`] | a Cynthion's Apollo microcontroller | vendor control requests on endpoint zero |
//!
//! They share [`super::jtag::Plan`] and nothing else, because they have
//! nothing else in common: one is a shift engine told about TMS and the
//! other is a TAP controller told about states. See
//! `docs/apollo-protocol.md`.
//!
//! It still contains no `unsafe`. [`rawusb`] is Karpeles Lab's own
//! dependency-free USB crate; its `src/sys/` is where the ioctls live.
//!
//! # Three things the FT2232H needs that are not MPSSE
//!
//! - **The kernel gets there first.** Linux binds `ftdi_sio` to *both*
//!   interfaces of an FT2232H, so channel A appears as a `/dev/ttyUSB*`
//!   even though it is the JTAG channel.
//!   [`Cable::open`] detaches it; a power cycle or a replug gives it
//!   back.
//! - **Vendor control requests switch the mode.** Reset, latency timer
//!   and bit mode are FTDI vendor requests on endpoint zero, not MPSSE
//!   commands. Their numbers are in FTDI's AN_135 and in the D2XX
//!   programmer's guide.
//! - **Every bulk-IN packet is prefixed.** Two modem-status bytes head
//!   each 512-byte packet whether or not the MPSSE produced anything,
//!   which also means a read never blocks: an idle device answers with
//!   status and no data. [`super::ftdi::strip_status`] removes them and
//!   [`Cable::run`] keeps reading until the job's bytes have arrived.

use std::time::{Duration, Instant};

use rawusb::{Context, DeviceHandle, types::Direction};

use super::jtag::{Job, TapState};
use super::{
    AdapterKind, FT2232H_PRODUCT_ID, FTDI_VENDOR_ID, MPSSE_INTERFACE, ProgramError, apollo, ftdi,
};

/// `bmRequestType` for an FTDI vendor request to the device, host to
/// device.
const REQ_TYPE_OUT: u8 = 0x40;

/// `SIO_RESET`: value 0 resets the channel, 1 purges its receive buffer,
/// 2 purges its transmit buffer.
const SIO_RESET: u8 = 0x00;
/// `SIO_SET_FLOW_CTRL`: value 0 is no flow control at all, which is what
/// a shift engine wants.
const SIO_SET_FLOW_CTRL: u8 = 0x02;
/// `SIO_SET_LATENCY_TIMER`: how long the part waits before sending a
/// short packet, in milliseconds.
const SIO_SET_LATENCY_TIMER: u8 = 0x09;
/// `SIO_SET_BITMODE`: value is `(mode << 8) | direction_mask`.
const SIO_SET_BITMODE: u8 = 0x0B;

/// Bit mode 0: back to whatever the EEPROM says.
const BITMODE_RESET: u16 = 0x00;
/// Bit mode 2: MPSSE.
const BITMODE_MPSSE: u16 = 0x02;

/// FTDI numbers its channels from one on the control endpoint, while USB
/// numbers interfaces from zero.
const fn channel_index(interface: u8) -> u16 {
    interface as u16 + 1
}

/// How long any single transfer may take.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to keep asking for a reply that has not arrived yet.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Bytes per bulk write. The kernel will take more, but a bounded chunk
/// keeps the timeout meaningful and gives a progress callback something
/// to report.
const WRITE_CHUNK: usize = 64 * 1024;

/// An open FTDI adapter with its MPSSE running.
pub struct Cable {
    handle: DeviceHandle,
    interface: u8,
    in_endpoint: u8,
    out_endpoint: u8,
    packet_size: usize,
    serial: String,
    chip: ftdi::Chip,
}

/// Turns a `rawusb` error into ours, with the operation that failed.
fn usb_err(what: &str, err: &rawusb::Error) -> ProgramError {
    ProgramError::Usb(format!("{what}: {err}"))
}

/// One device's serial number, read **without opening it** where the
/// operating system knows it.
///
/// `rawusb::Device::serial_number` reads it from sysfs on Linux, the IOKit
/// registry on macOS and the parent hub on Windows; a device the system
/// cannot answer for is opened, once, as a fallback. That is the same
/// two-step `rawusb::Context::find_device_by_serial` does internally, and
/// it is spelled out here rather than called because every caller below
/// wants to *list* or to *classify* devices, not to find one: `--list`
/// names what is attached and [`super::choose_adapter`] decides which kind
/// of adapter carries a serial before anything is opened.
///
/// Not opening a device that was not asked for is the point. A machine with
/// several boards on it is the normal case here, and opening somebody's
/// other board to ask its name is a side effect this has no business
/// having.
fn serial_of(device: &rawusb::Device) -> Option<String> {
    device.serial_number().map(str::to_owned).or_else(|| {
        device
            .open()
            .ok()
            .and_then(|handle| handle.read_serial_number_string().ok().flatten())
    })
}

/// Every attached device whose `(vendor, product)` pair is in `ids`, as
/// `(product_id, serial)`, in the order the system lists them.
///
/// **This is the one function in Reticle that turns the bus into a list of
/// adapters**, and it is deliberately the only one: it is what
/// `reticle program --list` prints and what [`super::choose_adapter`]
/// decides a `--device` value against, so there is one place where a
/// serial number comes from and one place to change if reading one ever
/// changes.
///
/// Each serial comes from [`serial_of`], so **listing what is attached does
/// not open it**. That matters here for a reason beyond tidiness: the
/// version of this before `rawusb` could report a serial from the operating
/// system opened every FT2232H and every Cynthion on the bus to ask its
/// name.
///
/// A device whose serial cannot be read at all is still listed, with where
/// it is instead of a name. A serial nobody can read is still an adapter
/// somebody has attached, and a device node that is not readable is
/// precisely the case where seeing the adapter is what explains the
/// failure.
fn serials_of(ids: &[(u16, u16)]) -> Result<Vec<(u16, String)>, ProgramError> {
    let context = Context::new().map_err(|e| usb_err("opening the USB subsystem", &e))?;
    let devices = context
        .devices()
        .map_err(|e| usb_err("listing USB devices", &e))?;
    let mut found = Vec::new();
    for device in devices {
        if !ids.contains(&(device.vendor_id(), device.product_id())) {
            continue;
        }
        let serial = serial_of(&device).unwrap_or_else(|| {
            format!(
                "bus {} address {} (no serial number)",
                device.bus_number(),
                device.address()
            )
        });
        found.push((device.product_id(), serial));
    }
    Ok(found)
}

/// Every attached programming adapter of either kind, in the order the
/// system lists them.
///
/// This is one enumeration of the bus covering both transports, which is
/// what [`super::choose_adapter`] needs to decide which transport a
/// `--device` serial belongs to **before anything is opened**. It is also
/// what `reticle program --list` prints.
///
/// # Errors
///
/// [`ProgramError::Usb`] when the USB subsystem cannot be reached at all.
/// An individual device that cannot be opened is listed without its
/// serial number rather than failing the listing.
pub fn adapters() -> Result<Vec<super::Adapter>, ProgramError> {
    let ids = [
        (FTDI_VENDOR_ID, FT2232H_PRODUCT_ID),
        (apollo::VENDOR_ID, apollo::GATEWARE_PRODUCT_ID),
        (apollo::VENDOR_ID, apollo::DEBUGGER_PRODUCT_ID),
    ];
    Ok(serials_of(&ids)?
        .into_iter()
        .filter_map(|(product, serial)| {
            let kind = match product {
                FT2232H_PRODUCT_ID => AdapterKind::Ftdi,
                apollo::GATEWARE_PRODUCT_ID => AdapterKind::CynthionGateware,
                apollo::DEBUGGER_PRODUCT_ID => AdapterKind::CynthionDebugger,
                _ => return None,
            };
            Some(super::Adapter { serial, kind })
        })
        .collect())
}

/// The serial numbers of every attached FT2232H, in the order the system
/// lists them.
///
/// # Errors
///
/// [`ProgramError::Usb`] when the USB subsystem cannot be reached at
/// all. A device that cannot be opened (no permission, already in use)
/// is listed without its serial number rather than failing the listing.
pub fn list() -> Result<Vec<String>, ProgramError> {
    Ok(serials_of(&[(FTDI_VENDOR_ID, FT2232H_PRODUCT_ID)])?
        .into_iter()
        .map(|(_, serial)| serial)
        .collect())
}

impl Cable {
    /// Opens the adapter, detaches whatever kernel driver holds its
    /// MPSSE channel, claims it and puts it into MPSSE mode.
    ///
    /// `serial` picks one adapter when several are attached; with
    /// `None` and more than one attached the call fails rather than
    /// guessing, because guessing means programming the wrong board.
    ///
    /// What comes back is a channel in MPSSE mode with nothing else set:
    /// no clock divisor and no pin directions, because those describe
    /// the *board* rather than the adapter. The first thing a caller
    /// runs is [`super::xilinx::init_job`] with
    /// [`super::default_divisor`] and [`super::BASYS3_PINS`], or its own.
    ///
    /// # Errors
    ///
    /// [`ProgramError::NoDevice`] when nothing matches,
    /// [`ProgramError::Ambiguous`] when several do and none was named,
    /// and [`ProgramError::Usb`] for anything the USB layer refuses —
    /// most often a permission problem on the device node.
    pub fn open(serial: Option<&str>) -> Result<Cable, ProgramError> {
        let context = Context::new().map_err(|e| usb_err("opening the USB subsystem", &e))?;
        let devices = context
            .devices()
            .map_err(|e| usb_err("listing USB devices", &e))?;

        let mut candidates = Vec::new();
        let mut found = Vec::new();
        for device in devices {
            if device.vendor_id() != FTDI_VENDOR_ID || device.product_id() != FT2232H_PRODUCT_ID {
                continue;
            }
            let handle = device
                .open()
                .map_err(|e| usb_err("opening the adapter (is the device node readable?)", &e))?;
            let this = handle
                .read_serial_number_string()
                .ok()
                .flatten()
                .unwrap_or_default();
            found.push(this.clone());
            if serial.is_none_or(|wanted| wanted == this) {
                // `bcdDevice` says which FTDI silicon this is, which
                // decides the MPSSE clock; keep it while the device is
                // still in scope.
                let bcd = device.device_descriptor().device_version.0;
                candidates.push((handle, this, bcd));
            }
        }

        if candidates.is_empty() {
            return Err(ProgramError::NoDevice {
                wanted: serial.map(str::to_owned),
                found,
            });
        }
        if candidates.len() > 1 {
            return Err(ProgramError::Ambiguous(
                candidates.into_iter().map(|(_, s, _)| s).collect(),
            ));
        }
        let (handle, serial, bcd) = candidates.pop().expect("exactly one candidate");

        // Which FTDI part this is decides the MPSSE master clock and
        // whether the H-only opcodes may be sent at all.
        let Some(chip) = ftdi::Chip::from_bcd_device(bcd) else {
            return Err(ProgramError::Usb(format!(
                "the adapter reports bcdDevice {bcd:#06x}, which is not an FTDI part this \
                 knows the MPSSE clock of; refusing rather than guessing at a clock"
            )));
        };

        let (in_endpoint, out_endpoint, packet_size) = endpoints(&handle)?;

        // The kernel's ftdi_sio binds both channels of an FT2232H, so
        // the MPSSE one has to be taken away from it first. Nothing is
        // lost: the driver rebinds on the next replug or power cycle.
        if handle
            .kernel_driver_active(MPSSE_INTERFACE)
            .unwrap_or(false)
        {
            handle
                .detach_kernel_driver(MPSSE_INTERFACE)
                .map_err(|e| usb_err("detaching the kernel's serial driver", &e))?;
        }
        handle
            .claim_interface(MPSSE_INTERFACE)
            .map_err(|e| usb_err("claiming the adapter's JTAG interface", &e))?;

        let cable = Cable {
            handle,
            interface: MPSSE_INTERFACE,
            in_endpoint,
            out_endpoint,
            packet_size,
            serial,
            chip,
        };
        cable.enter_mpsse()?;
        Ok(cable)
    }

    /// The adapter's serial number.
    #[must_use]
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Which FTDI part the adapter has.
    #[must_use]
    pub fn chip(&self) -> ftdi::Chip {
        self.chip
    }

    /// Runs a job: writes its commands and reads back exactly as many
    /// bytes of TDO as it says to expect.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] for a failed transfer and
    /// [`ProgramError::Job`] when the reply never arrives in full.
    pub fn run(&self, job: &Job) -> Result<Vec<u8>, ProgramError> {
        self.run_with_progress(job, &mut |_, _| {})
    }

    /// [`run`](Self::run), reporting `(written, total)` after each chunk
    /// so a caller can show progress on a two-megabyte bitstream.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run).
    pub fn run_with_progress(
        &self,
        job: &Job,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Vec<u8>, ProgramError> {
        let commands = job.commands();
        let mut written = 0;
        while written < commands.len() {
            let end = (written + WRITE_CHUNK).min(commands.len());
            let n = self
                .handle
                .bulk_write(self.out_endpoint, &commands[written..end], TRANSFER_TIMEOUT)
                .map_err(|e| usb_err("writing MPSSE commands", &e))?;
            if n == 0 {
                return Err(ProgramError::Usb(
                    "the adapter accepted no command bytes".to_owned(),
                ));
            }
            written += n;
            progress(written, commands.len());
        }
        if job.read_len() == 0 {
            return Ok(Vec::new());
        }
        self.read_exactly(job.read_len())
    }

    /// Reads until `wanted` bytes of MPSSE output have arrived, dropping
    /// the modem-status bytes that head every packet.
    fn read_exactly(&self, wanted: usize) -> Result<Vec<u8>, ProgramError> {
        let mut out = Vec::with_capacity(wanted);
        let deadline = Instant::now() + REPLY_TIMEOUT;
        let mut raw = vec![0u8; self.packet_size.max(64) * 8];
        while out.len() < wanted {
            if Instant::now() >= deadline {
                return Err(super::jtag::JobError::ShortReply {
                    wanted,
                    got: out.len(),
                }
                .into());
            }
            let n = match self
                .handle
                .bulk_read(self.in_endpoint, &mut raw, TRANSFER_TIMEOUT)
            {
                Ok(n) => n,
                Err(e) if e.is_timeout() => continue,
                Err(e) => return Err(usb_err("reading TDO back", &e)),
            };
            let data = ftdi::strip_status(&raw[..n], self.packet_size);
            // A rejected command is answered with `0xFA` and the opcode,
            // ahead of anything the job captured. It is only looked for
            // at the very head of a job's reply and only when the byte
            // after it is an opcode this crate emits: captured TDO is
            // arbitrary data, and one byte in 256 of it is `0xFA`, so a
            // scan of the whole reply would fail good runs.
            if out.is_empty()
                && data.first() == Some(&ftdi::BAD_COMMAND)
                && let Some(&opcode) = data.get(1)
                && let Some(name) = ftdi::opcode_name(opcode)
            {
                return Err(ProgramError::Usb(format!(
                    "the adapter rejected MPSSE command {opcode:#04x} ({name}); \
                     it is probably not in MPSSE mode"
                )));
            }
            out.extend_from_slice(&data);
        }
        out.truncate(wanted);
        Ok(out)
    }

    /// The vendor control requests that take the channel from serial
    /// port to shift engine (FTDI AN_135 §4.1), then the handshake that
    /// proves the MPSSE is listening.
    fn enter_mpsse(&self) -> Result<(), ProgramError> {
        let index = channel_index(self.interface);
        self.control(SIO_RESET, 0, index, "resetting the channel")?;
        self.control(SIO_RESET, 1, index, "purging the receive buffer")?;
        self.control(SIO_RESET, 2, index, "purging the transmit buffer")?;
        self.control(SIO_SET_FLOW_CTRL, 0, index, "disabling flow control")?;
        self.control(SIO_SET_LATENCY_TIMER, 2, index, "setting the latency timer")?;
        self.control(
            SIO_SET_BITMODE,
            BITMODE_RESET,
            index,
            "leaving bit-bang mode",
        )?;
        self.control(
            SIO_SET_BITMODE,
            BITMODE_MPSSE << 8,
            index,
            "entering MPSSE mode",
        )?;
        // AN_135 §4.2 asks for a short settling time after the mode
        // change before the first command.
        std::thread::sleep(Duration::from_millis(50));
        self.synchronise()
    }

    /// Sends a deliberately invalid opcode and checks that the MPSSE
    /// complains about it, which is AN_135 §2.2's way of confirming the
    /// engine is running and the command stream is aligned.
    fn synchronise(&self) -> Result<(), ProgramError> {
        const BOGUS: u8 = 0xAA;
        self.handle
            .bulk_write(self.out_endpoint, &[BOGUS], TRANSFER_TIMEOUT)
            .map_err(|e| usb_err("sending the MPSSE synchronisation byte", &e))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut raw = vec![0u8; self.packet_size.max(64) * 2];
        while Instant::now() < deadline {
            let n =
                match self
                    .handle
                    .bulk_read(self.in_endpoint, &mut raw, Duration::from_millis(200))
                {
                    Ok(n) => n,
                    Err(e) if e.is_timeout() => continue,
                    Err(e) => return Err(usb_err("reading the MPSSE's answer", &e)),
                };
            let data = ftdi::strip_status(&raw[..n], self.packet_size);
            if data.windows(2).any(|w| w == [ftdi::BAD_COMMAND, BOGUS]) {
                return Ok(());
            }
        }
        Err(ProgramError::Usb(
            "the adapter did not answer the MPSSE synchronisation byte; \
             interface 0 may not be the JTAG channel, or another program may hold it"
                .to_owned(),
        ))
    }

    /// One FTDI vendor control request with no data stage.
    fn control(&self, request: u8, value: u16, index: u16, what: &str) -> Result<(), ProgramError> {
        self.handle
            .control_write(REQ_TYPE_OUT, request, value, index, &[], TRANSFER_TIMEOUT)
            .map(|_| ())
            .map_err(|e| usb_err(what, &e))
    }
}

impl Drop for Cable {
    fn drop(&mut self) {
        // Leave the channel as a plain FTDI port again rather than a
        // half-configured shift engine. Failures here are not worth
        // reporting: the device may already be unplugged, and a power
        // cycle undoes everything either way.
        let index = channel_index(self.interface);
        let _ = self.handle.control_write(
            REQ_TYPE_OUT,
            SIO_SET_BITMODE,
            BITMODE_RESET,
            index,
            &[],
            TRANSFER_TIMEOUT,
        );
        let _ = self.handle.release_interface(self.interface);
    }
}

// ---------------------------------------------------------------------
// The Apollo debugger on a Cynthion
// ---------------------------------------------------------------------

/// How long any one Apollo control transfer may take. They are all tiny
/// — the largest data stage in the protocol is 256 bytes — so a long
/// timeout only delays a clear error.
const APOLLO_TIMEOUT: Duration = Duration::from_secs(2);

/// How long to wait for the board to come back as the debugger after the
/// gateware has been asked to give up the USB port. It has to
/// re-enumerate, which on a loaded bus is not instant.
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(8);

/// How often to look for it while waiting.
const HANDOVER_POLL: Duration = Duration::from_millis(100);

/// A Cynthion, opened as its Apollo debug microcontroller.
///
/// What comes back from [`Debugger::open`] is a board in debugger mode
/// with its JTAG pins **not** taken: [`Debugger::run_plan`] takes them
/// for the length of one plan and releases them again, which is the
/// smallest window the protocol allows and leaves the board in the state
/// it was found in.
///
/// Nothing this type sends is persistent. It does not reconfigure the
/// FPGA, hold it offline, or touch either flash; `docs/apollo-protocol.md`
/// §7 lists the requests that would and says why they are not here.
pub struct Debugger {
    handle: DeviceHandle,
    capability: apollo::Capability,
    /// Whether `capability` is the firmware's own answer or the fallback.
    reported: bool,
    serial: String,
    /// The gateware device this was reached through, if a handover was
    /// needed, so [`Debugger::release_usb_to_fpga`] knows there is
    /// something to put back.
    handed_over: bool,
}

/// The serial numbers of every attached Cynthion, in either mode, with
/// which mode it is in.
///
/// # Errors
///
/// [`ProgramError::Usb`] when the USB subsystem cannot be reached. A
/// device that cannot be opened is listed without its serial number
/// rather than failing the listing.
pub fn list_cynthions() -> Result<Vec<(String, bool)>, ProgramError> {
    Ok(serials_of(&[
        (apollo::VENDOR_ID, apollo::GATEWARE_PRODUCT_ID),
        (apollo::VENDOR_ID, apollo::DEBUGGER_PRODUCT_ID),
    ])?
    .into_iter()
    .map(|(product, serial)| (serial, product == apollo::DEBUGGER_PRODUCT_ID))
    .collect())
}

/// How a debugger is picked out of the ones on the bus.
enum Pick<'a> {
    /// The one whose serial number is this, or any if `None`.
    Serial(Option<&'a str>),
    /// Any whose serial number is not in this list — which is how a
    /// board that has just re-enumerated is recognised.
    ///
    /// It has to be done this way because **the two modes report
    /// different serial numbers**: in gateware mode the board advertises
    /// its flash UID, and as a debugger it reports the microcontroller's
    /// own string, which is neither the flash UID nor derived from it.
    /// Asking for the gateware's serial again after the handover would
    /// never match, and the flash UID cannot be read back from a debugger
    /// without forcing the FPGA offline and driving the board's
    /// configuration flash — `docs/apollo-protocol.md` §7's line.
    ///
    /// So this is a **heuristic**, and worth naming as one: it identifies
    /// the board by the fact that a debugger appeared where there was
    /// none. That is sound for one handover at a time and would pick the
    /// wrong board if two were handed over at once. Nothing here does
    /// that — [`hand_over`] is given a serial number when the caller
    /// named one — but a machine with several Cynthions on it is not a
    /// hypothetical, so the limit is written down rather than assumed
    /// away.
    NotAlreadyThere(&'a [String]),
}

/// Opens a debugger on the bus, if one matches.
fn find_debugger(pick: &Pick<'_>) -> Result<Option<(DeviceHandle, String)>, ProgramError> {
    let context = Context::new().map_err(|e| usb_err("opening the USB subsystem", &e))?;
    let devices = context
        .devices()
        .map_err(|e| usb_err("listing USB devices", &e))?;
    for device in devices {
        if device.vendor_id() != apollo::VENDOR_ID
            || device.product_id() != apollo::DEBUGGER_PRODUCT_ID
        {
            continue;
        }
        // Decided before anything is opened, so a board that was not
        // asked for is never touched. `Pick::Serial(None)` is "any
        // debugger", which is the only case that opens without matching a
        // name.
        let this = serial_of(&device).unwrap_or_default();
        let wanted = match pick {
            Pick::Serial(serial) => serial.is_none_or(|wanted| wanted == this),
            Pick::NotAlreadyThere(before) => !before.contains(&this),
        };
        if !wanted {
            continue;
        }
        let Ok(handle) = device.open() else { continue };
        return Ok(Some((handle, this)));
    }
    Ok(None)
}

/// The serial numbers of every debugger currently on the bus.
fn debugger_serials() -> Result<Vec<String>, ProgramError> {
    Ok(list_cynthions()?
        .into_iter()
        .filter_map(|(serial, debugger)| debugger.then_some(serial))
        .collect())
}

/// Asks every attached Cynthion gateware to give the USB port back to
/// its microcontroller, and says how many were asked.
///
/// The request goes to the *Apollo stub interface*, which is found by
/// its descriptor — vendor class, subclass zero, no endpoints — because
/// its number is a property of whatever gateware happens to be loaded
/// and not of the protocol.
fn hand_over(serial: Option<&str>) -> Result<usize, ProgramError> {
    let context = Context::new().map_err(|e| usb_err("opening the USB subsystem", &e))?;
    let devices = context
        .devices()
        .map_err(|e| usb_err("listing USB devices", &e))?;
    let mut asked = 0;
    for device in devices {
        if device.vendor_id() != apollo::VENDOR_ID
            || device.product_id() != apollo::GATEWARE_PRODUCT_ID
        {
            continue;
        }
        // A gateware advertises its flash UID as its serial number, so
        // this is the one place a `--device` value that names a *board*
        // matches a board. Read before opening: a Cynthion running
        // somebody's gateware that was not asked for is left alone.
        let this = serial_of(&device).unwrap_or_default();
        if serial.is_some_and(|wanted| wanted != this) {
            continue;
        }
        let Ok(handle) = device.open() else { continue };
        let Ok(config) = device.active_config_descriptor() else {
            continue;
        };
        let stub = config.interfaces.iter().find_map(|interface| {
            let alt = interface.first();
            (alt.class == apollo::VENDOR_SPECIFIC_CLASS
                && alt.sub_class == apollo::STUB_SUBCLASS
                && alt.endpoints.is_empty())
            .then_some(alt.number)
        });
        let Some(stub) = stub else { continue };

        // A control request whose recipient is an interface can be
        // refused while a driver holds that interface, so take it away
        // first if anything did bind. Nothing does on Linux today.
        if handle.kernel_driver_active(stub).unwrap_or(false) {
            let _ = handle.detach_kernel_driver(stub);
        }
        // The device answers the status stage and then drops off the
        // bus, so a failure here is as likely to mean "it worked and
        // left" as "it refused". Whether it worked is decided by
        // whether the debugger turns up, not by this return value.
        let _ = handle.control_write(
            apollo::REQ_TYPE_OUT_INTERFACE,
            apollo::REQUEST_ADVERTISEMENT_STOP,
            0,
            u16::from(stub),
            &[],
            APOLLO_TIMEOUT,
        );
        asked += 1;
    }
    Ok(asked)
}

impl Debugger {
    /// Opens a Cynthion's Apollo debugger, asking its gateware to give
    /// up the USB port first if that is what it takes.
    ///
    /// If the board is already in debugger mode this is one enumeration
    /// and nothing is sent. Otherwise the handover of
    /// `docs/apollo-protocol.md` §2 is performed and the board is waited
    /// for while it re-enumerates. Neither is persistent: the board
    /// comes back on a replug, a power cycle, or
    /// [`Debugger::release_usb_to_fpga`].
    ///
    /// `serial` picks one board when several are attached. It is matched
    /// against **whichever mode the board is in now**: in gateware mode
    /// the board advertises its flash UID as its USB serial number, and
    /// as a debugger it reports the microcontroller's own string. They
    /// are not the same string, so a board that has to be handed over is
    /// followed across the re-enumeration by being the debugger that was
    /// not there before rather than by its name. `Pick`, private just
    /// below, says what that heuristic does and does not cover.
    ///
    /// # Errors
    ///
    /// [`ProgramError::NoCynthion`] when none is attached, and
    /// [`ProgramError::Usb`] when one is but never comes back as a
    /// debugger — which is the shape a
    /// gateware with no Apollo stub takes, and the message says so,
    /// because the only route left is holding the board's PROGRAM button
    /// while plugging it in and no program can do that.
    pub fn open(serial: Option<&str>) -> Result<Debugger, ProgramError> {
        if let Some((handle, found)) = find_debugger(&Pick::Serial(serial))? {
            return Debugger::from_handle(handle, found, false);
        }

        let before = debugger_serials()?;
        let asked = hand_over(serial)?;
        if asked == 0 {
            return Err(ProgramError::NoCynthion {
                wanted: serial.map(str::to_owned),
                found: list_cynthions()?,
            });
        }

        let deadline = Instant::now() + HANDOVER_TIMEOUT;
        loop {
            if let Some((handle, found)) = find_debugger(&Pick::NotAlreadyThere(&before))? {
                return Debugger::from_handle(handle, found, true);
            }
            if Instant::now() >= deadline {
                return Err(ProgramError::Usb(format!(
                    "a Cynthion gateware was asked to give up the USB port \
                     (vendor request {:#04x} to its Apollo stub interface) but no \
                     debugger {:#06x}:{:#06x} appeared within {} s; if this gateware \
                     has no Apollo stub the only way in is to hold the board's \
                     PROGRAM button while plugging it in, and nothing here can do that",
                    apollo::REQUEST_ADVERTISEMENT_STOP,
                    apollo::VENDOR_ID,
                    apollo::DEBUGGER_PRODUCT_ID,
                    HANDOVER_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(HANDOVER_POLL);
        }
    }

    fn from_handle(
        handle: DeviceHandle,
        serial: String,
        handed_over: bool,
    ) -> Result<Debugger, ProgramError> {
        // **Most firmware stalls this**, including the v1.1.1 on the board
        // this was written against: `REQUEST_JTAG_GET_INFO` is defined by
        // Apollo's *host* package and has no case in the firmware's
        // request dispatcher at all. So the fallback is the usual path and
        // not the exception, and which of the two answered is recorded,
        // because the fallback's numbers are `DEFAULT_MAX_SCAN_BITS` and
        // no quirks — indistinguishable, in a printed line, from a
        // firmware that reported exactly that.
        let mut reply = [0u8; 8];
        let (capability, reported) = match handle.control_read(
            apollo::REQ_TYPE_IN,
            apollo::REQUEST_JTAG_GET_INFO,
            0,
            0,
            &mut reply,
            APOLLO_TIMEOUT,
        ) {
            Ok(n) => match apollo::Capability::from_reply(&reply[..n]) {
                Some(capability) => (capability, true),
                None => (apollo::Capability::default(), false),
            },
            Err(_) => (apollo::Capability::default(), false),
        };
        Ok(Debugger {
            handle,
            capability,
            reported,
            serial,
            handed_over,
        })
    }

    /// The board's serial number.
    #[must_use]
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Whether the gateware had to be asked to give up the USB port.
    #[must_use]
    pub fn handed_over(&self) -> bool {
        self.handed_over
    }

    /// What the firmware can do, which is usually what
    /// [`apollo::Capability::default`] assumes rather than what the
    /// firmware said: see [`Debugger::capability_reported`].
    #[must_use]
    pub fn capability(&self) -> apollo::Capability {
        self.capability
    }

    /// Whether [`Debugger::capability`] is the firmware's own answer to
    /// [`apollo::REQUEST_JTAG_GET_INFO`], rather than the fallback.
    ///
    /// It is worth reporting rather than swallowing. The fallback is
    /// [`apollo::DEFAULT_MAX_SCAN_BITS`] and no quirks, which is exactly
    /// what a firmware that *did* answer would most likely say, so a
    /// printed line cannot be told from a measurement unless this is
    /// printed with it. On the board this was written against — a
    /// Cynthion r1.4 running Apollo v1.1.1 — it is `false`: the request is
    /// the host package's and the firmware has no case for it.
    #[must_use]
    pub fn capability_reported(&self) -> bool {
        self.reported
    }

    /// The firmware's own name for itself. It contains `Apollo`.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] when the request fails.
    pub fn id(&self) -> Result<String, ProgramError> {
        self.string(apollo::REQUEST_GET_ID, "reading the firmware's identifier")
    }

    /// The firmware version, as the firmware writes it.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] when the request fails.
    pub fn firmware_version(&self) -> Result<String, ProgramError> {
        self.string(
            apollo::REQUEST_GET_FIRMWARE_VERSION,
            "reading the firmware version",
        )
    }

    /// The USB API version, major then minor. This is the one to test a
    /// capability against; the firmware version is for people.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] when the request fails.
    pub fn usb_api_version(&self) -> Result<(u8, u8), ProgramError> {
        let mut buf = [0u8; 2];
        let n = self
            .handle
            .control_read(
                apollo::REQ_TYPE_IN,
                apollo::REQUEST_GET_USB_API_VERSION,
                0,
                0,
                &mut buf,
                APOLLO_TIMEOUT,
            )
            .map_err(|e| usb_err("reading the USB API version", &e))?;
        if n < 2 {
            return Err(ProgramError::Usb(format!(
                "the USB API version came back as {n} byte(s), not 2"
            )));
        }
        Ok((buf[0], buf[1]))
    }

    /// The TAP state the firmware believes it is in, or `None` for a
    /// number outside the sixteen.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] when the request fails.
    pub fn tap_state(&self) -> Result<Option<TapState>, ProgramError> {
        let mut buf = [0u8; 1];
        let n = self
            .handle
            .control_read(
                apollo::REQ_TYPE_IN,
                apollo::REQUEST_JTAG_GET_STATE,
                0,
                0,
                &mut buf,
                APOLLO_TIMEOUT,
            )
            .map_err(|e| usb_err("reading the TAP state", &e))?;
        if n < 1 {
            return Err(ProgramError::Usb(
                "the TAP state came back empty".to_owned(),
            ));
        }
        Ok(apollo::state_from_number(buf[0]))
    }

    /// Compiles a [`super::jtag::Plan`] for this firmware, takes the
    /// JTAG pins, runs it, and releases them again.
    ///
    /// What comes back is the compiled program — which is what knows
    /// where the captures are — and one reply per read, in order.
    /// [`apollo::Program::capture_u32`] turns those into a register.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] for a transfer the device refused.
    pub fn run_plan(
        &self,
        plan: &super::jtag::Plan,
    ) -> Result<(apollo::Program, Vec<Vec<u8>>), ProgramError> {
        self.run_plan_with(plan, self.capability)
    }

    /// [`run_plan`](Self::run_plan) with a capability of the caller's
    /// choosing rather than the one the firmware reported.
    ///
    /// The only honest use for this is making a device *do less* than it
    /// said it could, so that a path which would otherwise never run on
    /// hardware does. Splitting a register across several scans is the
    /// one that matters: a firmware that reports 2048 bits never splits
    /// a 32-bit read, so the chunking — and with it the claim that a
    /// scan without [`apollo::FLAG_ADVANCE_STATE`] stays in the shift
    /// state — would only ever be exercised against a model. Asking for
    /// 16-bit chunks makes a real part prove it.
    ///
    /// Claiming a *larger* capability than the firmware reported is a
    /// way to have scans silently truncated, and nothing in this crate
    /// does it.
    ///
    /// # Errors
    ///
    /// As [`run_plan`](Self::run_plan).
    pub fn run_plan_with(
        &self,
        plan: &super::jtag::Plan,
        capability: apollo::Capability,
    ) -> Result<(apollo::Program, Vec<Vec<u8>>), ProgramError> {
        let program = apollo::compile(plan, capability);
        self.command(apollo::REQUEST_JTAG_START, 0, 0, "taking the JTAG pins")?;
        let replies = self.run(&program);
        // Release the pins whatever happened; a failed scan must not
        // leave them driven.
        let _ = self.command(apollo::REQUEST_JTAG_STOP, 0, 0, "releasing the JTAG pins");
        Ok((program, replies?))
    }

    /// Runs an already-compiled program, without taking or releasing the
    /// JTAG pins.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] for a transfer the device refused.
    pub fn run(&self, program: &apollo::Program) -> Result<Vec<Vec<u8>>, ProgramError> {
        self.run_with_progress(program, &mut |_, _| {})
    }

    /// [`run`](Self::run), reporting how much of a long write has gone.
    ///
    /// `progress` is called with `(bytes written, bytes total)` after each
    /// write, where a byte is one of the data stage of a
    /// `JTAG_SET_OUT_BUFFER`. It exists for the one program whose size is
    /// a design's rather than a protocol's — a bitstream burst — which on
    /// a full-speed link takes long enough that silence looks like a hang.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run).
    pub fn run_with_progress(
        &self,
        program: &apollo::Program,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Vec<Vec<u8>>, ProgramError> {
        let total: usize = program
            .steps()
            .iter()
            .map(|step| match step {
                apollo::Step::Write { data, .. } => data.len(),
                _ => 0,
            })
            .sum();
        let mut written = 0usize;
        let mut replies = Vec::new();
        for step in program.steps() {
            match step {
                apollo::Step::Command {
                    request,
                    value,
                    index,
                } => self.command(*request, *value, *index, "a JTAG request")?,
                apollo::Step::Write {
                    request,
                    value,
                    index,
                    data,
                } => {
                    self.handle
                        .control_write(
                            apollo::REQ_TYPE_OUT,
                            *request,
                            *value,
                            *index,
                            data,
                            APOLLO_TIMEOUT,
                        )
                        .map(|_| ())
                        .map_err(|e| {
                            usb_err(&format!("filling the JTAG out buffer ({request:#04x})"), &e)
                        })?;
                    written += data.len();
                    progress(written, total);
                }
                apollo::Step::Read {
                    request,
                    value,
                    index,
                    len,
                    ..
                } => {
                    let mut buf = vec![0u8; *len];
                    let n = self
                        .handle
                        .control_read(
                            apollo::REQ_TYPE_IN,
                            *request,
                            *value,
                            *index,
                            &mut buf,
                            APOLLO_TIMEOUT,
                        )
                        .map_err(|e| {
                            usb_err(&format!("reading the JTAG in buffer ({request:#04x})"), &e)
                        })?;
                    buf.truncate(n);
                    replies.push(buf);
                }
            }
        }
        Ok(replies)
    }

    /// Reads the 32-bit identifier of whatever part is on the chain,
    /// with no instruction shifted and nothing written.
    ///
    /// This is [`super::jtag::idcode_plan`] — five TMS-high clocks' worth
    /// of `Test-Logic-Reset` and then thirty-two bits out of DR. It asks
    /// nothing about the vendor and needs no instruction register width,
    /// which is what makes it the right thing to point at an unknown
    /// board.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] for a transfer the device refused, and
    /// [`ProgramError::Job`] when the reply is short.
    pub fn idcode(&self) -> Result<u32, ProgramError> {
        let (program, replies) = self.run_plan(&super::jtag::idcode_plan())?;
        Ok(program.capture_u32(0, &replies)?)
    }

    /// Tells Apollo it may give the USB port back to the FPGA.
    ///
    /// **This is not on its own enough to bring the gateware back, and
    /// on the board this was written against it did nothing visible.**
    /// The request is accepted, but the port only moves when the FPGA
    /// asks for it, and a gateware that has been told to stop
    /// advertising does not start again until the part is reconfigured
    /// or the board is power cycled. Apollo's own tooling pairs this
    /// with a reconfiguration; Reticle does not send that request (see
    /// `docs/apollo-protocol.md` §7), so the honest end of a session is
    /// this request and then a note to the person holding the board that
    /// a replug restores it.
    ///
    /// Nothing is lost by the board staying in debugger mode: the FPGA
    /// is still configured with whatever it was configured with, and a
    /// replug or a power cycle brings it back. It is a USB port's owner,
    /// not a state of the part.
    ///
    /// The handle is consumed because the debugger may leave the bus.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] when the request was refused. A device that
    /// has already left cannot answer, and that is reported as success,
    /// because it is the outcome that was asked for.
    pub fn release_usb_to_fpga(self) -> Result<(), ProgramError> {
        match self.handle.control_write(
            apollo::REQ_TYPE_OUT,
            apollo::REQUEST_ALLOW_FPGA_TAKEOVER_USB,
            0,
            0,
            &[],
            APOLLO_TIMEOUT,
        ) {
            Ok(_) => Ok(()),
            // A device that has gone is a device that did what was
            // asked; anything else is worth reporting.
            Err(e) if e.is_timeout() => Ok(()),
            Err(e) => Err(usb_err("handing the USB port back to the FPGA", &e)),
        }
    }

    /// One vendor request with no data stage.
    fn command(&self, request: u8, value: u16, index: u16, what: &str) -> Result<(), ProgramError> {
        self.handle
            .control_write(
                apollo::REQ_TYPE_OUT,
                request,
                value,
                index,
                &[],
                APOLLO_TIMEOUT,
            )
            .map(|_| ())
            .map_err(|e| usb_err(&format!("{what} ({request:#04x})"), &e))
    }

    /// Takes the JTAG pins for as long as the returned guard lives, so
    /// several plans can be run as **one session**.
    ///
    /// [`Debugger::run_plan`] takes the pins and gives them back around
    /// each plan, which is right for a read and wrong for a
    /// configuration sequence: a part that has been put into
    /// configuration mode has to stay there across the steps, and
    /// `REQUEST_JTAG_STOP` does not just stop driving — Apollo's firmware
    /// restores the pins to their other function, which is a UART on this
    /// board. Whatever that does to TCK, it is not "nothing", and a TAP
    /// that is clocked between two steps of a sequence is a TAP in an
    /// unknown state.
    ///
    /// So the sequence holds the pins from the first instruction to the
    /// last, and the guard's `Drop` gives them back however it ends.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] when the device will not give up the pins.
    pub fn jtag_session(&self) -> Result<JtagSession<'_>, ProgramError> {
        self.command(apollo::REQUEST_JTAG_START, 0, 0, "taking the JTAG pins")?;
        Ok(JtagSession { debugger: self })
    }

    /// One vendor request answering with a NUL-terminated string.
    fn string(&self, request: u8, what: &str) -> Result<String, ProgramError> {
        let mut buf = [0u8; 256];
        let n = self
            .handle
            .control_read(apollo::REQ_TYPE_IN, request, 0, 0, &mut buf, APOLLO_TIMEOUT)
            .map_err(|e| usb_err(what, &e))?;
        let bytes = &buf[..n];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

/// The JTAG pins, held. [`Debugger::jtag_session`] makes one.
///
/// Every plan run through this reaches the part without the pins being
/// released in between, which is what a configuration sequence needs and
/// what [`Debugger::run_plan`] deliberately does not do.
pub struct JtagSession<'a> {
    debugger: &'a Debugger,
}

impl JtagSession<'_> {
    /// Runs one plan in this session.
    ///
    /// What comes back is the compiled program — which is what knows
    /// where the captures are — and one reply per read, in order, exactly
    /// as [`Debugger::run_plan`] returns them.
    ///
    /// # Errors
    ///
    /// [`ProgramError::Usb`] for a transfer the device refused. The pins
    /// stay taken: the caller decides whether a failed step is the end of
    /// the session, and dropping this guard is what releases them.
    pub fn run_plan(
        &self,
        plan: &super::jtag::Plan,
    ) -> Result<(apollo::Program, Vec<Vec<u8>>), ProgramError> {
        let program = apollo::compile(plan, self.debugger.capability);
        let replies = self.debugger.run(&program)?;
        Ok((program, replies))
    }

    /// Runs one plan, reporting how much of a long write has gone.
    ///
    /// The callback is given `(bytes written, bytes total)` after every
    /// write step, which for a bitstream burst is once per scan. It exists
    /// because the burst is the one plan whose size is a design's rather
    /// than a protocol's, and on a full-speed link it takes long enough
    /// that silence looks like a hang.
    ///
    /// # Errors
    ///
    /// As [`run_plan`](Self::run_plan).
    pub fn run_plan_with_progress(
        &self,
        plan: &super::jtag::Plan,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<(apollo::Program, Vec<Vec<u8>>), ProgramError> {
        let program = apollo::compile(plan, self.debugger.capability);
        let replies = self.debugger.run_with_progress(&program, progress)?;
        Ok((program, replies))
    }
}

impl Drop for JtagSession<'_> {
    fn drop(&mut self) {
        // However the session ended, the pins go back. A failure here is
        // not worth reporting: the board may already be gone, and a power
        // cycle releases them either way.
        let _ = self
            .debugger
            .command(apollo::REQUEST_JTAG_STOP, 0, 0, "releasing the JTAG pins");
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        // Release the JTAG pins if a run left them taken. Failures are
        // not worth reporting: the board may already be gone, and the
        // pins are released by a power cycle either way.
        let _ = self.handle.control_write(
            apollo::REQ_TYPE_OUT,
            apollo::REQUEST_JTAG_STOP,
            0,
            0,
            &[],
            APOLLO_TIMEOUT,
        );
    }
}

/// The bulk endpoints of the MPSSE interface, and its packet size.
fn endpoints(handle: &DeviceHandle) -> Result<(u8, u8, usize), ProgramError> {
    let config = handle
        .device()
        .active_config_descriptor()
        .map_err(|e| usb_err("reading the adapter's configuration descriptor", &e))?;
    let interface = config.interface(MPSSE_INTERFACE).ok_or_else(|| {
        ProgramError::Usb(format!(
            "the adapter has no interface {MPSSE_INTERFACE}; it is not an FT2232H"
        ))
    })?;
    let mut in_endpoint = None;
    let mut out_endpoint = None;
    let mut packet_size = ftdi::PACKET_SIZE;
    for endpoint in &interface.first().endpoints {
        match Direction::from_address(endpoint.address) {
            Direction::In if in_endpoint.is_none() => {
                in_endpoint = Some(endpoint.address);
                packet_size = usize::from(endpoint.max_packet_size).max(64);
            }
            Direction::Out if out_endpoint.is_none() => out_endpoint = Some(endpoint.address),
            _ => {}
        }
    }
    match (in_endpoint, out_endpoint) {
        (Some(i), Some(o)) => Ok((i, o, packet_size)),
        _ => Err(ProgramError::Usb(
            "the adapter's JTAG interface has no pair of bulk endpoints".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FTDI counts channels from one on the control endpoint while USB
    /// counts interfaces from zero; sending interface 0 as index 0 is a
    /// request the part quietly ignores.
    #[test]
    fn the_channel_index_is_one_based() {
        assert_eq!(channel_index(0), 1);
        assert_eq!(channel_index(1), 2);
    }

    /// The bit-mode request packs the mode into the high byte, which is
    /// why `enter_mpsse` shifts it and the reset value does not need to.
    #[test]
    fn the_bitmode_request_puts_the_mode_high() {
        assert_eq!(BITMODE_MPSSE << 8, 0x0200);
        assert_eq!(BITMODE_RESET, 0x0000);
        assert_eq!(REQ_TYPE_OUT & 0x80, 0, "a vendor write is host to device");
    }
}
