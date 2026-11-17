//! `reticle program --ice40`: an iCE40 board whose FT2232H shares the
//! SPI bus between the FPGA and its configuration flash.
//!
//! The sequences are `reticle::program::ice40`, which is sans-I/O and
//! tested against a model of the board. What is here is the choice of
//! operation, the files, and the commentary.
//!
//! | Invocation | What happens to the board |
//! |---|---|
//! | `--probe` | the FPGA is held in reset while the flash is identified, then booted from the flash again |
//! | `design.bin` | the FPGA's configuration memory is loaded; the flash is not written |
//! | `--read-flash out.bin` | as `--probe`, and the flash is read into a file |
//! | `--write-flash design.bin` | the flash is written (only the sectors that change), read back and compared, and the FPGA booted from it |
//! | `--boot` | the FPGA is reset and configures itself from the flash |
//!
//! Only `--write-flash` writes anything non-volatile, and it says how much
//! it erased. `reticle::program::ice40::NOT_SENT` lists what is never sent.

use crate::Outcome;
use crate::args::Args;
use crate::sha256;

use reticle::program::ice40::{self, Programmer, SramPath};
use reticle::program::{ProgramError, ftdi, usb};

/// What went wrong, as the caller reports it.
pub(crate) enum Fault {
    /// The command line was wrong.
    Usage(String),
    /// The board, the file or the USB layer said no.
    Failed(String),
}

impl From<ProgramError> for Fault {
    fn from(err: ProgramError) -> Fault {
        Fault::Failed(err.to_string())
    }
}

/// Runs `program --ice40`.
pub(crate) fn run(args: &Args, say: &dyn Fn(&str)) -> Result<Outcome, Fault> {
    let quiet = args.flag("quiet");
    let offset = match args.option("offset") {
        None => 0,
        Some(text) => parse_number(text)
            .ok_or_else(|| Fault::Usage(format!("`--offset {text}` is not a number")))?,
    };
    let length = args
        .option("length")
        .map(|text| {
            parse_number(text)
                .map(|n| n as usize)
                .ok_or_else(|| Fault::Usage(format!("`--length {text}` is not a number")))
        })
        .transpose()?;
    let path = match args.option("sram-via") {
        None | Some("flash-output") => SramPath::FlashOutput,
        Some("host-output") => SramPath::HostOutput,
        Some(other) => {
            return Err(Fault::Usage(format!(
                "`--sram-via {other}`: the choices are `flash-output` (ADBUS2, for a board \
                 wired for flash programming) and `host-output` (ADBUS1, as iceprog -S)"
            )));
        }
    };

    enum Op {
        Probe,
        Sram(Vec<u8>),
        Read(String),
        Write(Vec<u8>),
        Boot,
    }
    let read_file = |path: &str| {
        std::fs::read(path).map_err(|e| Fault::Failed(format!("cannot read {path}: {e}")))
    };
    let op = match (
        args.flag("probe"),
        args.option("read-flash"),
        args.option("write-flash"),
        args.flag("boot"),
        args.positionals(),
    ) {
        (true, None, None, false, []) => Op::Probe,
        (false, Some(out), None, false, []) => Op::Read(out.to_owned()),
        (false, None, Some(file), false, []) => Op::Write(read_file(file)?),
        (false, None, None, true, []) => Op::Boot,
        (false, None, None, false, [file]) => Op::Sram(read_file(file)?),
        _ => {
            return Err(Fault::Usage(
                "with --ice40, give exactly one of: a .bin to load into the FPGA, \
                 --write-flash <file>, --read-flash <file>, --boot, or --probe"
                    .into(),
            ));
        }
    };
    // Check the file before the board is touched.
    if let Op::Sram(bytes) | Op::Write(bytes) = &op
        && !ice40::looks_like_bitstream(bytes)
        && !args.flag("raw")
    {
        return Err(Fault::Failed(
            "this is not an iCE40 bitstream: no 7E AA 99 7E synchronisation word near its \
             start. Nothing was sent to the board (pass --raw to write arbitrary bytes to the \
             flash, for restoring a backup)"
                .into(),
        ));
    }

    let cable = usb::Cable::open(args.option("device"))?;
    let chip = cable.chip();
    let hz = args
        .u32_option("clock")
        .ok()
        .flatten()
        .unwrap_or(ice40::DEFAULT_SPI_HZ);
    let divisor = ftdi::divisor_for_chip(chip, hz);
    say(&format!(
        "FTDI {} ({chip:?}), SPI clock {} Hz",
        if cable.serial().is_empty() {
            "with no serial number"
        } else {
            cable.serial()
        },
        ftdi::clock_hz_on(chip, divisor)
    ));
    let mut p = Programmer::new(cable, chip, divisor, ice40::ICEPROG_WIRING)?;

    let levels = p.bus_levels(false)?;
    say(&format!(
        "before: CDONE {}, CRESET_B {}, flash DI floats {} (ADBUS {:#04x})",
        u8::from(levels.cdone),
        u8::from(levels.creset),
        if levels.mosi { "high" } else { "low" },
        levels.raw
    ));
    if let Op::Probe = op {
        // The same with the FPGA held in reset, which is the state a
        // configuration-memory load starts from: what the flash's DI
        // floats to then is what the flash decodes during the load.
        let held = p.bus_levels(true)?;
        say(&format!(
            "in reset: CDONE {}, flash DI floats {} (ADBUS {:#04x})",
            u8::from(held.cdone),
            if held.mosi { "high" } else { "low" },
            held.raw
        ));
    }

    let percent = |what: &'static str| {
        let mut last = usize::MAX;
        move |done: usize, total: usize| {
            if quiet || total == 0 {
                return;
            }
            let pct = done * 100 / total;
            if pct / 10 != last / 10 || done == total {
                last = pct;
                eprint!("\r{what}: {pct:3}%");
                if done == total {
                    eprintln!();
                }
            }
        }
    };

    match op {
        Op::Sram(bytes) => {
            say(&format!(
                "loading {} bytes into the FPGA's configuration memory ({})",
                bytes.len(),
                match path {
                    SramPath::FlashOutput => "over ADBUS2, the net it shares with the flash's DO",
                    SramPath::HostOutput => "over ADBUS1, as iceprog -S",
                }
            ));
            let done = p.load_sram(&bytes, path)?;
            if !done {
                return Err(Fault::Failed(
                    "CDONE did not rise: the part did not accept the bitstream. The flash was \
                     not touched; a power cycle or `--boot` brings its image back"
                        .into(),
                ));
            }
            say("CDONE high: configured. The flash was not written");
        }
        Op::Probe | Op::Read(_) | Op::Write(_) => {
            p.take_flash()?;
            let id = p.flash_id()?;
            let capacity = id.capacity();
            say(&format!(
                "flash: JEDEC {:02X} {:02X} {:02X} ({}, {}), unique ID {}, status {:#04x}",
                id.jedec[0],
                id.jedec[1],
                id.jedec[2],
                id.maker(),
                capacity.map_or_else(|| "size unknown".to_owned(), |c| format!("{} KiB", c >> 10)),
                id.unique
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                p.status()?
            ));
            let result = match op {
                Op::Read(out) => {
                    let len =
                        length.unwrap_or(capacity.unwrap_or(0).saturating_sub(offset as usize));
                    let data = p.read(offset, len, &mut percent("reading"))?;
                    std::fs::write(&out, &data)
                        .map_err(|e| Fault::Failed(format!("cannot write {out}: {e}")))?;
                    say(&format!(
                        "read {} bytes from {offset:#x} into {out}, sha256 {}",
                        data.len(),
                        sha256::hex(&data)
                    ));
                    Ok(())
                }
                Op::Write(bytes) => p
                    .write(offset, &bytes, &mut percent("writing"))
                    .map(|report| {
                        say(&format!(
                            "wrote {} bytes at {offset:#x} and read them back: {} sectors \
                             erased, {} pages programmed, {} sectors already right",
                            bytes.len(),
                            report.sectors_erased,
                            report.pages_programmed,
                            report.sectors_unchanged
                        ));
                    })
                    .map_err(Fault::from),
                _ => Ok(()),
            };
            // Whatever happened, give the FPGA its flash back.
            boot(&mut p, say)?;
            result?;
        }
        Op::Boot => boot(&mut p, say)?,
    }
    Ok(Outcome::Ok)
}

/// Releases the FPGA to configure itself from the flash and reports
/// `CDONE`.
fn boot(p: &mut Programmer<usb::Cable>, say: &dyn Fn(&str)) -> Result<(), Fault> {
    match p.boot()? {
        Some(polls) => {
            say(&format!(
                "released CRESET_B: CDONE high after {polls} 1 ms polls (plus USB round trips)"
            ));
            Ok(())
        }
        None => Err(Fault::Failed(
            "released CRESET_B and CDONE stayed low for a second: the flash holds no image \
             this FPGA accepts"
                .into(),
        )),
    }
}

/// A decimal or `0x` hexadecimal number.
fn parse_number(text: &str) -> Option<u32> {
    match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}
