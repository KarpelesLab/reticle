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

use reticle::program::{self, ProgramError, usb, xilinx};

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
        let adapters = usb::list()?;
        if adapters.is_empty() {
            println!("no FTDI FT2232H adapter is attached");
        } else {
            for serial in adapters {
                println!("{serial}");
            }
        }
        return Ok(Outcome::Ok);
    }

    let probe = args.flag("probe");
    let expected = match args.option("expect") {
        None => xilinx::IDCODE_XC7A35T,
        Some(text) => parse_hex(text)?,
    };
    let clock = args
        .u32_option("clock")?
        .unwrap_or(program::DEFAULT_CLOCK_HZ);
    let divisor = reticle::program::ftdi::divisor_for(clock);

    // The file is read and checked first: a malformed bitstream must be
    // found before a cable is opened, not after the part has been
    // erased.
    let mut file = Vec::new();
    let mut payload_len = 0;
    if !probe {
        let [path] = args.positionals() else {
            return Err(Fault::Usage(
                "`program` takes exactly one .bit file (or `--probe`, which takes none)".into(),
            ));
        };
        file = std::fs::read(path)
            .map_err(|err| Fault::Failed(format!("cannot read `{path}`: {err}")))?;
        let bit = program::read_bit_container(&file)?;
        payload_len = bit.length;
        say(&format!(
            "{path}: {} for {}, built {} {}",
            bit.design, bit.part, bit.date, bit.time
        ));
        check_part(&bit.part, expected)?;
        // Keep only the payload, so the configuration job's copy is the
        // only other one of it in memory.
        file = bit.payload(&file).to_vec();
    } else if !args.positionals().is_empty() {
        return Err(Fault::Usage(
            "`--probe` reads the part and writes nothing, so it takes no file".into(),
        ));
    }

    let cable = usb::Cable::open(args.option("device"))?;
    say(&format!(
        "adapter {}, TCK {} Hz",
        cable.serial(),
        reticle::program::ftdi::clock_hz(divisor)
    ));
    let (pins, dirs) = program::BASYS3_PINS;
    cable.run(&xilinx::init_job(divisor, pins, dirs))?;

    // Nothing has been written to the part yet, and nothing will be if
    // this does not match.
    let job = xilinx::idcode_job();
    let idcode = job.capture_u32(0, &cable.run(&job)?)?;
    if !xilinx::idcode_matches(idcode, expected) {
        return Err(ProgramError::IdcodeMismatch {
            read: idcode,
            expected,
        }
        .into());
    }
    say(&format!(
        "IDCODE {idcode:#010x} (revision {}), as expected for {expected:#010x}",
        xilinx::idcode_revision(idcode)
    ));

    let before = status(&cable)?;
    say(&format!("status before: {before}"));

    if probe {
        return Ok(Outcome::Ok);
    }

    // From here the part's volatile configuration is being replaced. A
    // power cycle restores whatever it had.
    cable.run(&xilinx::erase_job())?;
    let erased = status(&cable)?;
    say(&format!("status after JPROGRAM: {erased}"));
    if erased.done() {
        return Err(Fault::Failed(format!(
            "DONE is still high after JPROGRAM; the part did not clear its \
             configuration memory (status {erased})"
        )));
    }

    let job = xilinx::configure_job(&file);
    let started = Instant::now();
    let mut last = 0;
    cable.run_with_progress(&job, &mut |written, total| {
        if quiet || total == 0 {
            return;
        }
        let percent = written * 100 / total;
        if percent >= last + 10 {
            last = percent - percent % 10;
            println!("  {last}% ({written} of {total} bytes)");
        }
    })?;
    say(&format!(
        "{payload_len} bytes of configuration data shifted in {:.1} s",
        started.elapsed().as_secs_f64()
    ));

    cable.run(&xilinx::start_job())?;
    let after = status(&cable)?;
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
