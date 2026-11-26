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
                    for (i, s) in self.sub.iter_mut().enumerate() {
                        *s = (d >> (12 - 4 * i)) & 0xF;
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

// ---------------------------------------------------------------------------
// Reference pictures and the prediction model, clause 8.4.2.2
// ---------------------------------------------------------------------------

/// One decoded picture, 4:2:0.
#[derive(Clone)]
struct Frame {
    w: usize,
    h: usize,
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
}

impl Frame {
    fn random(rng: &mut Rng, w: usize, h: usize) -> Frame {
        // Smooth enough that the filters have something to do and noisy
        // enough that a wrong tap shows: a gradient plus noise, clipped.
        let mut plane = |pw: usize, ph: usize, seed: i64| -> Vec<u8> {
            let mut v = Vec::with_capacity(pw * ph);
            for y in 0..ph {
                for x in 0..pw {
                    let base = (x as i64 * 7 + y as i64 * 3 + seed) % 256;
                    let n = rng.range(-40, 40);
                    v.push((base + n).clamp(0, 255) as u8);
                }
            }
            v
        };
        Frame {
            w,
            h,
            y: plane(w, h, 0),
            cb: plane(w / 2, h / 2, 90),
            cr: plane(w / 2, h / 2, 170),
        }
    }

    fn plane(&self, p: usize) -> (&[u8], usize, usize) {
        match p {
            0 => (&self.y, self.w, self.h),
            1 => (&self.cb, self.w / 2, self.h / 2),
            _ => (&self.cr, self.w / 2, self.h / 2),
        }
    }
}

/// A sample with its coordinates clamped into the plane, 8-239 / 8-240.
fn at(p: &[u8], w: usize, h: usize, x: i64, y: i64) -> i64 {
    let x = x.clamp(0, w as i64 - 1) as usize;
    let y = y.clamp(0, h as i64 - 1) as usize;
    i64::from(p[y * w + x])
}

fn tap6(v: [i64; 6]) -> i64 {
    v[0] - 5 * v[1] + 20 * v[2] + 20 * v[3] - 5 * v[4] + v[5]
}

fn clip1(v: i64) -> i64 {
    v.clamp(0, 255)
}

/// One luma prediction sample, 8.4.2.2.1, at full-sample (xi, yi) and
/// quarter offset (xf, yf). j is computed both ways 8-245 and 8-246 allow,
/// and the two are asserted equal, since the Recommendation says they are.
fn luma_sample(f: &Frame, xi: i64, yi: i64, xf: i64, yf: i64) -> u8 {
    let (p, w, h) = f.plane(0);
    let s = |x: i64, y: i64| at(p, w, h, x, y);
    let b1 = |x: i64, y: i64| {
        tap6([
            s(x - 2, y),
            s(x - 1, y),
            s(x, y),
            s(x + 1, y),
            s(x + 2, y),
            s(x + 3, y),
        ])
    };
    let h1 = |x: i64, y: i64| {
        tap6([
            s(x, y - 2),
            s(x, y - 1),
            s(x, y),
            s(x, y + 1),
            s(x, y + 2),
            s(x, y + 3),
        ])
    };
    let (x, y) = (xi, yi);
    let g = s(x, y);
    let hh = s(x + 1, y);
    let m_ = s(x, y + 1);
    let b = clip1((b1(x, y) + 16) >> 5);
    let hv = clip1((h1(x, y) + 16) >> 5);
    let ss = clip1((b1(x, y + 1) + 16) >> 5);
    let m = clip1((h1(x + 1, y) + 16) >> 5);
    let j1 = tap6([
        b1(x, y - 2),
        b1(x, y - 1),
        b1(x, y),
        b1(x, y + 1),
        b1(x, y + 2),
        b1(x, y + 3),
    ]);
    let j1v = tap6([
        h1(x - 2, y),
        h1(x - 1, y),
        h1(x, y),
        h1(x + 1, y),
        h1(x + 2, y),
        h1(x + 3, y),
    ]);
    assert_eq!(j1, j1v, "8-245 and 8-246 disagree at ({x}, {y})");
    let j = clip1((j1 + 512) >> 10);
    let avg = |a: i64, b: i64| (a + b + 1) >> 1;
    // Table 8-12.
    let v = match (xf, yf) {
        (0, 0) => g,
        (0, 1) => avg(g, hv),
        (0, 2) => hv,
        (0, 3) => avg(m_, hv),
        (1, 0) => avg(g, b),
        (1, 1) => avg(b, hv),
        (1, 2) => avg(hv, j),
        (1, 3) => avg(hv, ss),
        (2, 0) => b,
        (2, 1) => avg(b, j),
        (2, 2) => j,
        (2, 3) => avg(j, ss),
        (3, 0) => avg(hh, b),
        (3, 1) => avg(b, m),
        (3, 2) => avg(j, m),
        _ => avg(m, ss),
    };
    v as u8
}

/// One chroma prediction sample, 8.4.2.2.2 (8-262 .. 8-270).
fn chroma_sample(f: &Frame, plane: usize, xi: i64, yi: i64, xf: i64, yf: i64) -> u8 {
    let (p, w, h) = f.plane(plane);
    let a = at(p, w, h, xi, yi);
    let b = at(p, w, h, xi + 1, yi);
    let c = at(p, w, h, xi, yi + 1);
    let d = at(p, w, h, xi + 1, yi + 1);
    (((8 - xf) * (8 - yf) * a + xf * (8 - yf) * b + (8 - xf) * yf * c + xf * yf * d + 32) >> 6)
        as u8
}

/// A block's vector as `h264_inter` sees it: frame index, mvx, mvy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Vec3 {
    frame: usize,
    x: i64,
    y: i64,
}

/// The prediction of one macroblock in the contract's 96 words, every
/// sample from its own 4x4 block's vector (so it is independent of how
/// the hardware groups blocks into partitions).
fn predict_mb(frames: &[Frame], mbx: usize, mby: usize, mv: &[Vec3; 16]) -> Vec<u32> {
    let mut y = [[0u8; 16]; 16];
    let mut cb = [[0u8; 8]; 8];
    let mut cr = [[0u8; 8]; 8];
    for (r, row) in y.iter_mut().enumerate() {
        for (c, px) in row.iter_mut().enumerate() {
            let v = mv[(r / 4) * 4 + c / 4];
            let xa = (16 * mbx + c) as i64;
            let ya = (16 * mby + r) as i64;
            *px = luma_sample(
                &frames[v.frame],
                xa + (v.x >> 2),
                ya + (v.y >> 2),
                v.x & 3,
                v.y & 3,
            );
        }
    }
    for r in 0..8 {
        for c in 0..8 {
            let v = mv[(r / 2) * 4 + c / 2];
            let xa = (8 * mbx + c) as i64;
            let ya = (8 * mby + r) as i64;
            let (xi, yi, xf, yf) = (xa + (v.x >> 3), ya + (v.y >> 3), v.x & 7, v.y & 7);
            cb[r][c] = chroma_sample(&frames[v.frame], 1, xi, yi, xf, yf);
            cr[r][c] = chroma_sample(&frames[v.frame], 2, xi, yi, xf, yf);
        }
    }
    let word = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let mut out = Vec::with_capacity(96);
    for row in &y {
        for wd in 0..4 {
            out.push(word(&row[4 * wd..]));
        }
    }
    for p in [&cb, &cr] {
        for row in p {
            for wd in 0..2 {
                out.push(word(&row[4 * wd..]));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SDRAM behind the burst port
// ---------------------------------------------------------------------------

/// `h264_frame_addr_stub`'s mapping: the word holding samples (x, x+1),
/// x even, of row y of a plane of frame `frame`.
fn stub_addr(frame: usize, plane: usize, x: usize, y: usize) -> usize {
    let f = frame & 7;
    if plane == 0 {
        (f << 21)
            | (((y >> 5) & 31) << 15)
            | (((x >> 5) & 63) << 9)
            | ((y & 31) << 4)
            | ((x & 31) >> 1)
    } else {
        let cr = plane >> 1;
        (f << 21)
            | (1 << 20)
            | (cr << 19)
            | (((y >> 5) & 15) << 14)
            | (((x >> 5) & 31) << 9)
            | ((y & 31) << 4)
            | ((x & 31) >> 1)
    }
}

/// A behavioural SDRAM: a sparse word store, commands accepted with random
/// stalls, read data back in order after a latency, a word a cycle with
/// random gaps. It checks the port's rules as it goes: no write, a length
/// of 1..16 words, and no burst crossing a 512-word row.
struct Sdram {
    words: std::collections::HashMap<usize, u16>,
    queue: std::collections::VecDeque<(u64, usize, usize)>, // (ready at, addr, words left)
    rng: Rng,
    latency: u64,
    stall: u64,
    bursts: u64,
    fetched: u64,
}

impl Sdram {
    fn new(seed: u64, latency: u64, stall: u64) -> Sdram {
        Sdram {
            words: std::collections::HashMap::new(),
            queue: std::collections::VecDeque::new(),
            rng: Rng(seed | 1),
            latency,
            stall,
            bursts: 0,
            fetched: 0,
        }
    }

    fn store(&mut self, slot: usize, f: &Frame) {
        for p in 0..3 {
            let (px, w, h) = f.plane(p);
            for y in 0..h {
                for x in (0..w).step_by(2) {
                    let v = u16::from(px[y * w + x]) | (u16::from(px[y * w + x + 1]) << 8);
                    self.words.insert(stub_addr(slot, p, x, y), v);
                }
            }
        }
    }
}

/// The ports of `h264_inter`, and a run of it against an `Sdram`.
struct Inter<'d> {
    sim: CompiledSim<'d>,
    tok: NetHandle,
    tok_valid: NetHandle,
    tok_ready: NetHandle,
    map_frames: NetHandle,
    map_valid: NetHandle,
    map_ready: NetHandle,
    cmd_valid: NetHandle,
    cmd_ready: NetHandle,
    cmd_we: NetHandle,
    cmd_addr: NetHandle,
    cmd_len: NetHandle,
    rd_valid: NetHandle,
    rd_data: NetHandle,
    rd_last: NetHandle,
    wr_ready: NetHandle,
    pred_data: NetHandle,
    pred_valid: NetHandle,
    pred_ready: NetHandle,
}

/// What a run measured.
#[derive(Default, Debug)]
struct Run {
    pred: Vec<u32>,
    cycles: u64,
    bursts: u64,
    words: u64,
}

impl<'d> Inter<'d> {
    fn new(design: &'d Design) -> Inter<'d> {
        let sim = compiled(design);
        let mut i = Inter {
            tok: port(&sim, "tok"),
            tok_valid: port(&sim, "tok_valid"),
            tok_ready: port(&sim, "tok_ready"),
            map_frames: port(&sim, "map_frames"),
            map_valid: port(&sim, "map_valid"),
            map_ready: port(&sim, "map_ready"),
            cmd_valid: port(&sim, "cmd_valid"),
            cmd_ready: port(&sim, "cmd_ready"),
            cmd_we: port(&sim, "cmd_we"),
            cmd_addr: port(&sim, "cmd_addr"),
            cmd_len: port(&sim, "cmd_len"),
            rd_valid: port(&sim, "rd_valid"),
            rd_data: port(&sim, "rd_data"),
            rd_last: port(&sim, "rd_last"),
            wr_ready: port(&sim, "wr_ready"),
            pred_data: port(&sim, "pred_data"),
            pred_valid: port(&sim, "pred_valid"),
            pred_ready: port(&sim, "pred_ready"),
            sim,
        };
        for n in [
            i.tok_valid,
            i.map_valid,
            i.cmd_ready,
            i.rd_valid,
            i.rd_last,
            i.wr_ready,
            i.pred_ready,
        ] {
            set(&mut i.sim, n, 1, 0);
        }
        set(&mut i.sim, i.tok, 32, 0);
        set(&mut i.sim, i.rd_data, 16, 0);
        set(&mut i.sim, i.map_frames, 64, 0);
        reset(&mut i.sim);
        i
    }

    /// Feeds `tokens`; at each slice header the next entry of `maps` is
    /// offered as the reference map. Runs until `expect` words are out.
    fn run(
        &mut self,
        tokens: &[u32],
        maps: &[u64],
        mem: &mut Sdram,
        expect: usize,
        pressure: u64,
    ) -> Run {
        let mut rng = Rng(0x5EED ^ pressure);
        let mut run = Run::default();
        let mut ti = 0;
        let mut mi = 0;
        let start_bursts = mem.bursts;
        let start_words = mem.fetched;
        let mut now = 0u64;
        while run.pred.len() < expect {
            let tv = ti < tokens.len() && !rng.chance(pressure);
            set(&mut self.sim, self.tok_valid, 1, u64::from(tv));
            set(
                &mut self.sim,
                self.tok,
                32,
                u64::from(tokens[ti.min(tokens.len() - 1)]),
            );
            let mv = mi < maps.len();
            set(&mut self.sim, self.map_valid, 1, u64::from(mv));
            set(
                &mut self.sim,
                self.map_frames,
                64,
                maps[mi.min(maps.len().saturating_sub(1))],
            );
            let cr = !rng.chance(mem.stall);
            set(&mut self.sim, self.cmd_ready, 1, u64::from(cr));
            let pr = !rng.chance(pressure);
            set(&mut self.sim, self.pred_ready, 1, u64::from(pr));
            // Read data: the head burst, once its latency is over.
            let mut rv = false;
            let mut rlast = false;
            let mut rdata = 0u16;
            let head = mem
                .queue
                .front()
                .copied()
                .filter(|&(ready, _, _)| now >= ready);
            let go = head.is_some() && !mem.rng.chance(mem.stall);
            if let (true, Some((ready, addr, left))) = (go, head) {
                rv = true;
                rdata = *mem.words.get(&addr).unwrap_or(&0xDEAD);
                rlast = left == 1;
                mem.fetched += 1;
                if left == 1 {
                    mem.queue.pop_front();
                } else {
                    mem.queue[0] = (ready, addr + 1, left - 1);
                }
            }
            set(&mut self.sim, self.rd_valid, 1, u64::from(rv));
            set(&mut self.sim, self.rd_data, 16, u64::from(rdata));
            set(&mut self.sim, self.rd_last, 1, u64::from(rlast));

            let took = tv && get(&mut self.sim, self.tok_ready) != 0;
            let mapped = mv && get(&mut self.sim, self.map_ready) != 0;
            if cr && get(&mut self.sim, self.cmd_valid) != 0 {
                assert_eq!(get(&mut self.sim, self.cmd_we), 0, "h264_inter wrote");
                let addr = get(&mut self.sim, self.cmd_addr) as usize;
                let len = get(&mut self.sim, self.cmd_len) as usize + 1;
                assert!(
                    addr % 512 + len <= 512,
                    "a burst at {addr:#x} of {len} crosses a row"
                );
                mem.queue.push_back((now + mem.latency, addr, len));
                mem.bursts += 1;
            }
            if pr && get(&mut self.sim, self.pred_valid) != 0 {
                run.pred.push(get(&mut self.sim, self.pred_data) as u32);
            }
            self.sim.step();
            now += 1;
            if took {
                ti += 1;
            }
            if mapped {
                mi += 1;
            }
            assert!(
                now < 200_000 + 4000 * expect as u64,
                "h264_inter stalled: {} of {expect} words out, token {ti} of {}",
                run.pred.len(),
                tokens.len()
            );
        }
        run.cycles = now;
        run.bursts = mem.bursts - start_bursts;
        run.words = mem.fetched - start_words;
        run
    }
}

/// The token stream of one inter macroblock, vectors given by raster block.
fn mb_tokens(mbx: usize, mby: usize, refs: &[u32; 16], mv: &[(i64, i64); 16], out: &mut Vec<u32>) {
    use tok::make;
    out.push(make(tok::MB_START, ((mbx as u32) << 12) | mby as u32));
    out.push(make(tok::MB_TYPE, 4 << 19));
    for b in 0..16usize {
        let c = (b & 1) | ((b >> 1) & 2);
        let r = ((b >> 1) & 1) | ((b >> 2) & 2);
        let i = r * 4 + c;
        let bb = (b as u32) << 19;
        out.push(make(tok::MV, bb | (mv[i].0 as u32 & 0xFFFF)));
        out.push(make(
            tok::MV,
            bb | (1 << 18) | (refs[i] << 13) | (mv[i].1 as u32 & 0x1FFF),
        ));
    }
    out.push(make(tok::MB_CBP, 0));
    out.push(make(tok::MB_QP, 26));
    out.push(make(tok::MB_END, 0));
}

/// Random vectors with a random partition structure: a vector for each
/// coded-shaped rectangle, so the merge sees every shape.
fn random_mb_vectors(rng: &mut Rng, nref: u32, wide: bool) -> ([u32; 16], [(i64, i64); 16]) {
    let mut refs = [0u32; 16];
    let mut mvs = [(0i64, 0i64); 16];
    let mut fill = |rng: &mut Rng, x: usize, y: usize, w: usize, h: usize| {
        let r = rng.below(u64::from(nref)) as u32;
        let big = if wide { 600 } else { 40 };
        let v = if rng.chance(15) {
            (rng.range(-big, big), rng.range(-big, big))
        } else {
            (rng.range(-24, 24), rng.range(-24, 24))
        };
        for yy in y..y + h {
            for xx in x..x + w {
                refs[yy * 4 + xx] = r;
                mvs[yy * 4 + xx] = v;
            }
        }
    };
    match rng.below(5) {
        0 => fill(rng, 0, 0, 4, 4),
        1 => {
            fill(rng, 0, 0, 4, 2);
            fill(rng, 0, 2, 4, 2);
        }
        2 => {
            fill(rng, 0, 0, 2, 4);
            fill(rng, 2, 0, 2, 4);
        }
        _ => {
            for q in 0..4 {
                let (qx, qy) = (2 * (q % 2), 2 * (q / 2));
                match rng.below(4) {
                    0 => fill(rng, qx, qy, 2, 2),
                    1 => {
                        fill(rng, qx, qy, 2, 1);
                        fill(rng, qx, qy + 1, 2, 1);
                    }
                    2 => {
                        fill(rng, qx, qy, 1, 2);
                        fill(rng, qx + 1, qy, 1, 2);
                    }
                    _ => {
                        for s in 0..4 {
                            fill(rng, qx + s % 2, qy + s / 2, 1, 1);
                        }
                    }
                }
            }
        }
    }
    (refs, mvs)
}

/// The reference map's wire value: frame index of refIdx i in bits 4i+3..4i.
fn map_word(map: &[usize]) -> u64 {
    map.iter()
        .enumerate()
        .fold(0, |acc, (i, &f)| acc | ((f as u64 & 15) << (4 * i)))
}

fn inter_design() -> Design {
    design(
        &["h264_frame_addr_stub.v", "h264_inter.v"],
        "h264_inter",
        &[],
    )
}

/// `h264_inter` against the model over random pictures and vectors.
///
/// What this would catch: any of the sixteen luma positions computed
/// wrongly (each macroblock has several vectors with random fractions, so
/// every position occurs many times), the j position's two-dimensional
/// filter and its rounding, chroma's eighth-sample blend, clamping at all
/// four picture edges and beyond them (some vectors are large enough to
/// leave the picture altogether), the reference map, the merge of equal
/// blocks into partitions, the cache returning a stale or wrong line, a
/// line fetched from the wrong address, and the output order. What it
/// would not: a misreading of 8.4.2.2 shared by the model and the
/// hardware; the model agent's dump is the independent check for that.
#[test]
fn inter_matches_the_model_on_random_vectors() {
    let d = inter_design();
    // Which luma fractions and chroma eighths occurred, and whether some
    // block read entirely outside the picture: a run that never did would
    // pass without having tested them.
    let mut luma_frac = [false; 16];
    let mut chroma_frac = [false; 64];
    let mut outside = 0;
    for seed in 1..=3u64 {
        let mut rng = Rng(0xC0FF_EE00 ^ seed);
        let (wm, hm) = (2 + rng.below(3) as usize, 2 + rng.below(2) as usize);
        let nframes = 3;
        let frames: Vec<Frame> = (0..nframes)
            .map(|_| Frame::random(&mut rng, 16 * wm, 16 * hm))
            .collect();
        let mut mem = Sdram::new(seed, 4 + seed, 10 * seed);
        for (i, f) in frames.iter().enumerate() {
            mem.store(i, f);
        }
        let mut tokens = Vec::new();
        let mut maps = Vec::new();
        let mut want = Vec::new();
        let mut fetch = FetchModel::new(true);
        // Two pictures of random vectors, then one of sixteen 4x4 blocks
        // a macroblock sweeping every chroma eighth (and so every luma
        // quarter) in turn.
        let mut sweep = 0i64;
        for pic in 0..3 {
            fetch.flush();
            // One slice per picture; the map is a permutation of the frames.
            let mut map: Vec<usize> = (0..nframes).collect();
            if pic == 1 {
                map.reverse();
            }
            maps.push(map_word(&map));
            if pic == 0 {
                tokens.push(tok::make(tok::SPS_DIMS, ((wm as u32) << 12) | hm as u32));
            }
            tokens.push(tok::make(tok::SLICE_HDR, 0));
            tokens.push(tok::make(tok::SLICE_FIRST, 0));
            for mby in 0..hm {
                for mbx in 0..wm {
                    if rng.chance(10) {
                        // An intra macroblock: no vectors, no prediction.
                        tokens.push(tok::make(tok::MB_START, ((mbx as u32) << 12) | mby as u32));
                        tokens.push(tok::make(tok::MB_TYPE, 0));
                        tokens.push(tok::make(tok::MB_CBP, 0));
                        tokens.push(tok::make(tok::MB_END, 0));
                        continue;
                    }
                    let (refs, mvs) = if pic < 2 {
                        random_mb_vectors(&mut rng, nframes as u32, seed == 3)
                    } else {
                        let refs: [u32; 16] =
                            std::array::from_fn(|_| rng.below(nframes as u64) as u32);
                        let mvs: [(i64, i64); 16] = std::array::from_fn(|_| {
                            sweep += 1;
                            (
                                8 * rng.range(-3, 3) + sweep % 8,
                                8 * rng.range(-3, 3) + (sweep / 8) % 8,
                            )
                        });
                        (refs, mvs)
                    };
                    mb_tokens(mbx, mby, &refs, &mvs, &mut tokens);
                    let mut v = [Vec3 {
                        frame: 0,
                        x: 0,
                        y: 0,
                    }; 16];
                    for (i, slot) in v.iter_mut().enumerate() {
                        *slot = Vec3 {
                            frame: map[refs[i] as usize],
                            x: mvs[i].0,
                            y: mvs[i].1,
                        };
                    }
                    want.extend(predict_mb(&frames, mbx, mby, &v));
                    fetch.mb((wm, hm), mbx, mby, &v);
                    for (i, b) in v.iter().enumerate() {
                        luma_frac[((b.x & 3) * 4 + (b.y & 3)) as usize] = true;
                        chroma_frac[((b.x & 7) * 8 + (b.y & 7)) as usize] = true;
                        let x = (16 * mbx + 4 * (i % 4)) as i64 + (b.x >> 2);
                        let y = (16 * mby + 4 * (i / 4)) as i64 + (b.y >> 2);
                        if x + 4 < 0 || y + 4 < 0 || x >= 16 * wm as i64 || y >= 16 * hm as i64 {
                            outside += 1;
                        }
                    }
                }
            }
            tokens.push(tok::make(tok::SLICE_END, 0));
            tokens.push(tok::make(tok::PIC_END, 0));
        }
        let mut dut = Inter::new(&d);
        let run = dut.run(&tokens, &maps, &mut mem, want.len(), 5 * seed);
        if let Some(diff) = first_difference(&run.pred, &want) {
            let at = run
                .pred
                .iter()
                .zip(&want)
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            panic!(
                "seed {seed}: macroblock {} word {}: {diff}",
                at / 96,
                at % 96
            );
        }
        // The fetch model is what measures whole streams, so it has to be
        // the hardware's own traffic, burst for burst.
        assert_eq!(
            (run.bursts, run.words),
            (fetch.bursts, fetch.words),
            "seed {seed}: the hardware's bursts and words against the fetch model's"
        );
    }
    assert!(
        luma_frac.iter().all(|&f| f),
        "a luma position never occurred: {luma_frac:?}"
    );
    assert!(
        chroma_frac.iter().all(|&f| f),
        "a chroma position never occurred"
    );
    assert!(outside > 0, "no block read from wholly outside the picture");
}

// ---------------------------------------------------------------------------
// The model agent's dumps (docs/h264-decoder.md, "Model dumps")
// ---------------------------------------------------------------------------

/// One inter macroblock of a dump: position, vectors by raster block
/// (refIdx, mvx, mvy), refIdx -> dumped picture, and the prediction.
struct DumpMb {
    x: usize,
    y: usize,
    mv: [(u32, i64, i64); 16],
    refpic: Vec<(u32, usize)>,
    pred: Vec<u32>,
}

/// One picture of a dump: its macroblocks and, after them, the decoded
/// picture later ones predict from.
struct DumpPic {
    index: usize,
    mbs: Vec<DumpMb>,
    frame: Option<Frame>,
}

struct Dump {
    w: usize,
    h: usize,
    pics: Vec<DumpPic>,
}

fn hex_rows(rows: &[&str]) -> (usize, usize, Vec<u8>) {
    let mut px = Vec::new();
    for r in rows {
        for i in (0..r.len()).step_by(2) {
            px.push(u8::from_str_radix(&r[i..i + 2], 16).expect("hex sample"));
        }
    }
    let w = rows.first().map_or(0, |r| r.len() / 2);
    (w, rows.len(), px)
}

/// Parses an `h264_inter_*.txt` dump, or `None` when it is not there.
fn read_dump(path: &Path) -> Option<Dump> {
    let text = fs::read_to_string(path).ok()?;
    let mut d = Dump {
        w: 0,
        h: 0,
        pics: Vec::new(),
    };
    let mut planes: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let Some(&key) = t.first() else { continue };
        match key {
            "stream" => {
                d.w = t[2].parse().expect("width");
                d.h = t[3].parse().expect("height");
            }
            "pic" => {
                d.pics.push(DumpPic {
                    index: t[1].parse().expect("index"),
                    mbs: Vec::new(),
                    frame: None,
                });
                planes.clear();
            }
            "mb" => {
                let pic = d.pics.last_mut().expect("a pic");
                pic.mbs.push(DumpMb {
                    x: t[2].parse().expect("x"),
                    y: t[3].parse().expect("y"),
                    mv: [(0, 0, 0); 16],
                    refpic: Vec::new(),
                    pred: Vec::new(),
                });
            }
            "mv" => {
                let mb = d
                    .pics
                    .last_mut()
                    .and_then(|p| p.mbs.last_mut())
                    .expect("an mb");
                for (i, e) in t[2..].iter().enumerate() {
                    let f: Vec<&str> = e.split(':').collect();
                    mb.mv[i] = (
                        f[0].parse().expect("ref"),
                        f[1].parse().expect("mvx"),
                        f[2].parse().expect("mvy"),
                    );
                }
            }
            "refpic" => {
                let mb = d
                    .pics
                    .last_mut()
                    .and_then(|p| p.mbs.last_mut())
                    .expect("an mb");
                for e in &t[2..] {
                    let (a, b) = e.split_once('=').expect("i=p");
                    mb.refpic
                        .push((a.parse().expect("ref"), b.parse().expect("pic")));
                }
            }
            "pred" => {
                let mb = d
                    .pics
                    .last_mut()
                    .and_then(|p| p.mbs.last_mut())
                    .expect("an mb");
                mb.pred = t[1..]
                    .iter()
                    .map(|w| u32::from_str_radix(w, 16).expect("hex word"))
                    .collect();
            }
            "frame" => {
                planes.push(hex_rows(&t[2..]));
                if planes.len() == 3 {
                    let pic = d.pics.last_mut().expect("a pic");
                    pic.frame = Some(Frame {
                        w: planes[0].0,
                        h: planes[0].1,
                        y: planes[0].2.clone(),
                        cb: planes[1].2.clone(),
                        cr: planes[2].2.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    Some(d)
}

/// The token stream, maps and expected prediction for a dump, every
/// picture in SDRAM slot `index % 8`. With `limit`, only the first so many
/// inter macroblocks of each picture are fed.
fn dump_stream(
    d: &Dump,
    limit: usize,
    only: Option<usize>,
) -> (Vec<u32>, Vec<u64>, Vec<u32>, usize) {
    let mut tokens = vec![tok::make(tok::SPS_DIMS, ((d.w as u32) << 12) | d.h as u32)];
    let mut maps = Vec::new();
    let mut want = Vec::new();
    let mut mbs = 0;
    for pic in &d.pics {
        if pic.mbs.is_empty() || only.is_some_and(|i| i != pic.index) {
            continue;
        }
        let mut map = vec![0usize; 16];
        for mb in &pic.mbs {
            for &(r, p) in &mb.refpic {
                map[r as usize] = p % 8;
            }
        }
        maps.push(map_word(&map));
        tokens.push(tok::make(tok::SLICE_HDR, 0));
        tokens.push(tok::make(tok::SLICE_FIRST, 0));
        for mb in pic.mbs.iter().take(limit) {
            let refs: [u32; 16] = std::array::from_fn(|i| mb.mv[i].0);
            let mvs: [(i64, i64); 16] = std::array::from_fn(|i| (mb.mv[i].1, mb.mv[i].2));
            mb_tokens(mb.x, mb.y, &refs, &mvs, &mut tokens);
            want.extend_from_slice(&mb.pred);
            mbs += 1;
        }
        tokens.push(tok::make(tok::SLICE_END, 0));
        tokens.push(tok::make(tok::PIC_END, 0));
    }
    (tokens, maps, want, mbs)
}

/// The decoded pictures of a dump in their SDRAM slots: all of them, or
/// only those picture `only` refers to (the stub has eight slots, and a
/// long dump would put two pictures in one).
fn dump_memory(d: &Dump, mem: &mut Sdram, only: Option<usize>) {
    let wanted: Vec<usize> = match only {
        None => d.pics.iter().map(|p| p.index).collect(),
        Some(i) => d
            .pics
            .iter()
            .filter(|p| p.index == i)
            .flat_map(|p| p.mbs.iter().flat_map(|m| m.refpic.iter().map(|e| e.1)))
            .collect(),
    };
    for pic in &d.pics {
        if let Some(f) = pic.frame.as_ref().filter(|_| wanted.contains(&pic.index)) {
            mem.store(pic.index % 8, f);
        }
    }
}

/// The dump's own pictures through the model, which must reproduce the
/// dump's prediction before the hardware is held to it: a test of this
/// file's reading of the dump format as much as of the model.
fn model_matches_dump(d: &Dump) {
    let frames: Vec<Option<&Frame>> = {
        let n = d.pics.iter().map(|p| p.index).max().unwrap_or(0) + 1;
        let mut v = vec![None; n];
        for p in &d.pics {
            v[p.index] = p.frame.as_ref();
        }
        v
    };
    for pic in &d.pics {
        for mb in &pic.mbs {
            let mut used: Vec<Frame> = Vec::new();
            let mut v = [Vec3 {
                frame: 0,
                x: 0,
                y: 0,
            }; 16];
            for (i, slot) in v.iter_mut().enumerate() {
                let (r, x, y) = mb.mv[i];
                let p = mb.refpic.iter().find(|e| e.0 == r).expect("refpic").1;
                used.push(frames[p].expect("the reference picture is dumped").clone());
                *slot = Vec3 { frame: i, x, y };
            }
            let got = predict_mb(&used, mb.x, mb.y, &v);
            assert_eq!(
                got, mb.pred,
                "the model and the dump disagree at picture {} mb ({}, {})",
                pic.index, mb.x, mb.y
            );
        }
    }
}

fn dump_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/ip")
        .join(name)
}

/// `h264_inter` against the independent model's dump of a real stream:
/// an IDR and two P pictures whose macroblocks are P_8x8 with
/// sub-partitions over two reference pictures (`p_32x32_dump`).
///
/// What this would catch: anything about 8.4.2.2 that this file's own
/// model gets wrong in the same way as the hardware, since the dump comes
/// from a decoder written separately and checked against ffmpeg's output.
/// What it would not: positions the stream happens not to use, which is
/// what the random test is for. Skips, saying so, when the dump is absent.
#[test]
fn inter_matches_the_model_dump() {
    let path = dump_path("h264_inter_p_32x32_dump.txt");
    let Some(d) = read_dump(&path) else {
        println!("skipped: {} is not there", path.display());
        return;
    };
    model_matches_dump(&d);
    let (tokens, maps, want, mbs) = dump_stream(&d, usize::MAX, None);
    assert!(mbs >= 4, "the dump has {mbs} inter macroblocks");
    let design = inter_design();
    let mut mem = Sdram::new(7, 6, 20);
    dump_memory(&d, &mut mem, None);
    let mut dut = Inter::new(&design);
    let run = dut.run(&tokens, &maps, &mut mem, want.len(), 10);
    if let Some(diff) = first_difference(&run.pred, &want) {
        let at = run
            .pred
            .iter()
            .zip(&want)
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!("macroblock {} word {}: {diff}", at / 96, at % 96);
    }
}

// ---------------------------------------------------------------------------
// The fetch model: what h264_inter asks the SDRAM for
// ---------------------------------------------------------------------------

/// The rectangles (x, y, w, h in 4x4 blocks) `h264_inter` merges a
/// macroblock's vectors into, in the order it processes them.
fn merge_partitions(mv: &[Vec3; 16]) -> Vec<(usize, usize, usize, usize)> {
    let eq = |ix: &[usize]| ix.iter().all(|&i| mv[i] == mv[ix[0]]);
    let q = |qx: usize, qy: usize, c: usize, r: usize| (qy + r) * 4 + qx + c;
    let uni: Vec<bool> = (0..4)
        .map(|n| {
            let (qx, qy) = (2 * (n % 2), 2 * (n / 2));
            eq(&[
                q(qx, qy, 0, 0),
                q(qx, qy, 1, 0),
                q(qx, qy, 0, 1),
                q(qx, qy, 1, 1),
            ])
        })
        .collect();
    let all = uni.iter().all(|&u| u);
    let (q0, q1, q2, q3) = (mv[0], mv[2], mv[8], mv[10]);
    if all && q0 == q1 && q0 == q2 && q1 == q3 {
        return vec![(0, 0, 4, 4)];
    }
    if all && q0 == q1 && q2 == q3 {
        return vec![(0, 0, 4, 2), (0, 2, 4, 2)];
    }
    if all && q0 == q2 && q1 == q3 {
        return vec![(0, 0, 2, 4), (2, 0, 2, 4)];
    }
    let mut out = Vec::new();
    for (n, &whole) in uni.iter().enumerate() {
        let (qx, qy) = (2 * (n % 2), 2 * (n / 2));
        let t = mv[q(qx, qy, 0, 0)] == mv[q(qx, qy, 1, 0)];
        let u = mv[q(qx, qy, 0, 1)] == mv[q(qx, qy, 1, 1)];
        let l = mv[q(qx, qy, 0, 0)] == mv[q(qx, qy, 0, 1)];
        let r = mv[q(qx, qy, 1, 0)] == mv[q(qx, qy, 1, 1)];
        if whole {
            out.push((qx, qy, 2, 2));
        } else if t && u {
            out.extend([(qx, qy, 2, 1), (qx, qy + 1, 2, 1)]);
        } else if l && r {
            out.extend([(qx, qy, 1, 2), (qx + 1, qy, 1, 2)]);
        } else {
            out.extend((0..4).map(|s| (qx + s % 2, qy + s / 2, 1, 1)));
        }
    }
    out
}

/// The clamped fetch window of one plane of one rectangle: rows and
/// columns, inclusive, as the generator computes them.
fn fetch_window(
    pic: (usize, usize),
    mbx: usize,
    mby: usize,
    part: (usize, usize, usize, usize),
    v: Vec3,
    plane: usize,
) -> ((i64, i64), (i64, i64)) {
    let (bx, by, bw, bh) = (part.0 as i64, part.1 as i64, part.2 as i64, part.3 as i64);
    let (wmax, hmax) = if plane == 0 {
        (16 * pic.0 as i64 - 1, 16 * pic.1 as i64 - 1)
    } else {
        (8 * pic.0 as i64 - 1, 8 * pic.1 as i64 - 1)
    };
    let (xi, yi, xf, yf, pw, ph, mx, my) = if plane == 0 {
        (
            16 * mbx as i64 + 4 * bx + (v.x >> 2),
            16 * mby as i64 + 4 * by + (v.y >> 2),
            v.x & 3,
            v.y & 3,
            4 * bw,
            4 * bh,
            2,
            3,
        )
    } else {
        (
            8 * mbx as i64 + 2 * bx + (v.x >> 3),
            8 * mby as i64 + 2 * by + (v.y >> 3),
            v.x & 7,
            v.y & 7,
            2 * bw,
            2 * bh,
            0,
            1,
        )
    };
    let c0 = if xf != 0 { xi - mx } else { xi };
    let c1 = xi + pw - 1 + if xf != 0 { my } else { 0 };
    let r0 = if yf != 0 { yi - mx } else { yi };
    let r1 = yi + ph - 1 + if yf != 0 { my } else { 0 };
    let cl = |v: i64, hi: i64| v.clamp(0, hi);
    ((cl(r0, hmax), cl(r1, hmax)), (cl(c0, wmax), cl(c1, wmax)))
}

/// The reference cache and the request gathering of `h264_inter`, step
/// for step, so that a whole 720p stream can be measured without
/// simulating it. The random test holds the hardware's own counts to it.
struct FetchModel {
    cache: bool,
    tags: Vec<(u16, u8)>,
    bursts: u64,
    words: u64,
    lookups: u64,
}

impl FetchModel {
    fn new(cache: bool) -> FetchModel {
        FetchModel {
            cache,
            tags: vec![(0, 0); 3072],
            bursts: 0,
            words: 0,
            lookups: 0,
        }
    }

    fn flush(&mut self) {
        self.tags.iter_mut().for_each(|t| *t = (0, 0));
    }

    fn index(plane: usize, frame: usize, y: usize, xl: usize) -> (usize, u16) {
        let (baddr, tag) = if plane == 0 {
            (
                (((frame & 3) << 8) | ((y & 31) << 3) | ((xl >> 1) & 7)),
                ((frame >> 2) << 10) | (((y >> 5) & 63) << 4) | ((xl >> 4) & 15),
            )
        } else {
            (
                1024 | ((plane >> 1) << 8) | ((frame & 3) << 6) | ((y & 15) << 2) | ((xl >> 1) & 3),
                ((frame >> 2) << 10) | (((y >> 4) & 63) << 4) | ((xl >> 3) & 15),
            )
        };
        ((baddr << 1) | (xl & 1), tag as u16)
    }

    fn task(&mut self, plane: usize, frame: usize, rows: (i64, i64), cols: (i64, i64)) {
        let (w0, w1) = (cols.0 as usize / 2, cols.1 as usize / 2);
        let mut pending: Option<(usize, usize, usize)> = None; // (row, first word, words)
        for y in rows.0 as usize..=rows.1 as usize {
            for xl in cols.0 as usize / 8..=cols.1 as usize / 8 {
                self.lookups += 1;
                let need = (0..4)
                    .filter(|&j| (w0..=w1).contains(&(4 * xl + j)))
                    .fold(0u8, |m, j| m | (1 << j));
                let (idx, tag) = FetchModel::index(plane, frame, y, xl);
                let (t, m) = self.tags[idx];
                let have = if self.cache && t == tag && m != 0 {
                    m
                } else {
                    0
                };
                let lack = need & !have;
                if lack == 0 {
                    continue;
                }
                self.tags[idx] = (tag, have | need);
                let first = lack.trailing_zeros() as usize;
                let last = 7 - lack.leading_zeros() as usize;
                let (runw, n) = (4 * xl + first, last - first + 1);
                match pending {
                    Some((py, pw, pn)) if py == y && runw == pw + pn && runw % 16 != 0 => {
                        pending = Some((py, pw, pn + n));
                    }
                    _ => {
                        if let Some((_, _, pn)) = pending {
                            self.bursts += 1;
                            self.words += pn as u64;
                        }
                        pending = Some((y, runw, n));
                    }
                }
            }
        }
        if let Some((_, _, pn)) = pending {
            self.bursts += 1;
            self.words += pn as u64;
        }
    }

    /// One inter macroblock.
    fn mb(&mut self, pic: (usize, usize), mbx: usize, mby: usize, mv: &[Vec3; 16]) {
        for part in merge_partitions(mv) {
            let v = mv[part.1 * 4 + part.0];
            for plane in 0..3 {
                let (rows, cols) = fetch_window(pic, mbx, mby, part, v, plane);
                self.task(plane, v.frame, rows, cols);
            }
        }
    }
}

/// The vectors of a dump macroblock, as `h264_inter` sees them.
fn dump_vectors(mb: &DumpMb) -> [Vec3; 16] {
    std::array::from_fn(|i| {
        let (r, x, y) = mb.mv[i];
        let p = mb.refpic.iter().find(|e| e.0 == r).map_or(0, |e| e.1);
        Vec3 { frame: p % 8, x, y }
    })
}

/// What `h264_inter` fetches for a dump, by the fetch model: (words,
/// bursts, lookups) with the cache and without it, and the number of
/// macroblocks of the P pictures, inter or not.
fn dump_traffic(d: &Dump) -> ([u64; 3], [u64; 3], u64) {
    let mut with = FetchModel::new(true);
    let mut without = FetchModel::new(false);
    let mut mbs = 0;
    for pic in d.pics.iter().filter(|p| !p.mbs.is_empty()) {
        with.flush();
        mbs += (d.w * d.h) as u64;
        for mb in &pic.mbs {
            let v = dump_vectors(mb);
            with.mb((d.w, d.h), mb.x, mb.y, &v);
            without.mb((d.w, d.h), mb.x, mb.y, &v);
        }
    }
    (
        [with.words, with.bursts, with.lookups],
        [without.words, without.bursts, without.lookups],
        mbs,
    )
}

/// The traffic and cycles of `h264_inter` on real 720p streams, against
/// the budget of an SDR SDRAM. Prints a table; asserts nothing about the
/// numbers, which belong in `README.md`.
///
/// `RETICLE_H264_DUMPS` names a directory of `h264_inter_*.txt` dumps
/// written by `tools/h264_model.py dump` (the streams are too large to
/// commit). The fetch model gives the bytes for every picture; the
/// hardware is simulated over the first `RETICLE_H264_MBS` inter
/// macroblocks (default 400) of each P picture for the cycles, with an
/// SDRAM that returns a word a cycle after 12 cycles of latency, and with
/// one that is busy half the time.
#[test]
#[ignore = "needs RETICLE_H264_DUMPS and minutes"]
fn inter_traffic_on_real_streams() {
    let Some(dir) = std::env::var_os("RETICLE_H264_DUMPS") else {
        println!("skipped: RETICLE_H264_DUMPS is not set");
        return;
    };
    let limit: usize = std::env::var("RETICLE_H264_MBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let mut names: Vec<_> = fs::read_dir(&dir)
        .expect("the dump directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("h264_inter_"))
        })
        .collect();
    names.sort();
    let with_cache = inter_design();
    let without_cache = design(
        &["h264_frame_addr_stub.v", "h264_inter.v"],
        "h264_inter",
        &[("CACHE", "0")],
    );
    for path in names {
        let Some(d) = read_dump(&path) else { continue };
        let (w, wo, mbs) = dump_traffic(&d);
        let per = |x: u64| x as f64 / mbs as f64;
        println!(
            "{}: {} P pictures of {}x{} macroblocks",
            path.display(),
            d.pics.iter().filter(|p| !p.mbs.is_empty()).count(),
            d.w,
            d.h
        );
        println!("  fetch model, every macroblock of every P picture:");
        println!(
            "    with the cache     {:7.1} bytes/MB  {:5.1} bursts/MB  {:5.1} lookups/MB",
            2.0 * per(w[0]),
            per(w[1]),
            per(w[2])
        );
        println!(
            "    without            {:7.1} bytes/MB  {:5.1} bursts/MB",
            2.0 * per(wo[0]),
            per(wo[1])
        );
        let pics: Vec<usize> = d
            .pics
            .iter()
            .filter(|p| !p.mbs.is_empty())
            .map(|p| p.index)
            .collect();
        for (label, des) in [("with the cache", &with_cache), ("without", &without_cache)] {
            for (sdram, latency, stall) in
                [("idle SDRAM", 12, 0), ("SDRAM busy half the time", 12, 50)]
            {
                let (mut cycles, mut words, mut n) = (0u64, 0u64, 0usize);
                let t0 = std::time::Instant::now();
                for &pic in &pics {
                    let (tokens, maps, want, k) = dump_stream(&d, limit, Some(pic));
                    let mut mem = Sdram::new(11, latency, stall);
                    dump_memory(&d, &mut mem, Some(pic));
                    let mut dut = Inter::new(des);
                    let run = dut.run(&tokens, &maps, &mut mem, want.len(), 0);
                    if let Some(diff) = first_difference(&run.pred, &want) {
                        panic!("{} picture {pic}: {diff}", path.display());
                    }
                    cycles += run.cycles;
                    words += run.words;
                    n += k;
                }
                let secs = t0.elapsed().as_secs_f64();
                println!(
                    "  hardware, {n} inter MBs, {label}, {sdram}: {:6.1} cycles/MB, {:6.1} bytes/MB fetched ({:.0} k cycles/s simulated)",
                    cycles as f64 / n as f64,
                    2.0 * words as f64 / n as f64,
                    cycles as f64 / secs / 1000.0
                );
            }
        }
    }
}

/// The `tok` words of an `h264_tokens_*.txt` dump.
fn read_tokens(path: &Path) -> Option<Vec<u32>> {
    let text = fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter_map(|l| l.strip_prefix("tok "))
            .map(|l| u32::from_str_radix(&l[..8], 16).expect("a hex token"))
            .collect(),
    )
}

/// Runs a dump's token stream through `h264_mvpred` and holds its output to
/// the dump: every token but `MVD` through unchanged and in order, and for
/// every inter macroblock the 32 `MV` tokens carrying exactly the vectors
/// and reference indices the dump's `mv l0` line gives. Returns how many
/// macroblocks were compared.
fn check_mvpred_against_dump(tokens: &[u32], inter: &Dump, pressure: bool) -> usize {
    let d = design(&["h264_mvpred.v"], "h264_mvpred", &[]);
    let mut sim = compiled(&d);
    reset(&mut sim);
    let mut rng = Rng(0xD0D0);
    let (got, _) = run_mvpred(&mut sim, tokens, &mut rng, pressure);
    // Everything that is not a vector, against everything that is not a
    // difference.
    let rest: Vec<u32> = got
        .iter()
        .copied()
        .filter(|t| tok::tag(*t) != tok::MV)
        .collect();
    let want: Vec<u32> = tokens
        .iter()
        .copied()
        .filter(|t| tok::tag(*t) != tok::MVD)
        .collect();
    if let Some(diff) = first_difference(&rest, &want) {
        panic!("h264_mvpred changed the stream: {diff}");
    }
    // The vectors, macroblock by macroblock.
    let mut mbs = inter.pics.iter().flat_map(|p| p.mbs.iter());
    let mut pos = (0usize, 0usize);
    let mut cur: Option<[(u32, i64, i64); 16]> = None;
    let mut n = 0;
    for &t in &got {
        let d = tok::data(t);
        match tok::tag(t) {
            tok::MB_START => pos = ((d >> 12) as usize, (d & 0xFFF) as usize),
            tok::MV => {
                let blk = ((d >> 19) & 15) as usize;
                let raster = (((blk >> 3) & 1) * 2 + ((blk >> 1) & 1)) * 4
                    + ((blk >> 2) & 1) * 2
                    + (blk & 1);
                let e = cur.get_or_insert([(0, 0, 0); 16]);
                if (d >> 18) & 1 == 0 {
                    e[raster].1 = i64::from((d & 0xFFFF) as u16 as i16);
                } else {
                    e[raster].0 = (d >> 13) & 31;
                    e[raster].2 = i64::from((((d & 0x1FFF) << 3) as u16 as i16) >> 3);
                }
            }
            tok::MB_CBP => {
                if let Some(e) = cur.take() {
                    let mb = mbs.next().expect("the dump has this inter macroblock");
                    assert_eq!((mb.x, mb.y), pos, "macroblock order");
                    assert_eq!(e, mb.mv, "the vectors of macroblock ({}, {})", pos.0, pos.1);
                    n += 1;
                }
            }
            _ => {}
        }
    }
    assert!(
        mbs.next().is_none(),
        "the dump has inter macroblocks h264_mvpred gave no vectors"
    );
    n
}

/// `h264_mvpred` against the model agent's dump of a real stream: two P
/// pictures of P_8x8 macroblocks with sub-partitions over two references,
/// so median prediction, the C-to-D replacement inside a macroblock and
/// the picture edges all occur.
///
/// What this would catch: a misreading of 8.4.1 that this file's own
/// model shares with the hardware, since the dump comes from a separately
/// written decoder checked against ffmpeg's pictures. What it would not:
/// shapes the stream does not use. Skips, saying so, when either file is
/// absent.
#[test]
fn mvpred_matches_the_model_dump() {
    let tpath = dump_path("h264_tokens_p_32x32_dump.txt");
    let ipath = dump_path("h264_inter_p_32x32_dump.txt");
    let (Some(tokens), Some(inter)) = (read_tokens(&tpath), read_dump(&ipath)) else {
        println!(
            "skipped: {} or {} is not there",
            tpath.display(),
            ipath.display()
        );
        return;
    };
    if !tokens.iter().any(|t| tok::tag(*t) == tok::REF_IDX) {
        println!("skipped: {} predates the REF_IDX token", tpath.display());
        return;
    }
    let n = check_mvpred_against_dump(&tokens, &inter, true);
    assert!(n >= 4, "only {n} inter macroblocks compared");
}

/// The same over every pair of `h264_tokens_*` / `h264_inter_*` dumps in
/// `RETICLE_H264_DUMPS`, for the 720p streams too large to commit.
#[test]
#[ignore = "needs RETICLE_H264_DUMPS"]
fn mvpred_matches_large_dumps() {
    let Some(dir) = std::env::var_os("RETICLE_H264_DUMPS") else {
        println!("skipped: RETICLE_H264_DUMPS is not set");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut names: Vec<String> = fs::read_dir(&dir)
        .expect("the dump directory")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_prefix("h264_tokens_")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    for name in names {
        let (Some(tokens), Some(inter)) = (
            read_tokens(&dir.join(format!("h264_tokens_{name}"))),
            read_dump(&dir.join(format!("h264_inter_{name}"))),
        ) else {
            continue;
        };
        let n = check_mvpred_against_dump(&tokens, &inter, false);
        println!("{name}: {n} inter macroblocks, every vector as the dump has it");
    }
}

/// The input ports `port` depends on combinationally, after synthesis: a
/// walk back over the timing graph from the output port to the start
/// points it reaches, as `tests/ip_library.rs` does for the library's
/// other streams.
#[cfg(all(feature = "synth", feature = "timing"))]
fn combinational_inputs(
    files: &[&str],
    top: &str,
    port: &str,
) -> std::collections::BTreeSet<String> {
    use reticle::ir::hier::FlattenOptions;
    use reticle::synth::{SynthOptions, run as synth_run};
    use reticle::timing::graph::{PinDirection, PointKind, TimingGraph, flatten_for_timing};

    let mut d = design(files, top, &[]);
    let id = d.top.expect("a top");
    d.flatten(id, &FlattenOptions::default())
        .unwrap_or_else(|_| panic!("{top} does not flatten"));
    d.remove_unused_modules(id);
    let id = d.top.expect("the top survives");
    let mut diags = Diagnostics::new();
    synth_run(&mut d, &SynthOptions::default(), &mut diags);
    assert!(!diags.has_errors(), "{top} does not synthesise");
    let module = flatten_for_timing(&d, id).expect("a flat module");
    let graph = TimingGraph::build(&module);
    let start = graph
        .pins
        .iter()
        .find(|p| p.port == port && p.cell_type == "port" && p.direction == PinDirection::Input)
        .unwrap_or_else(|| panic!("{top} has no output port `{port}`"))
        .name
        .clone();
    let start = graph.pin_by_name(&start).expect("the pin names itself");
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = vec![start];
    let mut inputs = std::collections::BTreeSet::new();
    while let Some(pin) = queue.pop() {
        if !seen.insert(pin.raw()) {
            continue;
        }
        if pin != start && graph.is_start(pin) {
            let kind = graph
                .start_points
                .iter()
                .find(|s| s.pin == pin)
                .map(|s| s.kind);
            if kind == Some(PointKind::Port) {
                inputs.insert(graph.pin(pin).owner.clone());
            }
            continue;
        }
        for arc in graph.fanin(pin) {
            queue.push(arc.from);
        }
    }
    inputs
}

/// Every handshake output of both blocks is a function of registers.
///
/// `h264_mvpred` sits in the middle of a chain of streams, and a `ready`
/// that went straight through it would join the timing of `h264_recon`'s
/// input to `h264_parse`'s output; `h264_inter`'s `tok_ready` is half of
/// the fork in front of `h264_recon`, the same way. What this would catch:
/// `in_ready` written as `!out_valid || out_ready` (the first version of
/// `h264_mvpred` did exactly that, and its output is a two-entry queue
/// now because of it), or a `cmd_valid` that looks at `cmd_ready`. What it
/// would not: a slow registered path, which is the routed timing's
/// business.
#[cfg(all(feature = "synth", feature = "timing"))]
#[test]
fn handshakes_are_functions_of_registers() {
    for port in ["in_ready", "out_valid"] {
        let inputs = combinational_inputs(&["h264_mvpred.v"], "h264_mvpred", port);
        assert!(
            inputs.is_empty(),
            "h264_mvpred: `{port}` depends combinationally on {inputs:?}"
        );
    }
    for port in ["tok_ready", "map_ready", "cmd_valid", "pred_valid"] {
        let inputs = combinational_inputs(
            &["h264_frame_addr_stub.v", "h264_inter.v"],
            "h264_inter",
            port,
        );
        assert!(
            inputs.is_empty(),
            "h264_inter: `{port}` depends combinationally on {inputs:?}"
        );
    }
}
