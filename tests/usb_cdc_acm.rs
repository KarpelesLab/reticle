//! A serial port this compiler built, driven through the **operating
//! system's own driver**.
//!
//! `tests/ip_library.rs` proves `ip/usb/usb_cdc_acm` against a host model and a
//! transceiver model, in simulation, and that is where the interesting
//! assertions are — the class hook claiming a request and stalling the ones it
//! did not offer, the line coding read back byte for byte, the notification
//! endpoint NAKing twenty polls in a row. What none of it can prove is the
//! only question a class layer really has to answer: **does the host's driver
//! bind?** A descriptor set can be exactly what CDC 1.1 and PSTN 1.2 describe
//! and still not be what `cdc_acm` looks for, and no model of a host written
//! from the same specifications as the device can tell you that.
//!
//! So this test asks the kernel:
//!
//! ```console
//! cargo test --features program --test usb_cdc_acm -- --ignored --nocapture
//! ```
//!
//! It is in three parts, and they fail for different reasons.
//!
//! 1. **The driver bound.** A `/dev/ttyACM*` exists whose USB parent is
//!    `1209:0001`, and the descriptors the part reports are the ones the
//!    sources describe. No byte is moved, nothing is opened, and the whole
//!    assertion is read out of sysfs and endpoint 0 — so this half says
//!    exactly one thing: the kernel looked at this device's descriptors and
//!    decided it was a serial port. That is the thing simulation cannot
//!    reach.
//! 2. **Bytes went round.** The port is configured with `stty` and written
//!    to, and the same bytes are read back. With
//!    `testdata/fpga/cynthion/usb_cdc_uart.v` loaded, what happens in between
//!    is that each byte is serialised at 115200 baud by `ip/bus/uart`'s
//!    transmitter, recovered by its receiver, and handed back to the IN
//!    endpoint — so a byte that comes back proves the whole bridge and not
//!    only the USB half.
//! 3. **The device sent a SERIAL_STATE notification** in answer to a host
//!    opening the port. SET_CONTROL_LINE_STATE goes out over usbfs — which is
//!    what `cdc_acm` issues at every `open` — and the ten bytes of PSTN 1.2
//!    §6.5.4 are read off endpoint `82h` and checked field by field. Asking
//!    first is what makes it deterministic, and it is also the assertion whose
//!    absence let a one-notification-per-configuration device through: see
//!    `serial_state_notification`. This is the half a host can be asked about
//!    without an ioctl; `TIOCMGET` reads the same `wSerialState` out of
//!    `cdc_acm`'s own `ctrlin` and needs `libc` and `unsafe`, which this crate
//!    does not have, so `ip/usb/usb_cdc_acm/README.md` §5 has that reading across
//!    three opens, taken by hand.
//!
//!    **A blocking `open` is not the observation it looks like.**
//!    `cdc_acm`'s `tty_port_operations` has no `carrier_raised`, and
//!    `tty_port_carrier_raised` returns **true** when that is missing
//!    (`drivers/tty/tty_port.c`), so an `open` of a `/dev/ttyACM*` never waits
//!    for a carrier whatever `clocal` says. What `clocal` does reach on this
//!    driver is the *hangup* on a carrier that **drops**: `acm->clocal` is
//!    read in exactly one place, and it is the `tty_port_tty_hangup` in
//!    `acm_process_notification`. This file used to say that `clocal` was in
//!    the `stty` list because an open would otherwise wait for a carrier that
//!    never came, and that was wrong about this driver.
//!
//! **Why `stty` and not an ioctl.** Configuring a terminal means `tcsetattr`,
//! which means `libc` and `unsafe`, and this crate has neither. `stty` is the
//! operating system's own tool for it and shelling out to it is the honest
//! way to reach a terminal from a crate that refuses both — and it is what a
//! person does at a prompt anyway, which is the point of this class. It is
//! also why part three reads the notification off the wire rather than out of
//! the driver: there is no `stty` for `TIOCMGET`.
//!
//! **What this test would and would not catch.** It catches a descriptor set
//! the kernel refuses — a missing union functional descriptor, a
//! communications interface with no interrupt endpoint, a class or subclass
//! byte the driver does not recognise — because then no `/dev/ttyACM*`
//! appears and the first half fails naming it. It catches a class request
//! answered wrongly, because `cdc_acm` fails the open if
//! SET_CONTROL_LINE_STATE stalls. It catches the bridge losing or corrupting
//! a byte. It does **not** catch a device that works on Linux and not on
//! Windows or macOS, since it asks one host. It does **not** distinguish the
//! USB half from the UART half of a byte going missing — LED 2 and LED 4 on
//! the board do that, and `usb_cdc_uart.v`'s header says how to read them.
//! And it says nothing at all about a board that is not plugged in: it
//! **skips**, with a reason, because an unplugged board is not a broken
//! compiler.

#![cfg(feature = "program")]

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rawusb::Context;

/// pid.codes' test pair, which is `ip/usb/usb_cdc_acm`'s default `VID` / `PID`.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;

/// Long enough that a device answering at all answers inside it, short enough
/// that a device answering nothing does not hold the run up. It is a timeout
/// and **not an assertion**: nothing here is checked against a clock.
const TIMEOUT: Duration = Duration::from_millis(500);

/// How long the round trip may take before the test gives up on it.
///
/// `usb_cdc_uart.v` carries **one byte at a time** through the UART — its
/// header says why — so a byte costs 87 microseconds of 8N1 plus a host round
/// trip, and a few dozen bytes is comfortably inside a few seconds on any
/// machine. It is a timeout and not an assertion for the same reason as
/// above: a slow machine must not fail this, only a broken device must.
const ROUND_TRIP: Duration = Duration::from_secs(30);

/// The ten bytes of a SERIAL_STATE notification, and the bitmap this board's
/// design reports in them.
///
/// `testdata/fpga/cynthion/usb_cdc_uart.v` ties `serial_state` to
/// `7'b000_0011`: `bRxCarrier` and `bTxCarrier` set, every error bit clear.
/// PSTN 1.2 §6.5.4 makes bit 0 DCD and bit 1 DSR, and a port whose far end is
/// inside the same die has both.
const SERIAL_STATE_BYTES: usize = 10;
const SERIAL_STATE: u16 = 0x0003;

/// The interface numbers and endpoint addresses `ip/usb/usb_cdc_acm` declares.
const COMM_IFACE: u8 = 0;
const DATA_IFACE: u8 = 1;
const NOTIF_EP: u8 = 0x82;
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;

/// `wMaxPacketSize` of the bulk pair and of the notification endpoint.
///
/// 64 is the largest of the four sizes USB 2.0 §5.8.3 allows a full-speed bulk
/// endpoint; 16 is the smallest power of two that holds the ten bytes of a
/// SERIAL_STATE notification, and §5.7.3 allows an interrupt endpoint any size
/// up to 64. Both were 8, which is why the notification could not be sent.
const MAX_PACKET: u8 = 64;
const NOTIF_MAXPKT: u8 = 16;

/// The configuration descriptor `ip/usb/usb_cdc_acm` describes, written forwards
/// from CDC 1.1 and PSTN 1.2 — the arithmetic and not the answers, as
/// `tests/ip_library.rs` writes the same bytes and as `ip/usb/usb_cdc_acm/README.md`
/// tabulates them field by field.
fn expected_configuration() -> Vec<u8> {
    let mut iface: Vec<u8> = Vec::new();
    // INTERFACE 0: communications, Abstract Control Model, no class protocol.
    iface.extend_from_slice(&[9, 4, COMM_IFACE, 0, 0, 0x02, 0x02, 0x00, 0]);
    // Header functional descriptor, bcdCDC 1.10.
    iface.extend_from_slice(&[5, 0x24, 0x00, 0x10, 0x01]);
    // Call Management functional descriptor: none, over interface 1.
    iface.extend_from_slice(&[5, 0x24, 0x01, 0x00, DATA_IFACE]);
    // Abstract Control Management functional descriptor: D1 alone.
    iface.extend_from_slice(&[4, 0x24, 0x02, 0x02]);
    // Union functional descriptor: interface 1 under interface 0.
    iface.extend_from_slice(&[5, 0x24, 0x06, COMM_IFACE, DATA_IFACE]);
    // ENDPOINT 82h: interrupt IN, sixteen bytes — room for the ten of a
    // SERIAL_STATE — every 16 frames.
    iface.extend_from_slice(&[7, 5, NOTIF_EP, 0x03, NOTIF_MAXPKT, 0, 16]);
    // INTERFACE 1: CDC Data.
    iface.extend_from_slice(&[9, 4, DATA_IFACE, 0, 0, 0x0A, 0x00, 0x00, 0]);
    // ENDPOINT 01h and 81h: bulk, 64 bytes.
    iface.extend_from_slice(&[7, 5, EP_OUT, 0x02, MAX_PACKET, 0, 0]);
    iface.extend_from_slice(&[7, 5, EP_IN, 0x02, MAX_PACKET, 0, 0]);

    // bNumEndpoints of each interface: the ENDPOINT descriptors after it and
    // before the next INTERFACE, walked along the chain of bLength fields the
    // way a host does. The functional descriptors are stepped over, since
    // their bDescriptorType is 24h.
    let mut at = 0;
    while at + 1 < iface.len() {
        let len = usize::from(iface[at]);
        if iface[at + 1] == 4 {
            let mut n = 0u8;
            let mut k = at + len;
            while k + 1 < iface.len() && iface[k + 1] != 4 {
                if iface[k + 1] == 5 {
                    n += 1;
                }
                k += usize::from(iface[k]);
            }
            iface[at + 4] = n;
        }
        at += len;
    }

    let total = u16::try_from(9 + iface.len()).expect("a short descriptor");
    let [lo, hi] = total.to_le_bytes();
    let mut want = vec![9, 2, lo, hi, count(&iface, 4), 1, 0, 0x80, 50];
    want.extend(iface);
    want
}

/// Descriptors of one `bDescriptorType` in a run of them.
fn count(blob: &[u8], kind: u8) -> u8 {
    let mut at = 0;
    let mut n = 0u8;
    while at + 1 < blob.len() && blob[at] != 0 {
        if blob[at + 1] == kind {
            n += 1;
        }
        at += usize::from(blob[at]);
    }
    n
}

/// The terminal device the kernel gave this USB device, found by **asking the
/// kernel** rather than by guessing a number.
///
/// The number is not fixed and cannot be: a Cynthion's own Apollo debugger is
/// itself a CDC ACM device and is usually `ttyACM0`. What is fixed is the
/// relation — `/sys/class/tty/ttyACMn/device` is the USB *interface* the port
/// belongs to and its parent is the USB device — so this walks that and reads
/// the vendor and product identifiers off it.
///
/// Returns every match, because finding two would mean two boards are plugged
/// in and the test should say so rather than pick one.
fn tty_ports(vid: u16, pid: u16) -> Vec<PathBuf> {
    let want = (format!("{vid:04x}"), format!("{pid:04x}"));
    let Ok(entries) = fs::read_dir("/sys/class/tty") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("ttyACM") {
            continue;
        }
        // `device` is the interface; its parent is the device.
        let device = entry.path().join("device");
        let read = |field: &str| {
            fs::read_to_string(device.join("..").join(field))
                .ok()
                .map(|s| s.trim().to_ascii_lowercase())
        };
        if read("idVendor").as_deref() == Some(want.0.as_str())
            && read("idProduct").as_deref() == Some(want.1.as_str())
        {
            found.push(PathBuf::from("/dev").join(&name));
        }
    }
    found.sort();
    found
}

/// Which driver the kernel bound to one interface of the device, as sysfs
/// reports it: the basename of `.../driver`.
fn interface_driver(port: &std::path::Path) -> Option<String> {
    let name = port.file_name()?.to_string_lossy().into_owned();
    let driver = PathBuf::from("/sys/class/tty")
        .join(name)
        .join("device/driver");
    let target = fs::read_link(driver).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

#[test]
#[ignore = "needs a USB serial port built by this compiler attached: 1209:0001"]
fn a_serial_port_this_compiler_built_is_bound_by_the_kernels_own_driver() {
    // ------------------------------------------------------------------
    // Half one: the kernel decided this was a serial port.
    // ------------------------------------------------------------------
    let ports = tty_ports(VID, PID);
    if ports.is_empty() {
        // Say which of the two things is wrong, because they mean different
        // things: no device at all is an unplugged board, and a device with
        // no terminal is a descriptor set the kernel refused.
        match Context::new().and_then(|c| c.find_device(VID, PID)) {
            Ok(Some(_)) => panic!(
                "{VID:04x}:{PID:04x} is attached and the kernel gave it no /dev/ttyACM*: \
                 `cdc_acm` did not bind. That is a descriptor set the driver refused — the \
                 union functional descriptor, the interrupt IN endpoint on the communications \
                 interface, or the class triple. `dmesg` will say which, and \
                 ip/usb/usb_cdc_acm/README.md sections 2 and 4 say what each one is for."
            ),
            Ok(None) => {
                println!(
                    "no {VID:04x}:{PID:04x} is attached, skipping; load \
                     testdata/fpga/cynthion/usb_cdc_uart.v"
                );
                return;
            }
            Err(err) => {
                println!("cannot enumerate USB devices ({err}), skipping");
                return;
            }
        }
    }
    assert_eq!(
        ports.len(),
        1,
        "two devices claim to be {VID:04x}:{PID:04x}: {ports:?}"
    );
    let port = &ports[0];
    println!(
        "the kernel gave {VID:04x}:{PID:04x} the terminal {}",
        port.display()
    );

    // And it is `cdc_acm` that holds it, not some other driver that happens
    // to make terminals.
    match interface_driver(port) {
        Some(driver) => assert_eq!(
            driver,
            "cdc_acm",
            "{} is held by `{driver}` and not by the class driver",
            port.display()
        ),
        None => println!("  (sysfs does not name the driver for {})", port.display()),
    }

    // The descriptors the part is reporting, read off the part rather than
    // off the sources. `wTotalLength` first, then the whole thing at that
    // length, which is what a host does.
    //
    // Opening the device this way does not disturb `cdc_acm`: no interface is
    // claimed, and a descriptor read is endpoint 0's.
    let context = Context::new().expect("the USB subsystem is reachable, since sysfs named a port");
    let device = context
        .find_device(VID, PID)
        .expect("enumerating USB devices")
        .expect("the device sysfs just named");
    match device.open() {
        Ok(handle) => {
            let mut header = [0u8; 9];
            let n = handle
                .control_read(0x80, 0x06, 0x0200, 0, &mut header, TIMEOUT)
                .expect("the configuration descriptor's header");
            assert_eq!(n, 9, "nine bytes of configuration descriptor header");
            let total = usize::from(u16::from_le_bytes([header[2], header[3]]));
            let mut config = vec![0u8; total];
            let n = handle
                .control_read(0x80, 0x06, 0x0200, 0, &mut config, TIMEOUT)
                .expect("the whole configuration descriptor");
            config.truncate(n);
            println!("configuration descriptor ({n} bytes): {config:02x?}");
            assert_eq!(
                config,
                expected_configuration(),
                "the part reports the descriptor set the sources describe"
            );

            // The device descriptor's class triple, which is what tells a
            // host the two interfaces are one function.
            let mut device_desc = [0u8; 18];
            let n = handle
                .control_read(0x80, 0x06, 0x0100, 0, &mut device_desc, TIMEOUT)
                .expect("the device descriptor");
            assert_eq!(n, 18);
            assert_eq!(
                [device_desc[4], device_desc[5], device_desc[6]],
                [0x02, 0x00, 0x00],
                "bDeviceClass 02h, as CDC 1.1 Table 14 asks"
            );
            println!("bDeviceClass is 02h, bDeviceSubClass and bDeviceProtocol 00h");
        }
        Err(err) => {
            // Not fatal: the headline claim is that a terminal exists, and it
            // does. A descriptor read needs usbfs permission that reading a
            // terminal does not.
            println!("cannot open {VID:04x}:{PID:04x} over usbfs ({err})");
            println!("  the descriptors were not checked; a udev rule is usually why");
        }
    }

    // ------------------------------------------------------------------
    // Part three: the device sent a SERIAL_STATE notification.
    // ------------------------------------------------------------------
    // This asks for a notification and then reads it, rather than hoping one is
    // still pending, so it does not matter what has opened the terminal before
    // now — `serial_state_notification` says why that used to matter and what
    // it hid. What it does still take is the one packet that is pending when it
    // runs, so `ip/usb/usb_cdc_acm/README.md` §5's `TIOCMGET` readings come from a
    // separate run: whichever of usbfs and `cdc_acm` polls first gets a given
    // packet, and that much is just how one packet works.
    match device.open() {
        Ok(handle) => match serial_state_notification(&handle) {
            Ok(bytes) => {
                println!("SERIAL_STATE off endpoint {NOTIF_EP:#04x}: {bytes:02x?}");
                assert_eq!(
                    bytes.len(),
                    SERIAL_STATE_BYTES,
                    "a SERIAL_STATE is eight bytes of header and two of wSerialState"
                );
                // PSTN 1.2 §6.5 for the header — it has a SETUP packet's shape
                // (USB 2.0 Table 9-2) — and §6.5.4 for what follows it.
                assert_eq!(
                    bytes[0], 0xA1,
                    "bmRequestType: device to host, class, interface"
                );
                assert_eq!(bytes[1], 0x20, "bNotification: SERIAL_STATE");
                assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), 0, "wValue");
                assert_eq!(
                    u16::from_le_bytes([bytes[4], bytes[5]]),
                    u16::from(COMM_IFACE),
                    "wIndex names the communications interface"
                );
                assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 2, "wLength");
                assert_eq!(
                    u16::from_le_bytes([bytes[8], bytes[9]]),
                    SERIAL_STATE,
                    "wSerialState: bRxCarrier and bTxCarrier, no errors"
                );
                println!(
                    "  bRxCarrier and bTxCarrier set, wLength 2, wIndex {COMM_IFACE}: \
                     PSTN 1.2 §6.5.4, field for field"
                );
            }
            Err(why) => {
                println!("the SERIAL_STATE notification was not read ({why})");
                println!(
                    "  a udev rule or membership of the right group is usually why; the \
                     notification is then proven only in simulation"
                );
            }
        },
        Err(err) => {
            println!("cannot open {VID:04x}:{PID:04x} over usbfs ({err}) for the notification")
        }
    }

    // ------------------------------------------------------------------
    // Part two: bytes through the port, through the UART, and back.
    // ------------------------------------------------------------------
    // The path is looked up **again**, because part three took the
    // communications interface away from `cdc_acm` and gave it back: the driver
    // re-attaching destroys the terminal and makes a new one, udev sets its
    // owner and group a moment after the node appears, and an `stty` that lands
    // in between gets `Permission denied` from a device that is perfectly well.
    // That is what happened the first time this ran in this order.
    let path = match terminal_again() {
        Some(again) => {
            if again != *port {
                println!(
                    "`cdc_acm` came back on {} (it was {})",
                    again.display(),
                    port.display()
                );
            }
            again.to_string_lossy().into_owned()
        }
        None => panic!(
            "no /dev/ttyACM* for {VID:04x}:{PID:04x} came back after the notification was \
             read: `cdc_acm` did not re-attach to interface {COMM_IFACE}. Unplug and replug \
             the board."
        ),
    };
    // 115200 to match `usb_cdc_uart.v`'s divisor, raw so nothing translates a
    // newline, no echo so the terminal layer does not send our bytes back at
    // us and make a broken device look like a working one, `clocal` so that a
    // carrier this device reported and then dropped could not hang the terminal
    // up mid-transfer — which is the one thing `acm->clocal` reaches in
    // `cdc_acm`, and this board's `serial_state` is a constant so it cannot
    // happen — and a two-second read timeout with no minimum so a read returns
    // rather than blocking for ever.
    let stty = Command::new("stty")
        .args([
            "-F", &path, "115200", "raw", "-echo", "clocal", "min", "0", "time", "20",
        ])
        .status();
    match stty {
        Ok(status) if status.success() => {}
        Ok(status) => {
            println!("`stty -F {path}` exited {status}; the round trip was not attempted");
            println!("  membership of the group that owns {path} is usually why");
            return;
        }
        Err(err) => {
            println!("cannot run `stty` ({err}); the round trip was not attempted");
            return;
        }
    }

    // Enough bytes that a fault which happens once in a while has to show,
    // and every byte different from its neighbours so a byte held over from
    // the last one is visible. Short enough that one byte at a time through a
    // 115200 baud UART is seconds and not minutes.
    let payload: Vec<u8> = (0..48u8)
        .map(|i| i.wrapping_mul(37).wrapping_add(11))
        .collect();

    // In a thread, because an `open` of a terminal can block and a test that
    // hangs is worse than a test that fails. `recv_timeout` is a budget and
    // not an assertion about speed: it is thirty seconds for work that takes
    // well under one.
    let (tx, rx) = mpsc::channel();
    let want = payload.clone();
    let opening = path.clone();
    thread::spawn(move || {
        let _ = tx.send(round_trip(&opening, &want));
    });

    match rx.recv_timeout(ROUND_TRIP) {
        Ok(Ok(got)) => {
            println!("{} bytes out, {} bytes back", payload.len(), got.len());
            assert_eq!(
                got, payload,
                "what was written to {path} came back off the UART unchanged"
            );
            println!(
                "host -> USB -> UART transmit -> UART receive -> USB -> host, {} bytes, \
                 byte for byte",
                payload.len()
            );

            // What the host put in the device's line-coding registers. 115200
            // is `stty`'s and 9600 is the block's reset value, so which of the
            // two comes back says whether SET_LINE_CODING was ever sent.
            if let Ok(handle) = device.open() {
                match line_coding(&handle) {
                    Ok(coding) => {
                        let rate = u32::from_le_bytes([coding[0], coding[1], coding[2], coding[3]]);
                        println!(
                            "GET_LINE_CODING: {rate} baud, bCharFormat {}, bParityType {}, \
                             bDataBits {}",
                            coding[4], coding[5], coding[6]
                        );
                        assert_eq!(
                            rate, 115_200,
                            "the host set the line coding and the device kept it; 9600 would \
                             mean SET_LINE_CODING never arrived, since that is the reset value"
                        );
                    }
                    Err(why) => {
                        println!("GET_LINE_CODING was not asked ({why})");
                        println!(
                            "  a udev rule or membership of the right group is usually why; \
                             the class hook's host-to-device data stage is then unproven on \
                             this host and proven only in simulation"
                        );
                    }
                }
            }

            // And the driver is back, with a terminal of its own again. The
            // number may have moved, which is why this looks it up rather than
            // remembering it.
            match tty_ports(VID, PID).first() {
                Some(again) => println!("`cdc_acm` is back on {}", again.display()),
                None => panic!(
                    "the line coding was read and the device has no /dev/ttyACM* any more: \
                     `cdc_acm` did not re-attach. Unplug and replug the board."
                ),
            }
        }
        Ok(Err(why)) => panic!("the round trip through {path} failed: {why}"),
        Err(_) => panic!(
            "nothing came back from {path} within {ROUND_TRIP:?}. LED 2 lit and LED 4 dark on \
             the board means the byte reached the UART and no byte came back out of it; both \
             dark means it never left the endpoint. \
             testdata/fpga/cynthion/usb_cdc_uart.v says how to read them."
        ),
    }
}

/// The terminal for this device, waited for until a program can open it.
///
/// Detaching `cdc_acm` from an interface and attaching it again destroys the
/// terminal and creates a new one, and the two things a caller needs — that the
/// node is there, and that it is openable — do not become true at the same
/// instant: udev sets the owner and group from a rule that runs after the
/// kernel has created the device. So this waits for both, and the wait is a
/// **timeout and not an assertion**: five seconds of budget for work that takes
/// milliseconds, and nothing here is compared against a clock.
fn terminal_again() -> Option<PathBuf> {
    for _ in 0..50 {
        if let Some(path) = tty_ports(VID, PID).into_iter().next() {
            // Openable, which is the thing the `stty` after this needs and the
            // thing that is not true the instant the node appears.
            if fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .is_ok()
            {
                return Some(path);
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    None
}

/// The SERIAL_STATE notification, read off the notification endpoint itself
/// after **asking for one the way a host's driver does**.
///
/// It sends SET_CONTROL_LINE_STATE first, which is what `cdc_acm` issues from
/// `acm_port_dtr_rts` on every `open`, and `ip/usb/usb_cdc_acm` answers a request
/// like that with a notification whether or not the line state has moved. So
/// the `interrupt_read` that follows is **deterministic**: it does not depend on
/// a packet left over from configuration time, and therefore not on whether
/// anything else has opened the terminal since the device enumerated.
///
/// That matters because the first version of this test *did* depend on it, and
/// hid a defect behind the dependency: with one notification per configuration
/// there was exactly one packet in the device's whole life, this test consumed
/// it, and `cdc_acm` then reported no carrier for ever — `TIOCMGET = 0x026` on
/// every open. `ip/usb/usb_cdc_acm/README.md` §4 has that written up.
///
/// **It has to take the communications interface away from `cdc_acm` to ask**,
/// for the same reason `line_coding` below does: usbfs refuses a transfer on an
/// endpoint whose interface another driver holds. So it detaches, claims, asks,
/// releases and attaches the driver again, and the caller's later parts check
/// that the terminal came back.
///
/// **What this would and would not catch.** It catches the ten bytes being
/// wrong in any field, and it catches the notification not being sent in answer
/// to a host opening the port — which is the defect above and would show here
/// as a timeout. It does **not** show that a host driver decodes them: that is
/// `TIOCMGET`, which needs an ioctl this crate cannot make, and
/// `ip/usb/usb_cdc_acm/README.md` §5 has it taken by hand across three opens. It
/// also does not show a notification sent on a *change* of the line state,
/// because this board's `serial_state` is a constant; that half is
/// `tests/ip_library.rs`'s
/// `usb_cdc_acm_notification_endpoint_sends_the_serial_state`.
fn serial_state_notification(handle: &rawusb::DeviceHandle) -> Result<Vec<u8>, String> {
    let held = handle.kernel_driver_active(COMM_IFACE).unwrap_or(false);
    if held {
        handle
            .detach_kernel_driver(COMM_IFACE)
            .map_err(|e| format!("detaching the driver from interface {COMM_IFACE}: {e}"))?;
    }
    let claimed = handle.claim_interface(COMM_IFACE);
    // `wMaxPacketSize` of the endpoint, so a packet longer than the ten bytes
    // expected comes back whole and fails the length assertion rather than
    // being silently cut to fit.
    let mut buf = vec![0u8; usize::from(NOTIF_MAXPKT)];
    let asked = if claimed.is_ok() {
        // SET_CONTROL_LINE_STATE with DTR and RTS raised: bmRequestType 21h is
        // host to device, class, to an interface; bRequest 22h; wValue D0 is
        // DTR and D1 is RTS; no data stage (PSTN 1.2 §6.3.12). This is a host
        // saying it has opened the port, and it is what makes the read below
        // independent of anything that happened before this test ran.
        handle
            .control_write(0x21, 0x22, 0x0003, u16::from(COMM_IFACE), &[], TIMEOUT)
            .map_err(|e| format!("SET_CONTROL_LINE_STATE: {e}"))
            .and_then(|_| {
                handle
                    .interrupt_read(NOTIF_EP, &mut buf, TIMEOUT)
                    .map_err(|e| format!("{e}"))
            })
    } else {
        Err(format!("claiming interface {COMM_IFACE}: {claimed:?}"))
    };
    if claimed.is_ok() {
        let _ = handle.release_interface(COMM_IFACE);
    }
    let reattached = if held {
        handle.attach_kernel_driver(COMM_IFACE)
    } else {
        Ok(())
    };
    if let Err(err) = reattached {
        return Err(format!(
            "the notification was read but `cdc_acm` could not be put back on interface \
             {COMM_IFACE} ({err}); unplug and replug the board"
        ));
    }
    match asked {
        Ok(n) => {
            buf.truncate(n);
            Ok(buf)
        }
        Err(err) => Err(err),
    }
}

/// GET_LINE_CODING, asked of the part directly, so that what the **host** put
/// in the device's registers can be read back out of them.
///
/// This is the only thing in this file that proves the class hook's
/// **host-to-device data stage** on real hardware. The round trip proves the
/// bulk endpoints; the port opening at all proves SET_CONTROL_LINE_STATE,
/// since `cdc_acm` fails `open` if that stalls; but SET_LINE_CODING is the one
/// request with a data packet behind it, and nothing above would notice if its
/// seven bytes had never arrived. `dwDTERate` comes up at **9600** and `stty`
/// asked for **115200**, so which of the two comes back is the whole
/// assertion.
///
/// **It has to take the interface away from `cdc_acm` to ask**, and that is
/// not a choice: usbfs refuses a control transfer addressed to an interface
/// another driver holds — `resource busy (os error 16)`, which is what this
/// returned before it did the detaching. So it detaches, claims, asks,
/// releases and **attaches the driver again**, and the caller checks the
/// terminal came back. The line coding is read *after* `stty` has set it and
/// survives the detach, because it is a register in the device and not
/// anything the host is keeping.
fn line_coding(handle: &rawusb::DeviceHandle) -> Result<Vec<u8>, String> {
    let held = handle.kernel_driver_active(COMM_IFACE).unwrap_or(false);
    if held {
        handle
            .detach_kernel_driver(COMM_IFACE)
            .map_err(|e| format!("detaching the driver from interface {COMM_IFACE}: {e}"))?;
    }
    let claimed = handle.claim_interface(COMM_IFACE);
    let mut coding = [0u8; 7];
    let asked = if claimed.is_ok() {
        handle
            .control_read(
                0xA1,
                0x21,
                0x0000,
                u16::from(COMM_IFACE),
                &mut coding,
                TIMEOUT,
            )
            .map_err(|e| format!("{e}"))
    } else {
        Err(format!("claiming interface {COMM_IFACE}: {claimed:?}"))
    };
    // Give it back whatever happened, and say so if that fails, because a
    // device left without its driver is a device the next person finds broken.
    if claimed.is_ok() {
        let _ = handle.release_interface(COMM_IFACE);
    }
    let reattached = if held {
        handle.attach_kernel_driver(COMM_IFACE)
    } else {
        Ok(())
    };
    if let Err(err) = reattached {
        return Err(format!(
            "the line coding was read but `cdc_acm` could not be put back on interface \
             {COMM_IFACE} ({err}); unplug and replug the board"
        ));
    }
    match asked {
        Ok(7) => Ok(coding.to_vec()),
        Ok(n) => Err(format!("GET_LINE_CODING answered {n} bytes and not seven")),
        Err(err) => Err(err),
    }
}

/// Write `payload` to a terminal and read the same number of bytes back.
///
/// Returns the bytes, or what went wrong. A read that returns zero is the
/// `VTIME` timeout expiring with nothing there, which is not by itself a
/// failure — the device is one byte at a time and a byte may simply not have
/// come round yet — so a few of those are tolerated and a run of them is not.
fn round_trip(path: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut port = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("opening {path}: {e}"))?;
    port.write_all(payload)
        .map_err(|e| format!("writing {} bytes: {e}", payload.len()))?;
    port.flush().map_err(|e| format!("flushing: {e}"))?;

    let mut got: Vec<u8> = Vec::with_capacity(payload.len());
    let mut buf = [0u8; 64];
    let mut quiet = 0;
    while got.len() < payload.len() {
        match port.read(&mut buf) {
            Ok(0) => {
                quiet += 1;
                if quiet > 4 {
                    return Err(format!(
                        "the port went quiet after {} of {} bytes: {got:02x?}",
                        got.len(),
                        payload.len()
                    ));
                }
            }
            Ok(n) => {
                quiet = 0;
                got.extend_from_slice(&buf[..n]);
            }
            Err(e) => return Err(format!("reading after {} bytes: {e}", got.len())),
        }
    }
    Ok(got)
}
