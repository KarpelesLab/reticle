//! `ip/video/h264_parse`: the front end of the H.264 decoder, simulated.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `the_vlc_transcription_is_a_set_of_prefix_codes` | every VLC of `testdata/ip/h264_parse_vlc_tables.txt` is prefix-free with the Kraft sum its table's shape implies, and each cbp column is a permutation of 0..47 |
//! | `h264_vlc_decodes_every_code_of_the_tables` | the generated `h264_vlc` decodes every one of those codes, under every padding tried, to the transcribed value and length |
//! | `h264_parse_agrees_with_ffmpeg_on_every_macroblock` | every structural corpus decodes to the macroblock classes and QPY ffmpeg's decoder reports, picture for picture, with no `ERROR` token and the token grammar of docs/h264-decoder.md §1 respected |
//! | `h264_parse_holds_a_stream_under_backpressure` | the same token stream comes out when the consumer stalls at random and the producer gaps at random |
//! | `h264_parse_reports_truncation_and_survives_corruption` | a slice cut short is one `ERROR` code 6 and costs only its picture; forty single-bit corruptions of slice data never stop the parser or lose the last picture's end |
//! | `h264_parse_matches_the_models_token_dumps` | token for token against `tools/h264_model.py`'s dumps, `testdata/ip/h264_tokens_*.txt` |
//! | `h264_parse_cycles_per_macroblock` (`#[ignore]`) | prints cycles per macroblock on each corpus |
//!
//! What the ffmpeg comparison catches and what it does not: ffmpeg prints a
//! class and a QPY per macroblock, so a wrong coefficient value or a wrong
//! intra mode is invisible to it **unless** it changes how many bits are
//! consumed — and in CAVLC nearly every mistake does, because the next
//! syntax element is then read from the wrong place and the stream goes off
//! the rails within a macroblock or two. A wrong level *sign*, a wrong
//! scan position or a wrong resolved intra mode consumes the same bits and
//! is caught only by the model's dumps.
#![cfg(all(feature = "verilog", feature = "sim"))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ip::IpManifest;
use reticle::ir::Design;
use reticle::logic::Logic;
use reticle::sim::NetHandle;
use reticle::sim::compiled::{CompileOptions, CompiledSim};
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
        .replace("\r\n", "\n")
}

const PACKAGE: &str = "ip/video/h264_parse";

/// The package's sources, as its manifest lists them.
fn sources() -> Vec<(String, String)> {
    let path = format!("{PACKAGE}/reticle.ip");
    let text = read(&path);
    let mut map = SourceMap::new();
    let file = map.add(path.clone(), &text).expect("fits");
    let mut diags = Diagnostics::new();
    let ip = IpManifest::parse(&text, file, &mut diags).expect("the manifest parses");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    ip.sources
        .iter()
        .map(|s| {
            let p = format!("{PACKAGE}/{}", s.path);
            let t = read(&p);
            (p, t)
        })
        .collect()
}

fn design(top: &str) -> Design {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for (path, text) in sources() {
        let id = map.add(path.clone(), &text).expect("fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    assert!(!diags.has_errors(), "parse:\n{}", diags.render(&map));
    let options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &options, &mut diags);
    assert!(!diags.has_errors(), "elaborate:\n{}", diags.render(&map));
    design.expect("a design")
}

fn compiled(design: &Design) -> CompiledSim<'_> {
    let options = CompileOptions {
        zero_init: true,
        ..CompileOptions::default()
    };
    CompiledSim::new(design, options).unwrap_or_else(|why| {
        let lines: Vec<String> = why.iter().map(|w| format!("{w:?}")).collect();
        panic!(
            "not eligible for compiled simulation:\n{}",
            lines.join("\n")
        )
    })
}

fn net(sim: &CompiledSim<'_>, name: &str) -> NetHandle {
    let path = format!("{}.{name}", sim.top_name());
    sim.net(&path).unwrap_or_else(|| panic!("no net {path}"))
}

fn get(sim: &mut CompiledSim<'_>, h: NetHandle) -> u64 {
    sim.get(h).to_u64().expect("two-state")
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

/// One picture: its dimensions in macroblocks, and a class and QPY per
/// macroblock in raster order.
type Picture = (u32, u32, Vec<(String, u32)>);

/// One VLC of the transcription: (table, selector) -> [(code, value)].
type Vlcs = BTreeMap<(String, i32), Vec<(String, Vec<u32>)>>;

fn tables() -> (Vlcs, BTreeMap<(String, u32), u32>) {
    let mut vlcs: Vlcs = BTreeMap::new();
    let mut cbp = BTreeMap::new();
    for line in read("testdata/ip/h264_parse_vlc_tables.txt").lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() || p[0].starts_with('#') {
            continue;
        }
        let sel = |s: &str| -> i32 { s.parse().unwrap_or_else(|_| panic!("{line}")) };
        let num = |s: &str| -> u32 { s.parse().unwrap_or_else(|_| panic!("{line}")) };
        match p[0] {
            "coeff_token" => vlcs
                .entry(("coeff_token".into(), sel(p[1])))
                .or_default()
                .push((p[4].into(), vec![num(p[2]), num(p[3])])),
            "run_before" => vlcs
                .entry(("run_before".into(), sel(p[1])))
                .or_default()
                .push((p[3].into(), vec![num(p[2])])),
            t if t.starts_with("total_zeros") => vlcs
                .entry((t.into(), sel(p[1])))
                .or_default()
                .push((p[3].into(), vec![num(p[2])])),
            "cbp_intra" | "cbp_inter" => {
                cbp.insert((p[0].into(), num(p[1])), num(p[2]));
            }
            other => panic!("unknown table {other}"),
        }
    }
    (vlcs, cbp)
}

#[test]
fn the_vlc_transcription_is_a_set_of_prefix_codes() {
    let (vlcs, cbp) = tables();
    for ((table, sel), codes) in &vlcs {
        for (i, (a, _)) in codes.iter().enumerate() {
            assert!(
                a.chars().all(|c| c == '0' || c == '1'),
                "{table} {sel}: {a}"
            );
            for (j, (b, _)) in codes.iter().enumerate() {
                assert!(
                    i == j || !b.starts_with(a.as_str()),
                    "{table} {sel}: {a} is a prefix of {b}"
                );
            }
        }
        // Kraft sum, in units of 2^-16.
        let kraft: u64 = codes.iter().map(|(c, _)| 1u64 << (16 - c.len())).sum();
        // The tables that leave codes unused, and how much: everything else
        // is complete. coeff_token's columns leave the all-zero corner, the
        // 6-bit FLC of 8 <= nC leaves two words, total_zeros for
        // tzVlcIndex 1 leaves 000000000, and run_before for zerosLeft > 6
        // stops at 14.
        let expect: u64 = match (table.as_str(), *sel) {
            ("coeff_token", 0) => 65536 - 2,
            ("coeff_token", 2) => 65536 - 8,
            ("coeff_token", 4) => 65536 - 64,
            ("coeff_token", 8) => 65536 - 2048,
            ("coeff_token", -2) => 65536 - 56,
            ("total_zeros16", 1) => 65536 - 128,
            ("run_before", 7) => 65536 - 32,
            _ => 65536,
        };
        assert_eq!(kraft, expect, "{table} {sel}: Kraft sum");
    }
    let counts: BTreeMap<&str, usize> = vlcs.iter().fold(BTreeMap::new(), |mut m, ((t, _), c)| {
        *m.entry(t.as_str()).or_default() += c.len();
        m
    });
    assert_eq!(counts["coeff_token"], 4 * 62 + 14 + 30);
    assert_eq!(counts["total_zeros16"], 135);
    assert_eq!(counts["total_zeros4"], 9);
    assert_eq!(counts["run_before"], 2 + 3 + 4 + 5 + 6 + 7 + 15);
    for kind in ["cbp_intra", "cbp_inter"] {
        let values: BTreeSet<u32> = (0..48).map(|c| cbp[&(kind.to_owned(), c)]).collect();
        assert_eq!(
            values,
            (0..48).collect(),
            "{kind} is not a permutation of 0..47"
        );
    }
}

#[test]
fn h264_vlc_decodes_every_code_of_the_tables() {
    let (vlcs, cbp) = tables();
    let d = design("h264_vlc");
    let mut sim = compiled(&d);
    let n = |s: &CompiledSim<'_>, name: &str| net(s, name);
    let (ct_cls, ct_bits, ct_valid, ct_len, ct_t1, ct_tc) = (
        n(&sim, "ct_cls"),
        n(&sim, "ct_bits"),
        n(&sim, "ct_valid"),
        n(&sim, "ct_len"),
        n(&sim, "ct_t1"),
        n(&sim, "ct_tc"),
    );
    let (tz_dc, tz_tc, tz_bits, tz_valid, tz_len, tz_val) = (
        n(&sim, "tz_dc"),
        n(&sim, "tz_tc"),
        n(&sim, "tz_bits"),
        n(&sim, "tz_valid"),
        n(&sim, "tz_len"),
        n(&sim, "tz_val"),
    );
    let (rb_zl, rb_bits, rb_valid, rb_len, rb_val) = (
        n(&sim, "rb_zl"),
        n(&sim, "rb_bits"),
        n(&sim, "rb_valid"),
        n(&sim, "rb_len"),
        n(&sim, "rb_val"),
    );
    let (cbp_code, cbp_inter, cbp_valid, cbp_val) = (
        n(&sim, "cbp_code"),
        n(&sim, "cbp_inter"),
        n(&sim, "cbp_valid"),
        n(&sim, "cbp_val"),
    );
    // A code left-aligned in `width` bits, padded with `pad`.
    let aligned = |code: &str, width: usize, pad: u64| -> u64 {
        let v = u64::from_str_radix(code, 2).expect("binary");
        let rest = width - code.len();
        (v << rest) | (pad & ((1u64 << rest) - 1))
    };
    let pads = [0u64, u64::MAX, 0x5555_5555, 0xAAAA_AAAA, 0x1234_5678];
    let mut checked = 0;
    for ((table, sel), codes) in &vlcs {
        for (code, value) in codes {
            for pad in pads {
                match table.as_str() {
                    "coeff_token" => {
                        let cls = match sel {
                            0 => 0,
                            2 => 1,
                            4 => 2,
                            8 => 3,
                            -1 => 4,
                            _ => break, // 4:2:2 chroma DC is not decoded
                        };
                        sim.set(ct_cls, Logic::from_u64(cls, 3));
                        sim.set(ct_bits, Logic::from_u64(aligned(code, 16, pad), 16));
                        assert_eq!(get(&mut sim, ct_valid), 1, "coeff_token {sel} {code}");
                        assert_eq!(get(&mut sim, ct_len), code.len() as u64);
                        assert_eq!(get(&mut sim, ct_t1), u64::from(value[0]));
                        assert_eq!(get(&mut sim, ct_tc), u64::from(value[1]));
                    }
                    "total_zeros16" | "total_zeros4" => {
                        sim.set(tz_dc, Logic::from_bool(table == "total_zeros4"));
                        sim.set(
                            tz_tc,
                            Logic::from_u64(u64::try_from(*sel).expect("index"), 4),
                        );
                        sim.set(tz_bits, Logic::from_u64(aligned(code, 9, pad), 9));
                        assert_eq!(get(&mut sim, tz_valid), 1, "{table} {sel} {code}");
                        assert_eq!(get(&mut sim, tz_len), code.len() as u64);
                        assert_eq!(get(&mut sim, tz_val), u64::from(value[0]));
                    }
                    "run_before" => {
                        sim.set(
                            rb_zl,
                            Logic::from_u64(u64::try_from(*sel).expect("zerosLeft"), 3),
                        );
                        sim.set(rb_bits, Logic::from_u64(aligned(code, 11, pad), 11));
                        assert_eq!(get(&mut sim, rb_valid), 1, "run_before {sel} {code}");
                        assert_eq!(get(&mut sim, rb_len), code.len() as u64);
                        assert_eq!(get(&mut sim, rb_val), u64::from(value[0]));
                    }
                    _ => break, // 4:2:2 total_zeros is not decoded
                }
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 5 * (4 * 62 + 14 + 135 + 9 + 42),
        "only {checked} decodes"
    );
    for ((kind, code), value) in &cbp {
        sim.set(cbp_code, Logic::from_u64(u64::from(*code), 6));
        sim.set(cbp_inter, Logic::from_bool(kind == "cbp_inter"));
        assert_eq!(get(&mut sim, cbp_valid), 1);
        assert_eq!(get(&mut sim, cbp_val), u64::from(*value), "{kind} {code}");
    }
    // And codeNum 48 and up match nothing.
    sim.set(cbp_code, Logic::from_u64(48, 6));
    assert_eq!(get(&mut sim, cbp_valid), 0);
}

// ---------------------------------------------------------------------------
// Streams
// ---------------------------------------------------------------------------

/// One structural corpus: the stream, and per picture its dimensions and
/// ffmpeg's class and QPY for every macroblock.
struct Corpus {
    name: String,
    stream: Vec<u8>,
    pics: Vec<Picture>,
}

fn corpora() -> Vec<Corpus> {
    let dir = root().join("testdata/ip");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .expect("testdata/ip")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            n.starts_with("h264_parse_") && n.ends_with(".txt") && n != "h264_parse_vlc_tables.txt"
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|file| {
            let text = read(&format!("testdata/ip/{file}"));
            let mut stream = Vec::new();
            let mut pics: Vec<Picture> = Vec::new();
            for line in text.lines() {
                let p: Vec<&str> = line.split_whitespace().collect();
                match p.first() {
                    Some(&"stream") => {
                        let hex = p[1];
                        for i in (0..hex.len()).step_by(2) {
                            stream.push(u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"));
                        }
                    }
                    Some(&"pic") => pics.push((
                        p[1].parse().expect("w"),
                        p[2].parse().expect("h"),
                        Vec::new(),
                    )),
                    Some(&"mb") => pics
                        .last_mut()
                        .expect("pic before mb")
                        .2
                        .push((p[1].to_owned(), p[2].parse().expect("qp"))),
                    _ => {}
                }
            }
            Corpus {
                name: file
                    .trim_start_matches("h264_parse_")
                    .trim_end_matches(".txt")
                    .to_owned(),
                stream,
                pics,
            }
        })
        .collect()
}

/// A run of the parser over one stream.
struct Run {
    tokens: Vec<u32>,
    /// The cycle each token left on.
    cycles: Vec<u64>,
    total_cycles: u64,
}

/// A small deterministic generator for stalls.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

/// Feeds `stream` to h264_parse and collects the tokens. `stall` is the
/// percentage of cycles on which the producer offers nothing and, apart,
/// the consumer refuses; 0 is full speed.
fn run(sim: &mut CompiledSim<'_>, stream: &[u8], stall: u32, seed: u64) -> Run {
    let rst_n = net(sim, "rst_n");
    let in_byte = net(sim, "in_byte");
    let in_valid = net(sim, "in_valid");
    let in_last = net(sim, "in_last");
    let in_ready = net(sim, "in_ready");
    let tok = net(sim, "tok");
    let tok_valid = net(sim, "tok_valid");
    let tok_ready = net(sim, "tok_ready");
    let mut rng = Lcg(seed);

    sim.set(in_valid, Logic::from_bool(false));
    sim.set(in_last, Logic::from_bool(false));
    sim.set(in_byte, Logic::from_u64(0, 8));
    sim.set(tok_ready, Logic::from_bool(false));
    sim.set(rst_n, Logic::from_bool(false));
    sim.step();
    sim.step();
    sim.set(rst_n, Logic::from_bool(true));

    let mut pos = 0usize;
    let mut out = Run {
        tokens: Vec::new(),
        cycles: Vec::new(),
        total_cycles: 0,
    };
    let mut idle = 0u64;
    let mut cycle = 0u64;
    // Enough for any corpus here: the parser needs a few cycles a bit at
    // the very worst.
    let limit = 4000 + 400 * stream.len() as u64;
    while cycle < limit {
        let offer = pos < stream.len() && (stall == 0 || rng.next() % 100 >= stall);
        let accept = stall == 0 || rng.next() % 100 >= stall;
        sim.set(in_valid, Logic::from_bool(offer));
        if offer {
            sim.set(in_byte, Logic::from_u64(u64::from(stream[pos]), 8));
            sim.set(in_last, Logic::from_bool(pos + 1 == stream.len()));
        }
        sim.set(tok_ready, Logic::from_bool(accept));
        let took = offer && get(sim, in_ready) == 1;
        let gave = accept && get(sim, tok_valid) == 1;
        if gave {
            out.tokens
                .push(u32::try_from(get(sim, tok)).expect("32 bits"));
            out.cycles.push(cycle);
        }
        sim.step();
        cycle += 1;
        if took {
            pos += 1;
        }
        if pos == stream.len() && !gave {
            idle += 1;
            // Drained: nothing has come out for a long while after the
            // last byte went in.
            if idle > 2000 {
                break;
            }
        } else {
            idle = 0;
        }
    }
    assert!(
        pos == stream.len(),
        "the parser stopped taking bytes at {pos} of {}",
        stream.len()
    );
    out.total_cycles = cycle - idle;
    out
}

/// Checks the token grammar of docs/h264-decoder.md §1 and reduces the
/// stream to pictures of (class, QPY) per macroblock.
fn structure(tokens: &[u32]) -> Vec<Picture> {
    let mut pics: Vec<Picture> = Vec::new();
    let mut i = 0;
    let tag = |t: u32| t >> 24;
    let data = |t: u32| t & 0xFF_FFFF;
    let mut dims = (0, 0);
    let mut in_pic = false;
    while i < tokens.len() {
        let t = tokens[i];
        match tag(t) {
            0x7F => panic!("ERROR token {:06x} at token {i}", data(t)),
            0x01 => {
                dims = (data(t) >> 12, data(t) & 0xFFF);
                assert!(!in_pic, "SPS_DIMS inside a picture at {i}");
                in_pic = true;
                pics.push((
                    dims.0,
                    dims.1,
                    vec![(String::new(), 0); (dims.0 * dims.1) as usize],
                ));
                i += 1;
            }
            0x02..=0x0F => {
                assert!(in_pic, "slice token {t:08x} outside a picture at {i}");
                i += 1;
            }
            0x40 => i += 1,
            0x41 => {
                assert!(in_pic, "PIC_END outside a picture");
                let pic = pics.last().expect("a picture");
                assert!(
                    pic.2.iter().all(|(c, _)| !c.is_empty()),
                    "picture {} ended with macroblocks missing",
                    pics.len() - 1
                );
                in_pic = false;
                i += 1;
            }
            0x10 => {
                // One macroblock: MB_START MB_TYPE [SUB] [modes] [ref/mvd]
                // MB_CBP MB_QP [PCM x128 | (COEF_BLK COEF*)*] MB_END.
                let (x, y) = (data(t) >> 12, data(t) & 0xFFF);
                assert!(
                    x < dims.0 && y < dims.1,
                    "macroblock ({x}, {y}) outside the picture"
                );
                i += 1;
                assert_eq!(tag(tokens[i]), 0x11, "MB_TYPE after MB_START at {i}");
                let kind = data(tokens[i]) >> 19;
                i += 1;
                let class = match kind {
                    0 => "i",
                    1 => "I",
                    2 => "PCM",
                    4 => "P16",
                    5 => "P168",
                    6 => "P816",
                    7 | 8 => "P8",
                    9 => "S",
                    k => panic!("kind {k}"),
                };
                let mut modes = 0;
                let mut refs = 0;
                let mut mvds = 0;
                while matches!(tag(tokens[i]), 0x15 | 0x13 | 0x17 | 0x18 | 0x14) {
                    match tag(tokens[i]) {
                        0x13 | 0x17 => modes += 1,
                        0x18 => refs += 1,
                        0x14 => mvds += 1,
                        _ => {}
                    }
                    i += 1;
                }
                match kind {
                    0 => assert!(modes == 16 || modes == 4, "{modes} intra modes"),
                    4 => assert!(refs == 1 && mvds == 2),
                    5 | 6 => assert!(refs == 2 && mvds == 4),
                    7 | 8 => assert!(refs == 4 && (8..=32).contains(&mvds) && mvds % 2 == 0),
                    _ => assert!(modes == 0 && refs == 0 && mvds == 0),
                }
                assert_eq!(tag(tokens[i]), 0x19, "MB_CBP at {i}");
                let cbp = data(tokens[i]) & 0x3F;
                i += 1;
                assert_eq!(tag(tokens[i]), 0x12, "MB_QP at {i}");
                let qp = data(tokens[i]) & 0xFF;
                i += 1;
                if kind == 2 {
                    for _ in 0..128 {
                        assert_eq!(tag(tokens[i]), 0x30);
                        i += 1;
                    }
                }
                let mut blocks = 0;
                while tag(tokens[i]) == 0x20 {
                    let tc = data(tokens[i]) & 0x1F;
                    let cat = (data(tokens[i]) >> 20) & 0xF;
                    i += 1;
                    let mut last = 64;
                    for _ in 0..tc {
                        assert_eq!(tag(tokens[i]), 0x21, "COEF at {i}");
                        let p = data(tokens[i]) >> 18;
                        assert!(p < last, "positions not descending at {i}");
                        let level = data(tokens[i]) & 0xFFFF;
                        assert!(level != 0, "a zero level at {i}");
                        let limit = match cat {
                            0 | 2 => 16,
                            3 | 4 => 4,
                            _ => 16,
                        };
                        assert!(p < limit, "position {p} in category {cat}");
                        if matches!(cat, 1 | 5 | 6) {
                            assert!(p >= 1, "AC block coefficient at 0");
                        }
                        last = p;
                        i += 1;
                    }
                    blocks += 1;
                }
                if cbp == 0 && kind != 1 {
                    assert_eq!(blocks, 0, "residual with cbp 0");
                }
                assert_eq!(tag(tokens[i]), 0x3F, "MB_END at {i}, got {:08x}", tokens[i]);
                i += 1;
                let pic = pics.last_mut().expect("a picture");
                pic.2[(y * dims.0 + x) as usize] = (class.to_owned(), qp);
            }
            other => panic!("unexpected tag {other:02x} at token {i}"),
        }
    }
    assert!(!in_pic, "the stream ended inside a picture");
    pics
}

#[test]
fn h264_parse_agrees_with_ffmpeg_on_every_macroblock() {
    let d = design("h264_parse");
    let mut sim = compiled(&d);
    let all = corpora();
    assert!(!all.is_empty(), "no testdata/ip/h264_parse_*.txt corpora");
    for corpus in &all {
        let r = run(&mut sim, &corpus.stream, 0, 1);
        if let Some(dir) = std::env::var_os("H264_DUMP_DIR") {
            let text: String = r.tokens.iter().map(|t| format!("{t:08x}\n")).collect();
            fs::write(Path::new(&dir).join(format!("{}.tok", corpus.name)), text).expect("dump");
        }
        let got = structure(&r.tokens);
        assert_eq!(
            got.len(),
            corpus.pics.len(),
            "{}: picture count",
            corpus.name
        );
        for (n, (g, e)) in got.iter().zip(&corpus.pics).enumerate() {
            assert_eq!(
                (g.0, g.1),
                (e.0, e.1),
                "{} picture {n}: dimensions",
                corpus.name
            );
            for (a, (gm, em)) in g.2.iter().zip(&e.2).enumerate() {
                assert_eq!(
                    gm.0, em.0,
                    "{} picture {n} macroblock {a}: class",
                    corpus.name
                );
                // ffmpeg prints 0 for an I_PCM macroblock, which is the qP
                // deblocking uses for it (8.7.2.2), not its QPY: 7.4.5
                // infers mb_qp_delta = 0 for every macroblock without one,
                // so an I_PCM macroblock's QPY is its predecessor's, and
                // ffmpeg's own map shows the next macroblock carrying on
                // from that value and not from 0.
                if gm.0 != "PCM" {
                    assert_eq!(
                        gm.1, em.1,
                        "{} picture {n} macroblock {a}: QPY",
                        corpus.name
                    );
                }
            }
        }
        let mbs: usize = got.iter().map(|p| p.2.len()).sum();
        println!(
            "{}: {} pictures, {mbs} macroblocks, {} tokens, {} cycles",
            corpus.name,
            got.len(),
            r.tokens.len(),
            r.total_cycles
        );
    }
}

#[test]
fn h264_parse_holds_a_stream_under_backpressure() {
    let d = design("h264_parse");
    let mut sim = compiled(&d);
    let all = corpora();
    let corpus = all.iter().min_by_key(|c| c.stream.len()).expect("a corpus");
    let clean = run(&mut sim, &corpus.stream, 0, 1);
    for (stall, seed) in [(30, 7u64), (70, 11)] {
        let r = run(&mut sim, &corpus.stream, stall, seed);
        assert_eq!(r.tokens, clean.tokens, "{} at {stall}% stalls", corpus.name);
    }
}

/// The offsets of each start code's first payload byte.
fn nal_starts(stream: &[u8]) -> Vec<usize> {
    (2..stream.len())
        .filter(|&i| stream[i] == 1 && stream[i - 1] == 0 && stream[i - 2] == 0)
        .map(|i| i + 1)
        .collect()
}

/// A slice cut short reports `ERROR` code 6 and costs only its own
/// picture; and no single corrupted byte in slice data hangs the parser or
/// breaks the token grammar's framing of what follows.
///
/// What it would catch: a parser that waits forever for bits a NAL unit
/// does not have, one that reports the wrong code for it, one that does
/// not resynchronise at the next NAL unit, and a corruption that stops the
/// parser taking input (`run` asserts every byte was taken). What it would
/// not: a corruption decoded into plausible but wrong tokens, which no
/// parser can tell from a valid stream.
#[test]
fn h264_parse_reports_truncation_and_survives_corruption() {
    let d = design("h264_parse");
    let mut sim = compiled(&d);
    let all = corpora();
    let corpus = all
        .iter()
        .find(|c| c.name == "tiny")
        .expect("the tiny corpus");
    let starts = nal_starts(&corpus.stream);
    // The IDR slice, NAL type 5, and the NAL unit after it.
    let idr = starts
        .iter()
        .position(|&s| corpus.stream[s] & 0x1F == 5)
        .expect("an IDR slice");
    let (from, to) = (starts[idr], starts[idr + 1] - 4);
    let mut cut = corpus.stream[..from + (to - from) / 2].to_vec();
    cut.extend_from_slice(&corpus.stream[to..]);
    let r = run(&mut sim, &cut, 0, 1);
    let errors: Vec<u32> = r
        .tokens
        .iter()
        .filter(|t| *t >> 24 == 0x7F)
        .copied()
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "one ERROR for one short slice: {errors:08x?}"
    );
    assert_eq!(
        (errors[0] >> 16) & 0xFF,
        6,
        "the code for a NAL unit that ended early"
    );
    let pic_ends = r.tokens.iter().filter(|t| *t >> 24 == 0x41).count();
    assert_eq!(
        pic_ends,
        corpus.pics.len() - 1,
        "the other pictures still end"
    );

    let mut rng = Lcg(5);
    let mut reported = 0;
    let trials = 40;
    for _ in 0..trials {
        let mut bad = corpus.stream.clone();
        // A byte of slice data: past the first slice's header bytes.
        let at = from + 4 + (rng.next() as usize) % (to - from - 4);
        bad[at] ^= 1 << (rng.next() % 8);
        let r = run(&mut sim, &bad, 0, 1);
        if r.tokens.iter().any(|t| *t >> 24 == 0x7F) {
            reported += 1;
        }
        // Whatever happened to that slice, the last picture's header and
        // end still come out.
        assert_eq!(
            r.tokens.last().map(|t| t >> 24),
            Some(0x41),
            "corruption at {at}"
        );
    }
    println!("{reported} of {trials} single-bit corruptions reported an ERROR");
}

#[test]
#[ignore = "prints a measurement rather than asserting one"]
fn h264_parse_cycles_per_macroblock() {
    let d = design("h264_parse");
    let mut sim = compiled(&d);
    for corpus in corpora() {
        let r = run(&mut sim, &corpus.stream, 0, 1);
        // Cycles between consecutive MB_START tokens, per macroblock kind.
        let mut starts = Vec::new();
        for (k, t) in r.tokens.iter().enumerate() {
            if t >> 24 == 0x10 {
                starts.push((r.cycles[k], r.tokens[k + 1] >> 19 & 0x1F));
            }
        }
        let mut by_kind: BTreeMap<u32, (u64, u64, u64)> = BTreeMap::new();
        for w in starts.windows(2) {
            let span = w[1].0 - w[0].0;
            let e = by_kind.entry(w[0].1).or_insert((0, 0, 0));
            e.0 += 1;
            e.1 += span;
            e.2 = e.2.max(span);
        }
        let mbs = starts.len().max(1) as u64;
        println!(
            "{}: {} bytes, {mbs} macroblocks, {} cycles, {:.1} cycles/MB overall, {:.2} cycles/byte",
            corpus.name,
            corpus.stream.len(),
            r.total_cycles,
            r.total_cycles as f64 / mbs as f64,
            r.total_cycles as f64 / corpus.stream.len() as f64
        );
        for (kind, (n, sum, max)) in by_kind {
            println!(
                "    kind {kind:2}: {n:5} MBs, mean {:.1}, max {max} cycles",
                sum as f64 / n as f64
            );
        }
    }
}

/// The software model's token dumps, `testdata/ip/h264_tokens_*.txt`
/// (docs/h264-decoder.md, *Model dumps*): its `nal` lines are the input,
/// each behind a four-byte start code, and its `tok` lines the stream the
/// parser must produce, token for token.
///
/// This is the independent check the ffmpeg comparison is not: the model
/// was written separately from the same Recommendation and is bit-exact
/// with ffmpeg's decoded pictures, so a level sign, a scan position, a
/// resolved intra mode or an mvd that is wrong here is caught here.
#[test]
fn h264_parse_matches_the_models_token_dumps() {
    let dir = root().join("testdata/ip");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .expect("testdata/ip")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("h264_tokens_") && n.ends_with(".txt"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no testdata/ip/h264_tokens_*.txt");
    let d = design("h264_parse");
    let mut sim = compiled(&d);
    let mut total = 0;
    for file in &names {
        let text = read(&format!("testdata/ip/{file}"));
        let mut stream = Vec::new();
        let mut want = Vec::new();
        for line in text.lines() {
            let p: Vec<&str> = line.split_whitespace().collect();
            match p.first() {
                Some(&"nal") => {
                    stream.extend_from_slice(&[0, 0, 0, 1]);
                    for i in (0..p[1].len()).step_by(2) {
                        stream.push(u8::from_str_radix(&p[1][i..i + 2], 16).expect("hex"));
                    }
                }
                Some(&"tok") => want.push(u32::from_str_radix(p[1], 16).expect("hex")),
                _ => {}
            }
        }
        let r = run(&mut sim, &stream, 0, 1);
        for (k, (g, w)) in r.tokens.iter().zip(&want).enumerate() {
            assert_eq!(
                g, w,
                "{file}: token {k}: got {g:08x}, the model says {w:08x}"
            );
        }
        assert_eq!(r.tokens.len(), want.len(), "{file}: token count");
        total += want.len();
    }
    println!("{} model dumps, {total} tokens, matched", names.len());
}
