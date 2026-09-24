//! The `reticle program` command: load a bitstream into an attached
//! FPGA over JTAG.
//!
//! All the logic is in `reticle::program`, which is sans-I/O like the
//! rest of the library. What is here is the argument handling, the file
//! read, and the running commentary — and the order of operations,
//! which is the part with consequences:
//!
//! 1. read and check the `.bit` container, before touching the cable;
//! 2. open the adapter and bring up the MPSSE;
//! 3. read `IDCODE` and **stop on a mismatch**, having written nothing;
//! 4. read the status register, so the `DONE` bit before is on record;
//! 5. `JPROGRAM`, and confirm `DONE` went low — the part really did
//!    clear its configuration memory;
//! 6. shift the payload through `CFG_IN`;
//! 7. `JSTART`, then read the status back and report `DONE`.
//!
//! Step 3 is why the order matters. Everything after it is destructive
//! of the part's *volatile* configuration, and a part that answers the
//! wrong `IDCODE` is not the part the bitstream was built for.
//!
//! Nothing here can write flash, and nothing here touches the mode pins.

use std::time::Instant;

use crate::Outcome;
use crate::args::{ArgError, Args};

use reticle::program::{self, ProgramError, gowin, usb, xilinx};

/// Runs the command.
pub(crate) fn program_cmd(args: &Args) -> Result<Outcome, ArgError> {
    match go(args) {
        Ok(outcome) => Ok(outcome),
        Err(Fault::Usage(message)) => Ok(Outcome::Usage(message)),
        Err(Fault::Arg(err)) => Err(err),
        Err(Fault::Failed(err)) => {
            eprintln!("error: {err}");
            Ok(Outcome::Failed)
        }
    }
}

/// The three ways this command can stop early.
#[derive(Debug)]
enum Fault {
    /// The command line was wrong.
    Usage(String),
    /// An option's value was wrong.
    Arg(ArgError),
    /// The board, the file or the USB layer said no.
    Failed(String),
}

impl From<ArgError> for Fault {
    fn from(err: ArgError) -> Fault {
        Fault::Arg(err)
    }
}

impl From<ProgramError> for Fault {
    fn from(err: ProgramError) -> Fault {
        Fault::Failed(err.to_string())
    }
}

impl From<reticle::program::jtag::JobError> for Fault {
    fn from(err: reticle::program::jtag::JobError) -> Fault {
        Fault::Failed(err.to_string())
    }
}

fn go(args: &Args) -> Result<Outcome, Fault> {
    let quiet = args.flag("quiet");
    let say = |line: &str| {
        if !quiet {
            println!("{line}");
        }
    };

    if args.flag("list") {
        // Every adapter of both kinds, because `--device` takes a serial
        // and a serial nobody can see is a serial nobody can type. The
        // old listing showed FTDI cables only, so an attached Cynthion
        // did not appear at all.
        let cables = usb::list()?;
        let cynthions = usb::list_cynthions()?;
        if cables.is_empty() && cynthions.is_empty() {
            println!("no programming adapter is attached");
            return Ok(Outcome::Ok);
        }
        for serial in cables {
            println!("{serial}  FTDI cable");
        }
        for (serial, debugger) in cynthions {
            // A Cynthion reports *unrelated* serial numbers in its two
            // modes, so which mode this one is in decides which serial
            // `--device` should be given. Saying which is the point.
            let mode = if debugger {
                "Cynthion, Apollo debugger"
            } else {
                "Cynthion, running gateware (hands over on demand)"
            };
            println!("{serial}  {mode}");
        }
        return Ok(Outcome::Ok);
    }

    let probe = args.flag("probe");
    let expect = args.option("expect").map(parse_hex).transpose()?;
    let clock = args
        .u32_option("clock")?
        .unwrap_or(program::DEFAULT_CLOCK_HZ);

    // The file is read and checked first: a malformed bitstream must be
    // found before a cable is opened, not after the part has been
    // erased. A `.fs` is a Gowin file, anything else a Xilinx `.bit`.
    let load = if probe {
        if !args.positionals().is_empty() {
            return Err(Fault::Usage(
                "`--probe` reads the part and writes nothing, so it takes no file".into(),
            ));
        }
        None
    } else {
        let [path] = args.positionals() else {
            return Err(Fault::Usage(
                "`program` takes exactly one .bit or .fs file (or `--probe`, which takes none)"
                    .into(),
            ));
        };
        Some(read_load(path, expect, &say)?)
    };

    let cable = usb::Cable::open(args.option("device"))?;
    // The divisor depends on which FTDI part the adapter has, so it can
    // only be worked out once the cable is open.
    let chip = cable.chip();
    let divisor = reticle::program::ftdi::divisor_for_chip(chip, clock);
    say(&format!(
        "adapter {}, {:?}, TCK {} Hz",
        cable.serial(),
        chip,
        reticle::program::ftdi::clock_hz_on(chip, divisor)
    ));
    let (pins, dirs) = match load {
        Some(Load::Gowin { .. }) => program::SIPEED_PINS,
        _ => program::BASYS3_PINS,
    };
    cable.run(&xilinx::init_job_for(chip, divisor, pins, dirs))?;

    // Read the identifier the way that needs no vendor knowledge: after
    // Test-Logic-Reset every 1149.1 part with one presents it, whatever
    // its instruction register's width. Nothing has been written to the
    // part yet, and nothing will be if it is not the expected one.
    let job = reticle::program::jtag::idcode_after_reset();
    let idcode = job.capture_u32(0, &cable.run(&job)?)?;
    let expected = match &load {
        Some(Load::Xilinx { expected, .. }) | Some(Load::Gowin { expected, .. }) => Some(*expected),
        None => expect,
    };
    if let Some(expected) = expected
        && !xilinx::idcode_matches(idcode, expected)
    {
        return Err(ProgramError::IdcodeMismatch {
            read: idcode,
            expected,
        }
        .into());
    }

    // Everything from here is one vendor's: its status register, its
    // instruction codes and its instruction register width. A Xilinx
    // sequence shifted at a Gowin part, or the reverse, does not misread;
    // it executes whatever instruction those bits land on. So each
    // vendor's sequence runs only on a part that vendor's identifier
    // names.
    let is_gowin = gowin::part_name(idcode).is_some();
    if probe {
        if xilinx::is_xilinx(idcode) {
            say(&format!(
                "IDCODE {idcode:#010x}: Xilinx, revision {}",
                xilinx::idcode_revision(idcode)
            ));
            say(&format!("status: {}", status(&cable)?));
        } else if let Some(name) = gowin::part_name(idcode) {
            say(&format!("IDCODE {idcode:#010x}: Gowin {name}"));
            say(&format!("status: {}", gowin_status(&cable)?));
        } else {
            say(&format!(
                "IDCODE {idcode:#010x}: not a part this command knows, so nothing further was \
                 read; reading its status would mean shifting some vendor's instruction at it"
            ));
        }
        return Ok(Outcome::Ok);
    }

    match load {
        Some(Load::Xilinx { payload, .. }) => {
            if !xilinx::is_xilinx(idcode) {
                return Err(Fault::Failed(format!(
                    "IDCODE {idcode:#010x} is not a Xilinx part and this is a Xilinx .bit; \
                     nothing was written"
                )));
            }
            say(&format!(
                "IDCODE {idcode:#010x} (revision {}), as expected",
                xilinx::idcode_revision(idcode)
            ));
            load_xilinx(&cable, &payload, quiet, &say)
        }
        Some(Load::Gowin {
            payload, checksum, ..
        }) => {
            if !is_gowin {
                return Err(Fault::Failed(format!(
                    "IDCODE {idcode:#010x} is not a Gowin part this command knows, and this is \
                     a Gowin .fs; nothing was written"
                )));
            }
            say(&format!("IDCODE {idcode:#010x}, as expected"));
            load_gowin(&cable, &payload, checksum, quiet, &say)
        }
        None => Ok(Outcome::Ok),
    }
}

/// What a file asks to have loaded.
enum Load {
    /// A 7-series `.bit`'s configuration payload.
    Xilinx { payload: Vec<u8>, expected: u32 },
    /// A Gowin `.fs` file, packed for shifting, and its checksum.
    Gowin {
        payload: Vec<u8>,
        checksum: u32,
        expected: u32,
    },
}

/// Reads and checks a bitstream file, before any cable is opened.
fn read_load(path: &str, expect: Option<u32>, say: &dyn Fn(&str)) -> Result<Load, Fault> {
    let read_err = |err: std::io::Error| Fault::Failed(format!("cannot read `{path}`: {err}"));
    if path.ends_with(".fs") {
        let text = std::fs::read_to_string(path).map_err(read_err)?;
        let payload =
            gowin::fs_payload(&text).map_err(|err| Fault::Failed(format!("`{path}`: {err}")))?;
        let Some(checksum) = gowin::fs_checksum(&text) else {
            return Err(Fault::Failed(format!(
                "`{path}` has no checksum command (0x0a) in its footer, which a load needs"
            )));
        };
        let Some(named) = gowin::fs_idcode(&text) else {
            return Err(Fault::Failed(format!(
                "`{path}` has no IDCODE command, so there is nothing to check the part against"
            )));
        };
        let expected = expect.unwrap_or(named);
        if expected != named {
            return Err(Fault::Failed(format!(
                "`{path}` was built for IDCODE {named:#010x}, not the {expected:#010x} \
                 --expect names"
            )));
        }
        say(&format!(
            "{path}: Gowin, {} bytes, for IDCODE {named:#010x} ({})",
            payload.len(),
            gowin::part_name(named).unwrap_or("a part this command does not know")
        ));
        return Ok(Load::Gowin {
            payload,
            checksum,
            expected,
        });
    }
    let file = std::fs::read(path).map_err(read_err)?;
    let bit = program::read_bit_container(&file)?;
    say(&format!(
        "{path}: {} for {}, built {} {}",
        bit.design, bit.part, bit.date, bit.time
    ));
    let expected = expect.unwrap_or(xilinx::IDCODE_XC7A35T);
    check_part(&bit.part, expected)?;
    // Keep only the payload, so the configuration job's copy is the only
    // other one of it in memory.
    Ok(Load::Xilinx {
        payload: bit.payload(&file).to_vec(),
        expected,
    })
}

/// Runs one long job, reporting every tenth of it.
fn run_with_percent(
    cable: &usb::Cable,
    job: &reticle::program::jtag::Job,
    quiet: bool,
) -> Result<(), Fault> {
    let mut last = 0;
    cable.run_with_progress(job, &mut |written, total| {
        if quiet || total == 0 {
            return;
        }
        let percent = written * 100 / total;
        if percent >= last + 10 {
            last = percent - percent % 10;
            println!("  {last}% ({written} of {total} bytes)");
        }
    })?;
    Ok(())
}

/// The 7-series sequence, from `JPROGRAM` to `DONE`.
fn load_xilinx(
    cable: &usb::Cable,
    payload: &[u8],
    quiet: bool,
    say: &dyn Fn(&str),
) -> Result<Outcome, Fault> {
    let before = status(cable)?;
    say(&format!("status before: {before}"));

    // From here the part's volatile configuration is being replaced. A
    // power cycle restores whatever it had.
    cable.run(&xilinx::erase_job())?;
    let erased = status(cable)?;
    say(&format!("status after JPROGRAM: {erased}"));
    if erased.done() {
        return Err(Fault::Failed(format!(
            "DONE is still high after JPROGRAM; the part did not clear its \
             configuration memory (status {erased})"
        )));
    }

    let started = Instant::now();
    run_with_percent(cable, &xilinx::configure_job(payload), quiet)?;
    say(&format!(
        "{} bytes of configuration data shifted in {:.1} s",
        payload.len(),
        started.elapsed().as_secs_f64()
    ));

    cable.run(&xilinx::start_job())?;
    let after = status(cable)?;
    say(&format!("status after JSTART: {after}"));
    if !after.done() {
        return Err(ProgramError::NotDone(after).into());
    }
    if after.crc_error() || after.id_error() {
        return Err(Fault::Failed(format!(
            "the part asserted DONE but reported an error: status {after}"
        )));
    }
    println!("DONE is high: the part accepted the bitstream and is running it.");
    Ok(Outcome::Ok)
}

/// How many status reads to wait for a Gowin flag before giving up. A
/// read is a USB round trip, so this is a second or two, many times the
/// 6 ms UG290 gives a GW2A-18's erase.
const GOWIN_POLLS: usize = 500;

/// Reads the Gowin status register until `ready` holds.
fn gowin_wait(
    cable: &usb::Cable,
    what: &str,
    ready: impl Fn(gowin::Status) -> bool,
) -> Result<gowin::Status, Fault> {
    let mut last = gowin_status(cable)?;
    for _ in 0..GOWIN_POLLS {
        if ready(last) {
            return Ok(last);
        }
        last = gowin_status(cable)?;
    }
    Err(Fault::Failed(format!(
        "the part never reported {what}; status {last}. Its SRAM may be half \
         configured: a power cycle restores it"
    )))
}

/// The Gowin sequence (UG290): erase the SRAM, write it, read `DONE`.
fn load_gowin(
    cable: &usb::Cable,
    payload: &[u8],
    checksum: u32,
    quiet: bool,
    say: &dyn Fn(&str),
) -> Result<Outcome, Fault> {
    let before = gowin_status(cable)?;
    say(&format!("status before: {before}"));

    // From here the part's volatile configuration is being replaced. A
    // power cycle reloads whatever the board's flash holds.
    cable.run(&gowin::enable_job())?;
    gowin_wait(cable, "edit mode", gowin::Status::edit_mode)?;
    cable.run(&gowin::erase_job())?;
    let erased = gowin_wait(cable, "the SRAM erased", gowin::Status::memory_erase)?;
    say(&format!("status after erase: {erased}"));
    cable.run(&gowin::erase_done_job())?;
    gowin_wait(cable, "edit mode left", |s| !s.edit_mode())?;

    let started = Instant::now();
    run_with_percent(cable, &gowin::write_job(payload, checksum), quiet)?;
    say(&format!(
        "{} bytes shifted in {:.1} s",
        payload.len(),
        started.elapsed().as_secs_f64()
    ));

    // UG290: wait 60 ms for the status register to be refreshed.
    std::thread::sleep(std::time::Duration::from_millis(60));
    let after = gowin_status(cable)?;
    say(&format!("status after load: {after}"));
    if after.error() {
        return Err(Fault::Failed(format!(
            "the part reported an error after the load: status {after}"
        )));
    }
    if !after.done() {
        return Err(Fault::Failed(format!(
            "DONE is not set after the load: status {after}"
        )));
    }
    println!("DONE is set: the part accepted the bitstream and is running it.");
    Ok(Outcome::Ok)
}

/// Reads a Gowin part's status register.
fn gowin_status(cable: &usb::Cable) -> Result<gowin::Status, ProgramError> {
    let job = gowin::status_job();
    let reply = cable.run(&job)?;
    gowin::read_status(&job, &reply).map_err(ProgramError::Job)
}

/// Reads the status register.
fn status(cable: &usb::Cable) -> Result<xilinx::Status, ProgramError> {
    let job = xilinx::status_job();
    let reply = cable.run(&job)?;
    xilinx::read_status(&job, &reply).map_err(ProgramError::Job)
}

/// Refuses a bitstream built for another part.
///
/// The `.bit` header's `b` field names the part it was built for, so a
/// mismatch is visible before anything is written and without asking the
/// board. Only the one `IDCODE` this crate knows a part name for is
/// checked; `--expect` is the way past it for any other 7-series part,
/// and it then rests on the `IDCODE` check alone.
fn check_part(part: &str, expected: u32) -> Result<(), Fault> {
    if expected != xilinx::IDCODE_XC7A35T {
        return Ok(());
    }
    if part.starts_with("7a35t") || part.starts_with("xc7a35t") {
        return Ok(());
    }
    Err(Fault::Failed(format!(
        "this bitstream was built for `{part}`, not for the XC7A35T that \
         IDCODE {:#010x} names. Pass --expect <idcode> if that is deliberate.",
        xilinx::IDCODE_XC7A35T
    )))
}

/// Parses a hexadecimal `IDCODE`, with or without a `0x`.
fn parse_hex(text: &str) -> Result<u32, Fault> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    u32::from_str_radix(digits, 16).map_err(|_| {
        Fault::Arg(ArgError::BadValue {
            option: "expect".to_string(),
            value: text.to_string(),
            expected: "a 32-bit IDCODE in hexadecimal",
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitstream for another part is refused before the cable is
    /// opened, and the check is bypassed by naming another IDCODE.
    #[test]
    fn a_bitstream_for_another_part_is_refused() {
        assert!(check_part("7a35tcpg236", xilinx::IDCODE_XC7A35T).is_ok());
        assert!(check_part("xc7a35tcpg236-1", xilinx::IDCODE_XC7A35T).is_ok());
        assert!(check_part("7a100tcsg324", xilinx::IDCODE_XC7A35T).is_err());
        // With another part named, the header is not second-guessed.
        assert!(check_part("7a100tcsg324", 0x0362_5093).is_ok());
    }

    /// `--expect` takes hex with or without the prefix, and rejects
    /// anything else rather than programming against a silent default.
    #[test]
    fn the_expected_idcode_parses_as_hex() {
        assert_eq!(parse_hex("0362d093").unwrap(), 0x0362_D093);
        assert_eq!(parse_hex("0x0362D093").unwrap(), 0x0362_D093);
        assert_eq!(parse_hex("0X362d093").unwrap(), 0x0362_D093);
        assert!(parse_hex("").is_err());
        assert!(parse_hex("nonsense").is_err());
        assert!(parse_hex("12345678901").is_err());
    }
}
