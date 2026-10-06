//! A device the operating system reached **through a hub this compiler built**.
//!
//! `tests/ip_library.rs` proves `ip/usb/usb_proxy` against a host model and
//! three transceiver models, in simulation, and that is where the
//! interesting assertions are — the toggles on both buses, a data stage of
//! more than one packet, a STALL propagated, a port that forwards nothing
//! until it is reset. What none of it can prove is the only question a
//! proxy really has to answer: **does a kernel enumerate the device through
//! it?** A host model written from the same specification as the proxy can
//! agree with it about something they are both wrong about, which is the
//! sentence `ip/usb/usb_device_ulpi/README.md` §11 wrote before that block
//! had a board, and the one time it came true is written up in the same
//! section.
//!
//! So this test asks the kernel:
//!
//! ```console
//! cargo test --features program --test usb_proxy -- --ignored --nocapture
//! ```
//!
//! It needs `testdata/fpga/cynthion/usb_proxy_target.v` loaded **with
//! `VBUS_AUX = 1`** and something in the TARGET-A socket. That file's header
//! says what the switch is, why there is exactly one of it and why it is the
//! AUX port's and not the CONTROL port's.
//!
//! It is in four parts and they fail for different reasons.
//!
//! 1. **Our hub is there and the kernel bound its own driver**, which is
//!    `tests/usb_hub.rs`'s first half in one paragraph: `1209:0001` with
//!    `bDeviceClass` `09h`, the interface held by the driver named `hub`, and
//!    `maxchild` 1 — the kernel's own record of `bNbrPorts` out of the hub
//!    descriptor it fetched. Without this, nothing below means anything.
//! 2. **The port has a device on it**, read with GetPortStatus over usbfs. If
//!    it has not, this **skips** with a reason: a socket with no power in it is
//!    not a broken proxy.
//! 3. **The kernel enumerated that device, through the proxy.** This is the
//!    whole round, and it is one assertion: a **child** of our hub exists
//!    in sysfs. If the port reports a device and there is no child, the
//!    enumeration failed and this **fails**, because that is precisely the
//!    state `ip/usb/usb_hub/README.md` §8 quotes the kernel log of and
//!    precisely what `ip/usb/usb_proxy` was built to change.
//!
//! The child's `idVendor` and `idProduct` are asserted to be **different
//! from our hub's**, which is the one thing that says the descriptors the
//! kernel read are the real device's and not ours. With pass-through
//! addressing there is nothing of this project in them, and if there were
//! it would be a defect rather than a feature.
//! 4. **Control transfers still work through it, now.** The child's device
//!    descriptor and its whole configuration descriptor are read over usbfs
//!    — through our hub, through the relay, to the device and back — and
//!    compared with what sysfs says the kernel read at enumeration time.
//!    That is a hundred-odd bytes of multi-packet control traffic in both
//!    directions, with a status stage each way.
//!
//! Then, if the device declares a bulk IN endpoint and no kernel driver
//! holds its interface, one bulk read is attempted and **reported**. What
//! that can and cannot establish is below.
//!
//! **What this test would and would not catch.**
//!
//! It catches the whole of what the round claims: a proxy that does not
//! forward, one whose port reset does not reach the device so the PC's
//! SET_ADDRESS lands on a device that still has an old one, one whose data
//! stages are cut in the wrong place so a descriptor comes back wrong, and
//! one that answers with descriptors of its own rather than the device's.
//! Part 3 is a single boolean and it is the one that could not be reached
//! in simulation at all.
//!
//! It does **not** catch a proxy that works on Linux and not on Windows or
//! macOS, since it asks one host. It does **not** establish **which** device is
//! behind the port — that is whatever is in the socket, and the point of
//! pass-through addressing is that it does not matter — so the descriptor
//! assertions are self-consistency between usbfs and sysfs rather than against
//! expected bytes. A proxy that corrupted a descriptor **identically** on the
//! kernel's read and on this one would pass part 4, which is why part 3 is the
//! load-bearing half: a corrupted device descriptor does not enumerate.
//!
//! The **bulk** attempt asserts nothing and is printed, and that is
//! deliberate rather than lazy. A bulk IN to a device that has nothing to
//! say is NAKed for ever and a host turns that into a timeout, so a timeout
//! is the *expected* answer from a device with no outstanding data — and it
//! is indistinguishable from a device that was never reached. A `EPIPE`
//! would be a STALL, which would be the device refusing, and anything else
//! is an error worth seeing. So the result is reported with what each
//! outcome would mean, and what proves bytes move through this block is
//! part 4 and the five hundred thousand bytes the simulation moves through
//! `usb_proxy_moves_bytes_through_the_port`.
//!
//! And it says nothing at all about a board that is not plugged in: it
//! **skips**, with a reason, because an unplugged board is not a broken
//! compiler.

#![cfg(feature = "program")]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rawusb::Context;

/// pid.codes' test pair, which is the hub's default `VID` / `PID`.
const HUB_VID: u16 = 0x1209;
const HUB_PID: u16 = 0x0001;

/// The hub's one interface and its one port.
const HUB_IFACE: u8 = 0;
const PORT: u16 = 1;

/// `wPortStatus` bits, USB 2.0 §11.24.2.7.1.
const PORT_STAT_CONNECTION: u16 = 1 << 0;
const PORT_STAT_ENABLE: u16 = 1 << 1;
const PORT_STAT_POWER: u16 = 1 << 8;

/// Long enough that a device answering at all answers inside it, short enough
/// that one answering nothing does not hold the run up. It is a timeout and
/// **not an assertion**: nothing here is checked against a clock.
const TIMEOUT: Duration = Duration::from_millis(500);
/// The bulk attempt's own, which is shorter because a NAK for ever is the
/// expected answer and waiting it out twice is nothing but waiting.
const BULK_TIMEOUT: Duration = Duration::from_millis(200);

/// The sysfs directories of every USB device with this vendor and product
/// identifier, found by **asking the kernel** rather than by guessing a bus and
/// device number.
fn usb_devices(vid: u16, pid: u16) -> Vec<PathBuf> {
    let want = (format!("{vid:04x}"), format!("{pid:04x}"));
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let read = |field: &str| {
            fs::read_to_string(dir.join(field))
                .ok()
                .map(|s| s.trim().to_ascii_lowercase())
        };
        if read("idVendor").as_deref() == Some(want.0.as_str())
            && read("idProduct").as_deref() == Some(want.1.as_str())
        {
            found.push(dir);
        }
    }
    found.sort();
    found
}

/// One sysfs field of a device, trimmed, or the empty string.
fn field(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// The driver the kernel bound to one interface of a device, as sysfs reports
/// it.
fn interface_driver(device: &Path, interface: u8) -> Option<String> {
    let name = device.file_name()?.to_string_lossy().into_owned();
    let link = device.join(format!("{name}:1.{interface}")).join("driver");
    let target = fs::read_link(link).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

/// The devices sysfs says are **behind** this one.
///
/// A USB device's sysfs name is its topology: `7-5` is port 5 of bus 7's root
/// hub and `7-5.1` is port 1 of *that*. So a child of our hub is a directory
/// whose name is ours plus `.` and a port number, which is the kernel's own
/// statement that it enumerated something through the port — and is the thing
/// this whole test is for.
fn children(hub: &Path) -> Vec<PathBuf> {
    let Some(name) = hub.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let prefix = format!("{name}.");
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(child) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        // Only one level down: `7-5.1.2` is behind a hub behind our port and is
        // not a child of ours.
        let Some(rest) = child.strip_prefix(&prefix) else {
            continue;
        };
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            found.push(dir);
        }
    }
    found.sort();
    found
}

/// The endpoint descriptors of a configuration descriptor blob, as
/// `(bEndpointAddress, bmAttributes, wMaxPacketSize)`, walked along
/// the chain of `bLength` fields the way a host does.
fn endpoints(config: &[u8]) -> Vec<(u8, u8, u16)> {
    let mut at = 0;
    let mut found = Vec::new();
    while at + 1 < config.len() && config[at] >= 2 {
        let len = usize::from(config[at]);
        // ENDPOINT is bDescriptorType 5 and is seven bytes (USB 2.0 §9.6.6).
        if config[at + 1] == 5 && len >= 7 && at + 7 <= config.len() {
            found.push((
                config[at + 2],
                config[at + 3],
                u16::from_le_bytes([config[at + 4], config[at + 5]]),
            ));
        }
        at += len;
    }
    found
}

#[test]
#[ignore = "needs a USB proxy built by this compiler attached, with a device in its port"]
fn a_device_behind_a_hub_this_compiler_built_is_enumerated_by_the_kernel() {
    // ------------------------------------------------------------------
    // Part one: our hub is there and the kernel's own driver bound to it.
    // ------------------------------------------------------------------
    let hubs = usb_devices(HUB_VID, HUB_PID);
    if hubs.is_empty() {
        println!(
            "no {HUB_VID:04x}:{HUB_PID:04x} is attached, skipping; load \
             testdata/fpga/cynthion/usb_proxy_target.v with VBUS_AUX = 1"
        );
        return;
    }
    assert_eq!(
        hubs.len(),
        1,
        "two devices claim to be {HUB_VID:04x}:{HUB_PID:04x}: {hubs:?}"
    );
    let hub = &hubs[0];

    // `ip/usb/usb_cdc_acm`, `ip/usb/usb_hub` and `ip/usb/usb_proxy` share
    // this vendor and product pair and only one of them can be loaded at a
    // time. Saying which is more useful than an assertion about bytes a
    // serial port would also fail.
    let class = field(hub, "bDeviceClass");
    if class != "09" {
        println!(
            "{HUB_VID:04x}:{HUB_PID:04x} is attached with bDeviceClass {class}, which is not \
             a hub's 09h — some other design built on these sources is loaded. Skipping; \
             load testdata/fpga/cynthion/usb_proxy_target.v with VBUS_AUX = 1"
        );
        return;
    }
    assert_eq!(
        field(hub, "maxchild"),
        "1",
        "the kernel read bNbrPorts out of the hub descriptor and recorded one port"
    );
    match interface_driver(hub, HUB_IFACE) {
        Some(driver) => assert_eq!(
            driver,
            "hub",
            "interface {HUB_IFACE} of {} is held by `{driver}` and not by the hub driver",
            hub.display()
        ),
        None => println!("  (sysfs does not name the driver for {})", hub.display()),
    }
    println!(
        "the kernel bound `hub` to {} and recorded {} port(s)",
        hub.display(),
        field(hub, "maxchild")
    );

    // ------------------------------------------------------------------
    // Part two: the port has a device on it.
    // ------------------------------------------------------------------
    // Opening the hub this way does not disturb the hub driver: no interface is
    // claimed, and GetPortStatus is a control transfer to endpoint 0 with the
    // recipient "other", which is what `lsusb -v` already sends to print a
    // hub's port status.
    let context =
        Context::new().expect("the USB subsystem is reachable, since sysfs named a device");
    let device = context
        .find_device(HUB_VID, HUB_PID)
        .expect("enumerating USB devices")
        .expect("the hub sysfs just named");
    let handle = match device.open() {
        Ok(handle) => handle,
        Err(err) => {
            println!(
                "cannot open {HUB_VID:04x}:{HUB_PID:04x} over usbfs ({err}); the first part \
                 passed and the rest needs permission to send control transfers. Skipping."
            );
            return;
        }
    };

    let mut buf = [0xFFu8; 4];
    let n = handle
        .control_read(0xA3, 0x00, 0, PORT, &mut buf, TIMEOUT)
        .expect("GetPortStatus");
    assert_eq!(n, 4, "wPortStatus and wPortChange");
    let status = u16::from_le_bytes([buf[0], buf[1]]);
    let change = u16::from_le_bytes([buf[2], buf[3]]);
    println!("wPortStatus {status:#06x}, wPortChange {change:#06x}");
    assert_ne!(
        status & PORT_STAT_POWER,
        0,
        "the kernel's own hub driver sent SetPortFeature(PORT_POWER), which is what says it \
         is operating the port at all"
    );
    if status & PORT_STAT_CONNECTION == 0 {
        println!(
            "the port reports no device on it, so there is nothing for the proxy to forward \
             to. Load the design with VBUS_AUX = 1 and something in the TARGET-A socket; \
             testdata/fpga/cynthion/usb_proxy_target.v's header says what that switch is. \
             Skipping the rest."
        );
        return;
    }

    // ------------------------------------------------------------------
    // Part three: the kernel enumerated it, through the proxy.
    // ------------------------------------------------------------------
    // THIS IS THE WHOLE ROUND, AND IT IS ONE BOOLEAN
    //
    // A child of our hub in sysfs is the kernel saying it read a device
    // descriptor through the port, assigned an address, read a
    // configuration descriptor and configured the device.
    // `ip/usb/usb_hub/README.md` §8 is the kernel log of the state where
    // there is no child — four `device descriptor read/64, error -71` and
    // `unable to enumerate USB device` — which was the correct outcome of a
    // hub that forwarded nothing.
    let kids = children(hub);
    assert!(
        !kids.is_empty(),
        "the port reports a device ({status:#06x}) and the kernel enumerated nothing through \
         it: there is no child of {} in sysfs. That is the state ip/usb/usb_hub/README.md §8 \
         quotes the kernel log of, and it is what ip/usb/usb_proxy exists to change — so `dmesg` \
         should be read here rather than this message. Look for \
         `device descriptor read/64, error -71` and `unable to enumerate USB device`, and \
         for the T14 console's own view: byte 1 bit 4 is `saw_proxied` and byte 3's low \
         five bits count the SETUPs the PC sent to something behind the port.",
        hub.display()
    );
    assert_eq!(
        kids.len(),
        1,
        "a one-port hub has at most one child: {kids:?}"
    );
    let kid = &kids[0];
    let kid_vid = field(kid, "idVendor");
    let kid_pid = field(kid, "idProduct");
    println!(
        "the kernel enumerated {} behind our port: {kid_vid}:{kid_pid}, \
         bcdDevice {}, class {}, speed {} Mbit/s, bMaxPacketSize0 {}, \
         devnum {}",
        kid.display(),
        field(kid, "bcdDevice"),
        field(kid, "bDeviceClass"),
        field(kid, "speed"),
        field(kid, "bMaxPacketSize0"),
        field(kid, "devnum")
    );

    // **The descriptors are the device's and not ours.** With pass-through
    // addressing there is nothing of this project in them, so a child reporting
    // our own vendor and product pair would mean the proxy was answering for
    // the device rather than forwarding to it — which is a defect and not a
    // feature.
    assert!(
        kid_vid != format!("{HUB_VID:04x}") || kid_pid != format!("{HUB_PID:04x}"),
        "the device behind the port reports {kid_vid}:{kid_pid}, which is the **hub's** own \
         pair: something is answering with our descriptors instead of forwarding to the \
         device's"
    );
    // §11.5.1: a port reaches Enabled only by being reset, and the kernel
    // reset it to enumerate through it — so the reset `usb_proxy_dn` drove
    // at the real device completed, which is what pass-through addressing
    // depends on.
    assert_ne!(
        status & PORT_STAT_ENABLE,
        0,
        "a port the kernel enumerated through is Enabled, which USB 2.0 §11.5.1 only \
         reaches by a reset completing"
    );

    // ------------------------------------------------------------------
    // Part four: control transfers through it, now rather than at boot.
    // ------------------------------------------------------------------
    let kid_vid_n = u16::from_str_radix(&kid_vid, 16).expect("sysfs writes hex");
    let kid_pid_n = u16::from_str_radix(&kid_pid, 16).expect("sysfs writes hex");
    let kid_device = context
        .find_device(kid_vid_n, kid_pid_n)
        .expect("enumerating USB devices")
        .expect("the device sysfs just named behind our port");
    let kid_handle = match kid_device.open() {
        Ok(handle) => handle,
        Err(err) => {
            println!(
                "cannot open {kid_vid}:{kid_pid} over usbfs ({err}); parts one to three \
                 passed, which is the round's claim. Skipping the rest."
            );
            return;
        }
    };

    let mut dev_desc = [0u8; 18];
    let n = kid_handle
        .control_read(0x80, 0x06, 0x0100, 0, &mut dev_desc, TIMEOUT)
        .expect("the device descriptor, read through the proxy");
    assert_eq!(n, 18, "eighteen bytes of device descriptor");
    println!("device descriptor through the proxy: {dev_desc:02x?}");
    // Self-consistency with what the kernel read when it enumerated the
    // device, which is the strongest thing available without knowing what
    // the device is.
    assert_eq!(
        u16::from_le_bytes([dev_desc[8], dev_desc[9]]),
        kid_vid_n,
        "idVendor, as the kernel read it at enumeration and as it reads now"
    );
    assert_eq!(
        u16::from_le_bytes([dev_desc[10], dev_desc[11]]),
        kid_pid_n,
        "idProduct likewise"
    );
    assert_eq!(
        dev_desc[7].to_string(),
        field(kid, "bMaxPacketSize0"),
        "bMaxPacketSize0 — which pass-through addressing is what makes agree, because the \
         byte the kernel believed is the byte the device sent"
    );
    assert_eq!(
        dev_desc[17].to_string(),
        field(kid, "bNumConfigurations"),
        "bNumConfigurations"
    );

    // The whole configuration descriptor set: the header first and then the
    // length the header gave, which is what a host does and which is as many
    // packets as the device's `bMaxPacketSize0` takes.
    let mut header = [0u8; 9];
    let n = kid_handle
        .control_read(0x80, 0x06, 0x0200, 0, &mut header, TIMEOUT)
        .expect("the configuration descriptor's header, through the proxy");
    assert_eq!(n, 9, "nine bytes of configuration descriptor header");
    let total = usize::from(u16::from_le_bytes([header[2], header[3]]));
    let mut config = vec![0u8; total];
    let n = kid_handle
        .control_read(0x80, 0x06, 0x0200, 0, &mut config, TIMEOUT)
        .expect("the whole configuration descriptor, through the proxy");
    config.truncate(n);
    assert_eq!(
        n, total,
        "the whole configuration descriptor came back: {n} bytes of the {total} its own \
         wTotalLength asked for"
    );
    println!("configuration descriptor through the proxy ({n} bytes): {config:02x?}");
    assert_eq!(
        config[..9],
        header,
        "the header read on its own and the header of the whole read agree, which is the \
         same data stage cut in two different places"
    );

    let eps = endpoints(&config);
    println!("endpoints the device declares: {eps:02x?}");

    // ------------------------------------------------------------------
    // And one bulk transaction, reported and not asserted.
    // ------------------------------------------------------------------
    // The module comment says why this asserts nothing: a bulk IN to a device
    // with nothing to say is NAKed for ever, a host turns that into a timeout,
    // and a timeout is therefore the *expected* answer — and indistinguishable
    // from a device that was never reached.
    let bulk_in = eps
        .iter()
        .find(|(addr, attrs, _)| addr & 0x80 != 0 && attrs & 0x03 == 0x02);
    let Some(&(endpoint, _, maxpkt)) = bulk_in else {
        println!("the device declares no bulk IN endpoint, so there is nothing to try");
        return;
    };
    // Which interface the endpoint belongs to is the descriptor's own ordering,
    // and claiming the wrong one fails cleanly, so interface 0 is tried and the
    // failure is printed rather than asserted.
    if let Some(driver) = interface_driver(kid, 0) {
        println!(
            "interface 0 of the device behind the port is held by `{driver}`; not taking it \
             away from the kernel to try a bulk read"
        );
        return;
    }
    if let Err(err) = kid_handle.claim_interface(0) {
        println!("cannot claim interface 0 of the device behind the port ({err}); not trying");
        return;
    }
    // A bulk **OUT** first, if the device has one, because that direction
    // *is* assertable: an OUT that comes back `Ok` was acknowledged by the
    // device, and nothing but the device can acknowledge it. The proxy
    // holds the packet, NAKs the host, forwards it, and gives the host the
    // device's own answer on the retry, which is §7 of
    // `ip/usb/usb_proxy/README.md`.
    //
    // The eight bytes are a libgreat command header — class 0, verb 0,
    // which on a GreatFET is `read_board_id` — and are **not** asserted to
    // produce anything: what is being measured is the handshake, and a
    // device that does not know the bytes still acknowledges a bulk OUT it
    // had room for.
    let bulk_out = eps
        .iter()
        .find(|(addr, attrs, _)| addr & 0x80 == 0 && attrs & 0x03 == 0x02);
    if let Some(&(out_ep, _, _)) = bulk_out {
        let command = [0u8; 8];
        match kid_handle.bulk_write(out_ep, &command, BULK_TIMEOUT) {
            Ok(sent) => println!(
                "a bulk OUT of {sent} byte(s) on endpoint {out_ep:#04x} was **acknowledged by \
                 the device**, through the proxy: bytes moved downstream"
            ),
            Err(err) => println!(
                "a bulk OUT on endpoint {out_ep:#04x} did not complete: {err}. A timeout is the \
                 device NAKing for want of room, a broken pipe is it stalling the endpoint, and \
                 either is the device's own answer carried back."
            ),
        }
    }

    let mut data = vec![0u8; usize::from(maxpkt)];
    match kid_handle.bulk_read(endpoint, &mut data, BULK_TIMEOUT) {
        Ok(got) => println!(
            "a bulk IN on endpoint {endpoint:#04x} brought {got} byte(s) through the proxy: \
             {:02x?}",
            &data[..got]
        ),
        Err(err) => println!(
            "a bulk IN on endpoint {endpoint:#04x} did not complete: {err}. A timeout here is \
             the expected answer from a device with no outstanding data — it NAKs, and a host \
             turns a NAK for ever into a timeout — and is not evidence either way. A broken \
             pipe would be the device stalling the endpoint, which would be the device \
             refusing and the proxy carrying the refusal."
        ),
    }
    let _ = kid_handle.release_interface(0);
}
