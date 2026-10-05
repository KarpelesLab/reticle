//! A USB hub this compiler built, asked about by the **operating system's own
//! hub driver**.
//!
//! `tests/ip_library.rs` proves `ip/usb_hub` against a host model and a
//! transceiver model, in simulation, and that is where the interesting
//! assertions are — the class hook claiming six requests and stalling the
//! seventeen it did not offer, every bit of `wPortStatus`, a second
//! status-change report with the port state deliberately unchanged. What none
//! of it can prove is the only question a class layer really has to answer:
//! **does the host's driver bind?** A descriptor set can be exactly what USB
//! 2.0 chapter 11 describes and still not be what a kernel's hub driver looks
//! for, and no model of a host written from the same specification as the
//! device can tell you that.
//!
//! So this test asks the kernel:
//!
//! ```console
//! cargo test --features program --test usb_hub -- --ignored --nocapture
//! ```
//!
//! It is in three parts, and they fail for different reasons.
//!
//! 1. **The driver bound.** A device `1209:0001` exists whose `bDeviceClass`
//!    is `09h`, whose interface is held by the driver named `hub`, and whose
//!    `maxchild` sysfs attribute is 1 — which is the kernel's own record of
//!    `bNbrPorts` out of the hub descriptor it read and believed. Nothing is
//!    claimed and nothing is moved, so this half says exactly one thing: the
//!    kernel looked at this device's descriptors, asked it for its hub
//!    descriptor, and decided it was a hub with one port. That is the thing
//!    simulation cannot reach.
//! 2. **The descriptors are the ones the sources describe**, read off endpoint
//!    0 over usbfs: the device descriptor's class triple, the whole
//!    twenty-five byte configuration descriptor, the nine bytes of the hub
//!    descriptor of USB 2.0 §11.23.2.1, and the two bytes of the **standard**
//!    GET_STATUS that `ip/usb_hub/README.md` §7 is about — the one request
//!    endpoint 0 does not implement and Linux's `hub_configure` treats a
//!    failure of as fatal.
//! 3. **The port reports a device, loses it and reports it again.** This is
//!    the on-the-part half of the status-change assertion, done with the three
//!    class requests a host uses: GetPortStatus reads a connection,
//!    ClearPortFeature(PORT_POWER) takes the port's power away and the
//!    connection with it, C_PORT_CONNECTION is set and cleared, and
//!    SetPortFeature(PORT_POWER) brings both back with **C_PORT_CONNECTION set
//!    a second time**. A hub with a one-shot anywhere in it fails the last
//!    step, which is the defect `ip/usb_cdc_acm/README.md` §4 writes up.
//!
//!    Taking the port's power away is benign on this board and it is worth
//!    being clear why: the socket's real VBUS is the board's own
//!    `aux_vbus_en`, which a parameter of the bitstream holds and which no
//!    class request reaches, so what this switches is the hub's own idea of
//!    the port. The device on it stays attached throughout, which is what
//!    makes the connection come straight back.
//!
//! **Why usbfs and not sysfs.** Writing `1` and then `0` to the port's
//! `disable` attribute would make the kernel do the same power cycle, and it
//! needs root. These three requests are control transfers to endpoint 0 with a
//! device or "other" recipient, which usbfs allows without claiming any
//! interface — so they do not disturb the hub driver, and they are exactly
//! what `lsusb -v` already sends to print a hub's port status.
//!
//! **What this test would and would not catch.** It catches a descriptor set
//! the kernel refuses — a `bDeviceClass` or `bInterfaceClass` that is not
//! `09h`, an interface subclass that is not 0 or 1, more or fewer than one
//! endpoint, an endpoint that is not interrupt IN, a hub descriptor whose
//! `bDescLength` is wrong — because then no driver named `hub` holds the
//! interface and the first half fails naming it. It catches a class request
//! answered wrongly, and a change bit that does not set, does not clear, or
//! sets only once.
//!
//! It does **not** catch a hub that works on Linux and not on Windows or
//! macOS, since it asks one host. It does **not** establish that the
//! **interrupt endpoint** carried anything: reading endpoint `81h` would mean
//! taking the interface off the kernel's own hub driver, which this test
//! refuses to do, so the change bits are read through GetPortStatus and the
//! endpoint is proved in simulation only. `ip/usb_hub/README.md` §8 says what
//! one unplugging of the downstream cable would add to that and why nothing
//! here can do it.
//!
//! And it says nothing at all about a board that is not plugged in: it
//! **skips**, with a reason, because an unplugged board is not a broken
//! compiler.

#![cfg(feature = "program")]

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use rawusb::Context;

/// pid.codes' test pair, which is `ip/usb_hub`'s default `VID` / `PID`.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;

/// Long enough that a device answering at all answers inside it, short enough
/// that a device answering nothing does not hold the run up. It is a timeout
/// and **not an assertion**: nothing here is checked against a clock.
const TIMEOUT: Duration = Duration::from_millis(500);

/// The interface number, the endpoint address and the one port `ip/usb_hub`
/// declares.
const HUB_IFACE: u8 = 0;
const STATUS_EP: u8 = 0x81;
const PORT: u16 = 1;
/// `wMaxPacketSize` and `bInterval` of the status-change endpoint. Two bytes
/// for a bitmap that is one — `ip/usb_hub/README.md` §3 says why two and not
/// one — and twelve frames, which is that block's choice.
const STATUS_MAXPKT: u8 = 2;
const STATUS_INTERVAL: u8 = 12;

/// Feature selectors, USB 2.0 §11.24.2.
const FEAT_PORT_POWER: u16 = 8;
const FEAT_C_PORT_CONNECTION: u16 = 16;

/// `wPortStatus` and `wPortChange` bits, USB 2.0 §11.24.2.7.1 and §11.24.2.7.2.
const PORT_STAT_CONNECTION: u16 = 1 << 0;
const PORT_STAT_POWER: u16 = 1 << 8;
const PORT_CHG_CONNECTION: u16 = 1 << 0;

/// The configuration descriptor `ip/usb_hub` describes, written forwards from
/// USB 2.0 §11.23.1 — the arithmetic and not the answers, as
/// `tests/ip_library.rs` writes the same bytes from the same section and as
/// `ip/usb_hub/README.md` §3 tabulates them field by field.
///
/// Twenty-five bytes, and the thing to notice is what is **not** in them: a
/// hub's one class-specific descriptor is fetched by GetHubDescriptor and is
/// not carried in the configuration, so there is nothing between the interface
/// and its one endpoint.
fn expected_configuration() -> Vec<u8> {
    let mut iface: Vec<u8> = Vec::new();
    // INTERFACE 0: bInterfaceClass 09h, subclass 00h, protocol 00h — the
    // full-speed hub of §11.23.1, which has one alternate setting where a
    // high-speed one has two.
    iface.extend_from_slice(&[9, 4, HUB_IFACE, 0, 0, 0x09, 0x00, 0x00, 0]);
    // ENDPOINT 81h: the status change endpoint. bmAttributes 03h is interrupt.
    iface.extend_from_slice(&[7, 5, STATUS_EP, 0x03, STATUS_MAXPKT, 0, STATUS_INTERVAL]);
    // bNumEndpoints: the ENDPOINT descriptors after the interface, counted
    // along the chain of bLength fields the way a host does.
    iface[4] = count(&iface, 5);

    let total = u16::try_from(9 + iface.len()).expect("a short descriptor");
    let [lo, hi] = total.to_le_bytes();
    let mut want = vec![9, 2, lo, hi, count(&iface, 4), 1, 0, 0x80, 50];
    want.extend(iface);
    want
}

/// The nine bytes of the hub descriptor, written forwards from USB 2.0
/// §11.23.2.1.
///
/// `bDescLength` is **the arithmetic and not the answer**, because that is the
/// field a host and a hub most easily disagree about: the two fields at the end
/// are one bit a port plus a reserved bit 0, rounded up to a byte, so one port
/// is one byte each and seven plus two is nine.
fn expected_hub_descriptor() -> Vec<u8> {
    let ports: u8 = 1;
    let mask_bytes = (usize::from(ports) + 1 + 7) / 8;
    let mut d = vec![
        u8::try_from(7 + 2 * mask_bytes).expect("a short descriptor"),
        // bDescriptorType: 29h.
        0x29,
        ports,
    ];
    // wHubCharacteristics: D1:D0 = 01 individual port power switching, D2 = 0
    // not a compound device, D4:D3 = 10 no over-current protection, D6:D5 = 00
    // no transaction translator, D7 = 0 no port indicators.
    d.extend_from_slice(&0x0011u16.to_le_bytes());
    // bPwrOn2PwrGood, in 2 ms units, and bHubContrCurrent in mA.
    d.push(50);
    d.push(100);
    // DeviceRemovable: bit 0 reserved, bit n port n, 0 removable.
    d.extend(std::iter::repeat_n(0x00u8, mask_bytes));
    // PortPwrCtrlMask: §11.23.2.1 says every bit of it should be one.
    d.extend(std::iter::repeat_n(0xFFu8, mask_bytes));
    d
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

/// The sysfs directories of every USB device with this vendor and product
/// identifier, found by **asking the kernel** rather than by guessing a bus
/// and device number.
///
/// Returns every match, because finding two would mean two boards are plugged
/// in and the test should say so rather than pick one.
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

/// The driver the kernel bound to one interface of a device, as sysfs reports
/// it: the basename of `<device>/<device>:1.<n>/driver`.
fn interface_driver(device: &std::path::Path, interface: u8) -> Option<String> {
    let name = device.file_name()?.to_string_lossy().into_owned();
    let link = device.join(format!("{name}:1.{interface}")).join("driver");
    let target = fs::read_link(link).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

#[test]
#[ignore = "needs a USB hub built by this compiler attached: 1209:0001"]
fn a_usb_hub_this_compiler_built_is_bound_by_the_kernels_own_hub_driver() {
    // ------------------------------------------------------------------
    // Half one: the kernel decided this was a hub.
    // ------------------------------------------------------------------
    let devices = usb_devices(VID, PID);
    if devices.is_empty() {
        println!(
            "no {VID:04x}:{PID:04x} is attached, skipping; load \
             testdata/fpga/cynthion/usb_hub_target.v"
        );
        return;
    }
    assert_eq!(
        devices.len(),
        1,
        "two devices claim to be {VID:04x}:{PID:04x}: {devices:?}"
    );
    let device_dir = &devices[0];
    let field = |name: &str| {
        fs::read_to_string(device_dir.join(name))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };

    // `ip/usb_cdc_acm` and `ip/usb_hub` share this vendor and product pair,
    // and only one of them can be loaded at a time. Saying which is loaded is
    // more useful than an assertion about bytes a serial port would also fail.
    let class = field("bDeviceClass");
    if class != "09" {
        println!(
            "{VID:04x}:{PID:04x} is attached with bDeviceClass {class}, which is not a hub's \
             09h — some other design built on these sources is loaded. Skipping; load \
             testdata/fpga/cynthion/usb_hub_target.v"
        );
        return;
    }
    assert_eq!(
        field("bDeviceProtocol"),
        "00",
        "bDeviceProtocol 00h is a full-speed hub with no transaction translator \
         (USB 2.0 §11.23.1)"
    );
    assert_eq!(field("bNumInterfaces"), "1", "a hub has one interface");

    // **`maxchild` is the kernel's own record of `bNbrPorts`**, taken out of
    // the hub descriptor it fetched with GetHubDescriptor. A device whose
    // descriptors the hub driver refused has no `maxchild` at all, so this one
    // attribute says both that the driver bound and that it read and believed
    // the class-specific descriptor.
    assert_eq!(
        field("maxchild"),
        "1",
        "the kernel read bNbrPorts out of the hub descriptor and recorded one port"
    );

    // And it is the `hub` driver that holds the interface, not some other
    // driver that happens to bind to class 09h.
    match interface_driver(device_dir, HUB_IFACE) {
        Some(driver) => assert_eq!(
            driver,
            "hub",
            "interface {HUB_IFACE} of {} is held by `{driver}` and not by the hub driver",
            device_dir.display()
        ),
        None => println!(
            "  (sysfs does not name the driver for {})",
            device_dir.display()
        ),
    }
    println!(
        "the kernel bound `hub` to {} and recorded {} port(s)",
        device_dir.display(),
        field("maxchild")
    );

    // ------------------------------------------------------------------
    // Half two: the descriptors the part is reporting.
    // ------------------------------------------------------------------
    // Opening the device this way does not disturb the hub driver: no
    // interface is claimed, and everything below is a control transfer to
    // endpoint 0 with a device or "other" recipient, which is what `lsusb -v`
    // sends to print a hub's port status.
    let context = Context::new().expect("the USB subsystem is reachable, since sysfs named a device");
    let device = context
        .find_device(VID, PID)
        .expect("enumerating USB devices")
        .expect("the device sysfs just named");
    let handle = match device.open() {
        Ok(handle) => handle,
        Err(err) => {
            println!(
                "cannot open {VID:04x}:{PID:04x} over usbfs ({err}); the first half passed and \
                 the rest needs permission to send control transfers. Skipping the rest."
            );
            return;
        }
    };

    // The configuration descriptor: its header first, then the whole thing at
    // the length the header gave, which is what a host does.
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

    // The device descriptor's class triple, which is what makes the hub driver
    // match this device at all.
    let mut dev_desc = [0u8; 18];
    let n = handle
        .control_read(0x80, 0x06, 0x0100, 0, &mut dev_desc, TIMEOUT)
        .expect("the device descriptor");
    assert_eq!(n, 18, "eighteen bytes of device descriptor");
    assert_eq!(
        [dev_desc[4], dev_desc[5], dev_desc[6]],
        [0x09, 0x00, 0x00],
        "bDeviceClass, bDeviceSubClass and bDeviceProtocol (USB 2.0 §11.23.1)"
    );

    // GetHubDescriptor, USB 2.0 §11.24.2.5, asking for **fifteen** bytes and
    // getting nine.
    //
    // Fifteen is what Linux asks for — the whole of its own
    // `struct usb_hub_descriptor`, sized for its maximum port count — and nine
    // is what a one-port hub has. What makes the two work together is
    // `usb_ctrl_ep` capping a class data stage at `min(wLength, class_len)`
    // and a short packet ending a control read, which is the one place the
    // class hook's arithmetic is load-bearing for this block.
    let mut hub_desc = [0u8; 15];
    let n = handle
        .control_read(0xA0, 0x06, 0x2900, 0, &mut hub_desc, TIMEOUT)
        .expect("GetHubDescriptor is answered and not stalled");
    println!("hub descriptor ({n} bytes): {:02x?}", &hub_desc[..n]);
    assert_eq!(
        &hub_desc[..n],
        expected_hub_descriptor().as_slice(),
        "the hub descriptor of USB 2.0 §11.23.2.1, byte for byte"
    );

    // GetHubStatus, §11.24.2.6: four zeros. This hub has no local power supply
    // and no over-current detector, so neither the status nor the change can
    // ever be anything else.
    let mut hub_status = [0xFFu8; 4];
    let n = handle
        .control_read(0xA0, 0x00, 0, 0, &mut hub_status, TIMEOUT)
        .expect("GetHubStatus");
    assert_eq!(n, 4, "wHubStatus and wHubChange");
    assert_eq!(hub_status, [0, 0, 0, 0], "no local supply, no over-current");

    // The **standard** GET_STATUS of USB 2.0 §9.4.5, device recipient, which
    // `usb_ctrl_ep` does not implement and `usb_hub_req` claims on the class
    // hook. `ip/usb_hub/README.md` §7 is why a class block answers a standard
    // request and what the right fix is; Linux's `hub_configure` sends it
    // during hub probe and takes its failure path if it does not complete, so
    // a stall here is a hub that does not bind.
    let mut dev_status = [0xFFu8; 2];
    let n = handle
        .control_read(0x80, 0x00, 0, 0, &mut dev_status, TIMEOUT)
        .expect("the standard GET_STATUS a hub driver sends during probe");
    assert_eq!(n, 2, "wStatus is two bytes");
    assert_eq!(
        dev_status,
        [0, 0],
        "bus powered, remote wake-up not enabled, and bits 2..15 reserved"
    );

    // ------------------------------------------------------------------
    // Half three: the port reports a device, loses it, and reports it again.
    // ------------------------------------------------------------------
    let port_status = || {
        let mut buf = [0xFFu8; 4];
        let n = handle
            .control_read(0xA3, 0x00, 0, PORT, &mut buf, TIMEOUT)
            .expect("GetPortStatus");
        assert_eq!(n, 4, "wPortStatus and wPortChange");
        (
            u16::from_le_bytes([buf[0], buf[1]]),
            u16::from_le_bytes([buf[2], buf[3]]),
        )
    };
    let set_feature = |feature: u16| {
        handle
            .control_write(0x23, 0x03, feature, PORT, &[], TIMEOUT)
            .unwrap_or_else(|err| panic!("SetPortFeature({feature}): {err}"));
    };
    let clear_feature = |feature: u16| {
        handle
            .control_write(0x23, 0x01, feature, PORT, &[], TIMEOUT)
            .unwrap_or_else(|err| panic!("ClearPortFeature({feature}): {err}"));
    };

    let (status, change) = port_status();
    println!("wPortStatus {status:#06x}, wPortChange {change:#06x}");
    assert_ne!(
        status & PORT_STAT_POWER,
        0,
        "the kernel's own hub driver sent SetPortFeature(PORT_POWER), which is the \
         request this gateware answers that proves the driver is operating the port. \
         Without it, wHubCharacteristics D1:D0 is the field to look at — \
         ip/usb_hub/README.md section 3 says why it is 01 and not 1X."
    );
    if status & PORT_STAT_CONNECTION == 0 {
        println!(
            "the port reports no device on it, so the rest of this test has nothing to \
             watch appear and disappear. Load the design with VBUS_AUX = 1 and something \
             in the TARGET-A socket; testdata/fpga/cynthion/usb_hub_target.v's header says \
             what that switch is and why it has a parameter. Skipping the rest."
        );
        return;
    }

    // A clean slate: whatever change the kernel's own enumeration attempt left
    // behind is cleared, so that the three readings below are this test's.
    clear_feature(FEAT_C_PORT_CONNECTION);
    let (status, change) = port_status();
    assert_eq!(
        change & PORT_CHG_CONNECTION,
        0,
        "ClearPortFeature(C_PORT_CONNECTION) clears it"
    );
    assert_eq!(
        status & (PORT_STAT_POWER | PORT_STAT_CONNECTION),
        PORT_STAT_POWER | PORT_STAT_CONNECTION,
        "powered, with something on it"
    );

    // Take the port's power away. USB 2.0 §11.5.1.1 makes a powered-off port's
    // connection meaningless, so the connection goes and **the change says
    // so** — the first of the two reports this half is about.
    clear_feature(FEAT_PORT_POWER);
    let (status, change) = port_status();
    println!("powered off: wPortStatus {status:#06x}, wPortChange {change:#06x}");
    assert_eq!(status, 0, "a powered-off port reports nothing at all");
    assert_ne!(
        change & PORT_CHG_CONNECTION,
        0,
        "losing the connection is a connection change"
    );
    clear_feature(FEAT_C_PORT_CONNECTION);
    assert_eq!(
        port_status().1 & PORT_CHG_CONNECTION,
        0,
        "and the host clears it"
    );

    // And give it back. The device never went anywhere — the socket's real
    // VBUS is the board's own switch and no class request reaches it — so the
    // connection comes straight back, and **C_PORT_CONNECTION is set a second
    // time**. A hub with a one-shot anywhere in it fails here.
    set_feature(FEAT_PORT_POWER);
    let (status, change) = port_status();
    println!("powered on again: wPortStatus {status:#06x}, wPortChange {change:#06x}");
    assert_eq!(
        status & (PORT_STAT_POWER | PORT_STAT_CONNECTION),
        PORT_STAT_POWER | PORT_STAT_CONNECTION,
        "powered again, with the same device on it"
    );
    assert_ne!(
        change & PORT_CHG_CONNECTION,
        0,
        "THE SECOND CONNECTION CHANGE. A hub that reported one change and then held a \
         latch saying the host had been told would be silent here, and the host would go \
         on believing whatever it last heard — which is exactly the defect \
         ip/usb_cdc_acm/README.md section 4 writes up, measured on this same board."
    );

    // Leave the port as the kernel's own driver had it: powered, connected,
    // and with no change outstanding.
    clear_feature(FEAT_C_PORT_CONNECTION);
    let (status, change) = port_status();
    println!("left as found: wPortStatus {status:#06x}, wPortChange {change:#06x}");
    assert_eq!(change, 0, "nothing outstanding");
    assert_ne!(status & PORT_STAT_POWER, 0, "and the port still powered");
}
