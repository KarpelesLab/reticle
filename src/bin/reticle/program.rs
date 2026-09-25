//! The `reticle program` command: load a bitstream into an attached
//! FPGA over JTAG.
//!
//! All the logic is in `reticle::program`, which is sans-I/O like the
//! rest of the library. What is here is the argument handling, the file
//! read, and the running commentary — and the order of operations,
//! which is the part with consequences:
//!
//! 1. read and check the bitstream container, before touching a cable;
//! 2. work out **which kind of adapter** carries the serial number
//!    `--device` names, from the bus, before anything is opened;
//! 3. open that adapter and bring up its transport;
//! 4. read `IDCODE` and **stop on a mismatch**, having written nothing;
//! 5. read the status register, so the `DONE` bit before is on record;
//! 6. clear the configuration memory, and confirm `DONE` went low — the
//!    part really did;
//! 7. shift the payload;
//! 8. start the part, then read the status back and report `DONE`.
//!
//! Step 4 is why the order matters. Everything after it is destructive
//! of the part's *volatile* configuration, and a part that answers the
//! wrong `IDCODE` is not the part the bitstream was built for. On the
//! ECP5 that is not a nicety: an LFE5U-12F and an LFE5U-25F are the same
//! die and differ only in their identifier, so the wrong file would
//! configure the part and assert `DONE` while doing something else.
//!
//! Step 2 matters for a different reason. There are two transports —
//! an FTDI MPSSE and a Cynthion's Apollo microcontroller — and they have
//! no byte-level protocol in common, so which one to use is a decision,
//! not a retry: `reticle::program::choose_adapter` makes it from what the
//! bus says is attached, and this file then runs one transport or the
//! other. Opening a Cynthion is not free either, because reaching Apollo
//! may take the USB port away from the board's FPGA and that is not
//! undone in software; the choice is made before it happens, and a file
//! that transport cannot serve — another vendor's, pointed at a Lattice
//! part — is refused before any handover rather than after one.
//!
//! Each transport has the sequences it has been run with, and they are
//! not the same set. Over an FTDI cable: a Xilinx 7-series `.bit` and a
//! Gowin `.fs`. Over Apollo: `--probe`, and a Lattice ECP5 `.bit`. An
//! ECP5 bitstream over a cable is refused, because the plans are
//! transport-neutral but that pairing has never been run.
//!
//! Nothing here can write flash, and nothing here touches the mode pins.

use std::time::Instant;

use crate::Outcome;
use crate::args::{ArgError, Args};

use reticle::program::{self, ProgramError, gowin, lattice, usb, xilinx};

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
        // listing this replaced showed FTDI cables only, so an attached
        // Cynthion did not appear at all.
        //
        // What each one is comes from `AdapterKind::describe`, which is
        // also what the dispatch line and the "no such serial" message
        // use, so the same board is never described two ways. For a
        // Cynthion that phrase says which mode it is in, because which
        // string the board reports depends on the mode it is in now.
        //
        // `AdapterKind::identifier` is the other half, and it is here
        // rather than in a footnote because the strings in this column
        // are not all the same kind of name: a Cynthion in gateware mode
        // advertises the board's *flash UID*, and the same board in
        // debugger mode reports the microcontroller's own serial number.
        // Someone copying a string out of this listing should know which
        // they copied.
        let attached = usb::adapters()?;
        if attached.is_empty() {
            println!("no programming adapter is attached");
            return Ok(Outcome::Ok);
        }
        for adapter in attached {
            println!(
                "{}  {}  [{}]",
                adapter.serial,
                adapter.kind.describe(),
                adapter.kind.identifier()
            );
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

    // Which transport to use is decided here, from one enumeration of the
    // bus, and before anything is opened. An FTDI cable and a Cynthion
    // are not alternatives to try in turn: their protocols have nothing
    // in common, and with several boards attached a fallback is how the
    // wrong one gets opened.
    let device = args.option("device");
    let attached = usb::adapters()?;
    let chosen = program::choose_adapter(device, &attached)?;
    say(&format!("adapter {chosen}"));

    if chosen.kind.is_cynthion() {
        // Refused *before* the handover, not after. Reaching Apollo can
        // take the USB port away from the board's FPGA
        // (`docs/apollo-protocol.md` §2), and §6 is why that is not undone
        // in software: the request that asks for the port back is accepted
        // and is not enough. So the cost of opening a Cynthion is paid for
        // the session, and it is not worth paying to then refuse.
        //
        // What it can be asked is `--probe` and an ECP5 `.bit`. Anything
        // else is another vendor's file pointed at a Lattice part, and
        // that is refused here rather than after the handover.
        match &load {
            Some(Load::Ecp5 { .. }) | None => {}
            Some(_) => {
                return Err(Fault::Failed(format!(
                    "{} is a Cynthion, whose FPGA is a Lattice ECP5, and this file is not an \
                     ECP5 bitstream. Nothing was sent to the board",
                    chosen.serial
                )));
            }
        }
        return cynthion(
            device,
            chosen.kind == program::AdapterKind::CynthionGateware,
            load,
            expect,
            args.option("clock").is_some(),
            quiet,
            &say,
        );
    }
    if let Some(Load::Ecp5 { .. }) = &load {
        return Err(Fault::Failed(format!(
            "{} is an FTDI cable, and the ECP5 configuration sequence in this crate has only \
             ever been run over a Cynthion's Apollo debugger. Driving it over MPSSE needs the \
             same plans encoded for that transport, which is not written. Nothing was sent",
            chosen.serial
        )));
    }

    let cable = usb::Cable::open(device)?;
    // The divisor depends on which FTDI part the adapter has, so it can
    // only be worked out once the cable is open.
    let chip = cable.chip();
    let divisor = reticle::program::ftdi::divisor_for_chip(chip, clock);
    say(&format!(
        "{chip:?}, TCK {} Hz",
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
        // An ECP5 bitstream never reaches an FTDI cable: `go` refuses it
        // before one is opened, because the configuration sequence has
        // only ever been run over a Cynthion.
        Some(Load::Ecp5 { .. }) => unreachable!("an ECP5 bitstream never reaches a cable"),
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
        Some(Load::Ecp5 { .. }) => unreachable!("an ECP5 bitstream never reaches a cable"),
        None => Ok(Outcome::Ok),
    }
}

/// Everything this command does through a Cynthion's Apollo debugger:
/// `--probe`, and loading an ECP5 bitstream into the part's configuration
/// SRAM.
///
/// The identifier is read first and the vendor-neutral way — `Test-Logic-
/// Reset`, then thirty-two bits out of DR, with **no instruction shifted**
/// — which is `reticle::program::jtag::idcode_plan`, the same plan the
/// FTDI path runs. Only the encoding differs. Nothing after that point
/// happens if the part is not the one the bitstream names.
///
/// **Only the volatile configuration SRAM is written.** A power cycle
/// reloads the part from the board's flash, which nothing here touches:
/// `reticle::program::lattice::NOT_SHIFTED` names the instruction that
/// would, and a test asserts no plan carries it.
fn cynthion(
    device: Option<&str>,
    gateware: bool,
    load: Option<Load>,
    expect: Option<u32>,
    clock_given: bool,
    quiet: bool,
    say: &dyn Fn(&str),
) -> Result<Outcome, Fault> {
    if clock_given {
        // Apollo is a TAP controller, not a shift engine: the host names
        // states and lengths and never a clock divisor. Saying so beats
        // accepting the number and ignoring it.
        say("--clock does not apply to a Cynthion: Apollo owns the TAP and is not told a TCK rate");
    }
    if gateware {
        say(
            "this Cynthion is running gateware, so its FPGA owns the USB port; asking the \
             gateware to give it up (docs/apollo-protocol.md §2). Nothing persistent happens \
             and the FPGA stays configured, but a replug or a power cycle is what brings the \
             gateware back",
        );
    }

    let debugger = usb::Debugger::open(device)?;
    // The serial number reported here is the *microcontroller's*, which is
    // not the one a board in gateware mode reports, so it is printed
    // rather than assumed to be what `--device` was given.
    say(&format!(
        "Apollo on {}{}",
        debugger.serial(),
        if debugger.handed_over() {
            " (the gateware gave up the USB port; a replug brings it back)"
        } else {
            ""
        }
    ));
    match debugger.id() {
        Ok(id) => say(&format!("  identifier: {id}")),
        Err(err) => say(&format!("  the firmware would not name itself: {err}")),
    }
    if let Ok(version) = debugger.firmware_version() {
        say(&format!("  firmware: {version}"));
    }
    if let Ok((major, minor)) = debugger.usb_api_version() {
        say(&format!("  USB API: {major}.{minor}"));
    }
    let capability = debugger.capability();
    say(&format!(
        "  max scan {} bits, quirks {:#010x} ({})",
        capability.max_scan_bits,
        capability.quirks,
        if debugger.capability_reported() {
            "the firmware's own answer"
        } else {
            // Not a footnote: the fallback's numbers are exactly what a
            // firmware that answered would most likely report, so without
            // this the line reads as a measurement when it is a default.
            "assumed; this firmware has no case for that request"
        }
    ));

    let idcode = debugger.idcode()?;
    // Exact equality, not `xilinx::idcode_matches`: that masks the top
    // nibble off as a silicon revision, and on an ECP5 the top nibble is
    // part of which part it is — `program::lattice` says why. On this
    // family that is not a nicety: the LFE5U-12F and the LFE5U-25F are
    // the same die and only the identifier tells them apart, so a
    // bitstream built for one would configure the other and produce a
    // part that asserts DONE and does the wrong thing.
    let expected = match &load {
        Some(Load::Ecp5 { expected, .. }) => Some(*expected),
        _ => expect,
    };
    if let Some(expected) = expected
        && idcode != expected
    {
        return Err(ProgramError::IdcodeMismatch {
            read: idcode,
            expected,
        }
        .into());
    }
    say(&lattice::describe(idcode));
    if !lattice::is_lattice(idcode) {
        // Everything from here shifts a Lattice instruction, and an
        // instruction shifted at the wrong vendor's part does not
        // misread: it executes whatever those bits land on.
        return Err(Fault::Failed(format!(
            "IDCODE {idcode:#010x} is not a Lattice part, and everything this command would do \
             next shifts a Lattice instruction. Nothing was written"
        )));
    }

    let outcome = match load {
        None => probe_ecp5(&debugger, say),
        Some(Load::Ecp5 { payload, .. }) => load_ecp5(&debugger, &payload, quiet, say),
        // `go` refused every other kind before the board was opened.
        Some(_) => unreachable!("only an ECP5 bitstream reaches a Cynthion"),
    };

    if debugger.handed_over() {
        // The honest end of a session that took the port: ask for it
        // back, and say that asking is not enough.
        match debugger.release_usb_to_fpga() {
            Ok(()) => say(
                "Apollo was told it may give the USB port back to the FPGA. That request on its \
                 own does not bring the gateware back — replug or power cycle the board \
                 (docs/apollo-protocol.md §6)",
            ),
            Err(err) => say(&format!(
                "could not tell Apollo to give the USB port back: {err}"
            )),
        }
    }
    outcome
}

/// How many status reads to wait for the ECP5 to stop being busy.
///
/// A read is a USB round trip, so this is a second or two — many times the
/// tens of milliseconds an SRAM erase takes. It is a count of reads and
/// not a duration on purpose: a test may never assert on a clock, and a
/// board that answers slowly is a board that gets more time, not a
/// failure.
const ECP5_POLLS: usize = 400;

/// Reads the ECP5 status register through an open session.
fn ecp5_status(session: &usb::JtagSession<'_>) -> Result<lattice::Status, Fault> {
    let (program, replies) = session.run_plan(&lattice::status_plan())?;
    Ok(lattice::Status(program.capture_u32(0, &replies)?))
}

/// Polls `LSC_CHECK_BUSY` until the part says it has finished, or gives up
/// after [`ECP5_POLLS`] reads.
fn ecp5_settle(session: &usb::JtagSession<'_>, what: &str) -> Result<(), Fault> {
    for _ in 0..ECP5_POLLS {
        let (program, replies) = session.run_plan(&lattice::busy_plan())?;
        let captures = program.assemble(&replies)?;
        let byte = captures.first().and_then(|c| c.first()).copied();
        if byte.is_none_or(|b| b & 1 == 0) {
            return Ok(());
        }
    }
    Err(Fault::Failed(format!(
        "the part was still busy after {ECP5_POLLS} status reads, {what}"
    )))
}

/// `--probe` on an ECP5: report the configuration status register.
///
/// Unlike the identifier this **does** shift an instruction —
/// `LSC_READ_STATUS` — which is why it happens only after the identifier
/// has said the part is a Lattice one. It writes nothing.
fn probe_ecp5(debugger: &usb::Debugger, say: &dyn Fn(&str)) -> Result<Outcome, Fault> {
    let session = debugger.jtag_session()?;
    let status = ecp5_status(&session)?;
    say(&format!("status: {status}"));
    say("nothing was written: reading a status register shifts an instruction and no more");
    Ok(Outcome::Ok)
}

/// The ECP5 SRAM sequence, from `LSC_REFRESH` to `DONE`.
///
/// The five plans are `reticle::program::lattice::sram_plans`, which says
/// what each one is and where the sequence comes from. What is here is the
/// order, the waiting — polling, never sleeping — and the checks between
/// the steps, because a failure has to be attributed to a step rather than
/// to the whole load.
///
/// It runs in **one JTAG session**: the pins are held from the first
/// instruction to the last, because `REQUEST_JTAG_STOP` does not merely
/// stop driving them, and a part in configuration mode must not have its
/// TAP clocked by something else halfway through.
fn load_ecp5(
    debugger: &usb::Debugger,
    payload: &[u8],
    quiet: bool,
    say: &dyn Fn(&str),
) -> Result<Outcome, Fault> {
    let plans = lattice::sram_plans(payload);
    let session = debugger.jtag_session()?;

    let before = ecp5_status(&session)?;
    say(&format!("status before: {before}"));

    // `LSC_REFRESH` restarts configuration. From here the part's volatile
    // configuration is going; the flash it boots from is not.
    let (program, replies) = session.run_plan(&plans.refresh)?;
    let again = program.capture_u32(0, &replies)?;
    say(&format!("after LSC_REFRESH, IDCODE {again:#010x}"));
    if !lattice::is_lattice(again) {
        return Err(Fault::Failed(format!(
            "after LSC_REFRESH the part answered {again:#010x}, which is not a Lattice \
             identifier. Nothing has been erased"
        )));
    }
    ecp5_settle(&session, "restarting configuration")?;

    let (program, replies) = session.run_plan(&plans.enable)?;
    let enabled = lattice::Status(program.capture_u32(0, &replies)?);
    say(&format!("status after ISC_ENABLE: {enabled}"));
    if !enabled.isc_enabled() {
        return Err(Fault::Failed(format!(
            "the part did not enter configuration mode: ISC_ENABLE is clear in status \
             {enabled}. Nothing has been erased"
        )));
    }

    let (program, replies) = session.run_plan(&plans.erase)?;
    let erased = lattice::Status(program.capture_u32(0, &replies)?);
    ecp5_settle(&session, "erasing the configuration SRAM")?;
    say(&format!("status after ISC_ERASE: {erased}"));
    if erased.done() {
        return Err(Fault::Failed(format!(
            "DONE is still set after ISC_ERASE; the part did not clear its configuration \
             SRAM (status {erased})"
        )));
    }

    let started = Instant::now();
    let mut last = 0;
    session.run_plan_with_progress(&plans.burst, &mut |written, total| {
        if quiet || total == 0 {
            return;
        }
        let percent = written * 100 / total;
        if percent >= last + 10 {
            last = percent - percent % 10;
            println!("  {last}% ({written} of {total} bytes on the wire)");
        }
    })?;
    say(&format!(
        "{} bytes of bitstream shifted in {:.1} s",
        payload.len(),
        started.elapsed().as_secs_f64()
    ));
    ecp5_settle(&session, "taking the bitstream")?;

    let (program, replies) = session.run_plan(&plans.start)?;
    let after = lattice::Status(program.capture_u32(0, &replies)?);
    say(&format!("status after ISC_DISABLE: {after}"));
    let faults = after.faults();
    if !faults.is_empty() {
        return Err(Fault::Failed(format!(
            "the part reported {} after configuration: status {after}. Its configuration SRAM \
             has been overwritten; a power cycle reloads it from the board's flash",
            faults.join(", ")
        )));
    }
    if !after.done() {
        return Err(Fault::Failed(format!(
            "the part did not assert DONE after configuration: status {after}. Its \
             configuration SRAM has been overwritten; a power cycle reloads it from the \
             board's flash"
        )));
    }
    println!(
        "DONE is high: the part accepted the bitstream and is running it. Only the volatile \
         configuration SRAM was written; a power cycle reloads the board's flash."
    );
    Ok(Outcome::Ok)
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
    /// A Lattice ECP5 `.bit`, whole.
    ///
    /// Unlike the Xilinx one this keeps the **entire file**, header and
    /// all: `LSC_BITSTREAM_BURST` is handed the file as it is on disk and
    /// the part's own configuration engine finds the preamble in it. That
    /// is what `ecpprog` and Apollo both do, and it is why there is no
    /// `payload()` to take here.
    Ecp5 { payload: Vec<u8>, expected: u32 },
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
    // Two vendors use `.bit`, so the extension decides nothing and the
    // bytes decide everything. A Lattice file opens with a metadata
    // header and the ECP5 preamble; a Xilinx one opens with a big-endian
    // length and a run of `0x0f 0xf0`.
    if lattice::is_ecp5_bit(&file) {
        let header = lattice::read_bit_header(&file)
            .map_err(|err| Fault::Failed(format!("`{path}`: {err}")))?;
        let Some(named) = header.idcode else {
            return Err(Fault::Failed(format!(
                "`{path}` is an ECP5 bitstream with no VERIFY_ID command, so there is nothing \
                 to check the part against. Reticle will not load one: the LFE5U-12F and the \
                 LFE5U-25F are the same die with different identifiers, and a file that names \
                 neither cannot say which it was built for"
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
            "{path}: Lattice ECP5, {} bytes{}, for IDCODE {named:#010x} ({}){}",
            file.len(),
            if header.compressed {
                ", compressed"
            } else {
                ""
            },
            lattice::ecp5_part(named).unwrap_or("a part this command does not know"),
            if header.metadata.is_empty() {
                String::new()
            } else {
                format!(" — {}", header.metadata.join("; "))
            }
        ));
        return Ok(Load::Ecp5 {
            payload: file,
            expected,
        });
    }
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
            println!("  {last}% ({written} of {total} bytes on the wire)");
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
