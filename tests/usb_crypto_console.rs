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
//! So this test asks the part:
//!
//! ```console
//! cargo test --features program --test usb_crypto_console -- --ignored --nocapture
//! ```
//!
//! **It fails on the board as of this round, and that is the result.** The
//! console, the serial port, the registers and the error paths are right;
//! both cores are **wrong**, and the way they are wrong is the finding.
//! `ip/crypto/sha256/README.md` §8 is the account. Read part three below
//! before reading the assertions, because the order of the parts is the
//! argument.
//!
//! It is in five parts.
//!
//! 1. **The port is there and it is this design.** A `/dev/ttyACM*` whose USB
//!    parent is `1209:0001`, held by `cdc_acm`, that answers `?` with
//!    `crypto_console`'s help line. The last of those is what distinguishes
//!    this bitstream from `usb_cdc_uart.v`, which has the same identifiers
//!    and would echo the `?` straight back.
//! 2. **Which cores this build has.** `testdata/fpga/cynthion/crypto_console.v`
//!    takes `WITH_HASH` and `WITH_CIPHER`, because the design with both does
//!    not route on this part, so there are three bitstreams and only one of
//!    them answers everything. The test asks rather than assumes: a command
//!    whose core is absent answers `ERR`.
//! 3. **Determinism**, which is the part that localises the fault and the
//!    reason it comes before correctness. Every command here is a function of
//!    the line and of registers the line can read back, so **sixteen
//!    identical lines must get sixteen identical answers**. That is a
//!    property of the design and not of any standard, it needs no oracle, and
//!    it is the sharpest thing a terminal can measure: a wrong *constant* is
//!    a mis-mapped lookup table or an unrouted wire, and a wrong *varying*
//!    answer is a path that does not settle inside the clock period.
//! 4. **Correctness**, compared against `sha256sum` run as a subprocess for
//!    the digests and against RFC 8439's own hexdump for the keystream. The
//!    only SHA-256 in that comparison that this project wrote is the one on
//!    the part.
//! 5. **How fast it is**, three ways, which is the number neither simulation
//!    nor arithmetic can produce — and which is **valid whether or not the
//!    answers are right**, because a rate is a cycle count and the cycle
//!    count is the control path. `Z` makes its bytes on the device, so it
//!    measures the core; `h` sends its message in, so it measures the OUT
//!    direction of the link; `e` sends only hex out, so it measures the IN
//!    direction. All three are *printed* and none is asserted, because
//!    `CLAUDE.md` is explicit that a throughput figure belongs in a report
//!    and never in an assertion: CI runs on slower machines and a number of
//!    seconds is not a property of this design.
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
//! **What this test would and would not catch.** It catches a wrong digest or
//! a wrong keystream byte from the silicon, which is the whole point, and it
//! did. It catches the console hanging, as a read that returns nothing. It
//! catches the wrong bitstream being loaded, because `?` names this one. Part
//! three catches, and separates, the two reasons an answer can be wrong,
//! which no single reading can.
//!
//! It does **not** prove *where* a path that fails to settle is: it says the
//! deep ones fail and the shallow ones do not, which is an argument and not a
//! measurement of a delay. A slack number would need a vendor timing model
//! and this repository has none — `reticle timing` says in its own help that
//! its device numbers are placeholders. It does **not** establish anything
//! about what the cores **leak** — not power, not electromagnetic emission,
//! not gate delay — and `ip/crypto/sha256/README.md` §5 is the list of what
//! remains undefended; an answer off a real part, right or wrong, is not a
//! side-channel result. And it says nothing about a board that is not
//! plugged in: it **skips**, with a reason.

#![cfg(feature = "program")]

use std::collections::BTreeMap;
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

/// What a line whose core this build left out answers, and what a malformed
/// line answers. The two are the same string on purpose: a console that
/// distinguished them would be telling a host about its own parameters, and
/// `?` plus a probe says the same thing without a second error code.
const ERR: &str = "ERR";

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
    "8a3b0aa372600a92b57974cded2b9334794cba40c63e34cdea212c4cf07d41b7",
    "69a6749f3f630f4122cafe28ec4dc47e26d4346d70b98c73f3e9c53ac40c5945",
    "398b6eda1a832c89c167eacd901d7e2bf363740373201aa188fbbce83991c4ed",
);

/// RFC 8439 Appendix A.1 vector #1: the all-zero key, the all-zero nonce,
/// block counter 0, one block.
const KEYSTREAM_A1_1: &str = concat!(
    "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7",
    "da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586",
);

/// How many times a line is repeated to see whether its answer is a function
/// of the line.
///
/// Sixteen is chosen to be cheap and still decisive. The board's own reading,
/// which `ip/crypto/sha256/README.md` §8 quotes, is forty: `?` and `k` were
/// right forty times out of forty, `e 10` gave two distinct answers in a ratio
/// of 39 to 1, and `h` gave nine distinct answers in ten. A ratio that lopsided
/// is exactly what sixteen runs can still see and what one cannot.
const REPEATS: usize = 16;

/// How long a read of the terminal may block with nothing arriving before the
/// test gives up on the device.
///
/// It is a **timeout and not an assertion**: nothing here is compared against
/// a clock. A `Z` of sixteen mebibytes is half a second of silence and then
/// one line, and the budget has to be generous enough for the longest command
/// this file sends and no tighter.
const QUIET: Duration = Duration::from_secs(60);

/// `wMaxPacketSize` of the bulk pair, which is also the largest read this test
/// does in one call.
const MAX_PACKET: usize = 64;

/// The terminal device the kernel gave this USB device, found by **asking the
/// kernel** rather than by guessing a number.
///
/// The number is not fixed and cannot be: a Cynthion's own Apollo debugger is
/// itself a CDC ACM device and is usually `ttyACM0`. What is fixed is the
/// relation — `/sys/class/tty/ttyACMn/device` is the USB *interface* the port
/// belongs to and its parent is the USB device — so this walks that and reads
/// the vendor and product identifiers off it. `tests/usb_cdc_acm.rs` has the
/// same walk, for the same device, and says the same thing about why.
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
    /// neither; `stty` is the operating system's own tool for it and is what a
    /// person does at a prompt anyway. `tests/usb_cdc_acm.rs` has the longer
    /// form of that argument.
    ///
    /// `raw` is the part that matters here and is not a nicety: a terminal
    /// that is not raw translates carriage returns, which are this protocol's
    /// line terminator, and strips or expands characters inside a hex payload.
    /// `-echo` so the terminal layer does not send our own bytes back at us
    /// and make a dead device look alive. `clocal` so a carrier this device
    /// reported and then dropped could not hang the terminal up mid-transfer —
    /// it cannot, since `serial_state` is a constant, and
    /// `ip/usb/usb_cdc_acm/README.md` §5 says what `clocal` does and does not
    /// reach on this driver. The baud rate is arbitrary: there is no serial
    /// line inside this design, so nothing divides anything, and `stty` merely
    /// insists on a number.
    ///
    /// `min 0 time 10` is a one-second read timeout with no minimum, so a read
    /// returns rather than blocking for ever. It is **not** a budget for an
    /// answer: [`Console::ask`] keeps reading across as many of those as
    /// `QUIET` allows.
    ///
    /// **A persistent open matters, and the first attempt at this round got it
    /// wrong.** Driving the port with one `printf` and one `cat` per line —
    /// which is what a person types — opens and closes the terminal twice a
    /// command, and `cdc_acm` tears its read URBs down on the last close, so
    /// every answer went missing and a perfectly working device looked
    /// completely dead. One open for the session, which is what a terminal
    /// program does, and the answers are all there.
    fn open(path: &str) -> Result<Console, String> {
        let stty = Command::new("stty")
            .args([
                "-F", path, "115200", "raw", "-echo", "clocal", "min", "0", "time", "10",
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
    /// The console prints its help line the moment the host configures it, so
    /// a freshly enumerated device has a banner waiting and one that has
    /// already been read does not. Nothing is asserted about which: the banner
    /// is reported if it is there, and `?` below asks for the same line on
    /// purpose so that the identification does not depend on whether anything
    /// opened the port first.
    fn drain(&mut self) -> String {
        let mut got = Vec::new();
        let mut buf = [0u8; MAX_PACKET];
        let mut quiet = 0;
        // Two consecutive quiet reads and not one, because the banner arrives
        // as one packet and whatever a previous session left arrives as
        // several: one quiet read in between would stop early and leave the
        // rest to be mistaken for the next answer.
        while quiet < 2 && got.len() < 1 << 16 {
            match self.port.read(&mut buf) {
                Ok(0) => quiet += 1,
                Ok(n) => {
                    quiet = 0;
                    got.extend_from_slice(&buf[..n]);
                }
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&got).trim_end().to_owned()
    }

    /// Asks `?` until the help line comes back, draining whatever a previous
    /// session left in front of it.
    ///
    /// **This is not defensive padding, it is a thing that happened.** A board
    /// reprogrammed under an open terminal, or a test run that stopped half
    /// way through a command, leaves bytes in the kernel's buffer that belong
    /// to a device that no longer exists — and the first answer this test read
    /// was then a sixty-four digit digest in reply to a `?`, which looks
    /// exactly like a catastrophically broken design and was a stale buffer. A
    /// terminal program attaching to a port of unknown state has to
    /// re-synchronise, so this does, and says when it had to.
    fn sync(&mut self) -> Result<(), String> {
        for attempt in 0..5 {
            let left = self.drain();
            if !left.is_empty() {
                println!(
                    "  the port had {} byte(s) in front of the first answer, from the banner or \
                     from a previous session: {:?}",
                    left.len(),
                    left.chars().take(96).collect::<String>()
                );
            }
            let (got, _) = self.ask("?")?;
            if got == HELP {
                if attempt > 0 {
                    println!("  re-synchronised after {attempt} stale answer(s)");
                }
                return Ok(());
            }
            println!(
                "  `?` answered {got:?}, which is not the help line; draining and asking again"
            );
        }
        Err("`?` never answered the help line in five attempts".to_owned())
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

    /// The same line [`REPEATS`] times, and how many times each answer came
    /// back.
    ///
    /// This is the measurement part three is about. Every command this console
    /// has is a function of the line and of registers the line can read back,
    /// so one line has one answer; a map with more than one key in it is a
    /// design that does not.
    fn repeat(&mut self, line: &str) -> BTreeMap<String, usize> {
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for _ in 0..REPEATS {
            match self.ask(line) {
                Ok((answer, _)) => *seen.entry(answer).or_default() += 1,
                Err(why) => panic!("`{line}` stopped answering: {why}"),
            }
        }
        seen
    }
}

/// Asserts that one line has one answer, and says what it saw when it does
/// not.
fn one_answer(line: &str, seen: &BTreeMap<String, usize>) {
    if seen.len() == 1 {
        println!(
            "  {line:18} -> one answer in {REPEATS} runs: {}",
            seen.keys().next().expect("one key")
        );
        return;
    }
    let mut counted: Vec<(&String, &usize)> = seen.iter().collect();
    counted.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    let tally: Vec<String> = counted
        .iter()
        .map(|(answer, n)| format!("{n} x {answer}"))
        .collect();
    panic!(
        "`{line}` is a function of its own line and gave {} distinct answers in {REPEATS} runs, \
         so a path in it does not settle inside 16.67 ns:\n  {}",
        seen.len(),
        tally.join("\n  ")
    );
}

/// What the host's own `sha256sum` says of these bytes.
///
/// Spawned rather than linked, because this crate has no dependencies and
/// because the point of the comparison is that the other implementation is
/// **somebody else's**. Returns `None` when the tool is not there, which is a
/// reason to skip a comparison and not to fail one.
fn host_digest(bytes: &[u8]) -> Option<String> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Written in a thread-free way on purpose: the payloads here are at most a
    // few mebibytes and a pipe holds enough that `sha256sum` is never blocked
    // on a full buffer while this writes. A payload large enough to need a
    // thread would be a payload this test should not be timing.
    child.stdin.take()?.write_all(bytes).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.split_whitespace().next().map(str::to_owned)
}

/// A byte string as lower-case hexadecimal, which is what this protocol reads
/// and writes.
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
    assert!(
        digits.len().is_multiple_of(2),
        "an even number of hex digits"
    );
    digits
        .chunks(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect()
}

/// The bytes `crypto_pattern` makes in `tests/ip_library.rs`: byte *i* is
/// `(i * 7 + 13) & 0xff`, which is neither zeros nor text and is cheap to
/// restate on both sides. The same rule, so a length that is interesting there
/// is the same bytes here.
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
        // board, and a device with no terminal is a descriptor set the kernel
        // refused.
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

    // The line that says which bitstream this is, after draining whatever was
    // in front of it. `usb_cdc_uart.v` has the same identifiers and would send
    // the `?` straight back, so this is the identification and not a nicety.
    println!("synchronising:");
    console.sync().unwrap_or_else(|why| {
        panic!(
            "{why}: this port is not crypto_console. A design that echoes is \
             testdata/fpga/cynthion/usb_cdc_uart.v and not this one, and a design that says \
             nothing at all is one whose `configured` never went high — LED 1 on the board."
        )
    });
    println!("?  -> {HELP}");

    // ------------------------------------------------------------------
    // Part two: which cores this build has.
    // ------------------------------------------------------------------
    let (probe_hash, _) = console.ask("h").expect("the console answers `h`");
    let (probe_ciph, _) = console.ask("e 4").expect("the console answers `e`");
    let has_hash = probe_hash != ERR;
    let has_cipher = probe_ciph != ERR;
    println!(
        "this build has {}",
        match (has_hash, has_cipher) {
            (true, true) => "both cores",
            (true, false) => "ip/crypto/sha256 and not ip/crypto/chacha20",
            (false, true) => "ip/crypto/chacha20 and not ip/crypto/sha256",
            (false, false) =>
                "neither core, which is not a build this design has any use for — \
                 check WITH_HASH and WITH_CIPHER",
        }
    );
    assert!(
        has_hash || has_cipher,
        "a console with neither core is not worth loading"
    );

    // The key, the nonce and the counter the part is holding. They are in
    // every build, because they are three shift registers and a printer and
    // cost nothing — and because they are the widest thing in this design that
    // is *not* arithmetic, which makes them the control group part three
    // needs.
    //
    // **Reported and not asserted**, which is a correction: the first version
    // of this insisted on the reset values and then failed on a board that had
    // been typed at since it enumerated, which is a true thing about the
    // session and not a defect in the part. `K`, `N` and `C` set them
    // explicitly before anything depends on them, and the assertion is on
    // *that* readback.
    for (command, what) in [("k", "the key"), ("n", "the nonce"), ("c", "the counter")] {
        let (got, _) = console.ask(command).expect("the console answers");
        let reset = match command {
            "k" => RESET_KEY,
            "n" => RESET_NONCE,
            _ => RESET_COUNTER,
        };
        let note = if got == reset {
            " (the reset value, so nothing has set it since this part was configured)"
        } else {
            " (not the reset value, so something has set it; `K`, `N` and `C` below put it back)"
        };
        println!("{command}  -> {got}{note}   [{what}]");
    }

    // And put them where the rest of this test needs them, with the readback
    // asserted. This is the only place the three wide registers are *checked*
    // rather than reported, and it checks them against a value the host chose.
    for (line, want, what) in [
        (format!("K {RESET_KEY}"), "OK", "the key"),
        (format!("N {RESET_NONCE}"), "OK", "the nonce"),
        (format!("C {RESET_COUNTER}"), "OK", "the counter"),
        ("k".to_owned(), RESET_KEY, "the key, read back"),
        ("n".to_owned(), RESET_NONCE, "the nonce, read back"),
        ("c".to_owned(), RESET_COUNTER, "the counter, read back"),
    ] {
        let (got, _) = console.ask(&line).expect("an answer");
        assert_eq!(got, want, "{what}");
    }
    println!("K, N and C set to RFC 8439 2.4.2's values and read back byte for byte");

    // ------------------------------------------------------------------
    // Part three: one line, one answer.
    // ------------------------------------------------------------------
    // Nothing here needs an oracle and nothing here cites a standard. Every
    // one of these commands is a function of the line it is on and of
    // registers the line can read back, so repeating a line has to repeat its
    // answer — and **which** of them stop doing that is what separates a
    // mis-mapped lookup table from a path that misses the clock.
    //
    // The controls come first and they are the shallow ones: a string out of a
    // 64-entry table, and three registers rotated four bits at a time through a
    // hexadecimal encoder. Neither has an adder in it.
    println!();
    println!("--- one line, one answer: {REPEATS} runs each ---");
    for line in ["?", "k", "n", "c"] {
        let seen = console.repeat(line);
        one_answer(line, &seen);
    }

    // And then the cores, which are the two deepest things in this design:
    // SHA-256's five chained 32-bit additions are 39 levels of LUT4 on this
    // flow and ChaCha20's four are 87, because `src/fpga/trellis` describes
    // `CCU2C` without a port map and emits no carry cell.
    if has_hash {
        for line in ["h", "h 616263", "H abc", "Z 40"] {
            let seen = console.repeat(line);
            one_answer(line, &seen);
        }
    }
    if has_cipher {
        for line in ["e 10", "E 00000000"] {
            let seen = console.repeat(line);
            one_answer(line, &seen);
        }
    }

    // ------------------------------------------------------------------
    // Part four: and the answers are the right ones.
    // ------------------------------------------------------------------
    println!();
    println!("--- the answers, against sha256sum and RFC 8439 ---");
    if has_hash {
        if host_digest(b"").is_none() {
            println!("`sha256sum` is not on this host; the digests cannot be checked");
        } else {
            // Typed as text, which is the form a person uses. Nothing here
            // contains a carriage return, a line feed or a leading space,
            // which is the whole of what the text form cannot carry.
            for text in ["abc", "hello, world", "The quick brown fox jumps over it."] {
                let (got, _) = console.ask(&format!("H {text}")).expect("a digest");
                let want = host_digest(text.as_bytes()).expect("sha256sum");
                assert_eq!(got, want, "the part's digest of {text:?}");
                println!("H {text}\n  -> {got}\n  sha256sum agrees");
            }

            // As hex, at every length that reaches a different branch of
            // FIPS 180-4 §5.1.1's padding: nothing at all, one byte, 55 (no
            // zeros), 56 (the length field has no room, so the padding spills
            // into a whole extra block), 57 to 63 (the spill with the zeros
            // wrapping a boundary), 64 (a whole block of message and then a
            // whole block of padding), and three past two blocks. That is the
            // table `sha256_pads_every_length_purecrypto_was_asked_about`
            // drives in simulation, now driven at silicon.
            let lengths = [
                0usize, 1, 2, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 119, 120, 128, 200,
            ];
            for n in lengths {
                let bytes = pattern(n);
                let (got, _) = console
                    .ask(&format!("h {}", hex(&bytes)))
                    .expect("a digest");
                let want = host_digest(&bytes).expect("sha256sum");
                assert_eq!(got, want, "the part's digest of {n} pattern byte(s)");
            }
            println!(
                "h: {} lengths through the padding, every one agreeing with sha256sum",
                lengths.len()
            );

            // Zero bytes made on the device, which crosses the OUT endpoint
            // not at all. The digest is the same function of the same bytes
            // either way, so a `Z` that disagreed with an `h` of the same
            // zeros would be the on-device source and not the hash.
            for n in [0usize, 1, 55, 56, 64, 1000] {
                let (got, _) = console.ask(&format!("Z {n:x}")).expect("a digest");
                let want = host_digest(&vec![0u8; n]).expect("sha256sum");
                assert_eq!(got, want, "the part's digest of {n} zero byte(s)");
            }
            println!("Z: 0, 1, 55, 56, 64 and 1000 zero bytes made on the device, all agreeing");
        }
    }

    if has_cipher {
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
        let (got, _) = console
            .ask(&format!("E {}", hex(&plain)))
            .expect("a cipher");
        let want: Vec<u8> = plain.iter().zip(ks.iter()).map(|(a, b)| a ^ b).collect();
        assert_eq!(got, hex(&want), "the exclusive-or of the keystream");
        println!("E: 32 pattern bytes enciphered, against RFC 8439's keystream, byte for byte");

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

        // And put them back, because the measurements below are under the reset
        // key and `k`, `n` and `c` make that a checked step rather than a hope.
        for (line, want) in [
            (format!("K {RESET_KEY}"), "OK"),
            (format!("N {RESET_NONCE}"), "OK"),
            (format!("C {RESET_COUNTER}"), "OK"),
            ("k".to_owned(), RESET_KEY),
            ("n".to_owned(), RESET_NONCE),
            ("c".to_owned(), RESET_COUNTER),
        ] {
            let (got, _) = console.ask(&line).expect("an answer");
            assert_eq!(got, want, "`{line}` put the stream back where it started");
        }
    }

    // Both cores in series, when a build has both: the digest of the keystream,
    // which turns any length of cipher output into sixty-four digits. Its
    // expectation is `sha256sum` over the bytes of RFC 8439's own hexdump, so
    // both ends of the command are published values and nothing this project
    // wrote is on either side of the comparison.
    if has_hash && has_cipher && host_digest(b"").is_some() {
        let ks = unhex(KEYSTREAM_2_4_2);
        for n in [64usize, 128] {
            let (got, _) = console.ask(&format!("X {n:x}")).expect("a digest");
            let want = host_digest(&ks[..n]).expect("sha256sum");
            assert_eq!(
                got, want,
                "the digest of {n} bytes of keystream, both cores in series"
            );
            println!("X {n:x} -> {got}");
        }
    }

    // The five ways a line can be wrong, and that the console recovers from
    // each — which is the half of error handling that is easy to get wrong and
    // that every assertion above would pass without.
    for bad in ["Q", "h 6", "h 6g", "C 0000", "Z"] {
        let (got, _) = console.ask(bad).expect("an answer");
        assert_eq!(got, ERR, "`{bad}` is not a line this console can answer");
    }
    println!("ERR on five malformed lines, and the console still answers after them");
    let (help, _) = console.ask("?").expect("the console still answers");
    assert_eq!(help, HELP, "the console survived the whole session");

    // ------------------------------------------------------------------
    // Part five: how fast, and what the bottleneck is.
    // ------------------------------------------------------------------
    // Printed, never asserted. `CLAUDE.md`: a throughput figure belongs in a
    // report and never in an assertion, because CI runs on slower machines and
    // a number of seconds is not a property of this design.
    //
    // These are valid whether or not parts three and four passed: a rate is a
    // cycle count, the cycle count is the control path, and the control path is
    // the half of this design that works.
    println!();
    println!("--- throughput, measured end to end, asserted nowhere ---");

    if has_hash {
        // The core. `Z` makes its bytes on the device, so none of the message
        // crosses USB and what is measured is the 129 cycles a block takes.
        for bytes in [1usize << 16, 1 << 20, 1 << 24] {
            let (_, took) = console.ask(&format!("Z {bytes:x}")).expect("a digest");
            let secs = took.as_secs_f64();
            println!(
                "Z, {bytes} bytes on the device: {took:?} -> {:.2} MB/s hashed \
                 ({:.3} bytes/clock at 60 MHz)",
                bytes as f64 / secs / 1e6,
                bytes as f64 / secs / 60e6
            );
        }

        // The link, outbound. Every message byte crosses USB as two hex
        // digits, so the bytes on the wire are twice the bytes hashed and both
        // numbers are reported: the first is what this protocol costs, the
        // second is what the endpoint moved.
        for bytes in [16usize * 1024, 64 * 1024, 256 * 1024] {
            let message = pattern(bytes);
            let line = format!("h {}", hex(&message));
            let (_, took) = console.ask(&line).expect("a digest");
            let secs = took.as_secs_f64();
            println!(
                "h, {bytes} byte message: {took:?} -> {:.1} kB/s hashed, \
                 {:.1} kB/s on the wire out",
                bytes as f64 / secs / 1e3,
                (line.len() + 1) as f64 / secs / 1e3
            );
        }
    }

    if has_cipher {
        // The link, inbound: `e` sends nothing but a short line and gets two
        // hex digits a byte back, so it measures the IN direction, which is the
        // one a bulk endpoint can fill without being asked twice.
        for bytes in [4usize * 1024, 16 * 1024] {
            let (_, took) = console.ask(&format!("e {bytes:x}")).expect("a keystream");
            let secs = took.as_secs_f64();
            println!(
                "e, {bytes} bytes of keystream: {took:?} -> {:.1} kB/s of keystream, \
                 {:.1} kB/s on the wire in",
                bytes as f64 / secs / 1e3,
                (2 * bytes) as f64 / secs / 1e3
            );
        }
    }
}
