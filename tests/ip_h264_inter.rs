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
            let head = mem.queue.front().copied().filter(|&(ready, _, _)| now >= ready);
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
        for pic in 0..2 {
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
                    let (refs, mvs) = random_mb_vectors(&mut rng, nframes as u32, seed == 3);
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
    }
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
fn dump_stream(d: &Dump, limit: usize) -> (Vec<u32>, Vec<u64>, Vec<u32>, usize) {
    let mut tokens = vec![tok::make(tok::SPS_DIMS, ((d.w as u32) << 12) | d.h as u32)];
    let mut maps = Vec::new();
    let mut want = Vec::new();
    let mut mbs = 0;
    for pic in &d.pics {
        if pic.mbs.is_empty() {
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

/// Every decoded picture of a dump, in its SDRAM slot.
fn dump_memory(d: &Dump, mem: &mut Sdram) {
    for pic in &d.pics {
        if let Some(f) = &pic.frame {
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
    let (tokens, maps, want, mbs) = dump_stream(&d, usize::MAX);
    assert!(mbs >= 4, "the dump has {mbs} inter macroblocks");
    let design = inter_design();
    let mut mem = Sdram::new(7, 6, 20);
    dump_memory(&d, &mut mem);
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
