//! A serial port this compiler built, driven through the **operating
//! system's own driver**.
//!
//! `tests/ip_library.rs` proves `ip/usb_cdc_acm` against a host model and a
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
//! It is in two halves, and they fail for different reasons.
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
//!    is that each byte is serialised at 115200 baud by `ip/uart`'s
//!    transmitter, recovered by its receiver, and handed back to the IN
//!    endpoint — so a byte that comes back proves the whole bridge and not
//!    only the USB half.
//!
//! **Why `stty` and not an ioctl.** Configuring a terminal means `tcsetattr`,
//! which means `libc` and `unsafe`, and this crate has neither. `stty` is the
//! operating system's own tool for it and shelling out to it is the honest
//! way to reach a terminal from a crate that refuses both — and it is what a
//! person does at a prompt anyway, which is the point of this class. `clocal`
//! is in the list for a reason worth knowing: a CDC ACM device reports
//! carrier through the SERIAL_STATE notification, `ip/usb_cdc_acm` never
//! sends one, so without `clocal` an `open` of the port would wait for a
//! carrier that never arrives.
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

/// pid.codes' test pair, which is `ip/usb_cdc_acm`'s default `VID` / `PID`.
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

/// The interface numbers and endpoint addresses `ip/usb_cdc_acm` declares.
const COMM_IFACE: u8 = 0;
const DATA_IFACE: u8 = 1;
const NOTIF_EP: u8 = 0x82;
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;

/// The configuration descriptor `ip/usb_cdc_acm` describes, written forwards
/// from CDC 1.1 and PSTN 1.2 — the arithmetic and not the answers, as
/// `tests/ip_library.rs` writes the same bytes and as `ip/usb_cdc_acm/README.md`
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
    // ENDPOINT 82h: interrupt IN, eight bytes, every 16 frames.
    iface.extend_from_slice(&[7, 5, NOTIF_EP, 0x03, 8, 0, 16]);
    // INTERFACE 1: CDC Data.
    iface.extend_from_slice(&[9, 4, DATA_IFACE, 0, 0, 0x0A, 0x00, 0x00, 0]);
    // ENDPOINT 01h and 81h: bulk, eight bytes.
    iface.extend_from_slice(&[7, 5, EP_OUT, 0x02, 8, 0, 0]);
    iface.extend_from_slice(&[7, 5, EP_IN, 0x02, 8, 0, 0]);

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
                 ip/usb_cdc_acm/README.md sections 2 and 4 say what each one is for."
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
    // Half two: bytes through the port, through the UART, and back.
    // ------------------------------------------------------------------
    let path = port.to_string_lossy().into_owned();

    // 115200 to match `usb_cdc_uart.v`'s divisor, raw so nothing translates a
    // newline, no echo so the terminal layer does not send our bytes back at
    // us and make a broken device look like a working one, `clocal` because
    // this device never reports carrier, and a two-second read timeout with
    // no minimum so a read returns rather than blocking for ever.
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
