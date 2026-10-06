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
//! and load the bitstream. Any design with `ip/usb/usb_device_fs`'s default
//! descriptors and `out_*` wired into `in_*` will do.
//!
//! # The byte is also a constant-zero probe
//!
//! `usb_ulpi_device.v` XORs the byte it hands back with `zero_probe`, a
//! flip-flop whose data input is the **constant zero**. That costs one
//! flip-flop and it turns this test into the measurement the backend was
//! missing: on this family a fabric flip-flop takes its data from the slice's
//! `M` wire, an unrouted slice input reads as a **one**, and until
//! `techcells::drive_constant_data` built a lookup table to drive it such a
//! register came up **set** — which is why `reg [2:0] stage` for four states
//! read 5 and this very device would not enumerate for eight rounds.
//!
//! So: the constant is right, the XOR is with zero, and everything below is
//! byte-identical and passes exactly as it did before. The constant is
//! wrong and **every returned byte is the complement of the byte sent**,
//! which nothing else in this design can do; [`flipped_by_the_probe`] names
//! it rather than leaving a reader to work out why a whole stream came back
//! inside out.
//!
//! **What that would and would not catch.** It catches a flip-flop whose
//! data input is the constant zero coming up, or being clocked to, a one —
//! the defect itself — and it catches it on **every byte** this test moves,
//! which is a few hundred through the assertions and two thousand more through
//! the throughput measurement, so it cannot pass by accident. It does not catch a broken
//! constant *one*: the same design's `rst_q` is fed by one and releases the
//! transceiver's reset pin, so a one that came up zero means no device on
//! the bus, and this test **skips** with "no 1209:0001 is attached" rather
//! than failing. Nor does it say anything about a design that does not use
//! `drive_constant_data` at all; `tests/fpga_trellis.rs` is where the
//! netlist and the bits are checked, and
//! `the_usb_devices_constant_zero_probe_survives_synthesis` is what stops an
//! optimiser from deleting the probe and leaving this test green for ever.

#![cfg(feature = "program")]

use std::time::{Duration, Instant};

use rawusb::Context;

/// pid.codes' test pair, which is `ip/usb/usb_device_fs`'s default `VID` / `PID`.
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
/// `wMaxPacketSize`** or when the host's buffer is full, so a read of 128
/// bytes answered with 64 is not finished: the host asks again, the device
/// NAKs because it has nothing more, and the transfer times out. That bit this
/// test when the number here was 8 and the read asked for 64.
///
/// The device could end such a transfer itself by sending a zero-length
/// packet after a full one, which `usb_bulk_ep` can do — `in_commit` with no
/// bytes given — but a loopback has nothing to trigger it with, since an OUT
/// of no bytes hands the interface nothing. So the host asks for one packet
/// at a time, which is what a host that knows the protocol it is speaking
/// does anyway.
///
/// **64 is what the descriptors now say**, the largest of the four sizes USB
/// 2.0 §5.8.3 allows a full-speed bulk endpoint; it was 8, which is the
/// smallest. The assertion that this number and the part agree is below, read
/// off the descriptor the part reports rather than trusted: a host that
/// disagreed with a device about `wMaxPacketSize` is the failure this comment
/// is about, and it is worth catching as a mismatch rather than as a timeout.
const MAX_PACKET: usize = 64;

/// The configuration descriptor `usb_ctrl_ep`'s default parameters describe,
/// written forwards from the specification's layout — the arithmetic and not
/// the answers, as `tests/ip_library.rs` writes the same bytes.
fn expected_configuration() -> Vec<u8> {
    let mut iface: Vec<u8> = Vec::new();
    // INTERFACE: number 0, alternate 0, vendor specific.
    iface.extend_from_slice(&[9, 4, 0, 0, 0, 0xFF, 0x00, 0x00, 0]);
    // ENDPOINT 1 OUT and ENDPOINT 1 IN: bulk, `MAX_PACKET` bytes, no interval.
    let pkt = u8::try_from(MAX_PACKET).expect("a legal wMaxPacketSize");
    iface.extend_from_slice(&[7, 5, EP_OUT, 2, pkt, 0, 0]);
    iface.extend_from_slice(&[7, 5, EP_IN, 2, pkt, 0, 0]);
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

/// The one failure `usb_ulpi_device.v`'s constant-zero probe has, said in
/// words: every byte came back as its own complement.
///
/// `in_data` is `out_data ^ {8{zero_probe}}`, so this is not a byte lost, a
/// byte swapped or a toggle out of step — it is one flip-flop, whose data
/// input is the literal `1'b0`, holding a **one**. Nothing else in the design
/// inverts a whole packet. A partial inversion is *not* this and is left to
/// the general assertions, because the probe is one wire into all eight bits
/// and cannot flip some of them.
fn flipped_by_the_probe(sent: &[u8], back: &[u8]) -> Option<String> {
    if sent.is_empty() || sent.len() != back.len() {
        return None;
    }
    if !sent.iter().zip(back).all(|(a, b)| *a == !*b) {
        return None;
    }
    Some(format!(
        "every one of {} returned byte(s) is the complement of the byte sent, which is \
         `zero_probe` holding a ONE: a flip-flop whose data input is the constant zero came up \
         set. That is the defect `techcells::drive_constant_data` exists to prevent — an \
         unrouted `M` wire on this family reads as a one — so the constant driver is missing, \
         unrouted or holding the wrong truth table in this bitstream. See \
         docs/fpga-trellis.md, \"The constant is built now\", and \
         tests/fpga_trellis.rs::the_usb_devices_constant_zero_probe_reaches_the_bitstream",
        sent.len()
    ))
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
    // And `MAX_PACKET` above is what the part says, not what this file hopes:
    // every read below asks for exactly that many bytes, and a host asking for
    // more than the device promised turns a working device into a timeout.
    // Byte 4 of an endpoint descriptor is the low byte of `wMaxPacketSize`
    // (USB 2.0 Table 9-13) and the two endpoints are at 18 and 25.
    for at in [18usize, 25] {
        assert_eq!(
            u16::from_le_bytes([config[at + 4], config[at + 5]]),
            u16::try_from(MAX_PACKET).expect("small"),
            "endpoint {:#04x} declares a packet size this test does not use",
            config[at + 2]
        );
    }

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
        // A full packet, one a byte short of full, then short ones. The full
        // one is the case this round exists for and the 63-byte one is the
        // case a buffer base counter off by one gets wrong.
        (0..MAX_PACKET)
            .map(|i| u8::try_from(i % 256).expect("a byte").wrapping_mul(73))
            .collect(),
        (0..MAX_PACKET - 1)
            .map(|i| u8::try_from(i % 256).expect("a byte").wrapping_mul(29))
            .collect(),
        vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
        vec![0xDE, 0xAD, 0xBE, 0xEF, 0xFF],
        vec![0x5A],
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
        if let Some(why) = flipped_by_the_probe(payload, &buf) {
            panic!("{why}");
        }
        assert_eq!(&buf, payload, "what went out came back");
    }

    // And then enough of it that a fault which happens once in a while has
    // to show: 256 bytes through in `MAX_PACKET` packets, every byte different
    // from its neighbours' and from its own position modulo the packet size,
    // so a byte swapped with another or held over from the last packet is
    // visible.
    let stream: Vec<u8> = (0..=255u8)
        .map(|i| i.wrapping_mul(97).wrapping_add(13))
        .collect();
    let mut back: Vec<u8> = Vec::with_capacity(stream.len());
    for chunk in stream.chunks(MAX_PACKET) {
        handle
            .bulk_write(EP_OUT, chunk, TIMEOUT)
            .expect("writing a chunk");
        let mut buf = vec![0u8; MAX_PACKET];
        let got = handle
            .bulk_read(EP_IN, &mut buf, TIMEOUT)
            .expect("reading a chunk");
        back.extend_from_slice(&buf[..got]);
    }
    assert_eq!(back.len(), stream.len(), "every byte came back");
    if let Some(why) = flipped_by_the_probe(&stream, &back) {
        panic!("{why}");
    }
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
        "{} bytes through endpoint 1 and back, in {} packets of at most {MAX_PACKET}",
        stream.len(),
        stream.len().div_ceil(MAX_PACKET)
    );
    // Byte-identical, and the byte was XORed with `zero_probe` on its way
    // out, so this is the constant-zero measurement and not only a loopback.
    println!(
        "`zero_probe` read ZERO on every one of them: a flip-flop whose data input is the \
         constant zero holds zero in silicon on this family"
    );

    // ------------------------------------------------------------------
    // How fast it goes, which is a **measurement and not an assertion**.
    // ------------------------------------------------------------------
    // Nothing below is compared against a clock. `tools/check.sh` never runs
    // this test, a slow machine must not fail it, and there is no timing
    // number in any `assert`. What the figure is for is
    // `ip/usb/usb_cdc_acm/README.md` and `docs/ip-library.md`, where a claim
    // about throughput has to come off a part rather than out of arithmetic.
    //
    // The shape of the loop is the shape the device forces: it holds **one**
    // packet each way, so the host writes a packet, reads it back, and only
    // then writes the next. Every packet therefore costs one OUT
    // transaction, one IN transaction and two trips through the host's own
    // stack, and what `wMaxPacketSize` changes is only how many bytes ride
    // along with them. That is the quantity worth knowing: a full-speed host
    // is limited in transactions a frame and not in bytes.
    let bulk: Vec<u8> = (0..MAX_PACKET)
        .map(|i| u8::try_from(i % 256).expect("a byte").wrapping_mul(73))
        .collect();
    let rounds = 256u32;
    let started = Instant::now();
    let mut moved = 0usize;
    for _ in 0..rounds {
        handle
            .bulk_write(EP_OUT, &bulk, TIMEOUT)
            .expect("writing a packet");
        let mut buf = vec![0u8; MAX_PACKET];
        let got = handle
            .bulk_read(EP_IN, &mut buf, TIMEOUT)
            .expect("reading it back");
        assert_eq!(&buf[..got], &bulk[..], "the timed packets came back too");
        moved += got;
    }
    let took = started.elapsed();
    println!(
        "{moved} bytes in {rounds} round trips of {MAX_PACKET}: {took:?}, \
         {:.0} bytes/s each way, {:.0} round trips/s",
        moved as f64 / took.as_secs_f64(),
        f64::from(rounds) / took.as_secs_f64()
    );

    handle
        .release_interface(INTERFACE)
        .expect("releasing the interface");
}
