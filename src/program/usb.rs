//! The USB transport: opening an FTDI adapter and moving [`Job`] bytes
//! across it.
//!
//! This is the only file in Reticle that touches a device, and it is
//! deliberately the shallowest: it opens, detaches, claims, writes and
//! reads. Every decision about *what* to write is made by [`super::ftdi`],
//! [`super::jtag`] and [`super::xilinx`], which know nothing about USB.
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

use super::jtag::Job;
use super::{FT2232H_PRODUCT_ID, FTDI_VENDOR_ID, MPSSE_INTERFACE, ProgramError, ftdi};

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
}

/// Turns a `rawusb` error into ours, with the operation that failed.
fn usb_err(what: &str, err: &rawusb::Error) -> ProgramError {
    ProgramError::Usb(format!("{what}: {err}"))
}

/// The serial numbers of every attached FT2232H, in the order the system
/// lists them.
///
/// # Errors
///
/// [`ProgramError::Usb`] when the USB subsystem cannot be reached at
/// all. A device that cannot be opened (no permission, already in use)
/// is skipped rather than failing the listing.
pub fn list() -> Result<Vec<String>, ProgramError> {
    let context = Context::new().map_err(|e| usb_err("opening the USB subsystem", &e))?;
    let devices = context
        .devices()
        .map_err(|e| usb_err("listing USB devices", &e))?;
    let mut serials = Vec::new();
    for device in devices {
        if device.vendor_id() != FTDI_VENDOR_ID || device.product_id() != FT2232H_PRODUCT_ID {
            continue;
        }
        let Ok(handle) = device.open() else { continue };
        match handle.read_serial_number_string() {
            Ok(Some(serial)) => serials.push(serial),
            _ => serials.push(format!(
                "bus {} address {} (no serial number)",
                device.bus_number(),
                device.address()
            )),
        }
    }
    Ok(serials)
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
                candidates.push((handle, this));
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
                candidates.into_iter().map(|(_, s)| s).collect(),
            ));
        }
        let (handle, serial) = candidates.pop().expect("exactly one candidate");

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
        };
        cable.enter_mpsse()?;
        Ok(cable)
    }

    /// The adapter's serial number.
    #[must_use]
    pub fn serial(&self) -> &str {
        &self.serial
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
