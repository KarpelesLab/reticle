//! `ip/audio/aac_dec`, the AAC-LC decoder, in simulation.
//!
//! The decoder is checked against two references that answer different
//! questions:
//!
//! * **its own bit-accurate model**, `tools/aac_dec_model.py`, which makes
//!   every rounding and saturation the RTL makes: the RTL must produce the
//!   model's PCM bit for bit, frame for frame (an FNV-1a hash of each frame
//!   is in the corpus). This is what catches a wrong wire.
//! * **ffmpeg's decoder**, a decoder nobody here wrote, run as a black box
//!   when the corpus was made. AAC decoders are not required to agree to
//!   the bit, so this is a tolerance: one LSB, README.md says why. Where
//!   the corpus carries ffmpeg's samples the RTL's output is compared with
//!   them directly; everywhere else the corpus carries the model's
//!   difference from ffmpeg, and the test holds that to the same bound.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `every_codeword_decodes_through_the_tree_rom` | each of the 1362 codewords of the standard's twelve codebooks walks `aac_huff_rom` to its own entry, with the n-tuple Table 4.151's rule makes of its index |
//! | `band_tables_have_the_standards_shape` | `aac_swb` gives every rate the band count of Tables 4.129 to 4.141, increasing, in multiples of four, closing at 1024 and 128 |
//! | `decodes_*` | the corpus's streams through the whole decoder, input and output stalled, every frame equal to the model's; cycles per frame against the budget |
//! | `agrees_with_ffmpeg_to_one_lsb` | the RTL's samples against ffmpeg's own, and the corpus's record of the model against ffmpeg |
//! | `raw_access_units_decode_as_adts_does` | the same frames without their ADTS headers, as MP4 carries them, decode to the same samples |
//! | `refuses_broken_frames_and_resyncs` | lost syncwords, a CCE, a reserved codebook and a short ADTS frame are each reported with their code, produce nothing, and the next frame decodes from the state the last good one left |
//!
//! What none of this catches is a misreading of the standard that the
//! model, the RTL and ffmpeg share; and nothing here has run on a part.

#![cfg(all(feature = "verilog", feature = "sim"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::logic::Logic;
use reticle::sim::{MemHandle, NetHandle, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

// ---------------------------------------------------------------------------
// Elaboration and the simulator
// ---------------------------------------------------------------------------

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The package's sources, in the order its manifest lists them.
fn sources() -> Vec<(String, String)> {
    let dir = root().join("ip/audio/aac_dec");
    let manifest = fs::read_to_string(dir.join("reticle.ip")).expect("the manifest");
    manifest
        .lines()
        .filter_map(|l| l.strip_prefix("source "))
        .map(|rel| {
            let path = dir.join(rel.trim());
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .replace("\r\n", "\n");
            (path.display().to_string(), text)
        })
        .collect()
}

fn elaborate_top(top: &str) -> Design {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for (path, text) in sources() {
        let id = map.add(path, &text).expect("the source fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let refs: Vec<_> = files.iter().collect();
    let options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
    let design = elaborate(&refs, &options, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    design.expect("a design")
}

fn decoder_design() -> &'static Design {
    static DESIGN: OnceLock<Design> = OnceLock::new();
    DESIGN.get_or_init(|| elaborate_top("aac_dec"))
}

const HALF: u64 = 5;

/// A simulation, a cycle at a time when the design has a clock.
struct Sim<'d> {
    sim: Simulator<'d>,
    clk: Option<NetHandle>,
}

impl<'d> Sim<'d> {
    fn new(design: &'d Design) -> Sim<'d> {
        let mut sim = Simulator::new(design, SimOptions::default())
            .unwrap_or_else(|d| panic!("cannot simulate: {} problem(s)", d.len()));
        let clk = sim.net(&format!("{}.clk", sim.top_name()));
        if let Some(clk) = clk {
            sim.set(clk, Logic::from_bool(false));
        }
        Sim { sim, clk }
    }

    fn net(&self, name: &str) -> NetHandle {
        let path = format!("{}.{name}", self.sim.top_name());
        self.sim
            .net(&path)
            .unwrap_or_else(|| panic!("no net {path}"))
    }

    fn memory(&self, name: &str) -> MemHandle {
        let path = format!("{}.{name}", self.sim.top_name());
        self.sim
            .memory(&path)
            .unwrap_or_else(|| panic!("no memory {path}"))
    }

    fn set(&mut self, h: NetHandle, width: u32, v: u64) {
        self.sim.set(h, Logic::from_u64(v, width));
    }

    fn get(&self, h: NetHandle) -> u64 {
        let value = self.sim.get(h);
        value
            .to_u64()
            .unwrap_or_else(|| panic!("{} holds x: {value}", self.sim.net_name(h)))
    }

    /// Lets what was just set propagate, without a clock edge.
    fn settle(&mut self) {
        self.sim.run_for(1);
    }

    /// One rising edge, the inputs held across it.
    fn step(&mut self) {
        let clk = self.clk.expect("a clock");
        self.sim.run_for(HALF);
        self.sim.set(clk, Logic::from_bool(true));
        self.sim.run_for(HALF);
        self.sim.set(clk, Logic::from_bool(false));
    }
}

// ---------------------------------------------------------------------------
// The decoder
// ---------------------------------------------------------------------------

/// The decoder's ports.
struct Dec<'d> {
    s: Sim<'d>,
    cfg_raw: NetHandle,
    cfg_sfi: NetHandle,
    cfg_chan: NetHandle,
    in_data: NetHandle,
    in_valid: NetHandle,
    in_last: NetHandle,
    in_ready: NetHandle,
    out_data: NetHandle,
    out_right: NetHandle,
    out_last: NetHandle,
    out_valid: NetHandle,
    out_ready: NetHandle,
    frame_ok: NetHandle,
    frame_err: NetHandle,
    err_code: NetHandle,
}

/// What a run produced.
#[derive(Default)]
struct Decoded {
    /// Each frame's samples, interleaved left then right.
    frames: Vec<Vec<i16>>,
    /// The err_code of every frame_err, in order.
    errors: Vec<u64>,
    /// The cycle of every frame_ok.
    ok_at: Vec<u64>,
    /// Cycles the whole run took.
    cycles: u64,
}

impl<'d> Dec<'d> {
    fn new(design: &'d Design) -> Dec<'d> {
        let s = Sim::new(design);
        let mut d = Dec {
            cfg_raw: s.net("cfg_raw"),
            cfg_sfi: s.net("cfg_sfi"),
            cfg_chan: s.net("cfg_chan"),
            in_data: s.net("in_data"),
            in_valid: s.net("in_valid"),
            in_last: s.net("in_last"),
            in_ready: s.net("in_ready"),
            out_data: s.net("out_data"),
            out_right: s.net("out_right"),
            out_last: s.net("out_last"),
            out_valid: s.net("out_valid"),
            out_ready: s.net("out_ready"),
            frame_ok: s.net("frame_ok"),
            frame_err: s.net("frame_err"),
            err_code: s.net("err_code"),
            s,
        };
        let rst_n = d.s.net("rst_n");
        for (h, w) in [
            (d.cfg_raw, 1),
            (d.cfg_sfi, 4),
            (d.cfg_chan, 2),
            (d.in_data, 8),
            (d.in_valid, 1),
            (d.in_last, 1),
            (d.out_ready, 1),
            (rst_n, 1),
        ] {
            d.s.set(h, w, 0);
        }
        d.s.step();
        d.s.step();
        d.s.set(rst_n, 1, 1);
        d
    }

    /// Feeds `data` and collects `frames` frames of output. Bytes are
    /// offered on six cycles of seven and samples taken on four of five,
    /// so both handshakes are exercised and not assumed. `last` marks the
    /// last byte of each raw access unit, in raw mode.
    fn run(&mut self, data: &[u8], last: &[bool], frames: usize, stereo: bool) -> Decoded {
        let mut out = Decoded::default();
        let mut cur: Vec<i16> = Vec::new();
        let mut ip = 0usize;
        let mut cycle = 0u64;
        let limit = 400_000 * u64::try_from(frames + 4).expect("fits");
        while out.frames.len() < frames {
            assert!(
                cycle < limit,
                "no progress: {} frames after {cycle} cycles",
                out.frames.len()
            );
            let offer = ip < data.len() && cycle % 7 != 3;
            self.s.set(self.in_valid, 1, u64::from(offer));
            if ip < data.len() {
                self.s.set(self.in_data, 8, u64::from(data[ip]));
                self.s.set(
                    self.in_last,
                    1,
                    u64::from(last.get(ip).copied().unwrap_or(false)),
                );
            }
            let take = cycle % 5 != 1;
            self.s.set(self.out_ready, 1, u64::from(take));
            let accepted = offer && self.s.get(self.in_ready) == 1;
            if take && self.s.get(self.out_valid) == 1 {
                let v = self.s.get(self.out_data);
                let right = self.s.get(self.out_right) == 1;
                let want_right = stereo && cur.len() % 2 == 1;
                assert_eq!(
                    right,
                    want_right,
                    "out_right on sample {} of a frame",
                    cur.len()
                );
                cur.push(i16::from_le_bytes(
                    u16::try_from(v).expect("16 bits").to_le_bytes(),
                ));
                if self.s.get(self.out_last) == 1 {
                    let expect = if stereo { 2048 } else { 1024 };
                    assert_eq!(cur.len(), expect, "out_last after {} samples", cur.len());
                    out.frames.push(std::mem::take(&mut cur));
                }
            }
            if self.s.get(self.frame_err) == 1 {
                out.errors.push(self.s.get(self.err_code));
            }
            if self.s.get(self.frame_ok) == 1 {
                out.ok_at.push(cycle);
            }
            self.s.step();
            cycle += 1;
            if accepted {
                ip += 1;
            }
        }
        out.cycles = cycle;
        out
    }
}

fn fnv64(pcm: &[i16]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for v in pcm {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

/// One stream of `testdata/ip/aac_dec_corpus.txt`.
#[derive(Default, Clone)]
struct Stream {
    name: String,
    sfi: u64,
    ch: u64,
    frames: usize,
    tools: Vec<String>,
    adts: Vec<u8>,
    model: Vec<u64>,
    /// (frame, kind, max, differ, err2, sig2, ours2)
    ffmpeg: Vec<(usize, String, u64, u64, u64, u64, u64)>,
    /// ffmpeg's samples, by frame
    pcm: Vec<(usize, Vec<i16>)>,
    errors: Vec<u64>,
}

fn corpus() -> &'static [Stream] {
    static CORPUS: OnceLock<Vec<Stream>> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let path = root().join("testdata/ip/aac_dec_corpus.txt");
        let text = fs::read_to_string(&path).expect("the corpus");
        let mut out = Vec::new();
        let mut cur = Stream::default();
        for line in text.lines() {
            let mut w = line.split_whitespace();
            let Some(key) = w.next() else { continue };
            let rest: Vec<&str> = w.collect();
            let num = |i: usize| -> u64 { rest[i].parse().expect("a number") };
            match key {
                "stream" => {
                    cur = Stream {
                        name: rest[0].to_owned(),
                        ..Stream::default()
                    };
                }
                "info" => {
                    cur.sfi = num(1);
                    cur.ch = num(3);
                    cur.frames = usize::try_from(num(5)).expect("fits");
                }
                "tools" => cur.tools = rest.iter().map(|s| (*s).to_owned()).collect(),
                "adts" => cur.adts.extend(hex_bytes(rest[0])),
                "model" => cur
                    .model
                    .push(u64::from_str_radix(rest[1], 16).expect("a hash")),
                "ffmpeg" if rest[1] != "none" => cur.ffmpeg.push((
                    usize::try_from(num(0)).expect("fits"),
                    rest[1].to_owned(),
                    num(3),
                    num(5),
                    num(7),
                    num(9),
                    num(11),
                )),
                "pcm" => {
                    let f = usize::try_from(num(0)).expect("fits");
                    let samples = hex_bytes(rest[1])
                        .chunks(2)
                        .map(|p| i16::from_be_bytes([p[0], p[1]]))
                        .collect::<Vec<_>>();
                    match cur.pcm.last_mut() {
                        Some((g, v)) if *g == f => v.extend(samples),
                        _ => cur.pcm.push((f, samples)),
                    }
                }
                "errors" => cur.errors = (0..rest.len()).map(num).collect(),
                "end" => out.push(std::mem::take(&mut cur)),
                _ => {}
            }
        }
        out
    })
}

fn stream(name: &str) -> &'static Stream {
    corpus()
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no stream {name} in the corpus"))
}

fn hex_bytes(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits
        .chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).expect("ascii"), 16).expect("hex"))
        .collect()
}

/// The budget: 1024 samples of a channel pair in 1024 / 48000 seconds at
/// 50 MHz, the slowest clock the README quotes.
const CYCLES_AT_50MHZ_48KHZ: u64 = 50_000_000 * 1024 / 48_000;

/// Decodes `streams` from the corpus and requires the model's frames.
///
/// What this catches: any difference at all between the RTL and
/// `FixedDecoder` on these frames — a stage reading a word a cycle early, a
/// rounding off by half an LSB of the internal format, a band boundary, a
/// sign — since one wrong sample changes the frame's hash; and a frame
/// slower than a tenth of the 48 kHz budget at 50 MHz, which is counted
/// work and not time. What it does not: an error the model shares, which
/// `agrees_with_ffmpeg_to_one_lsb` is for, and anything a stream of the
/// corpus does not exercise (README, "Which coding tools the tests
/// exercise").
fn decode_and_compare(streams: &[&str]) {
    let design = decoder_design();
    for name in streams {
        let s = stream(name);
        let mut d = Dec::new(design);
        let got = d.run(&s.adts, &[], s.frames, s.ch == 2);
        assert!(got.errors.is_empty(), "{name}: errors {:?}", got.errors);
        for (f, (pcm, want)) in got.frames.iter().zip(&s.model).enumerate() {
            assert_eq!(
                fnv64(pcm),
                *want,
                "{name}: frame {f} is not the model's (first samples {:?})",
                &pcm[..8]
            );
        }
        let worst = got
            .ok_at
            .windows(2)
            .map(|w| w[1] - w[0])
            .max()
            .unwrap_or(got.cycles);
        println!(
            "{name}: {} frames, {} cycles, the slowest frame {worst} cycles ({}% of the 48 kHz budget at 50 MHz)",
            got.frames.len(),
            got.cycles,
            worst * 100 / CYCLES_AT_50MHZ_48KHZ
        );
        assert!(
            worst < CYCLES_AT_50MHZ_48KHZ / 10,
            "{name}: a frame took {worst} cycles"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `tools/aac_dec_codebooks.txt`: (book, index, length, codeword).
fn codebooks() -> Vec<(u32, u32, u32, u32)> {
    let text = fs::read_to_string(root().join("tools/aac_dec_codebooks.txt")).expect("codebooks");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (
                f[0].parse().expect("book"),
                f[1].parse().expect("index"),
                f[2].parse().expect("length"),
                u32::from_str_radix(f[3], 16).expect("codeword"),
            )
        })
        .collect()
}

/// What the leaf of `index` in `book` must hold: the index itself for the
/// scalefactor book; else the n-tuple 4.6.3.3's pseudo code makes of it
/// with Table 4.151's (unsigned, dimension, LAV), packed as the decoder
/// unpacks it — four 2-bit fields for a quad, two 5-bit fields for a pair.
fn expected_payload(book: u32, index: u32) -> u32 {
    if book == 0 {
        return index;
    }
    let (unsigned, dim, lav): (bool, u32, i32) = match book {
        1 | 2 => (false, 4, 1),
        3 | 4 => (true, 4, 2),
        5 | 6 => (false, 2, 4),
        7 | 8 => (true, 2, 7),
        9 | 10 => (true, 2, 12),
        _ => (true, 2, 16),
    };
    let modulus = if unsigned { lav + 1 } else { 2 * lav + 1 };
    let off = if unsigned { 0 } else { lav };
    let mut idx = i32::try_from(index).expect("fits");
    let mut out = 0u32;
    for d in (0..dim).rev() {
        let p = modulus.pow(d);
        let v = idx / p - off;
        idx %= p;
        let field = if dim == 4 { 2 } else { 5 };
        out = (out << field) | (u32::from_ne_bytes(v.to_ne_bytes()) & ((1 << field) - 1));
    }
    out
}

/// Every codeword of the twelve codebooks, as the standard prints them,
/// through `aac_huff` and the tree ROM the model generated from the same
/// file. What it would catch: a generator that builds the tree wrongly, a
/// leaf with the wrong tuple, a walker that takes a bit too many or too
/// few. What it would not: a codebook misread from the standard in the
/// first place, since this file is that reading; the corpus and ffmpeg are
/// what stand behind it.
#[test]
fn every_codeword_decodes_through_the_tree_rom() {
    let design = elaborate_top("aac_huff");
    let mut s = Sim::new(&design);
    let rst_n = s.net("rst_n");
    let start = s.net("start");
    let abort = s.net("abort");
    let book = s.net("book");
    let avail = s.net("bit_avail");
    let bit = s.net("bit_val");
    let take = s.net("bit_take");
    let hit = s.net("hit");
    let payload = s.net("payload");
    for (h, w) in [
        (rst_n, 1),
        (start, 1),
        (abort, 1),
        (book, 4),
        (avail, 1),
        (bit, 1),
    ] {
        s.set(h, w, 0);
    }
    s.step();
    s.set(rst_n, 1, 1);
    let books = codebooks();
    assert_eq!(
        books.len(),
        1362,
        "Tables 4.A.1 to 4.A.12 hold 1362 codewords"
    );
    for &(b, index, len, code) in &books {
        s.set(book, 4, u64::from(b));
        s.set(start, 1, 1);
        s.step();
        s.set(start, 1, 0);
        let mut pos = 0u32;
        let mut cycles = 0;
        loop {
            // A bit is offered on two cycles of three.
            let offer = cycles % 3 != 2 && pos < len;
            s.set(avail, 1, u64::from(offer));
            if offer {
                s.set(bit, 1, u64::from((code >> (len - 1 - pos)) & 1));
            }
            s.settle();
            if s.get(hit) == 1 {
                break;
            }
            if offer && s.get(take) == 1 {
                pos += 1;
            }
            s.step();
            cycles += 1;
            assert!(cycles < 100, "book {b} index {index}: no leaf");
        }
        assert_eq!(pos, len, "book {b} index {index}: took {pos} of {len} bits");
        assert_eq!(
            s.get(payload),
            u64::from(expected_payload(b, index)),
            "book {b} index {index}"
        );
        s.step();
    }
}

/// `aac_swb` against what the standard says of its tables: the number of
/// bands for each rate class and window length (Tables 4.129 to 4.141),
/// strictly increasing offsets, all multiples of four (4.6.3.1), and the
/// closing 1024 or 128. A wrong entry that kept all of that would pass;
/// the corpus at five rates is what covers the entries themselves.
#[test]
fn band_tables_have_the_standards_shape() {
    let design = elaborate_top("aac_swb");
    let mut s = Sim::new(&design);
    let sfi = s.net("sfi");
    let short = s.net("short_win");
    let band_a = s.net("band_a");
    let band_b = s.net("band_b");
    let off_a = s.net("off_a");
    let off_b = s.net("off_b");
    let num = s.net("num_swb");
    // index: (long bands, short bands)
    let counts = [
        (3, 49, 14),
        (4, 49, 14),
        (5, 51, 14),
        (6, 47, 15),
        (7, 47, 15),
        (8, 43, 15),
        (9, 43, 15),
        (10, 43, 15),
        (11, 40, 15),
        (12, 40, 15),
    ];
    for (i, long_n, short_n) in counts {
        for (sw, n, end) in [(0, long_n, 1024), (1, short_n, 128)] {
            s.set(sfi, 4, i);
            s.set(short, 1, sw);
            s.settle();
            assert_eq!(s.get(num), n, "sfi {i} short {sw}");
            let mut prev = None;
            for b in 0..=n {
                // The offsets are read on the clock, the next band on port b.
                s.set(band_a, 6, b);
                s.set(band_b, 6, (b + 1).min(63));
                s.step();
                let o = s.get(off_a);
                assert_eq!(o % 4, 0, "sfi {i} short {sw} band {b}: {o}");
                if let Some(p) = prev {
                    assert!(o > p, "sfi {i} short {sw} band {b}: {o} after {p}");
                }
                if b < n {
                    assert!(
                        s.get(off_b) > o,
                        "sfi {i} short {sw}: port b, band {}",
                        b + 1
                    );
                } else {
                    assert_eq!(s.get(off_b), o, "sfi {i} short {sw}: past num_swb");
                }
                prev = Some(o);
            }
            assert_eq!(prev, Some(end), "sfi {i} short {sw}");
        }
    }
}

#[test]
fn decodes_mono_at_every_band_table() {
    // 48, 24, 22.05 (with the CRC and four blocks a frame), 11.025 and
    // 8 kHz: four of the five band-table classes in one channel.
    decode_and_compare(&[
        "sine_m48",
        "craft_tns_m24",
        "craft_esc_m48",
        "craft_multi_crc_m22",
        "sine_m11",
        "sine_m8",
    ]);
}

#[test]
fn decodes_stereo_with_ms_and_intensity() {
    decode_and_compare(&[
        "corr_s48",
        "tones_s44",
        "loud_s48",
        "sweep_s32",
        "tones_s16",
        "craft_split_s48",
    ]);
}

#[test]
fn decodes_noise_substitution() {
    decode_and_compare(&["noise_s48_is", "craft_corr_noise_s48", "faac_s44"]);
}

#[test]
fn decodes_the_crafted_tools() {
    decode_and_compare(&[
        "craft_tns3_s48",
        "craft_pulse_s44",
        "craft_extras_s44",
        "faac_clk48",
    ]);
}

/// Every stream of the corpus is in one of the four tests above.
#[test]
fn the_decode_tests_cover_the_corpus() {
    let tested = [
        "sine_m48",
        "craft_tns_m24",
        "craft_esc_m48",
        "craft_multi_crc_m22",
        "sine_m11",
        "sine_m8",
        "corr_s48",
        "tones_s44",
        "loud_s48",
        "sweep_s32",
        "tones_s16",
        "craft_split_s48",
        "noise_s48_is",
        "craft_corr_noise_s48",
        "faac_s44",
        "craft_tns3_s48",
        "craft_pulse_s44",
        "craft_extras_s44",
        "faac_clk48",
    ];
    for s in corpus() {
        if s.errors.is_empty() {
            assert!(
                tested.contains(&s.name.as_str()),
                "{} is decoded by no test",
                s.name
            );
        }
    }
    // And what the streams exercise, by the generator's own parse.
    let mut tools: Vec<&str> = corpus()
        .iter()
        .flat_map(|s| s.tools.iter().map(String::as_str))
        .collect();
    tools.sort_unstable();
    tools.dedup();
    for t in [
        "sce",
        "cpe",
        "fil",
        "dse",
        "pce",
        "common_window",
        "only_long",
        "long_start",
        "eight_short",
        "long_stop",
        "shape_sine",
        "shape_kbd",
        "grouped_short",
        "ms_mask_1",
        "ms_mask_2",
        "intensity_bands",
        "intensity_hcb2",
        "noise_bands",
        "noise_correlated",
        "pulse",
        "tns",
        "tns_short",
        "tns_downward",
        "tns_compress",
        "tns_res3",
        "esc_values",
        "esc_over_1023",
        "adts_crc",
        "multi_rdb",
        "independent_windows",
        "cb1",
        "cb2",
        "cb3",
        "cb4",
        "cb5",
        "cb6",
        "cb7",
        "cb8",
        "cb9",
        "cb10",
        "cb11",
    ] {
        assert!(tools.contains(&t), "no stream of the corpus uses {t}");
    }
}

/// The RTL's samples against ffmpeg's own where the corpus has them, and
/// the corpus's record of the model against ffmpeg everywhere else.
///
/// The tolerance is one LSB on every sample of every frame that can be
/// compared at all (README, "Against ffmpeg"); a frame of noise
/// substitution can only be compared by its energy, which must agree to
/// within 0.1 dB. This would catch an RTL that drifted from the model and
/// a model that drifted from ffmpeg; it would not catch the two drifting
/// together, which the bit-exact tests above rule out for the RTL.
#[test]
fn agrees_with_ffmpeg_to_one_lsb() {
    let design = decoder_design();
    let mut compared = 0usize;
    for s in corpus().iter().filter(|s| !s.pcm.is_empty()) {
        let mut d = Dec::new(design);
        let got = d.run(&s.adts, &[], s.frames, s.ch == 2);
        for (f, want) in &s.pcm {
            let have = &got.frames[*f];
            assert_eq!(have.len(), want.len(), "{} frame {f}", s.name);
            for (i, (a, b)) in have.iter().zip(want).enumerate() {
                let diff = (i32::from(*a) - i32::from(*b)).abs();
                assert!(
                    diff <= 1,
                    "{} frame {f} sample {i}: RTL {a}, ffmpeg {b}",
                    s.name
                );
            }
            compared += have.len();
        }
    }
    assert!(
        compared >= 5120,
        "only {compared} samples compared with ffmpeg directly"
    );
    let mut exact = 0usize;
    for s in corpus() {
        for (f, kind, max, differ, _err2, sig2, ours2) in &s.ffmpeg {
            match kind.as_str() {
                "exact" => {
                    assert!(
                        *max <= 1,
                        "{} frame {f}: the model is {max} LSB from ffmpeg",
                        s.name
                    );
                    assert!(
                        *differ * 100 <= 1024 * s.ch,
                        "{} frame {f}: {differ} samples differ",
                        s.name
                    );
                    exact += 1;
                }
                "noise" => {
                    #[allow(clippy::cast_precision_loss)]
                    let db = 10.0 * ((*ours2 as f64 + 1.0) / (*sig2 as f64 + 1.0)).log10();
                    assert!(
                        db.abs() < 0.1,
                        "{} frame {f}: noise energy {db:+.3} dB",
                        s.name
                    );
                }
                _ => {}
            }
        }
    }
    assert!(exact >= 25, "only {exact} frames compared with ffmpeg");
}

/// The same frames as raw access units, as MP4 and a TS demultiplexer
/// hand them over: no ADTS header, in_last on the last byte of each, and
/// the AudioSpecificConfig's rate and channels on cfg_sfi and cfg_chan.
#[test]
fn raw_access_units_decode_as_adts_does() {
    let design = decoder_design();
    for name in ["corr_s48", "sine_m48"] {
        let s = stream(name);
        let mut data = Vec::new();
        let mut last = Vec::new();
        let mut pos = 0;
        while pos < s.adts.len() {
            let a = &s.adts[pos..];
            let flen =
                (usize::from(a[3] & 3) << 11) | (usize::from(a[4]) << 3) | usize::from(a[5] >> 5);
            assert_eq!(
                a[1] & 1,
                1,
                "{name}: a raw unit is cut from a CRC-less frame"
            );
            assert_eq!(a[6] & 3, 0, "{name}: and from one raw_data_block");
            data.extend_from_slice(&a[7..flen]);
            last.extend((7..flen).map(|i| i == flen - 1));
            pos += flen;
        }
        let mut d = Dec::new(design);
        d.s.set(d.cfg_raw, 1, 1);
        d.s.set(d.cfg_sfi, 4, s.sfi);
        d.s.set(d.cfg_chan, 2, s.ch);
        let got = d.run(&data, &last, s.frames, s.ch == 2);
        assert!(got.errors.is_empty(), "{name}: errors {:?}", got.errors);
        for (f, (pcm, want)) in got.frames.iter().zip(&s.model).enumerate() {
            assert_eq!(fnv64(pcm), *want, "{name}: raw frame {f}");
        }
    }
}

/// Streams with parts the decoder must refuse: bytes that are not a
/// syncword (err_code 1, once per stretch), a CCE (3), a reserved
/// codebook (5), and an ADTS frame whose length ends before its data does
/// (9). Each refused frame produces no samples, and the good frames after
/// it are the model's decode of the stream without the refused ones, which
/// is to say the decoder's state is the last good frame's.
#[test]
fn refuses_broken_frames_and_resyncs() {
    let design = decoder_design();
    for s in corpus().iter().filter(|s| !s.errors.is_empty()) {
        let mut d = Dec::new(design);
        let got = d.run(&s.adts, &[], s.frames, s.ch == 2);
        assert_eq!(got.errors, s.errors, "{}: the errors reported", s.name);
        for (f, (pcm, want)) in got.frames.iter().zip(&s.model).enumerate() {
            assert_eq!(fnv64(pcm), *want, "{}: good frame {f}", s.name);
        }
    }
}

// ---------------------------------------------------------------------------
// A debugging aid
// ---------------------------------------------------------------------------

fn sext(v: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    (i64::from_ne_bytes(v.to_ne_bytes()) << shift) >> shift
}

/// Dumps the decoder's internals while it decodes a stream, for
/// `tools/aac_dec_model.py` to compare: each channel's spectrum as
/// synthesis starts, the IMDCT work RAM after each step, and the PCM. Run
/// with `AAC_DUMP_IN` naming an ADTS file, `AAC_DUMP_OUT` the file to
/// write and `AAC_DUMP_FRAMES` how many frames.
#[test]
#[ignore = "a debugging aid, driven by environment variables"]
fn dump_internals() {
    let (Ok(input), Ok(output)) = (std::env::var("AAC_DUMP_IN"), std::env::var("AAC_DUMP_OUT"))
    else {
        println!("set AAC_DUMP_IN and AAC_DUMP_OUT");
        return;
    };
    let frames: usize = std::env::var("AAC_DUMP_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let data = fs::read(Path::new(&input)).expect("input");
    let design = decoder_design();
    let mut d = Dec::new(design);
    let syn_start = d.s.net("syn_start");
    let qch = d.s.net("qch");
    let ph = d.s.net("u_synth.ph");
    let ph_next = d.s.net("u_synth.ph_next");
    let drain = d.s.net("u_synth.drain");
    let spec_l = d.s.memory("u_spec_l.mem");
    let spec_r = d.s.memory("u_spec_r.mem");
    let work = d.s.memory("u_synth.u_work.mem");
    let mut text = String::new();
    let mut ip = 0usize;
    let mut done = 0usize;
    let mut cycles = 0u64;
    let mem =
        |d: &Dec<'_>, m: MemHandle, i: u64| d.s.sim.get_mem(m, i).unwrap_or_else(|| Logic::zero(1));
    d.s.set(d.out_ready, 1, 1);
    while done < frames && cycles < 20_000_000 {
        d.s.set(d.in_valid, 1, u64::from(ip < data.len()));
        if ip < data.len() {
            d.s.set(d.in_data, 8, u64::from(data[ip]));
        }
        let took = ip < data.len() && d.s.get(d.in_ready) == 1;
        if d.s.get(syn_start) == 1 {
            let c = d.s.get(qch);
            let m = if c == 1 { spec_r } else { spec_l };
            for i in 0..1024 {
                let v = mem(&d, m, i).to_u64().unwrap_or(0);
                text.push_str(&format!("X {c} {i} {}\n", sext(v, 35)));
            }
        }
        if d.s.get(ph) == 5 && d.s.get(drain) == 0 {
            let nx = d.s.get(ph_next);
            for i in 0..512 {
                let w = mem(&d, work, i);
                let re = w.slice(69, 35).to_u64().unwrap_or(0);
                let im = w.slice(34, 0).to_u64().unwrap_or(0);
                text.push_str(&format!("W {nx} {i} {} {}\n", sext(re, 35), sext(im, 35)));
            }
        }
        if d.s.get(d.out_valid) == 1 {
            let v = d.s.get(d.out_data);
            let last = d.s.get(d.out_last);
            let right = d.s.get(d.out_right);
            text.push_str(&format!("S {} {right} {last}\n", sext(v, 16)));
            if last == 1 {
                done += 1;
            }
        }
        if d.s.get(d.frame_err) == 1 {
            text.push_str(&format!("E {}\n", d.s.get(d.err_code)));
        }
        if d.s.get(d.frame_ok) == 1 {
            text.push_str(&format!("F {cycles}\n"));
        }
        d.s.step();
        cycles += 1;
        if took {
            ip += 1;
        }
    }
    fs::write(&output, text).expect("write the dump");
}
