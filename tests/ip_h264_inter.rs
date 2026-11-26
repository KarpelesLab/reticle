//! `ip/video/h264_inter`: H.264 inter prediction, simulated against models
//! written from the Recommendation.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `mvpred_matches_the_model_on_random_streams` | `h264_mvpred`'s output token stream is the model's, token for token, over random P and I pictures with random slices, every partition shape and P_Skip, under random back-pressure on both sides |
//!
//! The model in `mod mvmodel` is written from clause 8.4.1 and 6.4.11.7
//! with a deliberately different structure from the hardware: it keeps
//! the whole picture's vectors and decides a neighbour's availability
//! from its macroblock address and the slice that address belongs to, where
//! the hardware keeps a line buffer and decides it from positions. A
//! line-buffer or availability mistake in either shows up as a difference.
//! What it cannot catch is a misreading of the Recommendation that both
//! share, since one person wrote both; the model agent's dumps, decoded
//! from real streams by an independent decoder, are the check for that.

#![cfg(all(feature = "verilog", feature = "sim"))]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "models of hardware fields: every narrowing cast here is the field width the hardware has"
)]

use std::fs;
use std::path::Path;

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::logic::Logic;
use reticle::sim::NetHandle;
use reticle::sim::compiled::{CompileOptions, CompiledSim};
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

// ---------------------------------------------------------------------------
// Building and driving the design
// ---------------------------------------------------------------------------

const RTL: &str = "ip/video/h264_inter/rtl";

/// Elaborates the package's sources with `top` as the top.
fn design(files: &[&str], top: &str, params: &[(&str, &str)]) -> Design {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut parsed = Vec::new();
    for f in files {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(RTL).join(f);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let id = map.add(path.display().to_string(), &text).expect("fits");
        parsed.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let mut options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
    for (n, v) in params {
        options = options.with_param(*n, *v);
    }
    let refs: Vec<_> = parsed.iter().collect();
    let d = elaborate(&refs, &options, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    d.unwrap_or_else(|| panic!("{top} produced no design"))
}

/// A compiled simulation with every register starting at zero.
fn compiled(design: &Design) -> CompiledSim<'_> {
    CompiledSim::new(
        design,
        CompileOptions {
            zero_init: true,
            ..CompileOptions::default()
        },
    )
    .unwrap_or_else(|p| {
        let why: Vec<String> = p
            .iter()
            .map(|i| format!("{}: {}", i.object, i.reason))
            .collect();
        panic!("not eligible for compiled mode:\n  {}", why.join("\n  "))
    })
}

fn port(sim: &CompiledSim<'_>, name: &str) -> NetHandle {
    let path = format!("{}.{name}", sim.top_name());
    sim.net(&path).unwrap_or_else(|| panic!("no net {path}"))
}

fn get(sim: &mut CompiledSim<'_>, net: NetHandle) -> u64 {
    sim.get(net).to_u64().expect("two-state")
}

fn set(sim: &mut CompiledSim<'_>, net: NetHandle, width: u32, value: u64) {
    sim.set(net, Logic::from_u64(value, width));
}

fn reset(sim: &mut CompiledSim<'_>) {
    let rst = port(sim, "rst_n");
    set(sim, rst, 1, 0);
    sim.step();
    sim.step();
    set(sim, rst, 1, 1);
}

/// A small, reproducible generator, so a failure names a seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + i64::try_from(self.below(u64::try_from(hi - lo + 1).expect("range"))).expect("fits")
    }
}

// ---------------------------------------------------------------------------
// Tokens (docs/h264-decoder.md)
// ---------------------------------------------------------------------------

mod tok {
    pub(crate) const SPS_DIMS: u32 = 0x01;
    pub(crate) const SLICE_HDR: u32 = 0x02;
    pub(crate) const SLICE_QP: u32 = 0x03;
    pub(crate) const SLICE_REF: u32 = 0x05;
    pub(crate) const SLICE_FIRST: u32 = 0x06;
    pub(crate) const MB_START: u32 = 0x10;
    pub(crate) const MB_TYPE: u32 = 0x11;
    pub(crate) const MB_QP: u32 = 0x12;
    pub(crate) const MVD: u32 = 0x14;
    pub(crate) const SUB_MB_TYPE: u32 = 0x15;
    pub(crate) const MV: u32 = 0x16;
    pub(crate) const REF_IDX: u32 = 0x18;
    pub(crate) const MB_CBP: u32 = 0x19;
    pub(crate) const COEF_BLK: u32 = 0x20;
    pub(crate) const COEF: u32 = 0x21;
    pub(crate) const MB_END: u32 = 0x3F;
    pub(crate) const SLICE_END: u32 = 0x40;
    pub(crate) const PIC_END: u32 = 0x41;

    pub(crate) fn make(tag: u32, data: u32) -> u32 {
        (tag << 24) | (data & 0xFF_FFFF)
    }
    pub(crate) fn tag(t: u32) -> u32 {
        t >> 24
    }
    pub(crate) fn data(t: u32) -> u32 {
        t & 0xFF_FFFF
    }
}

/// luma4x4BlkIdx of the 4x4 block at raster position (column, row), 6-38.
fn blk_of(c: usize, r: usize) -> usize {
    8 * (r / 2) + 4 * (c / 2) + 2 * (r % 2) + (c % 2)
}

/// The partition (x, y, width, height), in 4x4 blocks, of mbPartIdx /
/// subMbPartIdx for a P macroblock kind (6.4.2.1, 6.4.2.2).
fn partition(kind: u32, sub: &[u32; 4], part: usize, s: usize) -> (usize, usize, usize, usize) {
    match kind {
        5 => (0, 2 * part, 4, 2),
        6 => (2 * part, 0, 2, 4),
        7 | 8 => {
            let (bx, by) = (2 * (part % 2), 2 * (part / 2));
            match sub[part] {
                0 => (bx, by, 2, 2),
                1 => (bx, by + s, 2, 1),
                2 => (bx + s, by, 1, 2),
                _ => (bx + s % 2, by + s / 2, 1, 1),
            }
        }
        _ => (0, 0, 4, 4),
    }
}

/// How many sub-macroblock partitions a P sub_mb_type has (Table 7-17).
fn sub_parts(t: u32) -> usize {
    match t {
        0 => 1,
        1 | 2 => 2,
        _ => 4,
    }
}

// ---------------------------------------------------------------------------
// The motion vector model, clause 8.4.1
// ---------------------------------------------------------------------------

mod mvmodel {
    use super::{blk_of, partition, tok};

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub(crate) struct Motion {
        pub(crate) r: i32,
        pub(crate) x: i32,
        pub(crate) y: i32,
    }

    pub(crate) const NONE: Motion = Motion { r: -1, x: 0, y: 0 };

    fn wrap16(v: i32) -> i32 {
        i32::from(v as i16)
    }

    fn median(a: i32, b: i32, c: i32) -> i32 {
        a.max(b).min(a.min(b).max(c))
    }

    /// The whole decoder state the derivation needs: every 4x4 block of
    /// the picture, and which slice each macroblock was decoded in.
    pub(crate) struct Model {
        w: usize,
        h: usize,
        blk: Vec<Motion>,
        slice_of: Vec<Option<u32>>,
        slice: u32,
        mbx: usize,
        mby: usize,
        kind: u32,
        sub: [u32; 4],
        refs: [i32; 4],
        decoded: [bool; 16],
        inter: bool,
        mvd_x: i32,
        /// Every inter macroblock's final vectors, by luma4x4BlkIdx, for
        /// the tests that want them rather than the tokens.
        pub(crate) out_mvs: Vec<(usize, usize, [Motion; 16])>,
    }

    impl Model {
        pub(crate) fn new() -> Model {
            Model {
                w: 0,
                h: 0,
                blk: Vec::new(),
                slice_of: Vec::new(),
                slice: 0,
                mbx: 0,
                mby: 0,
                kind: 0,
                sub: [0; 4],
                refs: [0; 4],
                decoded: [false; 16],
                inter: false,
                mvd_x: 0,
                out_mvs: Vec::new(),
            }
        }

        fn cur(&self) -> usize {
            self.mby * self.w + self.mbx
        }

        fn at(&self, x4: i64, y4: i64) -> usize {
            usize::try_from(y4).expect("y") * 4 * self.w + usize::try_from(x4).expect("x")
        }

        /// The neighbour at (xn, yn) in 4x4 blocks relative to the
        /// macroblock: availability and motion, 6.4.11.7 and 6.4.12 by
        /// address, not by position.
        fn neighbour(&self, xn: i64, yn: i64) -> (bool, Motion) {
            let gx = 4 * self.mbx as i64 + xn;
            let gy = 4 * self.mby as i64 + yn;
            if gx < 0 || gy < 0 || gx >= 4 * self.w as i64 {
                return (false, NONE);
            }
            let addr = (gy as usize / 4) * self.w + gx as usize / 4;
            let cur = self.cur();
            let avail = if addr > cur {
                false
            } else if addr == cur {
                self.decoded[(yn * 4 + xn) as usize]
            } else {
                self.slice_of[addr] == Some(self.slice)
            };
            if avail {
                (true, self.blk[self.at(gx, gy)])
            } else {
                (false, NONE)
            }
        }

        fn predict(&mut self, part: usize, s: usize, skip: bool, mvd: (i32, i32)) {
            let (x, y, w, h) = partition(self.kind, &self.sub, part, s);
            let (x, y, w) = (x as i64, y as i64, w as i64);
            let r = if skip { 0 } else { self.refs[part] };
            let (aa, a) = self.neighbour(x - 1, y);
            let (ab, b) = self.neighbour(x, y - 1);
            let (ac0, c0) = self.neighbour(x + w, y - 1);
            let (ac, c) = if ac0 {
                (ac0, c0)
            } else {
                self.neighbour(x - 1, y - 1)
            };
            let mv = if skip
                && (!aa
                    || !ab
                    || (a.r == 0 && a.x == 0 && a.y == 0)
                    || (b.r == 0 && b.x == 0 && b.y == 0))
            {
                (0, 0)
            } else {
                // 8-203 to 8-206, the directional rules.
                let up = self.kind == 5 && part == 0 && b.r == r;
                let left = (self.kind == 5 && part == 1 || self.kind == 6 && part == 0) && a.r == r;
                let right = self.kind == 6 && part == 1 && c.r == r;
                let p = if up {
                    (b.x, b.y)
                } else if left {
                    (a.x, a.y)
                } else if right {
                    (c.x, c.y)
                } else {
                    let (b, c) = if !ab && !ac && aa { (a, a) } else { (b, c) };
                    let hits = [a.r == r, b.r == r, c.r == r];
                    match hits {
                        [true, false, false] => (a.x, a.y),
                        [false, true, false] => (b.x, b.y),
                        [false, false, true] => (c.x, c.y),
                        _ => (median(a.x, b.x, c.x), median(a.y, b.y, c.y)),
                    }
                };
                (wrap16(p.0 + mvd.0), wrap16(p.1 + mvd.1))
            };
            let m = Motion {
                r,
                x: mv.0,
                y: mv.1,
            };
            for dy in 0..h {
                for dx in 0..w as usize {
                    let (cx, cy) = (x as usize + dx, y as usize + dy);
                    let i = self.at(
                        4 * self.mbx as i64 + cx as i64,
                        4 * self.mby as i64 + cy as i64,
                    );
                    self.blk[i] = m;
                    self.decoded[cy * 4 + cx] = true;
                }
            }
        }

        /// Feeds one token; returns what goes out for it, in order.
        pub(crate) fn token(&mut self, t: u32) -> Vec<u32> {
            let d = tok::data(t);
            let mut out = Vec::new();
            match tok::tag(t) {
                tok::SPS_DIMS => {
                    let (w, h) = ((d >> 12) as usize, (d & 0xFFF) as usize);
                    if (w, h) != (self.w, self.h) {
                        self.w = w;
                        self.h = h;
                        self.blk = vec![NONE; 16 * w * h];
                        self.slice_of = vec![None; w * h];
                    }
                }
                tok::SLICE_FIRST => {
                    if d == 0 {
                        self.slice_of.iter_mut().for_each(|s| *s = None);
                    }
                    self.slice += 1;
                }
                tok::MB_START => {
                    self.mbx = (d >> 12) as usize;
                    self.mby = (d & 0xFFF) as usize;
                    let cur = self.cur();
                    self.slice_of[cur] = Some(self.slice);
                }
                tok::MB_TYPE => {
                    self.kind = d >> 19;
                    self.decoded = [false; 16];
                    self.inter = (4..=9).contains(&self.kind);
                    for r in 0..4 {
                        for c in 0..4 {
                            let i = self.at(4 * self.mbx as i64 + c, 4 * self.mby as i64 + r);
                            self.blk[i] = NONE;
                        }
                    }
                    if self.kind == 9 {
                        out.push(t);
                        self.predict(0, 0, true, (0, 0));
                        return out;
                    }
                }
                tok::SUB_MB_TYPE => {
                    for i in 0..4 {
                        self.sub[i] = (d >> (12 - 4 * i)) & 0xF;
                    }
                }
                tok::REF_IDX => {
                    if d >> 23 == 0 {
                        self.refs[((d >> 21) & 3) as usize] = (d & 31) as i32;
                    }
                }
                tok::MVD => {
                    if d >> 23 == 0 {
                        let v = i32::from((d & 0xFFFF) as u16 as i16);
                        if (d >> 18) & 1 == 0 {
                            self.mvd_x = v;
                        } else {
                            let part = ((d >> 21) & 3) as usize;
                            let s = ((d >> 19) & 3) as usize;
                            self.predict(part, s, false, (self.mvd_x, v));
                        }
                    }
                    return out;
                }
                tok::MB_CBP if self.inter => {
                    let mut mvs = [NONE; 16];
                    for (b, slot) in mvs.iter_mut().enumerate() {
                        let c = (b & 1) | ((b >> 1) & 2);
                        let r = ((b >> 1) & 1) | ((b >> 2) & 2);
                        debug_assert_eq!(blk_of(c, r), b);
                        let m = self.blk[self.at(
                            4 * self.mbx as i64 + c as i64,
                            4 * self.mby as i64 + r as i64,
                        )];
                        *slot = m;
                        let bb = (b as u32) << 19;
                        out.push(tok::make(tok::MV, bb | (m.x as u32 & 0xFFFF)));
                        out.push(tok::make(
                            tok::MV,
                            bb | (1 << 18) | ((m.r as u32 & 31) << 13) | (m.y as u32 & 0x1FFF),
                        ));
                    }
                    self.out_mvs.push((self.mbx, self.mby, mvs));
                }
                _ => {}
            }
            out.push(t);
            out
        }
    }
}

// ---------------------------------------------------------------------------
// Random token streams
// ---------------------------------------------------------------------------

/// A picture of random macroblocks in random slices: P pictures with every
/// partition shape, refs up to `refs`, P_Skip, and intra macroblocks among
/// them, or an I picture.
fn random_picture(rng: &mut Rng, w: u32, h: u32, p: bool, refs: u32, out: &mut Vec<u32>) {
    use tok::make;
    let n = w * h;
    let mut addr = 0;
    while addr < n {
        // A slice of 1..n macroblocks, biased long.
        let len = if rng.chance(30) {
            1 + rng.below(u64::from(n)) as u32
        } else {
            n
        };
        let end = (addr + len).min(n);
        if addr == 0 {
            out.push(make(tok::SPS_DIMS, (w << 12) | h));
        }
        out.push(make(tok::SLICE_HDR, if p { 0 } else { 2 << 20 }));
        out.push(make(tok::SLICE_FIRST, addr));
        out.push(make(tok::SLICE_REF, (refs - 1) << 20));
        out.push(make(tok::SLICE_QP, 26 << 16));
        for a in addr..end {
            let (x, y) = (a % w, a / w);
            out.push(make(tok::MB_START, (x << 12) | y));
            let kind = if !p || rng.chance(10) {
                rng.below(3) as u32
            } else {
                4 + rng.below(6) as u32
            };
            out.push(make(tok::MB_TYPE, kind << 19));
            let mut sub = [0u32; 4];
            if kind == 7 || kind == 8 {
                for s in &mut sub {
                    *s = rng.below(4) as u32;
                }
                out.push(make(
                    tok::SUB_MB_TYPE,
                    (sub[0] << 12) | (sub[1] << 8) | (sub[2] << 4) | sub[3],
                ));
            }
            let parts = match kind {
                4 => 1,
                5 | 6 => 2,
                7 | 8 => 4,
                _ => 0,
            };
            for part in 0..parts {
                let r = if kind == 8 {
                    0
                } else {
                    rng.below(u64::from(refs)) as u32
                };
                out.push(make(tok::REF_IDX, (part << 21) | r));
            }
            for part in 0..parts {
                let subs = if kind >= 7 {
                    sub_parts(sub[part as usize])
                } else {
                    1
                };
                for s in 0..subs as u32 {
                    for comp in 0..2u32 {
                        let big = rng.chance(5);
                        let v = if big {
                            rng.range(-3000, 3000)
                        } else {
                            rng.range(-40, 40)
                        };
                        let v = if rng.chance(30) { 0 } else { v };
                        out.push(make(
                            tok::MVD,
                            (part << 21) | (s << 19) | (comp << 18) | (v as u32 & 0xFFFF),
                        ));
                    }
                }
            }
            out.push(make(tok::MB_CBP, rng.below(48) as u32));
            out.push(make(tok::MB_QP, 26));
            for _ in 0..rng.below(3) {
                out.push(make(tok::COEF_BLK, (2 << 20) | 3));
                out.push(make(tok::COEF, 0xFFFF));
            }
            out.push(make(tok::MB_END, 0));
        }
        out.push(make(tok::SLICE_END, 0));
        addr = end;
    }
    out.push(make(tok::PIC_END, 0));
}

// ---------------------------------------------------------------------------
// h264_mvpred
// ---------------------------------------------------------------------------

/// Runs a token stream through `h264_mvpred` with random gaps on the input
/// and random back-pressure on the output; returns the output and the
/// cycles it took.
fn run_mvpred(
    sim: &mut CompiledSim<'_>,
    input: &[u32],
    rng: &mut Rng,
    pressure: bool,
) -> (Vec<u32>, u64) {
    let in_tok = port(sim, "in_tok");
    let in_valid = port(sim, "in_valid");
    let in_ready = port(sim, "in_ready");
    let out_tok = port(sim, "out_tok");
    let out_valid = port(sim, "out_valid");
    let out_ready = port(sim, "out_ready");
    let mut out = Vec::new();
    let mut i = 0;
    let mut cycles = 0u64;
    let mut idle = 0;
    while i < input.len() || idle < 64 {
        let v = i < input.len() && (!pressure || rng.chance(80));
        set(sim, in_valid, 1, u64::from(v));
        set(sim, in_tok, 32, u64::from(input[i.min(input.len() - 1)]));
        let r = !pressure || rng.chance(70);
        set(sim, out_ready, 1, u64::from(r));
        let took = v && get(sim, in_ready) != 0;
        let gave = r && get(sim, out_valid) != 0;
        if gave {
            out.push(u32::try_from(get(sim, out_tok)).expect("32 bits"));
        }
        sim.step();
        cycles += 1;
        if took {
            i += 1;
        }
        if i >= input.len() {
            idle = if gave { 0 } else { idle + 1 };
        }
        if cycles >= 50 * input.len() as u64 + 1000 {
            let mut state = String::new();
            for n in [
                "state", "p_step", "w_r", "w_c", "gw", "gh", "kind", "decoded",
            ] {
                let h = port(sim, n);
                state.push_str(&format!(" {n}={}", get(sim, h)));
            }
            panic!(
                "h264_mvpred stalled at token {i} ({:08x}):{state}",
                input[i]
            );
        }
    }
    (out, cycles)
}

/// Where two token streams first part, with some context.
fn first_difference(got: &[u32], want: &[u32]) -> Option<String> {
    let at = got.iter().zip(want).position(|(a, b)| a != b);
    let at = match at {
        Some(a) => a,
        None if got.len() == want.len() => return None,
        None => got.len().min(want.len()),
    };
    let show = |s: &[u32]| {
        s[at.saturating_sub(6)..(at + 4).min(s.len())]
            .iter()
            .map(|t| format!("{t:08x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    Some(format!(
        "{} tokens out, {} expected; first difference at {at}\n got  {}\n want {}",
        got.len(),
        want.len(),
        show(got),
        show(want)
    ))
}

/// The model's output for a whole stream.
fn model_stream(input: &[u32]) -> Vec<u32> {
    let mut m = mvmodel::Model::new();
    input.iter().flat_map(|&t| m.token(t)).collect()
}

/// `h264_mvpred` against the model over random streams.
///
/// What this would catch: a wrong neighbour (A, B, C, D, or C's
/// replacement by D), a wrong availability at a picture edge, a slice
/// edge or inside the macroblock (a partition not yet decoded), the
/// directional rules, the median and its two special cases, P_Skip's zero
/// rule, the line buffer and the left column carried wrongly between
/// macroblocks, a token lost, duplicated or reordered under back-pressure,
/// and an MV token packed wrongly. What it would not: anything the model
/// and the hardware misread the same way.
#[test]
fn mvpred_matches_the_model_on_random_streams() {
    let d = design(&["h264_mvpred.v"], "h264_mvpred", &[("MAX_WIDTH_MBS", "8")]);
    for seed in 1..=6u64 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed);
        let (w, h) = (1 + rng.below(8) as u32, 1 + rng.below(5) as u32);
        let mut input = Vec::new();
        for pic in 0..4 {
            let p = pic != 0 && rng.chance(85);
            let refs = 1 + rng.below(4) as u32;
            random_picture(&mut rng, w, h, p, refs, &mut input);
        }
        let want = model_stream(&input);
        let mut sim = compiled(&d);
        reset(&mut sim);
        let (got, cyc) = run_mvpred(&mut sim, &input, &mut rng, seed % 2 == 0);
        // A stream with no vectors in it would pass trivially.
        let nonzero = got
            .iter()
            .filter(|t| tok::tag(**t) == tok::MV && (**t & 0x1FFF) != 0)
            .count();
        assert!(
            nonzero > 50 || w * h < 8,
            "seed {seed}: only {nonzero} nonzero vectors in {cyc} cycles"
        );
        if let Some(diff) = first_difference(&got, &want) {
            panic!("seed {seed} ({w}x{h} macroblocks): {diff}");
        }
    }
}
