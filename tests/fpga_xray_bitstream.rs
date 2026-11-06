//! The container around the frames, against what Vivado 2026.1's
//! `write_bitstream` wrote.
//!
//! The answers are under `testdata/fpga/vivado/bitstream/`: the frames Vivado
//! wrote for one small design, a sample of its frames from two larger ones, and
//! for each value of each `BITSTREAM.*` property the length and hash of its
//! whole stream. `tools/vivado-parity/` has what made them. None of it needs
//! Vivado to run, and only the two tests that say so need the chip database.
//!
//! **What these would catch:** any word of a stream that differs from Vivado's,
//! since the hash covers every word after the sync word, CRCs and frames with
//! their ECC included; and the number of lead-in words. **What they would not:**
//! a stream both tools get wrong. Vivado's stream is the reference, and none of
//! this has been loaded into a part by this test: a stream that matches is one
//! Vivado would write, not one a board has been seen to take.

#![cfg(feature = "fpga")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use reticle::fpga::xc7::options::{BitstreamOptions, ConfigTiles, OptionError, stream_words};
use reticle::fpga::xc7::{
    self, BitHeader, ConfigRow, ECC_MASK, ECC_WORD, FrameAddress, FrameData, FrameLayout, Part,
    WORDS_PER_FRAME,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/bitstream")
}

fn read(name: &str) -> String {
    let path = fixtures().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The hash `tools/vivado-parity/bit_packets.py` writes.
fn fnv(words: &[u32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for word in words {
        for byte in word.to_be_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// One part's answers.
struct Fixture {
    part: Part,
    tiles: ConfigTiles,
    /// The frames Vivado wrote for the base design, ECC and all.
    frames: FrameData,
    base: Stream,
    runs: Vec<Run>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stream {
    words: usize,
    lead: usize,
    fnv: u64,
}

struct Run {
    name: String,
    sets: Vec<(String, String)>,
    stream: Option<Stream>,
    refused: Option<String>,
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(key).and_then(|f| f.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no {key} in {line}"))
}

fn stream_of(line: &str) -> Stream {
    Stream {
        words: field(line, "words").parse().unwrap(),
        lead: field(line, "lead").parse().unwrap(),
        fnv: u64::from_str_radix(field(line, "fnv"), 16).unwrap(),
    }
}

fn fixture(tag: &str, idcode: u32) -> Fixture {
    let mut rows = Vec::new();
    let mut tiles = ConfigTiles::default();
    for line in read(&format!("{tag}.layout.txt")).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["tiles", center, monitor] => {
                tiles = ConfigTiles {
                    center: u32::from_str_radix(center.trim_start_matches("0x"), 16).unwrap(),
                    monitor: u32::from_str_radix(monitor.trim_start_matches("0x"), 16).unwrap(),
                };
            }
            ["row", block, half, row, columns] => rows.push(ConfigRow {
                block: block.parse().unwrap(),
                half: half.parse().unwrap(),
                row: row.parse().unwrap(),
                columns: columns.split(',').map(|c| c.parse().unwrap()).collect(),
            }),
            _ => {}
        }
    }
    let layout = FrameLayout::new(rows);
    let mut words = vec![0u32; layout.words()];
    for line in read(&format!("{tag}.frames.txt")).lines() {
        if line.starts_with('#') {
            continue;
        }
        let (index, value) = line.split_once(' ').unwrap();
        words[index.parse::<usize>().unwrap()] = u32::from_str_radix(value, 16).unwrap();
    }
    let frames = FrameData::from_stream(layout.clone(), words).unwrap();
    let part = Part {
        name: tag.to_owned(),
        idcode,
        layout,
    };
    let mut base = None;
    let mut runs: Vec<Run> = Vec::new();
    for line in read(&format!("{tag}.options.txt")).lines() {
        if let Some(rest) = line.strip_prefix("base ") {
            base = Some(stream_of(rest));
        } else if let Some(name) = line.strip_prefix("run ") {
            runs.push(Run {
                name: name.to_owned(),
                sets: Vec::new(),
                stream: None,
                refused: None,
            });
        } else if let Some(kv) = line.strip_prefix("set ") {
            let (name, value) = kv.split_once('=').unwrap();
            let run = runs.last_mut().unwrap();
            run.sets.push((name.to_owned(), value.to_owned()));
        } else if let Some(rest) = line.strip_prefix("stream ") {
            runs.last_mut().unwrap().stream = Some(stream_of(rest));
        } else if let Some(rest) = line.strip_prefix("refused ") {
            runs.last_mut().unwrap().refused = Some(rest.to_owned());
        }
    }
    Fixture {
        part,
        tiles,
        frames,
        base: base.unwrap(),
        runs,
    }
}

fn a100t() -> Fixture {
    fixture("xc7a100t", 0x0363_1093)
}

fn a35t() -> Fixture {
    fixture("xc7a35t", 0x0362_D093)
}

/// Runs whose stream needs a pad configured, so the chip database's features:
/// without it they are not checked by `every_option_writes_the_stream_vivado_wrote`.
fn needs_the_database(run: &Run) -> bool {
    run.sets.iter().any(|(name, value)| {
        name.ends_with("UNUSEDPIN") && value != "PULLDOWN"
            || name.ends_with("EXTMASTERCCLK_EN") && value != "DISABLE"
    })
}

/// What `BitstreamOptions` makes of a run, or why it refused it.
fn options_of(run: &Run) -> Result<BitstreamOptions, OptionError> {
    let mut options = BitstreamOptions::default();
    for (name, value) in &run.sets {
        options.set(name, value)?;
    }
    options.check()?;
    Ok(options)
}

/// The base frames with what these options add, as `finish_frames` adds them
/// where no pad is concerned.
fn frames_with(fixture: &Fixture, options: &BitstreamOptions) -> FrameData {
    let mut frames = fixture.frames.clone();
    for (address, word, bit) in options.config_tile_bits(fixture.tiles) {
        frames.set(address, word, bit).unwrap();
    }
    frames
}

fn check_all_options(fixture: &Fixture) -> (usize, Vec<String>) {
    let mut matched = 0;
    let mut not_done = Vec::new();
    let mut wrong = Vec::new();
    for run in &fixture.runs {
        if needs_the_database(run) {
            continue;
        }
        let options = match options_of(run) {
            Ok(options) => options,
            Err(OptionError::NotImplemented { .. }) => {
                not_done.push(run.name.clone());
                continue;
            }
            Err(error) => {
                // Refused here, as it was by Vivado.
                assert!(
                    run.refused.is_some(),
                    "{}: Vivado wrote this and Reticle refused it: {error}",
                    run.name
                );
                continue;
            }
        };
        let Some(wanted) = &run.stream else {
            wrong.push(format!(
                "{}: Vivado refused it: {:?}",
                run.name, run.refused
            ));
            continue;
        };
        let words = stream_words(&fixture.part, &frames_with(fixture, &options), &options).unwrap();
        let ours = Stream {
            words: words.len(),
            lead: options.leading_words() + 4,
            fnv: fnv(&words),
        };
        if ours == *wanted {
            matched += 1;
        } else {
            wrong.push(format!("{}: {ours:?}, Vivado {wanted:?}", run.name));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    (matched, not_done)
}

/// The stream with no options, word for word: the sequence, every register and
/// both CRCs, from Vivado's frames.
#[test]
fn the_default_stream_is_vivados_word_for_word() {
    for fixture in [a100t(), a35t()] {
        let options = BitstreamOptions::default();
        let words = stream_words(&fixture.part, &fixture.frames, &options).unwrap();
        assert_eq!(words.len(), fixture.base.words, "{}", fixture.part.name);
        assert_eq!(fnv(&words), fixture.base.fnv, "{}", fixture.part.name);
        assert_eq!(options.leading_words() + 4, fixture.base.lead);
    }
}

/// Every run of the fixture but those a pad is in, which the test after this
/// one has. A run Vivado refused has to be refused here, and one Reticle does
/// not do has to be in the list here, so one cannot quietly join them.
/// `ICAP_SELECT=TOP`, `BOTTOM` and `STARTUPCLK=USERCLK` left the list when
/// Reticle placed an `ICAPE2` and a `STARTUPE2`: on this design, which has
/// neither, they are refused as Vivado refused them (`Designutils 12-1536`,
/// `12-1517`), and `tests/fpga_xray_config.rs` has them where they build.
#[test]
fn every_option_writes_the_stream_vivado_wrote() {
    let (matched, not_done) = check_all_options(&a100t());
    eprintln!("XC7A100T: {matched} option run(s) identical to Vivado's, word for word");
    assert!(matched > 200, "{matched}");
    assert_eq!(
        not_done,
        [
            "CONFIG.PERSIST=YES",
            "ENCRYPT=YES",
            "ENCRYPTION.ENCRYPT=YES",
            "PERSIST+BPI16",
            "PERSIST+SMAP32",
            "PERSIST+SMAP8",
            "PERSIST+SPI4",
        ]
    );
    let (matched, _) = check_all_options(&a35t());
    eprintln!("XC7A35T: {matched} option run(s) identical to Vivado's");
    assert!(matched > 30, "{matched}");
}

/// What a replay of each kind of stream leaves in memory is the image it was
/// given: the compressed stream's frames are found nowhere but there.
#[test]
fn a_stream_of_any_form_replays_to_the_image_it_was_given() {
    let fixture = a100t();
    for sets in [
        &[("BITSTREAM.GENERAL.COMPRESS", "TRUE")][..],
        &[("BITSTREAM.GENERAL.PERFRAMECRC", "YES")][..],
        &[("BITSTREAM.GENERAL.DEBUGBITSTREAM", "YES")][..],
        &[][..],
    ] {
        let mut options = BitstreamOptions::default();
        for (name, value) in sets {
            options.set(name, value).unwrap();
        }
        let bytes = xc7::write_bit_with(
            &BitHeader::new("t", "7a100tftg256"),
            &fixture.part,
            &fixture.frames,
            &options,
        )
        .unwrap();
        let bit = xc7::read_bit(&bytes).unwrap_or_else(|e| panic!("{sets:?}: {e}"));
        let image = bit.image(fixture.part.layout.clone()).unwrap();
        assert_eq!(image.words(), fixture.frames.words(), "{sets:?}");
    }
}

/// What the compression is for, on a design with little in it.
#[test]
fn a_compressed_stream_is_much_shorter_and_says_so() {
    let fixture = a100t();
    let run = fixture
        .runs
        .iter()
        .find(|r| r.name == "GENERAL.COMPRESS=TRUE")
        .unwrap();
    let wanted = run.stream.as_ref().unwrap();
    assert!(wanted.words * 8 < fixture.base.words);
}

/// Frames of two designs built to set as many bits as they can, on two parts,
/// and held to the ECC exactly. A frame's ECC is one 13-bit answer, so a wrong
/// code would have to agree by chance on every one of these.
#[test]
fn the_frame_ecc_is_exact_on_vivados_frames() {
    let text = read("ecc_frames.txt");
    let mut frames = 0usize;
    let mut bits = 0u64;
    let mut seen_word = [false; WORDS_PER_FRAME];
    let mut by_file: BTreeMap<&str, usize> = BTreeMap::new();
    let mut block_ram = 0;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let mut fields = line.split(' ');
        let file = fields.next().unwrap();
        let address =
            FrameAddress::from_u32(u32::from_str_radix(fields.next().unwrap(), 16).unwrap());
        let frame: Vec<u32> = fields
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect();
        assert_eq!(frame.len(), WORDS_PER_FRAME);
        let found = frame[ECC_WORD] & ECC_MASK;
        assert_eq!(
            xc7::wanted_ecc(Some(address), &frame),
            found,
            "{file} {address}"
        );
        if address.block == 0 {
            assert_eq!(xc7::frame_ecc(&frame), found, "{file} {address}");
        } else {
            block_ram += 1;
        }
        for (word, value) in frame.iter().enumerate() {
            bits += u64::from(value.count_ones());
            seen_word[word] |= *value != 0;
        }
        frames += 1;
        *by_file.entry(file).or_default() += 1;
    }
    eprintln!(
        "the frame ECC is exact on {frames} real frames, {bits} set bits, from {by_file:?}, \
         {block_ram} of them block RAM"
    );
    assert!(frames >= 75, "{frames}");
    assert!(by_file.len() >= 2, "{by_file:?}");
    assert!(block_ram > 0);
    // Would catch a sample that says nothing about part of the frame.
    assert!(seen_word.iter().all(|s| *s), "{seen_word:?}");
}

/// **What a clock manager's and an XADC's frames' ECC leaves out**, on
/// Vivado's frames: `drp_ecc.txt` has frames 28 and 29 of the block's tile in
/// Vivado's bitstreams of designs that wire its DRP port each way
/// (`tools/vivado-parity/drp_ecc.py`), with the positions of the block's own
/// bits. A clock manager's frame has the code of the frame without them when
/// its `DWE` is a signal, whatever its `DEN` and `DCLK` are, and of the whole
/// frame when `DWE` is tied low or the cell is a `_BASE`, which has none; an
/// XADC's has the code without its registers always. That is
/// `xc7::ecc_leaves_out_tile`'s rule, which `--decode` reads by.
///
/// It would catch the rule read off the wrong pin (`DEN` or `DCLK` rather
/// than `DWE`), the XADC taken for a clock manager or the other way round, or
/// a block's bit left in the code or another's taken out of it; every frame
/// here tells the two codes apart. It says nothing of what the part does with
/// the code: **nothing here has been on a board**, and this flow still writes
/// the whole frame's code in both (`ecc_leaves_out_tile`).
#[test]
fn a_drp_blocks_frames_ecc_is_vivados() {
    let text = read("drp_ecc.txt");
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let (frame, positions) = line.split_once(" | ").unwrap();
        let mut fields = frame.split(' ');
        let design = fields.next().unwrap();
        let address = fields.next().unwrap();
        let stored = u32::from_str_radix(fields.next().unwrap(), 16).unwrap();
        let words: Vec<u32> = fields
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect();
        assert_eq!(words.len(), WORDS_PER_FRAME, "{design} {address}");
        assert_eq!(words[ECC_WORD] & ECC_MASK, stored, "{design} {address}");
        let mut without = words.clone();
        for position in positions.split(' ') {
            let (word, bit) = position.split_once('.').unwrap();
            let (word, bit): (usize, u32) = (word.parse().unwrap(), bit.parse().unwrap());
            without[word] &= !(1 << bit);
        }
        let whole = xc7::frame_ecc(&words);
        let left_out = xc7::frame_ecc(&without);
        assert_ne!(
            whole, left_out,
            "{design} {address}: the frame tells nothing apart"
        );
        // The design's DRP, from its name: `gen_primitives.py`'s probes wire
        // every pin, and the variants tie the pins they name.
        let kind = if design.starts_with("xadc") {
            "XADC"
        } else if design.contains("_base_") {
            "no DWE"
        } else if design.contains("drp_off") || design.contains("dwe_off") {
            "DWE tied low"
        } else {
            "DWE a signal"
        };
        let wanted = if matches!(kind, "XADC" | "DWE a signal") {
            left_out
        } else {
            whole
        };
        assert_eq!(stored, wanted, "{design} {address} ({kind})");
        *kinds.entry(kind).or_default() += 1;
    }
    eprintln!("frames of each kind: {kinds:?}");
    assert_eq!(kinds.len(), 4, "{kinds:?}");
    // And the classification the decode uses.
    assert_eq!(xc7::ecc_leaves_out_tile("MONITOR_BOT"), Some(true));
    assert_eq!(xc7::ecc_leaves_out_tile("CMT_TOP_L_LOWER_B"), Some(false));
    assert_eq!(xc7::ecc_leaves_out_tile("CMT_TOP_R_UPPER_T"), Some(false));
    assert_eq!(xc7::ecc_leaves_out_tile("HCLK_CMT_L"), None);
}

/// Every frame of the base design, with the ECC taken out and put back: exact.
#[test]
fn taking_out_a_frames_ecc_and_filling_it_in_gives_vivados_frames() {
    for fixture in [a100t(), a35t()] {
        assert_eq!(fixture.frames.ecc_faults(), vec![], "{}", fixture.part.name);
        let mut words = fixture.frames.words().to_vec();
        for frame in words.chunks_mut(WORDS_PER_FRAME) {
            frame[ECC_WORD] &= !ECC_MASK;
        }
        let bare = FrameData::from_stream(fixture.part.layout.clone(), words).unwrap();
        assert_ne!(bare.words(), fixture.frames.words());
        assert_eq!(bare.with_ecc().words(), fixture.frames.words());
        assert!(!bare.ecc_faults().is_empty());
    }
}

/// `.bin` is the stream as it stands and `.rbt` is that in ones and zeros.
#[test]
fn the_side_files_hold_the_same_stream() {
    let fixture = a100t();
    let header = BitHeader::new("options_probe", "7a100tftg256");
    let bytes = xc7::write_bit(&header, &fixture.part, &fixture.frames).unwrap();
    let payload = xc7::bit_payload(&bytes).unwrap();
    let wanted: Vec<u32> = payload
        .chunks(4)
        .skip(fixture.base.lead + 1)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    assert_eq!(wanted.len(), fixture.base.words);
    assert_eq!(fnv(&wanted), fixture.base.fnv);

    let rbt = xc7::options::raw_bitfile(&header, "artix7", payload);
    let lines: Vec<&str> = rbt.lines().collect();
    assert_eq!(lines[0], "Xilinx ASCII Bitstream");
    assert_eq!(lines[1], "Created by Reticle");
    assert_eq!(lines[6], format!("Bits:        \t{}", payload.len() * 8));
    let back: Vec<u32> = lines[7..]
        .iter()
        .skip(fixture.base.lead + 1)
        .map(|l| u32::from_str_radix(l, 2).unwrap())
        .collect();
    assert_eq!(fnv(&back), fixture.base.fnv);
}

/// The database, for the two tests that ask it which bits a pad's features are.
fn chipdb() -> Option<String> {
    use std::env::var;
    if let Ok(root) = var("RETICLE_CHIPDB") {
        return Some(root);
    }
    let var = |v| var(v).ok().filter(|s: &String| !s.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")));
    let dir = cache.map(|c| format!("{c}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3"));
    match dir {
        Some(dir)
            if Path::new(&format!("{dir}/artix7/xc7a100tftg256-1/package_pins.csv")).is_file() =>
        {
            Some(dir)
        }
        _ => {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` or set \
                 RETICLE_CHIPDB"
            );
            None
        }
    }
}

struct DiskFiles;

impl reticle::ir::memfile::FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The pads `write_bitstream` configures with no design asking, on both parts:
/// `PUDC_B` always, `EMCCLK` for its option, and every unused pad of the die
/// for `UNUSEDPIN`. These are held to Vivado's whole stream too.
#[test]
fn the_pads_no_design_asked_for_are_vivados() {
    use reticle::fpga::xray::{XrayDatabase, XrayOptions};
    let Some(root) = chipdb() else { return };
    for (fixture, device) in [(a100t(), "xc7a100t-ftg256"), (a35t(), "xc7a35t-cpg236")] {
        let db = XrayDatabase::open(&DiskFiles, &root, device, &XrayOptions::new()).unwrap();
        assert_eq!(db.config_tiles(&DiskFiles).unwrap(), fixture.tiles);

        // `PUDC_B` is not on nothing at all: this asserted that it was, and
        // every design measured then had a pin in its bank. See `xray::finish`.
        let mut empty = FrameData::empty(fixture.part.layout.clone());
        let done = db
            .finish_frames(&DiskFiles, &mut empty, &BitstreamOptions::default())
            .unwrap();
        assert_eq!(done.config_pads, 0, "{device}");
        assert_eq!(empty.ones(), 0, "{device}");

        // Beside another pad of its bank, from frames with it taken out.
        let pin = db.pudc_pad_bits(&DiskFiles).unwrap();
        let mut words = fixture.frames.words().to_vec();
        for (address, word, bit) in &pin {
            let index = fixture
                .part
                .layout
                .position(*address)
                .expect("a frame of the part");
            let at = index * reticle::fpga::xc7::WORDS_PER_FRAME + *word;
            assert_ne!(
                words[at] & (1 << bit),
                0,
                "{device}: Vivado's frames hold it"
            );
            words[at] &= !(1 << bit);
        }
        assert_eq!(pin.len(), 9, "{device}: the input and its pull-up");
        let without = FrameData::from_stream(fixture.part.layout.clone(), words).unwrap();
        let mut back = without.clone();
        let done = db
            .finish_frames(&DiskFiles, &mut back, &BitstreamOptions::default())
            .unwrap();
        assert_eq!(done.config_pads, 1, "{device}");
        assert_eq!(back.with_ecc().words(), fixture.frames.words(), "{device}");

        let mut checked = 0;
        for run in fixture.runs.iter().filter(|r| needs_the_database(r)) {
            let Ok(options) = options_of(run) else {
                continue;
            };
            let Some(wanted) = &run.stream else { continue };
            let mut frames = fixture.frames.clone();
            db.finish_frames(&DiskFiles, &mut frames, &options).unwrap();
            let words = stream_words(&fixture.part, &frames, &options).unwrap();
            assert_eq!(fnv(&words), wanted.fnv, "{device} {}", run.name);
            assert_eq!(words.len(), wanted.words, "{device} {}", run.name);
            checked += 1;
        }
        eprintln!("{device}: {checked} run(s) with a pad in them identical to Vivado's");
        assert!(checked >= 2, "{device}: {checked}");
    }
}

/// `--decode`'s account of Vivado's frames with the option bits in them: every
/// set bit named or ECC, none unexplained in the base design's, and a wrong ECC
/// counted as what it is.
#[test]
fn the_frames_decode_with_nothing_unexplained_and_a_wrong_ecc_is_a_finding() {
    use reticle::fpga::xray::{XrayDatabase, XrayOptions};
    let Some(root) = chipdb() else { return };
    for (fixture, device) in [(a100t(), "xc7a100t-ftg256"), (a35t(), "xc7a35t-cpg236")] {
        let db = XrayDatabase::open(&DiskFiles, &root, device, &XrayOptions::new()).unwrap();
        let decoded = db.decode(&DiskFiles, &fixture.frames).unwrap();
        assert_eq!(decoded.unexplained, 0, "{device}: {}", decoded.to_text());
        assert_eq!(decoded.ecc_faults, vec![], "{device}");
        assert!(decoded.ecc_bits > 100, "{}", decoded.ecc_bits);
        eprintln!("{device}: {}", decoded.to_text().trim_end());

        // Every option bit named.
        let mut options = BitstreamOptions::default();
        for (name, value) in [
            ("BITSTREAM.CONFIG.USERID", "0X00000000"),
            ("BITSTREAM.CONFIG.M0PIN", "PULLDOWN"),
            ("BITSTREAM.CONFIG.PROGPIN", "PULLNONE"),
            ("BITSTREAM.GENERAL.DISABLE_JTAG", "YES"),
            ("BITSTREAM.GENERAL.XADCPOWERDOWN", "ENABLE"),
        ] {
            options.set(name, value).unwrap();
        }
        let frames = frames_with(&fixture, &options).with_ecc();
        let decoded = db.decode(&DiskFiles, &frames).unwrap();
        assert_eq!(decoded.unexplained, 0, "{device}: {}", decoded.to_text());
        let named: Vec<&str> = decoded.features.iter().map(|(_, f)| f.as_str()).collect();
        for name in [
            "USERID[31]=0",
            "M0PIN.PULLNONE",
            "M0PIN.PULLDOWN",
            "PROGPIN.PULLNONE",
            "DISABLE_JTAG",
        ] {
            assert!(named.contains(&name), "{device}: no {name}");
        }

        // One flipped ECC bit is one finding, and no unexplained bit.
        let mut words = frames.words().to_vec();
        let mut flipped = FrameAddress::default();
        for (position, address) in fixture.part.layout.order().iter().enumerate() {
            let Some(address) = address else { continue };
            if address.block == 0 && words[position * WORDS_PER_FRAME + ECC_WORD] & ECC_MASK != 0 {
                words[position * WORDS_PER_FRAME + ECC_WORD] ^= 1 << 12;
                flipped = *address;
                break;
            }
        }
        let wrong = FrameData::from_stream(fixture.part.layout.clone(), words).unwrap();
        let decoded = db.decode(&DiskFiles, &wrong).unwrap();
        assert_eq!(decoded.unexplained, 0);
        assert_eq!(decoded.ecc_faults.len(), 1);
        assert_eq!(decoded.ecc_faults[0].address, flipped);
        assert!(decoded.to_text().contains("1 frame(s) with a wrong ECC"));
    }
}

/// The flip-flops of two slices of each of two tile kinds, every bel, and a
/// sample of block RAM bits: Vivado's `.ll`, quoted, and the same places here.
#[test]
fn the_logic_location_is_vivados() {
    use reticle::fpga::xray::{LogicLocation, XrayDatabase, XrayOptions};
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, "xc7a100t-ftg256", &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let tiles = db.tiles(&DiskFiles).unwrap();
    let mut of_site: BTreeMap<String, std::collections::HashMap<(bool, u32), LogicLocation>> =
        BTreeMap::new();
    let (mut latches, mut rams) = (0, 0);
    for line in read("ll_probe.ll.txt").lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.first() != Some(&"Bit") {
            continue;
        }
        let wanted = LogicLocation {
            bit: fields[1].parse().unwrap(),
            frame: u32::from_str_radix(fields[2].trim_start_matches("0x"), 16).unwrap(),
            offset: fields[3].parse().unwrap(),
        };
        let site = fields[4].strip_prefix("Block=").unwrap();
        if let Some(latch) = fields[5].strip_prefix("Latch=") {
            let bel = match latch {
                "AQ" => "AFF",
                "BQ" => "BFF",
                "CQ" => "CFF",
                "DQ" => "DFF",
                other => other.strip_suffix(".Q").unwrap(),
            };
            let (at, name) = XrayDatabase::flip_flop_location(&part, &tiles, site, bel)
                .unwrap_or_else(|| panic!("{line}"));
            assert_eq!(at, wanted, "{line}");
            assert_eq!(name, latch, "{line}");
            latches += 1;
        } else {
            let ram = fields[5].strip_prefix("Ram=B:").unwrap();
            let (parity, number) = match ram.strip_prefix("PARBIT") {
                Some(number) => (true, number),
                None => (false, ram.strip_prefix("BIT").unwrap()),
            };
            // A RAMB36 is the two RAMB18s of its tile, by `BIT` parity: the
            // even bits are the even site's and the odd bits the other's.
            let number: u32 = number.parse().unwrap();
            let site = if let Some(name) = site.strip_prefix("RAMB36_X") {
                let (x, y) = name.split_once('Y').unwrap();
                let y: u32 = y.parse().unwrap();
                format!("RAMB18_X{x}Y{}", y * 2 + number % 2)
            } else {
                site.to_owned()
            };
            if !of_site.contains_key(&site) {
                let bits = db
                    .block_ram_locations(&DiskFiles, &part, &tiles, &site)
                    .unwrap();
                of_site.insert(site.clone(), bits);
            }
            let at = of_site[&site]
                .get(&(parity, number))
                .unwrap_or_else(|| panic!("{line}"));
            assert_eq!(*at, wanted, "{line}");
            rams += 1;
        }
    }
    eprintln!("{latches} flip-flop lines and {rams} block RAM lines at Vivado's positions");
    assert_eq!(latches, 32);
    assert!(rams > 100, "{rams}");
}

/// A design of Reticle's own, through the command line, with options set: it
/// reads back, every set bit decodes to a name, every frame's ECC is right, and
/// the side files hold the same stream.
#[cfg(feature = "cli")]
#[test]
fn a_design_built_here_carries_the_options_and_nothing_unexplained() {
    use reticle::fpga::xray::{XrayDatabase, XrayOptions};
    use std::process::Command;
    let Some(root) = chipdb() else { return };
    let dir = std::env::temp_dir().join(format!("reticle-bitstream-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bit = dir.join("options_probe.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--chipdb",
            &root,
            "--constraints",
        ])
        .arg(fixtures().join("options_probe.rcf"))
        .arg("--output-dir")
        .arg(&dir)
        .arg("--bitstream")
        .arg(&bit)
        .args([
            "--bin-file",
            "--raw-bitfile",
            "--logic-location-file",
            "--bitstream-option",
            "BITSTREAM.GENERAL.COMPRESS=TRUE",
            "--bitstream-option",
            "BITSTREAM.CONFIG.CONFIGRATE=33",
            "--bitstream-option",
            "BITSTREAM.CONFIG.USERID=0X12345678",
            "--bitstream-option",
            "BITSTREAM.CONFIG.UNUSEDPIN=PULLUP",
            "--bitstream-option",
            "BITSTREAM.CONFIG.M0PIN=PULLDOWN",
            "--bitstream-option",
            "BITSTREAM.GENERAL.XADCPOWERDOWN=ENABLE",
        ])
        .arg(fixtures().join("options_probe.v"))
        .output()
        .expect("the reticle binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");

    let db = XrayDatabase::open(&DiskFiles, &root, "xc7a100t-ftg256", &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let bytes = std::fs::read(&bit).unwrap();
    let read = xc7::read_bit(&bytes).unwrap();
    assert!(read.header.design.contains("UserID=0X12345678"));
    let frames = read.image(part.layout.clone()).unwrap();
    let decoded = db.decode(&DiskFiles, &frames).unwrap();
    assert_eq!(decoded.unexplained, 0, "{}", decoded.to_text());
    assert_eq!(decoded.ecc_faults, vec![]);
    assert!(decoded.features.iter().any(|(_, f)| f == "USERID[0]=0"));
    assert!(
        decoded
            .features
            .iter()
            .any(|(_, f)| f == "IOB_Y0.PULLTYPE.PULLUP")
    );
    eprintln!("{}", decoded.to_text().trim_end());

    let payload = xc7::bit_payload(&bytes).unwrap();
    assert_eq!(
        std::fs::read(dir.join("options_probe.bin")).unwrap(),
        payload
    );
    let rbt = std::fs::read_to_string(dir.join("options_probe.rbt")).unwrap();
    assert_eq!(rbt.lines().count(), 7 + payload.len() / 4);
    let ll = std::fs::read_to_string(dir.join("options_probe.ll")).unwrap();
    // Eight of the counter and one of the output, as Vivado has them.
    assert_eq!(
        ll.lines().filter(|l| l.contains("Latch=")).count(),
        9,
        "{ll}"
    );
    assert_eq!(ll.lines().filter(|l| l.contains("Ram=B:")).count(), 18_432);

    // An option Reticle does not do is refused, and nothing is built with it.
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--chipdb",
            &root,
            "--constraints",
        ])
        .arg(fixtures().join("options_probe.rcf"))
        .arg("--output-dir")
        .arg(&dir)
        .arg("--bitstream")
        .arg(dir.join("no.bit"))
        .args(["--bitstream-option", "BITSTREAM.ENCRYPTION.ENCRYPT=YES"])
        .arg(fixtures().join("options_probe.v"))
        .output()
        .expect("the reticle binary runs");
    assert!(!run.status.success());
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("AES-256"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
