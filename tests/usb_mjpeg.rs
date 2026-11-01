//! `testdata/fpga/cynthion/usb_mjpeg.v`: a Cynthion that is a camera with
//! no lens.
//!
//! `ip/video/pattern_gen` into `ip/video/mjpeg_enc` into
//! `ip/usb/usb_cdc_acm`, so that what a host reads off `/dev/ttyACM*` is a
//! Motion JPEG stream — whole baseline greyscale JPEG files, one after
//! another, which `ffmpeg` demuxes by looking for `FF D8` and needs no
//! container and no framing of ours to play.
//!
//! **Why this fits a 12F and `examples/nes`'s encoder does not.** The same
//! encoder behind a 256x240 frame buffer is 46 `DP16KD` against the 32
//! Lattice specifies for an `LFE5U-12`, and `examples/nes/README.md` says
//! so. `pattern_gen` has no frame buffer: its picture is a function of `x`
//! and `y`, so it can be asked for its pixels block at a time as readily as
//! in raster order, and the memory here is the encoder's seven blocks and
//! nothing else.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `the_tables_in_the_rom_are_annex_k_as_the_corpus_records_them` | the two Huffman tables this design walks into the encoder are the bytes `testdata/ip/mjpeg_enc_corpus.txt` already holds, read out of the Verilog and compared |
//! | `the_header_is_the_one_the_nes_examples_golden_already_carries` | the 310 header bytes of a 256x240 frame are byte-for-byte the golden's, so the loader wrote the right tables in the right order |
//! | `the_stream_is_one_whole_jpeg_file_after_another` | three frames at 64x64, each `SOI` to `EOI` with one of every marker, and consecutive frames that differ — a still picture could not |
//! | `the_design_maps_onto_the_cynthions_ecp5` | it fits an `LFE5U-12F`: the block RAMs and the lookup tables, against Lattice's Table 1.1 |
//! | `the_board_streams_video_over_its_usb_port` | **on the part**: programmed, enumerated, read off `/dev/ttyACM*`, cut at `FF D8` and handed to `ffprobe` |
//!
//! What none of this would catch: anything about the picture's *fidelity*.
//! The transform is not bit-exact against any other implementation and
//! cannot be — T.81 defines it mathematically — so
//! `ip/video/mjpeg_enc/README.md` is where the measured distance from a
//! double-precision transform lives, and `examples/nes`'s golden is where a
//! whole file is compared against ffmpeg's decoder.

#![cfg(all(feature = "verilog", feature = "sim"))]

use std::fs;
use std::path::PathBuf;

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::sim::{NetHandle, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

/// The design and everything it instantiates, in the order
/// `reticle fpga` is given them.
const SOURCES: [&str; 12] = [
    "testdata/fpga/cynthion/usb_mjpeg.v",
    "ip/usb/usb_cdc_acm/rtl/usb_cdc_acm.v",
    "ip/usb/usb_cdc_acm/rtl/usb_cdc_req.v",
    "ip/usb/usb_cdc_acm/rtl/usb_cdc_acm_ulpi.v",
    "ip/usb/usb_device_ulpi/rtl/usb_ulpi_link.v",
    "ip/usb/usb_device_ulpi/rtl/usb_device_ulpi.v",
    "ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v",
    "ip/video/mjpeg_enc/rtl/jpeg_fdct.v",
    "ip/video/mjpeg_enc/rtl/jpeg_quant.v",
    "ip/video/mjpeg_enc/rtl/jpeg_entropy.v",
    "ip/video/mjpeg_enc/rtl/mjpeg_enc.v",
    "ip/video/pattern_gen/rtl/pattern_gen.v",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    let path = root().join(path);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Elaborates `usb_mjpeg` with the picture size given, leaking the design
/// so a `Simulator` can borrow it for the rest of the test.
///
/// The size is a parameter override rather than a second copy of the
/// design: 64x64 is 64 blocks where 256x240 is 960, and a test that wants
/// three whole frames can afford the first and not the second.
///
/// `None` when the library is not in this copy of the crate. `Cargo.toml`
/// excludes `/ip` from the package, so the published crate has the design
/// and not the blocks it instantiates, and every test here says so and
/// skips rather than failing — the same thing `tests/ip_library.rs` does
/// for the crypto console.
fn design(width: u32, height: u32) -> Option<&'static Design> {
    if !root().join(SOURCES[1]).is_file() {
        println!("skipping: ip/ is not in this copy of the crate");
        return None;
    }
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let files: Vec<_> = SOURCES
        .iter()
        .map(|name| {
            let id = map.add(*name, read(name)).expect("the source fits");
            parse_source(
                &mut map,
                id,
                Dialect::Verilog2005,
                &mut NoIncludes,
                &mut diags,
            )
        })
        .collect();
    assert!(
        !diags.has_errors(),
        "the sources do not parse:\n{}",
        diags.render(&map)
    );
    let refs: Vec<_> = files.iter().collect();
    let options = ElabOptions::new(Dialect::Verilog2005)
        .with_top("usb_mjpeg")
        .with_param("WIDTH", width.to_string())
        .with_param("HEIGHT", height.to_string())
        // Four clocks of power-on reset instead of sixteen. The board's
        // sixteen is 0.27 us of settling it does not need here.
        .with_param("POR", "4");
    let design = elaborate(&refs, &options, &mut diags).expect("usb_mjpeg elaborates");
    assert!(
        !diags.has_errors(),
        "usb_mjpeg does not elaborate:\n{}",
        diags.render(&map)
    );
    Some(Box::leak(Box::new(design)))
}

/// A simulation driven one clock edge at a time.
struct Run<'d> {
    sim: Simulator<'d>,
}

impl<'d> Run<'d> {
    fn new(design: &'d Design) -> Run<'d> {
        let options = SimOptions {
            top: Some("usb_mjpeg".to_owned()),
            ..SimOptions::default()
        };
        Run {
            sim: Simulator::new(design, options).expect("the design simulates"),
        }
    }

    fn net(&self, name: &str) -> NetHandle {
        self.sim
            .net(name)
            .unwrap_or_else(|| panic!("no net `{name}`"))
    }

    fn get(&self, handle: NetHandle) -> u64 {
        self.sim.get(handle).to_u64().unwrap_or_else(|| {
            panic!(
                "`{}` is not a number: {}",
                self.sim.net_name(handle),
                self.sim.get(handle)
            )
        })
    }

    fn set(&mut self, handle: NetHandle, value: u64, width: u32) {
        self.sim
            .set(handle, reticle::logic::Logic::from_u64(value, width));
    }
}

const HALF: u64 = 1;

/// Runs the design until `frames` files have been finished, or until
/// `limit` clocks have passed, and returns the bytes and where each file
/// ended.
///
/// **The USB side is forced out of the way.** `in_ready` is the bulk
/// endpoint's, and with no host to collect a packet the endpoint arms at 64
/// bytes and the encoder correctly stalls for ever. Forcing it high is the
/// same thing a host that always had room would do, and it is what makes
/// this a test of the picture path rather than of the endpoint —
/// `tests/ip_library.rs` proves the endpoint against a host model and
/// `tests/usb_loopback.rs` against a real one.
fn stream(design: &'static Design, frames: usize, limit: u64) -> (Vec<u8>, Vec<usize>) {
    let mut run = Run::new(design);

    let clk = run.net("usb_mjpeg.clk");
    let dir = run.net("usb_mjpeg.ulpi_dir");
    let nxt = run.net("usb_mjpeg.ulpi_nxt");
    let ready = run.net("usb_mjpeg.in_ready");
    let valid = run.net("usb_mjpeg.enc_valid");
    let byte = run.net("usb_mjpeg.enc_byte");
    let commit = run.net("usb_mjpeg.commit");

    run.set(dir, 0, 1);
    run.set(nxt, 0, 1);
    run.sim.force(ready, reticle::logic::Logic::from_u64(1, 1));

    let mut bytes: Vec<u8> = Vec::new();
    let mut ends: Vec<usize> = Vec::new();
    for _ in 0..limit {
        run.sim.run_for(HALF);
        // Read in the half cycle before the edge, which is where a
        // ready/valid handshake lives.
        if run.get(valid) == 1 {
            bytes.push(u8::try_from(run.get(byte)).expect("a byte"));
        }
        if run.get(commit) == 1 {
            ends.push(bytes.len());
            if ends.len() == frames {
                return (bytes, ends);
            }
        }
        run.set(clk, 1, 1);
        run.sim.run_for(HALF);
        run.set(clk, 0, 1);
    }
    panic!(
        "only {} of {frames} frame(s) in {limit} clocks ({} bytes)",
        ends.len(),
        bytes.len()
    );
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

/// The bytes of one `case` ROM in `usb_mjpeg.v`, read back out of the
/// Verilog.
///
/// The design is the caller `ip/video/mjpeg_enc` has none of — its README
/// is deliberate that "Nothing in this package contains Annex K" — so these
/// 206 bytes are written out in the design and nowhere else. Reading them
/// back and comparing against the corpus is what keeps that from being a
/// second source of truth that can drift.
fn rom(name: &str) -> Vec<u8> {
    let text = read("testdata/fpga/cynthion/usb_mjpeg.v");
    let mut found: Vec<(usize, u8)> = Vec::new();
    for line in text.lines() {
        for piece in line.split(';') {
            let Some((index, value)) = piece.split_once(&format!(": {name} = 8'h")) else {
                continue;
            };
            let Ok(index) = index.trim().parse::<usize>() else {
                continue;
            };
            let value = value.trim();
            assert_eq!(value.len(), 2, "`{name}` entry {index} is not two digits");
            found.push((
                index,
                u8::from_str_radix(value, 16).expect("two hex digits"),
            ));
        }
    }
    assert!(!found.is_empty(), "no `{name}` entries in the design");
    found.sort_unstable();
    for (i, (index, _)) in found.iter().enumerate() {
        assert_eq!(*index, i, "`{name}` skips or repeats an index");
    }
    found.into_iter().map(|(_, v)| v).collect()
}

/// `dcbits`/`dcvals`/`acbits`/`acvals` from the corpus, in the form a
/// `DHT` segment carries them: sixteen counts and then the symbols.
fn corpus_tables() -> (Vec<u8>, Vec<u8>) {
    let text = read("testdata/ip/mjpeg_enc_corpus.txt");
    let line = |tag: &str| -> Vec<u8> {
        let rest = text
            .lines()
            .find_map(|l| l.trim().strip_prefix(tag))
            .map(str::trim)
            .unwrap_or_else(|| panic!("the corpus has no `{tag}` line"));
        assert!(rest.len().is_multiple_of(2), "`{tag}` is not whole bytes");
        (0..rest.len() / 2)
            .map(|i| u8::from_str_radix(&rest[2 * i..2 * i + 2], 16).expect("two hex digits"))
            .collect()
    };
    let dc = [line("dcbits"), line("dcvals")].concat();
    let ac = [line("acbits"), line("acvals")].concat();
    (dc, ac)
}

/// The two Huffman tables in the design are the corpus's, byte for byte,
/// and each one's sixteen counts agree with how many symbols follow.
///
/// What it would not catch: a table that is self-consistent and wrong in
/// both places. The counts-against-symbols check below is what makes that
/// unlikely rather than impossible, and the header test is what ties the
/// whole lot to a file a decoder has already read.
#[test]
fn the_tables_in_the_rom_are_annex_k_as_the_corpus_records_them() {
    let (dc, ac) = corpus_tables();
    assert_eq!(dc.len(), 28, "T.81 Table K.3 is 16 counts and 12 symbols");
    assert_eq!(ac.len(), 178, "T.81 Table K.5 is 16 counts and 162 symbols");
    for (what, table) in [("DC", &dc), ("AC", &ac)] {
        let counted: usize = table[..16].iter().map(|b| usize::from(*b)).sum();
        assert_eq!(
            counted,
            table.len() - 16,
            "{what}: the sixteen counts and the symbols that follow"
        );
    }
    assert_eq!(rom("dc_rom"), dc, "the DC table in the design");
    assert_eq!(rom("ac_rom"), ac, "the AC table in the design");
}

// ---------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------

/// Where `testdata/nes/frame.jpg`'s scan starts, which is the length of
/// everything a decoder reads before the entropy-coded data: `SOI`, the
/// `DQT`, the `SOF0`, both `DHT`s and the `SOS`.
const HEADER: usize = 310;

/// The bytes of `testdata/nes/frame.jpg`, which `tests/nes.rs` checks
/// against ffmpeg's decoder and keeps as a golden.
fn golden() -> Vec<u8> {
    let path = root().join("testdata/nes/frame.jpg");
    fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every marker of one kind in a file, skipping the `FF 00` a stuffed
/// `FF` in the entropy-coded data is written as (T.81 F.1.2.3).
fn markers(file: &[u8], code: u8) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i + 1 < file.len() {
        if file[i] == 0xFF && file[i + 1] != 0x00 {
            if file[i + 1] == code {
                count += 1;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    count
}

/// The 310 bytes before the scan are the golden's, byte for byte.
///
/// This is the strongest single check in the file and it is nearly free:
/// `examples/nes`'s `nes_jpeg` writes the same 256x240 picture through the
/// same encoder with the same flat-16 quantisation table and the same Annex
/// K Huffman tables, so everything a decoder reads before the entropy-coded
/// data **has to be identical**, and `tests/nes.rs` has already put that
/// golden through ffmpeg's decoder. A table loaded in the wrong order, a
/// `DQT` that is not flat, a `SOF0` with the dimensions transposed and a
/// `DHT` whose counts and symbols disagree would every one of them change a
/// byte in here.
///
/// It stops as soon as the header is out — the encoder assembles it at
/// `start`, before it asks for a pixel — so this costs about a thousand
/// clocks where a whole 256x240 frame is six hundred thousand.
///
/// What it would not catch: anything in the scan. The two designs' pictures
/// are different and their entropy-coded data is nothing alike.
#[test]
fn the_header_is_the_one_the_nes_examples_golden_already_carries() {
    let Some(design) = design(256, 240) else {
        return;
    };
    let (bytes, _) = header_of(design, HEADER, 20_000);
    let golden = golden();
    assert_eq!(
        bytes,
        golden[..HEADER],
        "the header this design writes is not `testdata/nes/frame.jpg`'s"
    );
    // And spelt out, so a failure above says which part moved.
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "SOI");
    assert_eq!(&bytes[2..5], &[0xFF, 0xDB, 0x00], "DQT");
    assert!(
        bytes[5..71].iter().skip(2).all(|b| *b == 16),
        "the quantisation table is flat 16"
    );
    assert_eq!(&bytes[71..73], &[0xFF, 0xC0], "SOF0");
    assert_eq!(
        (usize::from(bytes[76]) << 8) | usize::from(bytes[77]),
        240,
        "the height"
    );
    assert_eq!(
        (usize::from(bytes[78]) << 8) | usize::from(bytes[79]),
        256,
        "the width"
    );
    assert_eq!(bytes[80], 1, "one component: this encoder is greyscale");
    assert_eq!(&bytes[84..86], &[0xFF, 0xC4], "the DC DHT");
    assert_eq!(&bytes[117..119], &[0xFF, 0xC4], "the AC DHT");
    assert_eq!(&bytes[300..302], &[0xFF, 0xDA], "SOS");
}

/// Runs until `want` bytes have come out.
fn header_of(design: &'static Design, want: usize, limit: u64) -> (Vec<u8>, u64) {
    let mut run = Run::new(design);
    let clk = run.net("usb_mjpeg.clk");
    let dir = run.net("usb_mjpeg.ulpi_dir");
    let nxt = run.net("usb_mjpeg.ulpi_nxt");
    let ready = run.net("usb_mjpeg.in_ready");
    let valid = run.net("usb_mjpeg.enc_valid");
    let byte = run.net("usb_mjpeg.enc_byte");

    run.set(dir, 0, 1);
    run.set(nxt, 0, 1);
    run.sim.force(ready, reticle::logic::Logic::from_u64(1, 1));

    let mut bytes: Vec<u8> = Vec::new();
    for clock in 0..limit {
        run.sim.run_for(HALF);
        if run.get(valid) == 1 {
            bytes.push(u8::try_from(run.get(byte)).expect("a byte"));
            if bytes.len() == want {
                return (bytes, clock);
            }
        }
        run.set(clk, 1, 1);
        run.sim.run_for(HALF);
        run.set(clk, 0, 1);
    }
    panic!("only {} of {want} byte(s) in {limit} clocks", bytes.len());
}

/// Three whole files come out, one after another, and they are not the
/// same file three times.
///
/// 64x64 rather than the board's 256x240: 64 blocks against 960, which is
/// what makes three frames affordable here. The size is the only thing that
/// changes — the tables, the header, the loader and the restart are the
/// design's own.
///
/// **The frames differ, and that is the point.** Pattern 6 is a bar one
/// pixel further right each frame, so a design that encoded one picture and
/// repeated it, or that never restarted the sweep's own frame counter,
/// would give three identical files. A still picture cannot pass this.
///
/// What it would not catch: a frame rate. Nothing here is timed, and
/// `the_board_streams_video_over_its_usb_port` is where a rate is measured
/// — printed, never asserted, because CI runs on slower machines.
#[test]
fn the_stream_is_one_whole_jpeg_file_after_another() {
    let Some(design) = design(64, 64) else {
        return;
    };
    let (bytes, ends) = stream(design, 3, 400_000);
    assert_eq!(ends.len(), 3, "three frames");

    let mut start = 0;
    let mut files: Vec<&[u8]> = Vec::new();
    for end in &ends {
        let file = &bytes[start..*end];
        assert_eq!(&file[..2], &[0xFF, 0xD8], "every file starts with SOI");
        assert_eq!(
            &file[file.len() - 2..],
            &[0xFF, 0xD9],
            "every file ends with EOI"
        );
        for (code, what) in [(0xDBu8, "DQT"), (0xC0, "SOF0"), (0xDA, "SOS")] {
            assert_eq!(markers(file, code), 1, "exactly one {what}");
        }
        assert_eq!(markers(file, 0xC4), 2, "a DC and an AC DHT");
        assert_eq!(
            (usize::from(file[76]) << 8) | usize::from(file[77]),
            64,
            "the height the parameter asked for"
        );
        assert_eq!(
            (usize::from(file[78]) << 8) | usize::from(file[79]),
            64,
            "the width"
        );
        files.push(file);
        start = *end;
    }
    assert_eq!(
        bytes.len(),
        *ends.last().expect("three frames"),
        "nothing came out after the last EOI"
    );
    assert_ne!(files[0], files[1], "the bar did not move between 1 and 2");
    assert_ne!(files[1], files[2], "the bar did not move between 2 and 3");
    // Same size, same tables: only the scan differs between frames.
    assert_eq!(&files[0][..HEADER], &files[1][..HEADER]);
    assert_eq!(&files[1][..HEADER], &files[2][..HEADER]);
}

// ---------------------------------------------------------------------------
// The part
// ---------------------------------------------------------------------------

/// The board this design is for.
const DEVICE: &str = "ecp5-12f-CABGA256";

/// What Lattice specifies for an `LFE5U-12`, from Table 1.1 of
/// FPGA-DS-02012-3.4: 12k LUTs and 32 sysMEM blocks.
/// `src/fpga/devices/ecp5.dev` has the reading in full, including why the
/// fabric database enumerates twice both figures — the 12F and the 25F are
/// the same die and prjtrellis-db has one directory for the pair.
const LUT4S: usize = 12_144;
const SYSMEM: u32 = 32;

/// It fits the board it is written for, by the memory and by the logic.
///
/// `examples/nes`'s `nes_jpeg` is the same encoder and does not: a 256x240
/// frame buffer of palette indices is 39 `DP16KD` on top of the encoder's
/// seven, and `tests/nes.rs` has the assertion that says so. The only
/// difference here is where the pixels come from, which is the whole point
/// of the design.
///
/// What it would not catch: whether it meets timing. The ECP5 delay model
/// in this backend is unit placeholders — `docs/fpga-trellis.md` says so —
/// so the depth printed below is a count of logic levels and not a time,
/// and the part is the only thing that can answer the question.
#[cfg(feature = "fpga")]
#[test]
fn the_design_maps_onto_the_cynthions_ecp5() {
    use reticle::diag::Severity;
    use reticle::fpga::{self, Constraints, FpgaOptions};

    let Some(design) = design(256, 240) else {
        return;
    };
    let mut design = design.clone();
    let top = design.top.expect("a top");
    let device = fpga::target(DEVICE).expect("the ECP5 12F is a built-in device");

    let mut diags = Diagnostics::new();
    let flow = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap_or_else(|e| panic!("the ECP5 flow failed: {e:?}"));
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    assert!(errors.is_empty(), "the ECP5 flow reported: {errors:?}");

    println!("usb_mjpeg on {DEVICE}:");
    for (cell, count) in &flow.netlist {
        println!("  {count:>5} x {cell}");
    }
    println!("  {} LUTs, depth {}", flow.luts, flow.lut_depth);

    assert!(
        flow.netlist.iter().all(|(cell, _)| !cell.starts_with('$')),
        "a generic cell survived: {:?}",
        flow.netlist
    );

    // The encoder's five, and nothing else. A frame buffer appearing here
    // would be the thing that stopped this fitting.
    let mut in_block: Vec<&str> = flow
        .primitives
        .block_rams
        .iter()
        .map(|b| b.memory.as_str())
        .collect();
    in_block.sort_unstable();
    assert_eq!(
        in_block,
        [
            "u_enc.cfg",
            "u_enc.u_ent.tbl",
            "u_enc.u_fdct.blk",
            "u_enc.u_quant.blk",
            "u_enc.u_quant.tbl",
        ],
        "a memory did not become block RAM, or a new one appeared"
    );

    for b in &flow.primitives.block_rams {
        println!(
            "  {} -> {} x DP16KD in {}x{} mode",
            b.memory,
            b.blocks(),
            b.mode.0,
            b.mode.1
        );
    }

    let blocks: u32 = flow.primitives.block_rams.iter().map(|b| b.blocks()).sum();
    println!("block RAM: {blocks} x DP16KD wanted, {SYSMEM} on an LFE5U-12F");
    assert!(
        blocks <= SYSMEM,
        "{blocks} blocks does not fit the 12F's {SYSMEM}"
    );
    assert!(
        flow.luts <= LUT4S,
        "{} lookup tables does not fit the 12F's {LUT4S}",
        flow.luts
    );
}

// ---------------------------------------------------------------------------
// The board
// ---------------------------------------------------------------------------

/// pid.codes' test pair, which is `ip/usb/usb_cdc_acm`'s default.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;

/// Every `/dev/ttyACM*` the kernel gave a device with these identifiers.
///
/// By sysfs rather than by guessing a number: a Cynthion's own Apollo
/// debugger is a CDC ACM device too and is usually `ttyACM0`, so this one
/// is normally `ttyACM1` — but normally is not always.
/// `tests/usb_cdc_acm.rs` has the same walk and says the same thing.
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

/// The whole files in a stream, cut at `SOI`.
///
/// The first and last are dropped: a reader that opens an endless stream
/// arrives in the middle of a file and leaves in the middle of another, and
/// neither of those is this design's fault. What is left is checked whole.
fn whole_files(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0;
    while i + 1 < stream.len() {
        if stream[i] == 0xFF && stream[i + 1] == 0xD8 {
            starts.push(i);
            i += 2;
        } else {
            i += 1;
        }
    }
    starts
        .windows(2)
        .map(|w| &stream[w[0]..w[1]])
        .collect::<Vec<_>>()
}

/// **On the part.** A Cynthion with this design loaded is read as a
/// terminal, the bytes are cut at `FF D8`, and every whole file between two
/// cuts is a baseline greyscale JPEG of the size the design was built for.
///
/// This is the half no simulation reaches. `the_stream_is_one_whole_jpeg_file_after_another`
/// proves the encoder and the restart with the endpoint forced out of the
/// way; what it cannot prove is that a **real host** collects the packets,
/// that the endpoint arms at 64 bytes on a real bus, and that the whole
/// pipeline — `pattern_gen`, the luminance, the FDCT, the quantiser, the
/// coder and the bulk endpoint — holds 60 MHz on silicon with a delay model
/// that is placeholders. `ip/usb/usb_cdc_acm/README.md` §6 has two faults
/// that were invisible in simulation and found exactly here.
///
/// The rate is **printed and never asserted**: CI runs on slower and
/// Windows machines, and nothing in this repository compares a throughput
/// against a clock.
#[test]
#[ignore = "needs a Cynthion with testdata/fpga/cynthion/usb_mjpeg.v loaded"]
fn the_board_streams_video_over_its_usb_port() {
    use std::process::Command;
    use std::time::Instant;

    let ports = tty_ports(VID, PID);
    if ports.is_empty() {
        println!(
            "no {VID:04x}:{PID:04x} is attached, skipping; load \
             testdata/fpga/cynthion/usb_mjpeg.v"
        );
        return;
    }
    assert_eq!(
        ports.len(),
        1,
        "two devices claim to be {VID:04x}:{PID:04x}: {ports:?}"
    );
    let port = &ports[0];
    println!("reading {}", port.display());

    // Raw, so nothing translates a `0D` in the entropy-coded data into
    // `0D 0A` on the way up. `tests/usb_cdc_acm.rs` says why this is `stty`
    // and not `tcsetattr`: that would mean `libc` and `unsafe`, and this
    // crate has neither.
    let stty = Command::new("stty")
        .arg("-F")
        .arg(port)
        .args(["raw", "-echo", "-echoe", "-echok", "-crtscts", "clocal"])
        .status();
    match stty {
        Ok(status) if status.success() => {}
        Ok(status) => {
            println!("stty failed ({status}), skipping");
            return;
        }
        Err(err) => {
            println!("cannot run stty ({err}), skipping");
            return;
        }
    }

    const SECONDS: u32 = 3;
    let began = Instant::now();
    let read = Command::new("timeout")
        .arg(SECONDS.to_string())
        .arg("cat")
        .arg(port)
        .output();
    let Ok(read) = read else {
        println!("cannot run `timeout cat`, skipping");
        return;
    };
    let elapsed = began.elapsed().as_secs_f64();
    let stream = read.stdout;
    assert!(
        !stream.is_empty(),
        "nothing came out of {} in {SECONDS} s. LED 0 is the tables, LED 1 is \
         the host's configuration and LED 2 is a frame being fed; \
         testdata/fpga/cynthion/usb_mjpeg.v says how to read all six.",
        port.display()
    );

    let files = whole_files(&stream);
    assert!(
        files.len() >= 2,
        "{} bytes in {SECONDS} s held {} whole file(s), which is not a stream",
        stream.len(),
        files.len()
    );

    for (n, file) in files.iter().enumerate() {
        assert_eq!(&file[..2], &[0xFF, 0xD8], "file {n} starts with SOI");
        assert_eq!(
            &file[file.len() - 2..],
            &[0xFF, 0xD9],
            "file {n} ends with EOI"
        );
        assert_eq!(markers(file, 0xDA), 1, "file {n} has one SOS");
        assert_eq!(markers(file, 0xC4), 2, "file {n} has a DC and an AC DHT");
        assert_eq!(
            (usize::from(file[76]) << 8) | usize::from(file[77]),
            240,
            "file {n}: the height"
        );
        assert_eq!(
            (usize::from(file[78]) << 8) | usize::from(file[79]),
            256,
            "file {n}: the width"
        );
    }

    // The header is this design's and the scans are not each other's: the
    // bar moved. Two consecutive files being identical would mean the
    // picture was frozen even though the files kept coming.
    assert_eq!(
        &files[0][..HEADER],
        &golden()[..HEADER],
        "the header off the wire is not the one simulation and \
         `testdata/nes/frame.jpg` agree on"
    );
    assert!(
        files.windows(2).any(|w| w[0] != w[1]),
        "every file in the stream is the same bytes: the picture is frozen"
    );

    let coded: usize = files.iter().map(|f| f.len()).sum();
    println!(
        "{} whole frames in {elapsed:.2} s: {:.1} frames/s, {} bytes/frame, {:.0} bytes/s",
        files.len(),
        files.len() as f64 / elapsed,
        coded / files.len(),
        stream.len() as f64 / elapsed,
    );

    // And let a decoder that is not ours say the same thing.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("usb_mjpeg");
    fs::create_dir_all(&dir).expect("a directory for the capture");
    let path = dir.join("stream.mjpg");
    let body: Vec<u8> = files.concat();
    fs::write(&path, &body).expect("the capture is written");
    println!("the capture is at {}", path.display());

    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-show_entries"])
        .arg("stream=codec_name,pix_fmt,width,height,nb_read_frames")
        .args(["-of", "default=nw=1"])
        .arg(&path)
        .output();
    match probe {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            println!("ffprobe says:\n{text}");
            for want in [
                "codec_name=mjpeg",
                "pix_fmt=gray",
                "width=256",
                "height=240",
                &format!("nb_read_frames={}", files.len()),
            ] {
                assert!(text.lines().any(|l| l.trim() == want), "ffprobe: {want}");
            }
        }
        Ok(out) => panic!(
            "ffprobe refused the capture, which is this design's failure and \
             not the test's:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(err) => println!("no ffprobe ({err}); the structure checks above stand"),
    }
}
