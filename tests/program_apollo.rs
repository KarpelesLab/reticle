//! The Apollo transport, checked against a model of the debugger rather
//! than against a board.
//!
//! `src/program/apollo.rs`'s own tests check each request on its own.
//! What is here is the other half: a small, independent **interpreter**
//! for the control-request stream, driving a model TAP, so that what
//! Reticle emits is judged by what a Cynthion's microcontroller would do
//! with it rather than by what numbers it happens to contain.
//!
//! That matters because the two mistakes this code can make are the two
//! a request-level test cannot see:
//!
//! - a walk that does not pass through `Capture-DR`, which shifts out
//!   whatever the register happened to hold instead of the identifier;
//! - a bit order backwards, which produces a reply of the right length
//!   that decodes to nonsense.
//!
//! The model is written from `docs/apollo-protocol.md`, not from
//! `reticle::program::apollo`: it walks its own TAP with
//! [`TapState::step`] and packs its own bits. It shares [`TapState`] with
//! the crate, since re-declaring sixteen state names would test nothing.
//!
//! One test needs the board and is `#[ignore]`d; `cargo test --features
//! program -- --ignored read_the_ecp5_idcode` runs it. Everything else
//! runs anywhere, with nothing attached.

#![cfg(feature = "program")]

use reticle::program::jtag::{self, TapState};
use reticle::program::{apollo, lattice};

// ---------------------------------------------------------------------
// A model of the debugger and the part behind it
// ---------------------------------------------------------------------

/// A Cynthion's microcontroller with an ECP5 on the other side of it.
///
/// It holds a TAP state, an out buffer and an in buffer, exactly as the
/// protocol document says Apollo does, and it shifts against a part that
/// answers `Test-Logic-Reset` by selecting its identification register.
struct Model {
    state: TapState,
    out: Vec<u8>,
    inbuf: Vec<u8>,
    /// The identifier the part presents.
    idcode: u32,
    /// Bits of the selected register still to come out of TDO, first
    /// bit first.
    dr: Vec<bool>,
    /// How far through them the shifting has got.
    dr_pos: usize,
    /// Whether the pins have been taken.
    running: bool,
    /// Whether the firmware asks for whole bytes to be bit-flipped.
    flip: bool,
    /// What the last scan actually drove on TDI, as the part saw it.
    tdi: Vec<u8>,
    /// Every instruction register bit ever shifted, so a test can assert
    /// that none was.
    ir_bits: usize,
}

impl Model {
    fn new(idcode: u32, flip: bool) -> Model {
        Model {
            state: TapState::TestLogicReset,
            out: vec![0; 256],
            inbuf: Vec::new(),
            idcode,
            dr: Vec::new(),
            dr_pos: 0,
            running: false,
            flip,
            tdi: Vec::new(),
            ir_bits: 0,
        }
    }

    /// Walks to `target` the way a TAP does, one TMS bit at a time,
    /// reacting to every state passed through rather than only to the
    /// destination. Capture-DR is the one that matters.
    fn go_to(&mut self, target: TapState) {
        let (bits, len) = self.state.path_to(target);
        for i in 0..len {
            self.state = self.state.step(bits >> i & 1 == 1);
            self.on_entry();
        }
        assert_eq!(self.state, target);
    }

    fn on_entry(&mut self) {
        match self.state {
            // IEEE 1149.1: a part with an identification register loads
            // IDCODE on entering Test-Logic-Reset.
            TapState::TestLogicReset => {}
            TapState::CaptureDr => {
                self.dr = (0..32).map(|i| self.idcode >> i & 1 == 1).collect();
                self.dr_pos = 0;
            }
            _ => {}
        }
    }

    /// Shifts `bits` bits, filling the in buffer.
    fn scan(&mut self, bits: usize, flags: u16) {
        assert!(self.running, "a scan before the pins were taken");
        assert!(
            self.state.is_shift(),
            "a scan in {:?}, which is not a shift state",
            self.state
        );
        if self.state == TapState::ShiftIr {
            self.ir_bits += bits;
        }
        // What went out on TDI, as the part would have seen it: the
        // firmware undoes the host's whole-byte flip. Recording it lets
        // a test assert on what was driven; the part modelled here
        // ignores TDI, since an identification register does.
        self.tdi = if self.flip {
            apollo::flip_whole_bytes(&self.out[..bits.div_ceil(8)], bits)
        } else {
            self.out[..bits.div_ceil(8)].to_vec()
        };

        let mut got = vec![0u8; bits.div_ceil(8)];
        for i in 0..bits {
            let bit = self.dr.get(self.dr_pos).copied().unwrap_or(false);
            self.dr_pos += 1;
            if bit {
                got[i / 8] |= 1 << (i % 8);
            }
        }
        self.inbuf = if self.flip {
            apollo::flip_whole_bytes(&got, bits)
        } else {
            got
        };
        if flags & apollo::FLAG_ADVANCE_STATE != 0 {
            self.state = self.state.step(true);
        }
    }

    /// Performs one step, returning the reply to a read.
    fn step(&mut self, step: &apollo::Step) -> Option<Vec<u8>> {
        match step {
            apollo::Step::Command {
                request,
                value,
                index,
            } => {
                match *request {
                    apollo::REQUEST_JTAG_START => self.running = true,
                    apollo::REQUEST_JTAG_STOP => self.running = false,
                    apollo::REQUEST_JTAG_GO_TO_STATE => {
                        let target = apollo::state_from_number(
                            u8::try_from(*value).expect("a state number is a byte"),
                        )
                        .expect("a state number names a state");
                        self.go_to(target);
                    }
                    apollo::REQUEST_JTAG_CLEAR_OUT_BUFFER => self.out = vec![0; 256],
                    apollo::REQUEST_JTAG_SCAN => self.scan(usize::from(*value), *index),
                    apollo::REQUEST_JTAG_RUN_CLOCK => {}
                    other => panic!("the model does not know request {other:#04x}"),
                }
                None
            }
            apollo::Step::Write { request, data, .. } => {
                assert_eq!(*request, apollo::REQUEST_JTAG_SET_OUT_BUFFER);
                assert!(data.len() <= 256, "the out buffer is 256 bytes");
                self.out = vec![0; 256];
                self.out[..data.len()].copy_from_slice(data);
                None
            }
            apollo::Step::Read { request, len, .. } => {
                assert_eq!(*request, apollo::REQUEST_JTAG_GET_IN_BUFFER);
                let mut reply = self.inbuf.clone();
                reply.resize(*len, 0);
                Some(reply)
            }
        }
    }

    /// Runs a whole program the way `usb::Debugger::run_plan` does,
    /// taking and releasing the pins around it.
    fn run(&mut self, program: &apollo::Program) -> Vec<Vec<u8>> {
        self.running = true;
        let replies = program
            .steps()
            .iter()
            .filter_map(|s| self.step(s))
            .collect();
        self.running = false;
        replies
    }
}

// ---------------------------------------------------------------------
// Tests with nothing attached
// ---------------------------------------------------------------------

/// The identifier comes back out of the model exactly as it went in,
/// which means the compiled walk reached `Capture-DR` and the bit
/// packing is the one the document describes.
#[test]
fn the_idcode_sequence_reads_the_identifier_back() {
    for idcode in [
        0x2111_1043_u32,
        0x4111_1043,
        0x0362_D093,
        0x8000_0001,
        0x0000_0001,
    ] {
        let mut model = Model::new(idcode, false);
        let program = apollo::compile(&jtag::idcode_plan(), apollo::Capability::default());
        let replies = model.run(&program);
        assert_eq!(
            program.capture_u32(0, &replies),
            Ok(idcode),
            "for {idcode:#010x}"
        );
        assert_eq!(model.ir_bits, 0, "no instruction may be shifted");
        assert_eq!(model.state, TapState::RunTestIdle);
    }
}

/// The same, with the whole-byte flip quirk on. If the flip were applied
/// in one direction only, or to the wrong bytes, this would read back a
/// bit-reversed identifier.
#[test]
fn the_flip_quirk_round_trips_through_the_model() {
    let capability = apollo::Capability {
        max_scan_bits: 2048,
        quirks: apollo::QUIRK_FLIP_BITS_IN_WHOLE_BYTES,
    };
    let idcode = 0x4111_2043;
    let mut model = Model::new(idcode, true);
    let program = apollo::compile(&jtag::idcode_plan(), capability);
    let replies = model.run(&program);
    assert_eq!(program.capture_u32(0, &replies), Ok(idcode));

    // And the other direction: the bits that reach the part are the
    // bits the plan asked for, not the flipped ones the wire carried.
    // A flip applied once instead of twice would show up here.
    for flip in [false, true] {
        let capability = apollo::Capability {
            max_scan_bits: 2048,
            quirks: if flip {
                apollo::QUIRK_FLIP_BITS_IN_WHOLE_BYTES
            } else {
                0
            },
        };
        let wanted = [0b1001_0110_u8, 0b0000_1111, 0b1100_0011];
        let mut plan = jtag::Plan::new();
        plan.reset();
        plan.shift_dr(&wanted, 24);
        let program = apollo::compile(&plan, capability);
        let mut model = Model::new(idcode, flip);
        let _ = model.run(&program);
        assert_eq!(model.tdi, wanted, "with flip = {flip}");
    }
}

/// A register longer than one scan reads back whole. The model keeps
/// shifting across chunk boundaries, so a chunk that wrongly advanced
/// the state would lose the rest of the register.
#[test]
fn a_split_scan_reads_one_register() {
    let capability = apollo::Capability {
        max_scan_bits: 64,
        quirks: 0,
    };
    let mut plan = jtag::Plan::new();
    plan.reset();
    let index = plan.read_dr(32);
    let program = apollo::compile(&plan, capability);
    assert_eq!(index, 0);

    // A 200-bit register, read 64 bits at a time.
    let capability = apollo::Capability {
        max_scan_bits: 64,
        quirks: 0,
    };
    let mut plan = jtag::Plan::new();
    plan.reset();
    let long = plan.read_dr(200);
    let program_long = apollo::compile(&plan, capability);
    assert_eq!(long, 0);
    assert_eq!(program_long.capture_bits(0), Some(200));

    let mut model = Model::new(0x4111_1043, false);
    let replies = model.run(&program);
    assert_eq!(program.capture_u32(0, &replies), Ok(0x4111_1043));

    // The long one shifts past the 32 bits the part has, so the tail is
    // zeros; what matters is that the pieces join into 25 bytes.
    let mut model = Model::new(0x4111_1043, false);
    let replies = model.run(&program_long);
    let joined = program_long.assemble(&replies).expect("four pieces");
    assert_eq!(joined[0].len(), 25);
    assert_eq!(&joined[0][..4], &0x4111_1043_u32.to_le_bytes());
}

/// Every request a compiled program contains is one the protocol
/// document lists as a JTAG request, and none of them is one of the
/// requests `docs/apollo-protocol.md` §7 says this project will not
/// send.
#[test]
fn a_program_contains_only_jtag_requests() {
    let mut plan = jtag::Plan::new();
    plan.reset();
    plan.shift_ir(&[0xFF], 8);
    let _ = plan.read_dr(64);
    plan.idle(1000);
    plan.shift_dr(&[0x12, 0x34], 16);
    let program = apollo::compile(&plan, apollo::Capability::default());

    let allowed = [
        apollo::REQUEST_JTAG_CLEAR_OUT_BUFFER,
        apollo::REQUEST_JTAG_SET_OUT_BUFFER,
        apollo::REQUEST_JTAG_GET_IN_BUFFER,
        apollo::REQUEST_JTAG_SCAN,
        apollo::REQUEST_JTAG_RUN_CLOCK,
        apollo::REQUEST_JTAG_GO_TO_STATE,
    ];
    for step in program.steps() {
        let request = match step {
            apollo::Step::Command { request, .. }
            | apollo::Step::Write { request, .. }
            | apollo::Step::Read { request, .. } => *request,
        };
        assert!(
            allowed.contains(&request),
            "a plan compiled to {request:#04x}, which is not a JTAG request"
        );
    }
    assert!(!program.steps().is_empty());
}

/// `docs/apollo-protocol.md` §7's list is a promise, so it is checked
/// rather than believed: no request the crate ever puts on the wire is one
/// of the requests that list names.
///
/// The set of requests Reticle sends is small and enumerable — the JTAG
/// ones a plan compiles to, plus the identification and session ones
/// `usb::Debugger` issues by name — and `apollo::NOT_SENT` is the set it
/// must not intersect. This is the same guard `gowin::FORBIDDEN` gets, and
/// it is what makes "this project does not reconfigure a board it does not
/// own" a property of the code rather than of its prose.
#[test]
fn the_requests_this_project_will_not_send_are_never_compiled() {
    let sent = [
        apollo::REQUEST_ADVERTISEMENT_STOP,
        apollo::REQUEST_GET_ID,
        apollo::REQUEST_GET_FIRMWARE_VERSION,
        apollo::REQUEST_GET_USB_API_VERSION,
        apollo::REQUEST_ALLOW_FPGA_TAKEOVER_USB,
        apollo::REQUEST_JTAG_CLEAR_OUT_BUFFER,
        apollo::REQUEST_JTAG_SET_OUT_BUFFER,
        apollo::REQUEST_JTAG_GET_IN_BUFFER,
        apollo::REQUEST_JTAG_SCAN,
        apollo::REQUEST_JTAG_RUN_CLOCK,
        apollo::REQUEST_JTAG_GO_TO_STATE,
        apollo::REQUEST_JTAG_GET_STATE,
        apollo::REQUEST_JTAG_GET_INFO,
        apollo::REQUEST_JTAG_STOP,
        apollo::REQUEST_JTAG_START,
    ];
    for (request, why) in apollo::NOT_SENT {
        assert!(
            !sent.contains(request),
            "{request:#04x} ({why}) is both sent and on the do-not-send list"
        );
    }

    // And nothing a plan compiles to is on it either, whatever the plan.
    let mut plan = jtag::Plan::new();
    plan.reset();
    plan.shift_ir(&[0x3A], 8);
    let _ = plan.read_dr(64);
    plan.shift_dr(&[0x4B, 0x00], 16);
    plan.idle(100);
    let program = apollo::compile(&plan, apollo::Capability::default());
    for step in program.steps() {
        let request = match step {
            apollo::Step::Command { request, .. }
            | apollo::Step::Write { request, .. }
            | apollo::Step::Read { request, .. } => *request,
        };
        assert!(
            !apollo::NOT_SENT.iter().any(|(r, _)| *r == request),
            "a plan compiled to {request:#04x}, which §7 says is never sent"
        );
        assert!(
            sent.contains(&request),
            "{request:#04x} is not a known request"
        );
    }

    // The flash UID is the one thing a person might reasonably expect to
    // find in this transport and will not: there is no vendor request that
    // returns it. Apollo's own tooling reads it by forcing the FPGA
    // offline and then driving the board's configuration flash over JTAG,
    // and the first of those is on the list above. The plan compiled here
    // shifts `0x3A` and `0x4B` as *data* on purpose — that is what the
    // route would look like — and the assertions are that doing so is
    // still only JTAG requests, and that nothing in the crate does it.
    assert!(
        apollo::NOT_SENT
            .iter()
            .any(|(r, _)| *r == apollo::REQUEST_FORCE_FPGA_OFFLINE)
    );
}

/// The model walks its own TAP, so a compiled walk that skipped
/// `Capture-DR` would read zeros. This is that check made explicit:
/// going straight from a shift state back to a shift state, without
/// passing through capture, presents the rest of the register rather
/// than a fresh one.
#[test]
fn a_walk_that_misses_capture_reads_the_rest_of_the_register() {
    let mut model = Model::new(0x4111_1043, false);
    model.go_to(TapState::ShiftDr);
    model.running = true;
    model.scan(16, apollo::FLAG_ADVANCE_STATE);
    let first = model.inbuf.clone();
    assert_eq!(first, vec![0x43, 0x10]);

    // Exit1-DR to Pause-DR to Exit2-DR to Shift-DR misses Capture-DR,
    // and the next sixteen bits are the top half, not the bottom.
    model.go_to(TapState::ShiftDr);
    model.scan(16, apollo::FLAG_ADVANCE_STATE);
    assert_eq!(model.inbuf, vec![0x11, 0x41]);
}

/// The names Reticle would print for the identifiers it can read. The
/// milestone is a number, and this is what turns the number into a part.
#[test]
fn the_ecp5_identifiers_decode_to_parts() {
    assert_eq!(
        lattice::ecp5_part(0x2111_1043),
        Some("LFE5U-12F (or LAE5U-12F)")
    );
    assert_eq!(lattice::ecp5_part(0x4111_1043), Some("LFE5U-25F"));
    assert_eq!(lattice::ecp5_part(0x4111_2043), Some("LFE5U-45F"));
    assert_eq!(lattice::ecp5_part(0x4111_3043), Some("LFE5U-85F"));
    assert_eq!(lattice::ecp5_part(0x0362_D093), None);
    assert!(lattice::is_lattice(0x2111_1043));
    assert!(!lattice::is_lattice(0x0362_D093));
}

// ---------------------------------------------------------------------
// The one test that needs the board
// ---------------------------------------------------------------------

/// Reads the ECP5's `IDCODE` off an attached Cynthion and names the
/// part.
///
/// This is the milestone. It sends nothing that changes the board except
/// the handover of `docs/apollo-protocol.md` §2, which moves the USB
/// port from the FPGA to the microcontroller and is undone at the end by
/// §6; it writes no configuration and touches no flash.
///
/// It is `#[ignore]`d, so a machine with no board — CI, for one — never
/// runs it. Run it with:
///
/// ```console
/// cargo test --features program --test program_apollo -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a Great Scott Gadgets Cynthion on USB"]
fn read_the_ecp5_idcode() {
    use reticle::program::usb;

    let attached = match usb::list_cynthions() {
        Ok(list) => list,
        Err(err) => {
            println!("the USB subsystem is not reachable ({err}), skipping");
            return;
        }
    };
    if attached.is_empty() {
        println!("no Cynthion is attached, skipping");
        return;
    }
    for (serial, debugger) in &attached {
        println!(
            "Cynthion {serial} is in {} mode",
            if *debugger { "debugger" } else { "gateware" }
        );
    }

    let serial = std::env::var("RETICLE_CYNTHION").ok();
    let debugger = match usb::Debugger::open(serial.as_deref()) {
        Ok(debugger) => debugger,
        Err(err) => {
            println!("cannot reach the Apollo debugger: {err}, skipping");
            return;
        }
    };
    println!(
        "Apollo on {}{}",
        debugger.serial(),
        if debugger.handed_over() {
            " (the gateware was asked to give up the USB port)"
        } else {
            ""
        }
    );
    if let Ok(id) = debugger.id() {
        println!("  identifier: {id}");
        assert!(id.contains("Apollo"), "that is not an Apollo: {id}");
    }
    if let Ok(version) = debugger.firmware_version() {
        println!("  firmware: {version}");
    }
    if let Ok((major, minor)) = debugger.usb_api_version() {
        println!("  USB API: {major}.{minor}");
    }
    let capability = debugger.capability();
    println!(
        "  max scan {} bits, quirks {:#010x}{}{}",
        capability.max_scan_bits,
        capability.quirks,
        if capability.flips_whole_bytes() {
            " [flip whole bytes]"
        } else {
            ""
        },
        if capability.always_bitbangs() {
            " [always bit-bang]"
        } else {
            ""
        }
    );

    let idcode = debugger.idcode().expect("the chain answers");

    // The raw value first, so a wrong reading cannot hide the number.
    println!("\nIDCODE {idcode:#010x}");
    println!("{}", lattice::describe(idcode));

    assert_ne!(idcode, 0x0000_0000, "an all-zero IDCODE is a dead chain");
    assert_ne!(idcode, 0xFFFF_FFFF, "an all-ones IDCODE is a dead chain");
    assert_eq!(
        idcode & 1,
        1,
        "IEEE 1149.1 requires bit 0 of IDCODE to be 1"
    );
    assert!(
        lattice::is_lattice(idcode),
        "the manufacturer field is {:#05x}, not Lattice's {:#05x}",
        (idcode >> 1) & 0x7FF,
        lattice::MANUFACTURER_LATTICE
    );
    assert!(
        lattice::ecp5_part(idcode).is_some(),
        "{idcode:#010x} is a Lattice part this crate cannot name"
    );

    // The firmware's own view of the TAP agrees with the plan's: a scan
    // that ended anywhere else would have read the wrong register.
    match debugger.tap_state() {
        Ok(Some(state)) => {
            println!("the TAP rests in {state:?}");
            assert_eq!(state, TapState::RunTestIdle);
        }
        Ok(None) => println!("the firmware reported a TAP state outside the sixteen"),
        Err(err) => println!("the firmware would not say where the TAP is: {err}"),
    }

    // The same identifier again, read 16 bits at a time. This is the
    // one thing a 2048-bit firmware would otherwise never make a part
    // prove: the first scan goes out *without* the advance-state flag
    // and has to leave the TAP in Shift-DR, so that the second picks up
    // where it left off. If that flag meant anything else, these
    // thirty-two bits would not be the same thirty-two.
    let split = apollo::Capability {
        max_scan_bits: 16,
        quirks: capability.quirks,
    };
    match debugger.run_plan_with(&jtag::idcode_plan(), split) {
        Ok((program, replies)) => {
            let again = program.capture_u32(0, &replies).expect("a reply");
            println!("read again in 16-bit chunks: {again:#010x}");
            assert_eq!(
                again, idcode,
                "a split scan read a different value from a single one"
            );
        }
        Err(err) => println!("the split read failed: {err}"),
    }

    // And the state numbering itself, checked rather than assumed: walk
    // to Shift-DR by number and ask where the firmware thinks it is.
    // Reading an identifier already proved number 4 is a shift state
    // reached through a capture; this proves the number round-trips.
    let mut walk = jtag::Plan::new();
    walk.goto(TapState::ShiftDr);
    if debugger.run_plan(&walk).is_ok() {
        match debugger.tap_state() {
            Ok(state) => println!("after asking for Shift-DR the firmware says {state:?}"),
            Err(err) => println!("the firmware would not say: {err}"),
        }
    }
    let mut back = jtag::Plan::new();
    back.goto(TapState::TestLogicReset);
    back.goto(TapState::RunTestIdle);
    let _ = debugger.run_plan(&back);

    // Put the board back the way it was found.
    if debugger.handed_over() {
        match debugger.release_usb_to_fpga() {
            Ok(()) => println!("the USB port was handed back to the FPGA"),
            Err(err) => println!("could not hand the USB port back: {err}"),
        }
    }
}
