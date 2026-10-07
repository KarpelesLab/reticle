//! SHA-256 and ChaCha20 **on a part**, checked against the host's own tools.
//!
//! `tests/ip_library.rs` proves both blocks of `ip/crypto/` against
//! FIPS 180-4, RFC 8439 and `purecrypto`, and
//! `a_crypto_console_answers_through_the_transceiver_that_is_on_the_board`
//! proves the whole console through a host model and a transceiver model —
//! all of it in simulation. What none of that can prove is the thing both
//! crypto READMEs said they were not doing: that the **silicon** computes
//! these functions, at a clock place and route closed, with the carry-less
//! adders this backend builds.
//!
//! So this test asks the part, and asks the host's own `sha256sum` what the
//! answer should have been:
//!
//! ```console
//! cargo test --features program --test usb_crypto_console -- --ignored --nocapture
//! ```
//!
//! It is in four parts, and they fail for different reasons.
//!
//! 1. **The port is there and it is this design.** A `/dev/ttyACM*` whose
//!    USB parent is `1209:0001`, held by `cdc_acm`, that answers `?` with
//!    `crypto_console`'s help line. The last of those is what distinguishes
//!    this bitstream from `usb_cdc_uart.v`, which has the same identifiers
//!    and would echo the `?` straight back.
//! 2. **The digests are right**, compared against `sha256sum` run as a
//!    subprocess over the same bytes. That is the headline: the only
//!    SHA-256 in the comparison that this project wrote is the one on the
//!    part.
//! 3. **The keystream is right**, compared against the hexdump RFC 8439
//!    §2.4.2 prints, and the digest *of* that keystream against `sha256sum`
//!    over the bytes of that hexdump — which is the two cores in series
//!    with a published value at each end.
//! 4. **How fast it is**, two ways, which is the number neither simulation
//!    nor arithmetic can produce. `h` sends its message over USB, so it
//!    measures the **link**; `Z` and `X` make their bytes on the device, so
//!    they measure the **cores**. Both figures are *printed* and neither is
//!    asserted, because `CLAUDE.md` is explicit that a throughput figure
//!    belongs in a report and never in an assertion: CI runs on slower
//!    machines and a number of seconds is not a property of this design.
//!
//! **Why `sha256sum` and not a reference implementation in this file.** A
//! second SHA-256 written here would be written by whoever wrote the first
//! one, from the same reading of the same standard, and two implementations
//! with one author agreeing is not a measurement. `sha256sum` is somebody
//! else's code, shipped by the operating system, and it is also what a
//! person checking this by hand would reach for. The ChaCha20 side has no
//! equivalent tool, so its oracle is RFC 8439's own hexdump — the authority
//! rather than a second implementation — and `ip/crypto/chacha20/README.md`
//! §7 says where `purecrypto` agreed with it.
//!
//! **What this test would and would not catch.** It catches a wrong digest
//! or a wrong keystream byte from the silicon, which is the whole point, and
//! it would catch a timing path that does not close in the only way a
//! failure like that shows at this level: as a wrong answer, not as a
//! report. It catches the console hanging, as a read that returns nothing.
//! It catches the wrong bitstream being loaded, because `?` names this one.
//!
//! It does **not** separate a wrong answer's causes: a mis-mapped lookup
//! table, an unrouted wire and a path that misses 16.67 ns all look the same
//! from here, and telling them apart needs the LEDs
//! `testdata/fpga/cynthion/usb_crypto_console.v`'s header describes and a
//! narrower design. It does **not** establish anything about what the cores
//! **leak** — not power, not electromagnetic emission, not gate delay — and
//! `ip/crypto/sha256/README.md` §5 is the list of what remains undefended;
//! a correct answer off a real part is not a side-channel result and must
//! not be read as one. And it says nothing about a board that is not
//! plugged in: it **skips**, with a reason.

#![cfg(feature = "program")]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rawusb::Context;

/// pid.codes' test pair, which is `ip/usb/usb_cdc_acm`'s default `VID` / `PID`.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;

/// The help line `crypto_console`'s string table holds, which is also what
/// the port says the moment it is configured. It is the fingerprint of this
/// bitstream: no other design on this board answers `?` with it.
const HELP: &str = "H t|h x|E x|e n|Z n|X n|K x|N x|C x|k|n|c|?";

/// The key, nonce and counter `crypto_console` comes out of reset holding,
/// which are RFC 8439 §2.4.2's.
const RESET_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const RESET_NONCE: &str = "000000000000004a00000000";
const RESET_COUNTER: &str = "00000001";

/// The 128 bytes of keystream RFC 8439 §2.4.2 prints beside its ciphertext,
/// for that key, that nonce and block counter 1. Two blocks, so a stream
/// that does not carry the counter from 1 to 2 gets the second half wrong.
const KEYSTREAM_2_4_2: &str = concat!(
    "224f51f3401bd9e12fde276fb8631ded8c131f823d2c06e27e4fcaec9ef3cf78",
    "88a3b0aa372600a92b57974cded2b9334794cba40c63e34cdea212c4cf07d41b",
    "769a6749f3f630f4122cafe28ec4dc47e26d4346d70b98c73f3e9c53ac40c594",
    "5398b6eda1a832c89c167eacd901d7e2bf363740373201aa188fbbce83991c4ed",
);

/// RFC 8439 Appendix A.1 vector #1: the all-zero key, the all-zero nonce,
/// block counter 0, one block.
const KEYSTREAM_A1_1: &str = concat!(
    "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7",
    "da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586",
);

/// How long a read of the terminal may block with nothing arriving before
/// the test gives up on the device.
///
/// It is a **timeout and not an assertion**: nothing here is compared
/// against a clock. A `Z` of sixty-four mebibytes is two seconds of silence
/// and then one line, so the budget has to be generous enough for the
/// longest command this file sends and no tighter.
const QUIET: Duration = Duration::from_secs(60);

/// `wMaxPacketSize` of the bulk pair, which is also the largest read this
/// test does in one call.
const MAX_PACKET: usize = 64;

/// The terminal device the kernel gave this USB device, found by **asking
/// the kernel** rather than by guessing a number.
///
/// The number is not fixed and cannot be: a Cynthion's own Apollo debugger
/// is itself a CDC ACM device and is usually `ttyACM0`. What is fixed is the
/// relation — `/sys/class/tty/ttyACMn/device` is the USB *interface* the
/// port belongs to and its parent is the USB device — so this walks that and
/// reads the vendor and product identifiers off it. `tests/usb_cdc_acm.rs`
/// has the same walk, for the same device, and says the same thing about why.
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
fn interface_driver(port: &Path) -> Option<String> {
    let name = port.file_name()?.to_string_lossy().into_owned();
    let driver = PathBuf::from("/sys/class/tty")
        .join(name)
        .join("device/driver");
    let target = fs::read_link(driver).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

/// The console, over an open terminal.
struct Console {
    port: fs::File,
    path: String,
}

impl Console {
    /// Opens the terminal and puts it in the one mode this protocol needs.
    ///
    /// **Why `stty` and not an ioctl.** Configuring a terminal means
    /// `tcsetattr`, which means `libc` and `unsafe`, and this crate has
    /// neither; `stty` is the operating system's own tool for it and is what
    /// a person does at a prompt anyway. `tests/usb_cdc_acm.rs` has the
    /// longer form of that argument.
    ///
    /// `raw` is the part that matters here and is not a nicety: a terminal
    /// that is not raw translates carriage returns, which are this
    /// protocol's line terminator, and strips or expands characters inside a
    /// hex payload. `-echo` so the terminal layer does not send our own
    /// bytes back at us and make a dead device look alive. `clocal` so a
    /// carrier this device reported and then dropped could not hang the
    /// terminal up mid-transfer — it cannot, since `serial_state` is a
    /// constant, and `ip/usb/usb_cdc_acm/README.md` §5 says what `clocal`
    /// does and does not reach on this driver. The baud rate is arbitrary:
    /// there is no serial line inside this design, so nothing divides
    /// anything, and `stty` merely insists on a number.
    fn open(path: &str) -> Result<Console, String> {
        let stty = Command::new("stty")
            .args([
                "-F", path, "115200", "raw", "-echo", "clocal", "min", "0", "time", "100",
            ])
            .status()
            .map_err(|e| format!("cannot run `stty` ({e})"))?;
        if !stty.success() {
            return Err(format!(
                "`stty -F {path}` exited {stty}; membership of the group that owns {path} is \
                 usually why"
            ));
        }
        let port = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("opening {path}: {e}"))?;
        Ok(Console {
            port,
            path: path.to_owned(),
        })
    }

    /// Anything the port has to say right now, with a short budget.
    ///
    /// The console prints its help line the moment the host configures it,
    /// so a freshly enumerated device has a banner waiting and one that has
    /// already been read does not. Nothing is asserted about which: the
    /// banner is reported if it is there, and `?` below asks for the same
    /// line on purpose so that the identification does not depend on
    /// whether anything opened the port first.
    fn drain(&mut self) -> String {
        let mut got = Vec::new();
        let mut buf = [0u8; MAX_PACKET];
        for _ in 0..2 {
            match self.port.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&got).trim_end().to_owned()
    }

    /// One line typed at the console, and the one line it answers, with the
    /// time the whole exchange took.
    ///
    /// Returns the answer without its CR LF. The elapsed time is for the
    /// caller's report and is never compared against anything here.
    fn ask(&mut self, line: &str) -> Result<(String, Duration), String> {
        let started = Instant::now();
        self.port
            .write_all(line.as_bytes())
            .and_then(|()| self.port.write_all(b"\r"))
            .and_then(|()| self.port.flush())
            .map_err(|e| format!("writing {} bytes to {}: {e}", line.len() + 1, self.path))?;

        let mut got: Vec<u8> = Vec::new();
        let mut buf = [0u8; MAX_PACKET];
        loop {
            match self.port.read(&mut buf) {
                Ok(0) => {
                    if started.elapsed() > QUIET {
                        return Err(format!(
                            "{} said nothing for {QUIET:?} after a {} byte line; LED 2 lit and \
                             LED 4 dark on the board means the line arrived and the console is \
                             stuck waiting for a core. Got so far: {:?}",
                            self.path,
                            line.len() + 1,
                            String::from_utf8_lossy(&got)
                        ));
                    }
                }
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.ends_with(b"\r\n") {
                        got.truncate(got.len() - 2);
                        return Ok((
                            String::from_utf8_lossy(&got).into_owned(),
                            started.elapsed(),
                        ));
                    }
                }
                Err(e) => return Err(format!("reading {}: {e}", self.path)),
            }
        }
    }
}

/// What the host's own `sha256sum` says of these bytes.
///
/// Spawned rather than linked, because this crate has no dependencies and
/// because the point of the comparison is that the other implementation is
/// **somebody else's**. Returns `None` when the tool is not there, which is
/// a reason to skip a comparison and not to fail one.
fn host_digest(bytes: &[u8]) -> Option<String> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Written in a thread-free way on purpose: the payloads here are at most
    // a few mebibytes and a pipe holds enough that `sha256sum` is never
    // blocked on a full buffer while this writes. A payload large enough to
    // need a thread would be a payload this test should not be timing.
    child.stdin.take()?.write_all(bytes).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.split_whitespace().next().map(str::to_owned)
}

/// A byte string as lower-case hexadecimal, which is what this protocol
/// reads and writes.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).expect("a nibble"));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("a nibble"));
    }
    out
}

/// Hexadecimal back to bytes, for the published keystreams above.
fn unhex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|b| {
            char::from(b)
                .to_digit(16)
                .map(|v| u8::try_from(v).expect("a nibble"))
                .expect("a hex digit")
        })
        .collect();
    assert!(digits.len() % 2 == 0, "an even number of hex digits");
    digits.chunks(2).map(|pair| (pair[0] << 4) | pair[1]).collect()
}

/// The bytes `crypto_pattern` makes in `tests/ip_library.rs`: byte *i* is
/// `(i * 7 + 13) & 0xff`, which is neither zeros nor text and is cheap to
/// restate on both sides. The same rule, so a length that is interesting
/// there is the same bytes here.
fn pattern(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| u8::try_from((i * 7 + 13) & 0xff).expect("a byte"))
        .collect()
}

#[test]
#[ignore = "needs a Cynthion holding testdata/fpga/cynthion/usb_crypto_console.v: 1209:0001"]
fn sha256_and_chacha20_answer_a_host_from_a_real_part() {
    // ------------------------------------------------------------------
    // Part one: the port is there, and it is this design.
    // ------------------------------------------------------------------
    let ports = tty_ports(VID, PID);
    if ports.is_empty() {
        // Say which of the two things is wrong, because they mean different
        // things: no device at all is an unplugged or differently programmed
        // board, and a device with no terminal is a descriptor set the
        // kernel refused.
        match Context::new().and_then(|c| c.find_device(VID, PID)) {
            Ok(Some(_)) => panic!(
                "{VID:04x}:{PID:04x} is attached and the kernel gave it no /dev/ttyACM*: \
                 `cdc_acm` did not bind. `dmesg` will say why, and \
                 ip/usb/usb_cdc_acm/README.md sections 2 and 4 say what each descriptor is for."
            ),
            Ok(None) => {
                println!(
                    "no {VID:04x}:{PID:04x} is attached, skipping; load \
                     testdata/fpga/cynthion/usb_crypto_console.v"
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
    let path = ports[0].to_string_lossy().into_owned();
    println!("the kernel gave {VID:04x}:{PID:04x} the terminal {path}");
    match interface_driver(&ports[0]) {
        Some(driver) => assert_eq!(
            driver, "cdc_acm",
            "{path} is held by `{driver}` and not by the class driver"
        ),
        None => println!("  (sysfs does not name the driver for {path})"),
    }

    let mut console = match Console::open(&path) {
        Ok(console) => console,
        Err(why) => {
            println!("{why}; skipping");
            return;
        }
    };

    // The banner, if nothing has read it yet. Reported and not asserted:
    // `ip/usb/usb_cdc_acm` sends one notification per open but the console
    // prints its help line once per **configuration**, so whether it is
    // still there depends on what opened the port before this ran.
    let banner = console.drain();
    if banner.is_empty() {
        println!("no banner was waiting (something has read this port since it enumerated)");
    } else {
        println!("banner: {banner:?}");
    }

    // And the line that says which bitstream this is. `usb_cdc_uart.v` has
    // the same identifiers and would send the `?` straight back.
    let (help, _) = console.ask("?").expect("the console answers `?`");
    assert_eq!(
        help, HELP,
        "`?` should answer crypto_console's help line; a design that echoes \
         is testdata/fpga/cynthion/usb_cdc_uart.v and not this one"
    );
    println!("?  -> {help}");

    // The key, the nonce and the counter the part came up holding.
    for (command, want, what) in [
        ("k", RESET_KEY, "the key"),
        ("n", RESET_NONCE, "the nonce"),
        ("c", RESET_COUNTER, "the counter"),
    ] {
        let (got, _) = console.ask(command).expect("the console answers");
        assert_eq!(got, want, "{what} out of reset");
        println!("{command}  -> {got}");
    }

    // ------------------------------------------------------------------
    // Part two: the digests, against the host's own sha256sum.
    // ------------------------------------------------------------------
    let Some(_) = host_digest(b"") else {
        println!("`sha256sum` is not on this host; the digests cannot be checked, skipping");
        return;
    };

    // Typed as text, which is the form a person uses. Nothing here contains
    // a carriage return or a leading space, which is the whole of what the
    // text form cannot carry.
    for text in ["abc", "hello, world", "The quick brown fox jumps over it."] {
        let (got, _) = console.ask(&format!("H {text}")).expect("a digest");
        let want = host_digest(text.as_bytes()).expect("sha256sum");
        assert_eq!(got, want, "the part's digest of {text:?}");
        println!("H {text}\n  -> {got}\n  sha256sum agrees");
    }

    // As hex, at every length that reaches a different branch of
    // FIPS 180-4 §5.1.1's padding: nothing at all, one byte, 55 (no zeros),
    // 56 (the length field has no room, so the padding spills into a whole
    // extra block), 57 to 63 (the spill with the zeros wrapping a boundary),
    // 64 (a whole block of message and then a whole block of padding), and
    // three past two blocks. That is the table
    // `sha256_pads_every_length_purecrypto_was_asked_about` drives in
    // simulation, now driven at silicon.
    let lengths = [
        0usize, 1, 2, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 119, 120, 128, 200,
    ];
    for n in lengths {
        let bytes = pattern(n);
        let (got, _) = console.ask(&format!("h {}", hex(&bytes))).expect("a digest");
        let want = host_digest(&bytes).expect("sha256sum");
        assert_eq!(got, want, "the part's digest of {n} pattern byte(s)");
    }
    println!(
        "h: {} lengths through the padding, every one agreeing with sha256sum \
         (0, 1, 2, 55..=64, 65, 119, 120, 128, 200)",
        lengths.len()
    );

    // Zero bytes made on the device, which crosses the OUT endpoint not at
    // all. The digest is the same function of the same bytes either way, so
    // a `Z` that disagreed with an `h` of the same zeros would be the
    // on-device source and not the hash.
    for n in [0usize, 1, 55, 56, 64, 1000] {
        let (got, _) = console.ask(&format!("Z {n:x}")).expect("a digest");
        let want = host_digest(&vec![0u8; n]).expect("sha256sum");
        assert_eq!(got, want, "the part's digest of {n} zero byte(s)");
    }
    println!("Z: 0, 1, 55, 56, 64 and 1000 zero bytes made on the device, all agreeing");

    // ------------------------------------------------------------------
    // Part three: the keystream, against RFC 8439's own hexdump.
    // ------------------------------------------------------------------
    let ks = unhex(KEYSTREAM_2_4_2);
    assert_eq!(ks.len(), 128, "RFC 8439 2.4.2 prints two blocks");

    let (got, _) = console.ask("e 80").expect("a keystream");
    assert_eq!(
        got,
        hex(&ks),
        "RFC 8439 2.4.2's keystream, 128 bytes over two blocks, off the part"
    );
    println!("e 80\n  -> {got}\n  RFC 8439 2.4.2 agrees");

    // The same keystream reached the other way: exclusive-or with zero is
    // the identity, so `E` of zeros is `e`. And `E` of the pattern is the
    // keystream exclusive-ored with it, which is the only assertion here
    // that touches the exclusive-or itself.
    let plain = pattern(32);
    let (got, _) = console.ask(&format!("E {}", hex(&plain))).expect("a cipher");
    let want: Vec<u8> = plain
        .iter()
        .zip(ks.iter())
        .map(|(a, b)| a ^ b)
        .collect();
    assert_eq!(got, hex(&want), "the exclusive-or of the keystream");
    println!("E: 32 pattern bytes enciphered, against RFC 8439's keystream, byte for byte");

    // And the two cores in series: the digest of the keystream. The
    // expectation is `sha256sum` over the bytes of RFC 8439's hexdump, so
    // both ends of this command are published values and nothing this
    // project wrote is on either side of the comparison.
    for n in [64usize, 128] {
        let (got, _) = console.ask(&format!("X {n:x}")).expect("a digest");
        let want = host_digest(&ks[..n]).expect("sha256sum");
        assert_eq!(
            got, want,
            "the digest of {n} bytes of keystream, both cores in series"
        );
        println!("X {n:x} -> {got}");
    }

    // A key typed at it, and RFC 8439 Appendix A.1 vector #1.
    for (line, what) in [
        (
            "K 0000000000000000000000000000000000000000000000000000000000000000",
            "the key",
        ),
        ("N 000000000000000000000000", "the nonce"),
        ("C 00000000", "the counter"),
    ] {
        let (got, _) = console.ask(line).expect("an acknowledgement");
        assert_eq!(got, "OK", "{what} was accepted");
    }
    let (got, _) = console.ask("e 40").expect("a keystream");
    assert_eq!(
        got,
        hex(&unhex(KEYSTREAM_A1_1)),
        "RFC 8439 A.1 vector 1, under a key the host typed"
    );
    println!("e 40 under an all-zero key -> RFC 8439 A.1 vector 1, byte for byte");

    // The five ways a line can be wrong, and that the console recovers from
    // each — which is the half of error handling that is easy to get wrong
    // and that every assertion above would pass without.
    for bad in ["Q", "h 6", "h 6g", "C 0000", "Z"] {
        let (got, _) = console.ask(bad).expect("an answer");
        assert_eq!(got, "ERR", "`{bad}` is not a line this console can answer");
    }
    let (got, _) = console.ask("H abc").expect("a digest");
    assert_eq!(
        got,
        host_digest(b"abc").expect("sha256sum"),
        "and it still works after five bad lines"
    );
    println!("ERR on five malformed lines, and a correct digest after them");

    // ------------------------------------------------------------------
    // Part four: how fast, and what the bottleneck is.
    // ------------------------------------------------------------------
    // Printed, never asserted. `CLAUDE.md`: a throughput figure belongs in a
    // report and never in an assertion, because CI runs on slower machines
    // and a number of seconds is not a property of this design.
    println!();
    println!("--- throughput, measured end to end, asserted nowhere ---");

    // The link. Every message byte crosses USB as two hex digits, so the
    // bytes on the wire are twice the bytes hashed and both numbers are
    // reported: the first is what this protocol costs, the second is what
    // the endpoint moved.
    for bytes in [16usize * 1024, 256 * 1024] {
        let message = pattern(bytes);
        let line = format!("h {}", hex(&message));
        let (got, took) = console.ask(&line).expect("a digest");
        assert_eq!(
            got,
            host_digest(&message).expect("sha256sum"),
            "{bytes} bytes streamed over the link and hashed on the part"
        );
        let secs = took.as_secs_f64();
        println!(
            "h, {bytes} byte message: {took:?} -> {:.0} message bytes/s, \
             {:.0} bytes/s on the wire",
            bytes as f64 / secs,
            (line.len() + 1) as f64 / secs
        );
    }

    // The cores. `Z` is SHA-256 alone and `X` is ChaCha20 feeding SHA-256,
    // and neither sends a message byte over USB — so the difference between
    // these and the figures above is the whole of the answer to "what is the
    // bottleneck".
    //
    // The digests are **not** checked at these sizes and that is said
    // plainly: `Z` is checkable at any length because zero bytes are, and it
    // is checked; a sixty-four mebibyte keystream would need `purecrypto`
    // out of tree, and the lengths RFC 8439 publishes are checked above
    // instead.
    for bytes in [1usize << 20, 1 << 26] {
        let (got, took) = console.ask(&format!("Z {bytes:x}")).expect("a digest");
        let secs = took.as_secs_f64();
        let want = host_digest(&vec![0u8; bytes]).expect("sha256sum");
        assert_eq!(got, want, "{bytes} zero bytes hashed on the part");
        println!(
            "Z, {bytes} bytes on the device: {took:?} -> {:.0} bytes/s hashed \
             ({:.3} bytes/clock at 60 MHz)",
            bytes as f64 / secs,
            bytes as f64 / secs / 60e6
        );
    }
    for bytes in [1usize << 20, 1 << 24] {
        let (_, took) = console.ask(&format!("X {bytes:x}")).expect("a digest");
        let secs = took.as_secs_f64();
        println!(
            "X, {bytes} bytes on the device: {took:?} -> {:.0} bytes/s enciphered \
             and hashed ({:.3} bytes/clock at 60 MHz), digest not checked at this \
             length",
            bytes as f64 / secs,
            bytes as f64 / secs / 60e6
        );
    }

    // And the device is still the device, after all of that.
    let (help, _) = console.ask("?").expect("the console still answers");
    assert_eq!(help, HELP, "the console survived the whole session");
}
