//! Bytes through a USB device this compiler built, driven by our own host
//! code.
//!
//! `tests/ip_library.rs` proves the endpoint against a host model and a
//! transceiver model, in simulation, and that is where the interesting
//! assertions are — a NAK counted, a repeated packet delivered once, a
//! zero-length packet sent from `in_commit` alone. What it cannot prove is
//! that a **real host** and a **real transceiver** agree with those models,
//! and the last round is the reason that distinction is written down rather
//! than assumed: a three-bit state register for four states enumerated
//! perfectly in simulation and NAKed every IN token on the part, because a
//! simulator has no opinion about a flip-flop whose data input is a constant.
//!
//! So this test is the other half, and it is deliberately thin. It opens
//! `1209:0001`, claims the interface, and moves bytes:
//!
//! ```console
//! cargo test --features program --test usb_loopback -- --ignored --nocapture
//! ```
//!
//! **No OS driver is involved.** The device declares one vendor-specific
//! interface — `bInterfaceClass` `FFh` — so no class driver claims it, which
//! is exactly why a vendor bulk loopback is the right first thing to move
//! bytes over: a HID or a CDC device would put the kernel between this test
//! and the part, and a failure would have two places to be. Here the only
//! things between a byte and the FPGA are `rawusb`, usbfs and the
//! transceiver.
//!
//! It is `#[ignore]`d, so a machine with no board never runs it, and it skips
//! with a reason rather than failing when the device is not there: an
//! unplugged board is not a broken compiler.
//!
//! What has to be loaded for it to pass is
//! `testdata/fpga/cynthion/usb_ulpi_device.v`, whose header says how to build
//! and load the bitstream. Any design with `ip/usb_device_fs`'s default
//! descriptors and `out_*` wired into `in_*` will do.

#![cfg(feature = "program")]

use std::time::Duration;

use rawusb::Context;

/// pid.codes' test pair, which is `ip/usb_device_fs`'s default `VID` / `PID`.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;

/// The interface the descriptors declare, and its two endpoint addresses.
const INTERFACE: u8 = 0;
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;

/// Long enough that a device answering at all answers inside it, short
/// enough that a device answering nothing does not hold the run up. It is a
/// timeout and **not an assertion**: nothing here is checked against a clock,
/// because CI runs on slower machines than this one.
const TIMEOUT: Duration = Duration::from_millis(500);

/// `wMaxPacketSize` of both endpoints, and **the size every read here asks
/// for**.
///
/// This is the one thing about bulk transfers that has to be got right on the
/// host side, and getting it wrong looked exactly like a broken device. A
/// bulk IN transfer ends when the device sends a packet **shorter than
/// `wMaxPacketSize`** or when the host's buffer is full, so a read of 64
/// bytes answered with eight is not finished: the host asks again, the device
/// NAKs because it has nothing more, and the transfer times out. Payloads of
/// one to seven bytes came back and eight did not — from a device that was
/// behaving perfectly.
///
/// The device could end such a transfer itself by sending a zero-length
/// packet after a full one, which `usb_bulk_ep` can do — `in_commit` with no
/// bytes given — but a loopback has nothing to trigger it with, since an OUT
/// of no bytes hands the interface nothing. So the host asks for one packet
/// at a time, which is what a host that knows the protocol it is speaking
/// does anyway.
const MAX_PACKET: usize = 8;

/// The configuration descriptor `usb_ctrl_ep`'s default parameters describe,
/// written forwards from the specification's layout — the arithmetic and not
/// the answers, as `tests/ip_library.rs` writes the same bytes.
fn expected_configuration() -> Vec<u8> {
    let mut iface: Vec<u8> = Vec::new();
    // INTERFACE: number 0, alternate 0, vendor specific.
    iface.extend_from_slice(&[9, 4, 0, 0, 0, 0xFF, 0x00, 0x00, 0]);
    // ENDPOINT 1 OUT and ENDPOINT 1 IN: bulk, eight bytes, no interval.
    iface.extend_from_slice(&[7, 5, EP_OUT, 2, 8, 0, 0]);
    iface.extend_from_slice(&[7, 5, EP_IN, 2, 8, 0, 0]);
    // bNumEndpoints and bNumInterfaces are counted along the chain of
    // bLength fields, the way a host reads them and the way the block
    // computes them, so a descriptor changed in one place changes this
    // expectation with it.
    iface[4] = count(&iface, 5);
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

#[test]
#[ignore = "needs a USB device built by this compiler attached: 1209:0001"]
fn usb_endpoint_one_loops_bytes_back_on_a_real_host() {
    let context = match Context::new() {
        Ok(context) => context,
        Err(err) => {
            println!("the USB subsystem is not reachable ({err}), skipping");
            return;
        }
    };
    let device = match context.find_device(VID, PID) {
        Ok(Some(device)) => device,
        Ok(None) => {
            println!("no {VID:04x}:{PID:04x} is attached, skipping");
            return;
        }
        Err(err) => {
            println!("cannot enumerate USB devices ({err}), skipping");
            return;
        }
    };
    let handle = match device.open() {
        Ok(handle) => handle,
        Err(err) => {
            println!("cannot open {VID:04x}:{PID:04x} ({err}), skipping");
            println!("  a udev rule or membership of the right group is usually why");
            return;
        }
    };

    // The descriptors the part is reporting, read back from the part rather
    // than from the sources. `wTotalLength` first, then the whole thing at
    // that length, which is what a host does.
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
        "the part reports the descriptor the sources describe"
    );

    // Nothing should be holding this interface — a vendor class has no
    // driver — but say so rather than fail obscurely if something is.
    if let Ok(true) = handle.kernel_driver_active(INTERFACE) {
        println!("a kernel driver holds interface {INTERFACE}; detaching it");
        handle
            .detach_kernel_driver(INTERFACE)
            .expect("detaching the driver");
    }
    handle
        .claim_interface(INTERFACE)
        .expect("claiming the vendor interface");

    // Put both data toggles back to DATA0 on the device **and** in the
    // kernel, which is the one thing a host must do before it can trust a
    // bulk pipe it did not open from a fresh enumeration. `usb_ctrl_ep`
    // accepts CLEAR_FEATURE(ENDPOINT_HALT) for exactly this; without it a
    // second run of this test would send a packet the device reads as a
    // repeat, discard it, and acknowledge it, and the host would believe the
    // bytes arrived.
    for endpoint in [EP_OUT, EP_IN] {
        handle
            .clear_halt(endpoint)
            .unwrap_or_else(|err| panic!("clearing endpoint {endpoint:#04x}: {err}"));
    }

    // One packet out, the same packet back, for a set that covers the packet
    // size, a short packet and one byte. The device holds one packet each
    // way, so a host that wrote two before reading one would be NAKed until
    // it read — which is correct and is what `usb_bulk_endpoint_naks_an_out_
    // until_the_bytes_are_taken` proves in simulation. A loopback is read
    // after each write.
    let payloads: Vec<Vec<u8>> = vec![
        vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
        vec![0xDE, 0xAD, 0xBE, 0xEF, 0xFF],
        vec![0x5A],
        (0..8u8).map(|i| i.wrapping_mul(37)).collect(),
    ];
    for payload in &payloads {
        assert!(payload.len() <= MAX_PACKET, "one packet at a time");
        let sent = handle
            .bulk_write(EP_OUT, payload, TIMEOUT)
            .expect("writing to endpoint 1 OUT");
        assert_eq!(sent, payload.len(), "the whole packet went out");
        let mut buf = vec![0u8; MAX_PACKET];
        let got = handle
            .bulk_read(EP_IN, &mut buf, TIMEOUT)
            .expect("reading from endpoint 1 IN");
        buf.truncate(got);
        println!("{:02x?} -> {:02x?}", payload, buf);
        assert_eq!(&buf, payload, "what went out came back");
    }

    // And then enough of it that a fault which happens once in a while has
    // to show: 256 bytes through in 8-byte packets, every byte different
    // from its neighbours' and from its own position modulo the packet size,
    // so a byte swapped with another or held over from the last packet is
    // visible.
    let stream: Vec<u8> = (0..=255u8).map(|i| i.wrapping_mul(97).wrapping_add(13)).collect();
    let mut back: Vec<u8> = Vec::with_capacity(stream.len());
    for chunk in stream.chunks(MAX_PACKET) {
        handle
            .bulk_write(EP_OUT, chunk, TIMEOUT)
            .expect("writing a chunk");
        let mut buf = [0u8; MAX_PACKET];
        let got = handle
            .bulk_read(EP_IN, &mut buf, TIMEOUT)
            .expect("reading a chunk");
        back.extend_from_slice(&buf[..got]);
    }
    assert_eq!(back.len(), stream.len(), "every byte came back");
    if back != stream {
        let first = back
            .iter()
            .zip(&stream)
            .position(|(a, b)| a != b)
            .expect("they differ somewhere");
        panic!(
            "byte {first} came back as {:#04x} and not {:#04x}",
            back[first], stream[first]
        );
    }
    println!(
        "{} bytes through endpoint 1 and back, in {} packets of at most 8",
        stream.len(),
        stream.len().div_ceil(MAX_PACKET)
    );

    handle
        .release_interface(INTERFACE)
        .expect("releasing the interface");
}
