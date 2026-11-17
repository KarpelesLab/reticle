//! The iCE40 SPI programmer against a real board.
//!
//! The sequences are checked against a model of the bus in
//! `src/program/ice40/tests.rs`, which runs anywhere. This is the part
//! that needs a board — an iCE40 with an FT2232H wired as `iceprog`
//! expects — and is `#[ignore]`d:
//!
//! ```sh
//! cargo test --features program --test program_ice40 -- --ignored
//! ```
//!
//! It **writes nothing**: it holds the FPGA in reset, reads the flash's
//! identifiers and its first kilobyte twice, and releases the FPGA to boot
//! from the flash again, checking that `CDONE` comes back. With no FT2232H
//! attached it says so and passes, as every hardware test here does.
//!
//! What it would catch: a wiring table that does not match the board (the
//! flash would not answer its JEDEC identifier), a read that is not
//! repeatable, and a release that leaves the part unconfigured. What it
//! would not: anything about writing, which is left to the model and to a
//! person who has backed the flash up first.

#![cfg(feature = "program")]

use reticle::program::{ProgramError, ftdi, ice40, usb};

#[test]
#[ignore = "needs an iCE40 board with an FT2232H attached"]
fn identify_and_read_a_real_flash() {
    let cable = match usb::Cable::open(None) {
        Ok(cable) => cable,
        Err(ProgramError::NoDevice { .. }) => {
            eprintln!("skipping: no FT2232H is attached");
            return;
        }
        Err(err) => panic!("opening the FT2232H: {err}"),
    };
    let chip = cable.chip();
    let divisor = ftdi::divisor_for_chip(chip, ice40::DEFAULT_SPI_HZ);
    let mut p = ice40::Programmer::new(cable, chip, divisor, ice40::ICEPROG_WIRING)
        .expect("configuring the MPSSE");
    p.take_flash().expect("holding the FPGA in reset");
    let id = p.flash_id().expect("reading the flash identifiers");
    let Some(capacity) = id.capacity() else {
        p.boot().ok();
        panic!(
            "the flash answered JEDEC {:02X?}: the wiring is not iceprog's, or nothing is there",
            id.jedec
        );
    };
    eprintln!(
        "flash {:02X?} ({}, {} KiB)",
        id.jedec,
        id.maker(),
        capacity >> 10
    );
    let a = p.read(0, 1024, &mut |_, _| {}).expect("first read");
    let b = p.read(0, 1024, &mut |_, _| {}).expect("second read");
    assert_eq!(a, b, "two reads of the same kilobyte differ");
    let booted = p.boot().expect("releasing the FPGA");
    if ice40::looks_like_bitstream(&a) {
        assert!(
            booted.is_some(),
            "the flash holds an image and CDONE stayed low"
        );
    } else {
        eprintln!("the flash holds no image at 0, so CDONE was not expected");
    }
}
