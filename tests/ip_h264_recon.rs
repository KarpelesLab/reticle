//! `ip/video/h264_recon`: the H.264 reconstruction back end, simulated.
//!
//! Every block is checked against a model in this file written from the
//! equations of ITU-T H.264 (V16, 06/2026) — never from another
//! decoder's source. The model is deliberately the plainest reading of
//! each clause: a two-dimensional array where the Recommendation has
//! one, a loop where it has a sum, no tables shared with the HDL.
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `dbf_tables_match_tables_8_16_and_8_17` | all 52 indices of α′, β′ and every bS of t′C0 |
//! | `dbf_filter_matches_8_7_2_3_and_8_7_2_4` | the edge filter, luma and chroma, every bS, against the equations |

#![cfg(all(feature = "verilog", feature = "sim"))]

use std::fs;
use std::path::Path;

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::logic::{Bit, Logic};
use reticle::sim::{NetHandle, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

// ---------------------------------------------------------------------------
// Elaboration and simulation helpers
// ---------------------------------------------------------------------------

/// The package's directory.
const PKG: &str = "ip/video/h264_recon";

/// Reads one file of the repository.
fn read_repo(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// Every source the package's manifest lists, in its order.
fn package_sources() -> Vec<String> {
    let manifest = read_repo(&format!("{PKG}/reticle.ip"));
    manifest
        .lines()
        .filter_map(|l| l.strip_prefix("source "))
        .map(|p| format!("{PKG}/{}", p.trim()))
        .collect()
}

/// Elaborates the package with `top` as the top.
fn design(top: &str) -> Design {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for path in package_sources() {
        let text = read_repo(&path);
        let id = map.add(path.clone(), &text).expect("source fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    assert!(
        !diags.has_errors(),
        "h264_recon does not parse:\n{}",
        diags.render(&map)
    );
    let options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &options, &mut diags);
    assert!(
        !diags.has_errors(),
        "{top} does not elaborate:\n{}",
        diags.render(&map)
    );
    design.unwrap_or_else(|| panic!("{top} produced no design"))
}

fn word(width: u32, value: u64) -> Logic {
    Logic::from_u64(value, width)
}

/// A value wider than 64 bits.
fn logic_u128(width: u32, value: u128) -> Logic {
    let bits: Vec<Bit> = (0..width)
        .map(|i| if value >> i & 1 == 1 { Bit::One } else { Bit::Zero })
        .collect();
    Logic::from_bits(&bits)
}

/// A net of the top instance.
fn top_net(sim: &Simulator<'_>, name: &str) -> NetHandle {
    let path = format!("{}.{}", sim.top_name(), name);
    sim.net(&path)
        .unwrap_or_else(|| panic!("no net `{path}` in the simulation"))
}

fn get_u64(sim: &Simulator<'_>, handle: NetHandle) -> u64 {
    sim.get(handle)
        .to_u64()
        .unwrap_or_else(|| panic!("net holds x or z: {}", sim.get(handle)))
}

/// A small, deterministic generator: the tests must not depend on a
/// crate, and must repeat.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + i32::try_from(self.below(u64::try_from(hi - lo + 1).expect("a span"))).expect("fits")
    }
}

// ---------------------------------------------------------------------------
// The model: §8.7, the deblocking filter
// ---------------------------------------------------------------------------

/// Table 8-16, α′ by indexA.
const ALPHA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 17, 20, 22,
    25, 28, 32, 36, 40, 45, 50, 56, 63, 71, 80, 90, 101, 113, 127, 144, 162, 182, 203, 226, 255,
    255,
];

/// Table 8-16, β′ by indexB.
const BETA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18,
];

/// Table 8-17, t′C0 by bS (1..3) and indexA.
const TC0: [[u8; 52]; 3] = [
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        2, 2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 7, 8, 8, 10, 11, 12, 13, 15, 17,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2,
        3, 3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 23, 25,
    ],
];

fn clip3(lo: i32, hi: i32, v: i32) -> i32 {
    v.clamp(lo, hi)
}

fn clip1(v: i32) -> i32 {
    v.clamp(0, 255)
}

/// §8.7.2.2 to §8.7.2.4 for one line `s = [p3, p2, p1, p0, q0, q1, q2, q3]`,
/// with the thresholds already looked up. Returns the line after
/// filtering.
fn filter_line(s: [i32; 8], bs: u32, chroma: bool, alpha: i32, beta: i32, tc0: i32) -> [i32; 8] {
    let [p3, p2, p1, p0, q0, q1, q2, q3] = s;
    // 8-460.
    let filter = bs != 0 && (p0 - q0).abs() < alpha && (p1 - p0).abs() < beta && (q1 - q0).abs() < beta;
    if !filter {
        return s;
    }
    let ap = (p2 - p0).abs();
    let aq = (q2 - q0).abs();
    let mut out = s;
    if bs < 4 {
        // 8-465, 8-466.
        let tc = if chroma {
            tc0 + 1
        } else {
            tc0 + i32::from(ap < beta) + i32::from(aq < beta)
        };
        // 8-467 to 8-469.
        let delta = clip3(-tc, tc, (((q0 - p0) << 2) + (p1 - q1) + 4) >> 3);
        out[3] = clip1(p0 + delta);
        out[4] = clip1(q0 - delta);
        // 8-470 to 8-473.
        if !chroma && ap < beta {
            out[2] = p1 + clip3(-tc0, tc0, (p2 + ((p0 + q0 + 1) >> 1) - (p1 << 1)) >> 1);
        }
        if !chroma && aq < beta {
            out[5] = q1 + clip3(-tc0, tc0, (q2 + ((p0 + q0 + 1) >> 1) - (q1 << 1)) >> 1);
        }
    } else {
        // 8-476 to 8-482.
        if !chroma && ap < beta && (p0 - q0).abs() < ((alpha >> 2) + 2) {
            out[3] = (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3;
            out[2] = (p2 + p1 + p0 + q0 + 2) >> 2;
            out[1] = (2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3;
        } else {
            out[3] = (2 * p1 + p0 + q1 + 2) >> 2;
        }
        // 8-483 to 8-489.
        if !chroma && aq < beta && (p0 - q0).abs() < ((alpha >> 2) + 2) {
            out[4] = (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3;
            out[5] = (p0 + q0 + q1 + q2 + 2) >> 2;
            out[6] = (2 * q3 + 3 * q2 + q1 + q0 + p0 + 4) >> 3;
        } else {
            out[4] = (2 * q1 + q0 + p1 + 2) >> 2;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The deblocking filter's leaves
// ---------------------------------------------------------------------------

/// The three tables, every index and every bS.
///
/// Would catch: any entry typed wrongly in the HDL, or a bS routed to
/// the wrong row of Table 8-17. Would not catch an entry this file and
/// the HDL both have wrong — the two were typed from the same PDF, which
/// is why the end-to-end tests against ffmpeg's pictures matter more.
#[test]
fn dbf_tables_match_tables_8_16_and_8_17() {
    let design = design("h264_dbf_tables");
    let mut sim = Simulator::new(&design, SimOptions::default()).expect("simulates");
    let ia = top_net(&sim, "index_a");
    let ib = top_net(&sim, "index_b");
    let bs = top_net(&sim, "bs");
    let alpha = top_net(&sim, "alpha");
    let beta = top_net(&sim, "beta");
    let tc0 = top_net(&sim, "tc0");
    for index in 0..52u64 {
        for b in 0..5u64 {
            sim.set(ia, word(6, index));
            sim.set(ib, word(6, 51 - index));
            sim.set(bs, word(3, b));
            sim.run_for(1);
            let i = usize::try_from(index).expect("small");
            assert_eq!(get_u64(&sim, alpha), u64::from(ALPHA[i]), "alpha[{index}]");
            assert_eq!(get_u64(&sim, beta), u64::from(BETA[51 - i]), "beta[{}]", 51 - index);
            let want = match b {
                1..=3 => TC0[usize::try_from(b - 1).expect("small")][i],
                _ => 0,
            };
            assert_eq!(get_u64(&sim, tc0), u64::from(want), "tc0[{b}][{index}]");
        }
    }
}

/// The edge filter against 8-460 to 8-489, luma and chroma, bS 0 to 4.
///
/// The lines are drawn so that every branch is taken often: each starts
/// from a flat level and adds small steps, a real edge, or noise, and
/// the thresholds come from a random indexA and indexB so α and β range
/// over the whole table.
///
/// Would catch: a wrong rounding constant, a shift that is logical where
/// it must be arithmetic (negative deltas are half the cases), a missing
/// Clip1 on p′0 or q′0, the strong filter's condition read with the
/// wrong α, a chroma line that changes p1 or q1. Would not catch a
/// misreading of the Recommendation shared by this model and the HDL.
#[test]
fn dbf_filter_matches_8_7_2_3_and_8_7_2_4() {
    let design = design("h264_dbf_filter");
    let mut sim = Simulator::new(&design, SimOptions::default()).expect("simulates");
    let names = ["p3", "p2", "p1", "p0", "q0", "q1", "q2", "q3"];
    let ins: Vec<NetHandle> = names.iter().map(|n| top_net(&sim, n)).collect();
    let outs: Vec<NetHandle> = ["p2o", "p1o", "p0o", "q0o", "q1o", "q2o"]
        .iter()
        .map(|n| top_net(&sim, n))
        .collect();
    let bs_n = top_net(&sim, "bs");
    let chroma_n = top_net(&sim, "chroma");
    let alpha_n = top_net(&sim, "alpha");
    let beta_n = top_net(&sim, "beta");
    let tc0_n = top_net(&sim, "tc0");
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    let mut filtered = 0u32;
    let mut strong = 0u32;
    for case in 0..20_000u32 {
        let base = rng.range(0, 255);
        let step = rng.range(-12, 12);
        let mut s = [0i32; 8];
        for (k, v) in s.iter_mut().enumerate() {
            let k = i32::try_from(k).expect("small");
            let edge = if k >= 4 { step } else { 0 };
            let noise = match case % 3 {
                0 => 0,
                1 => rng.range(-2, 2),
                _ => rng.range(-20, 20),
            };
            *v = clip1(base + edge + noise);
        }
        let bs = u32::try_from(rng.below(5)).expect("small");
        let chroma = rng.below(2) == 1;
        let index_a = usize::try_from(rng.below(52)).expect("small");
        let index_b = usize::try_from(rng.below(52)).expect("small");
        let alpha = i32::from(ALPHA[index_a]);
        let beta = i32::from(BETA[index_b]);
        let tc0 = if (1..=3).contains(&bs) {
            i32::from(TC0[usize::try_from(bs - 1).expect("small")][index_a])
        } else {
            0
        };
        let want = filter_line(s, bs, chroma, alpha, beta, tc0);
        if want != s {
            filtered += 1;
            if bs == 4 {
                strong += 1;
            }
        }
        for (n, v) in ins.iter().zip(s) {
            sim.set(*n, word(8, u64::try_from(v).expect("a sample")));
        }
        sim.set(bs_n, word(3, u64::from(bs)));
        sim.set(chroma_n, word(1, u64::from(chroma)));
        sim.set(alpha_n, word(8, u64::try_from(alpha).expect("small")));
        sim.set(beta_n, word(5, u64::try_from(beta).expect("small")));
        sim.set(tc0_n, word(5, u64::try_from(tc0).expect("small")));
        sim.run_for(1);
        let got: Vec<i32> = outs
            .iter()
            .map(|n| i32::try_from(get_u64(&sim, *n)).expect("a sample"))
            .collect();
        assert_eq!(
            got,
            want[1..7].to_vec(),
            "case {case}: line {s:?} bS {bs} chroma {chroma} alpha {alpha} beta {beta} tc0 {tc0}"
        );
    }
    // The draw must actually exercise the filter, or the test proves
    // nothing about it.
    assert!(filtered > 4000, "only {filtered} lines filtered");
    assert!(strong > 800, "only {strong} strong lines filtered");
}

// ---------------------------------------------------------------------------
// The model: §8.5, scaling and transforms
// ---------------------------------------------------------------------------

/// Table 8-13's zig-zag column: idx -> (i, j), as raster position 4i + j.
const ZIGZAG4: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// 8-315, v[m][column].
const NORM: [[i64; 3]; 6] = [
    [10, 16, 13],
    [11, 18, 14],
    [13, 20, 16],
    [14, 23, 18],
    [16, 25, 20],
    [18, 29, 23],
];

/// LevelScale4x4(m, i, j) with the flat weights of Flat_4x4_16 (8-313).
fn level_scale(m: usize, i: usize, j: usize) -> i64 {
    let col = match (i % 2, j % 2) {
        (0, 0) => 0,
        (1, 1) => 1,
        _ => 2,
    };
    16 * NORM[m][col]
}

/// Table 8-15: QPC by qPI.
fn qpc_of(qpi: i32) -> i32 {
    const HIGH: [i32; 22] = [
        29, 30, 31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38, 39, 39, 39, 39,
    ];
    if qpi < 30 { qpi } else { HIGH[usize::try_from(qpi - 30).expect("30..51")] }
}

/// 8-311 and Table 8-15 together: the chroma QP of a luma QP.
fn chroma_qp(qpy: i32, offset: i32) -> i32 {
    qpc_of((qpy + offset).clamp(0, 51))
}

/// 8.5.12.1 for a 4x4 array `c` in raster order. `dc` is d00 when the
/// block takes its DC from elsewhere (8-335).
fn scale_4x4(c: &[i64; 16], qp: i32, dc: Option<i64>) -> [i64; 16] {
    let m = usize::try_from(qp % 6).expect("small");
    let q6 = i64::from(qp / 6);
    let mut d = [0i64; 16];
    for (p, v) in d.iter_mut().enumerate() {
        let (i, j) = (p / 4, p % 4);
        if p == 0
            && let Some(x) = dc
        {
            *v = x;
            continue;
        }
        let x = c[p] * level_scale(m, i, j);
        *v = if qp >= 24 {
            x << (q6 - 4)
        } else {
            (x + (1 << (3 - q6))) >> (4 - q6)
        };
    }
    d
}

/// One one-dimensional pass of 8-338 to 8-345, or of the Hadamard.
fn pass(x: [i64; 4], hadamard: bool) -> [i64; 4] {
    let e0 = x[0] + x[2];
    let e1 = x[0] - x[2];
    let (e2, e3) = if hadamard {
        (x[1] - x[3], x[1] + x[3])
    } else {
        ((x[1] >> 1) - x[3], x[1] + (x[3] >> 1))
    };
    [e0 + e3, e1 + e2, e1 - e2, e0 - e3]
}

/// Rows then columns.
fn transform_2d(d: &[i64; 16], hadamard: bool) -> [i64; 16] {
    let mut f = [0i64; 16];
    for i in 0..4 {
        let row = pass([d[4 * i], d[4 * i + 1], d[4 * i + 2], d[4 * i + 3]], hadamard);
        f[4 * i..4 * i + 4].copy_from_slice(&row);
    }
    let mut h = [0i64; 16];
    for j in 0..4 {
        let col = pass([f[j], f[4 + j], f[8 + j], f[12 + j]], hadamard);
        for i in 0..4 {
            h[4 * i + j] = col[i];
        }
    }
    h
}

/// 8.5.12.2: the residual of a scaled block.
fn residual_4x4(d: &[i64; 16]) -> [i64; 16] {
    let h = transform_2d(d, false);
    h.map(|v| (v + 32) >> 6)
}

/// 8.5.10: dcY from the Intra16x16DCLevel array (raster order).
fn luma_dc(c: &[i64; 16], qp: i32) -> [i64; 16] {
    let f = transform_2d(c, true);
    let m = usize::try_from(qp % 6).expect("small");
    let q6 = i64::from(qp / 6);
    f.map(|v| {
        let x = v * level_scale(m, 0, 0);
        if qp >= 36 {
            x << (q6 - 6)
        } else {
            (x + (1 << (5 - q6))) >> (6 - q6)
        }
    })
}

/// 8.5.11 for 4:2:0: dcC from ChromaDCLevel (c00, c01, c10, c11).
fn chroma_dc(c: &[i64; 4], qp: i32) -> [i64; 4] {
    let f = [
        c[0] + c[1] + c[2] + c[3],
        c[0] - c[1] + c[2] - c[3],
        c[0] + c[1] - c[2] - c[3],
        c[0] - c[1] - c[2] + c[3],
    ];
    let m = usize::try_from(qp % 6).expect("small");
    let q6 = qp / 6;
    f.map(|v| ((v * level_scale(m, 0, 0)) << q6) >> 5)
}

/// 6.4.3: luma4x4BlkIdx -> the block's (x, y) in 4-sample units.
fn blk_xy(b: usize) -> (usize, usize) {
    let x = (b & 1) | ((b >> 1) & 2);
    let y = ((b >> 1) & 1) | ((b >> 2) & 2);
    (x, y)
}

/// One macroblock's coefficient levels, raster positions in each block,
/// numbered as the HDL numbers them: 0..15 luma, 16..23 chroma AC,
/// 24 luma DC, 25 and 26 chroma DC.
#[derive(Clone)]
struct Levels {
    blocks: [[i64; 16]; 27],
    coded: [bool; 27],
}

impl Levels {
    fn zero() -> Levels {
        Levels {
            blocks: [[0; 16]; 27],
            coded: [false; 27],
        }
    }
}

/// The 24 residual blocks of a macroblock, as §8.5 has them.
/// `kind`: 0 Intra_4x4, 1 Intra_16x16, 2 inter, 3 I_PCM.
fn mb_residuals(lv: &Levels, kind: u32, qp: i32, qpc: [i32; 2]) -> [[i64; 16]; 24] {
    let mut out = [[0i64; 16]; 24];
    if kind == 3 {
        out.copy_from_slice(&lv.blocks[..24]);
        return out;
    }
    let dcy = (kind == 1).then(|| luma_dc(&lv.blocks[24], qp));
    for (b, o) in out.iter_mut().enumerate().take(16) {
        let dc = dcy.map(|d| {
            let (x, y) = blk_xy(b);
            d[4 * y + x]
        });
        *o = residual_4x4(&scale_4x4(&lv.blocks[b], qp, dc));
    }
    for comp in 0..2 {
        let c = &lv.blocks[25 + comp];
        let dcc = chroma_dc(&[c[0], c[1], c[2], c[3]], qpc[comp]);
        for k in 0..4 {
            let b = 16 + 4 * comp + k;
            out[b] = residual_4x4(&scale_4x4(&lv.blocks[b], qpc[comp], Some(dcc[k])));
        }
    }
    out
}

/// True when every intermediate §8.5 bounds stays inside its range for
/// this macroblock — a random draw can break the bitstream constraints
/// of 8.5.10 to 8.5.12, and then the HDL's word widths are allowed to
/// disagree with the model's unbounded integers.
fn levels_conform(lv: &Levels, kind: u32, qp: i32, qpc: [i32; 2]) -> bool {
    const LIM: i64 = 1 << 15;
    let ok = |v: i64| (-LIM..LIM).contains(&v);
    let ok_h = |v: i64| (-LIM..LIM - 32).contains(&v);
    let check_block = |c: &[i64; 16], qp: i32, dc: Option<i64>| -> bool {
        if !c.iter().all(|&v| ok(v)) {
            return false;
        }
        let d = scale_4x4(c, qp, dc);
        if !d.iter().all(|&v| ok(v)) {
            return false;
        }
        for i in 0..4 {
            let row = pass([d[4 * i], d[4 * i + 1], d[4 * i + 2], d[4 * i + 3]], false);
            if !row.iter().all(|&v| ok(v)) {
                return false;
            }
        }
        transform_2d(&d, false).iter().all(|&v| ok_h(v))
    };
    if kind == 3 {
        return true;
    }
    let dcy = if kind == 1 {
        let f = transform_2d(&lv.blocks[24], true);
        if !f.iter().all(|&v| ok(v)) {
            return false;
        }
        let d = luma_dc(&lv.blocks[24], qp);
        if !d.iter().all(|&v| ok(v)) {
            return false;
        }
        Some(d)
    } else {
        None
    };
    for b in 0..16 {
        let dc = dcy.map(|d| {
            let (x, y) = blk_xy(b);
            d[4 * y + x]
        });
        if !check_block(&lv.blocks[b], qp, dc) {
            return false;
        }
    }
    for comp in 0..2 {
        let c = &lv.blocks[25 + comp];
        let dcc = chroma_dc(&[c[0], c[1], c[2], c[3]], qpc[comp]);
        if !dcc.iter().all(|&v| ok(v)) {
            return false;
        }
        for k in 0..4 {
            if !check_block(&lv.blocks[16 + 4 * comp + k], qpc[comp], Some(dcc[k])) {
                return false;
            }
        }
    }
    true
}

/// Draws a macroblock's levels: each block coded or not, a few
/// non-zero levels in the low frequencies where real streams put them,
/// thinned until the draw conforms.
fn draw_levels(rng: &mut Rng, kind: u32, qp: i32, qpc: [i32; 2]) -> Levels {
    if kind == 3 {
        let mut lv = Levels::zero();
        for b in 0..24 {
            for p in 0..16 {
                lv.blocks[b][p] = i64::try_from(rng.below(256)).expect("a sample");
            }
            lv.coded[b] = true;
        }
        return lv;
    }
    let mut amp = 1 + rng.below(60);
    loop {
        let mut lv = Levels::zero();
        for b in 0..27 {
            if b == 24 && kind != 1 {
                continue;
            }
            if rng.below(3) == 0 {
                continue;
            }
            // How many, and how far down the zig-zag.
            let n = 1 + rng.below(8);
            let reach = 1 + rng.below(16);
            let reach = if b >= 25 { reach.min(4) } else { reach };
            for _ in 0..n {
                let idx = usize::try_from(rng.below(reach)).expect("small");
                let pos = if b >= 25 { idx } else { ZIGZAG4[idx] };
                let v = i64::try_from(1 + rng.below(amp)).expect("small");
                lv.blocks[b][pos] = if rng.below(2) == 0 { v } else { -v };
            }
            // An Intra_16x16 AC block and a chroma AC block hold no c00.
            if (kind == 1 && b < 16) || (16..24).contains(&b) {
                lv.blocks[b][0] = 0;
            }
            lv.coded[b] = lv.blocks[b].iter().any(|&v| v != 0);
        }
        if levels_conform(&lv, kind, qp, qpc) {
            return lv;
        }
        amp = (amp / 2).max(1);
    }
}

/// A bench over one module: named ports, a clock, a cycle count.
struct Bench<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    n: std::collections::HashMap<String, (NetHandle, u32)>,
    cycles: u64,
}

impl<'d> Bench<'d> {
    /// Builds the bench and holds `rst_n` low over two edges. `inputs`
    /// are set to zero first.
    fn new(design: &'d Design, inputs: &[(&str, u32)], outputs: &[&str]) -> Bench<'d> {
        let sim = Simulator::new(design, SimOptions::default()).expect("simulates");
        let clk = top_net(&sim, "clk");
        let mut n = std::collections::HashMap::new();
        for (name, w) in inputs {
            n.insert((*name).to_owned(), (top_net(&sim, name), *w));
        }
        for name in outputs {
            n.insert((*name).to_owned(), (top_net(&sim, name), 0));
        }
        n.insert("rst_n".to_owned(), (top_net(&sim, "rst_n"), 1));
        let mut b = Bench {
            sim,
            clk,
            n,
            cycles: 0,
        };
        for (name, _) in inputs {
            b.set(name, 0);
        }
        b.set("rst_n", 0);
        b.cycle();
        b.cycle();
        b.set("rst_n", 1);
        b.cycle();
        b
    }

    fn set(&mut self, name: &str, value: u64) {
        let (h, w) = self.n[name];
        self.sim.set(h, word(w, value));
    }

    fn get(&mut self, name: &str) -> u64 {
        self.sim.run_for(1);
        get_u64(&self.sim, self.n[name].0)
    }

    /// A net up to 64 bits wide, as `get`.
    fn get_wide(&mut self, name: &str) -> u64 {
        self.get(name)
    }

    fn cycle(&mut self) {
        self.sim.run_for(5);
        self.sim.set(self.clk, word(1, 1));
        self.sim.run_for(5);
        self.sim.set(self.clk, word(1, 0));
        self.cycles += 1;
    }
}

/// Writes a macroblock's levels into half `half` of `h264_residual`, as
/// stage A would: a clear, the non-zero levels, each block's mask.
fn residual_load(b: &mut Bench<'_>, half: u64, lv: &Levels, pcm: bool) {
    b.set("clr_en", 1);
    b.set("clr_half", half);
    b.cycle();
    b.set("clr_en", 0);
    for blk in 0..27u64 {
        let levels = &lv.blocks[usize::try_from(blk).expect("small")];
        if !lv.coded[usize::try_from(blk).expect("small")] {
            continue;
        }
        let mut mask = 0u64;
        for p in 0..16u64 {
            let v = levels[usize::try_from(p).expect("small")];
            // I_PCM writes every sample, zero or not.
            if v != 0 || pcm {
                mask |= 1 << p;
                b.set("cw_en", 1);
                b.set("cw_addr", (half << 9) | (blk << 4) | p);
                b.set("cw_data", u64::from(i16::try_from(v).expect("16 bits") as u16));
                b.cycle();
            }
        }
        b.set("cw_en", 0);
        b.set("mw_en", 1);
        b.set("mw_addr", (half << 5) | blk);
        b.set("mw_mask", mask);
        b.cycle();
        b.set("mw_en", 0);
    }
}

/// Runs one macroblock through `h264_residual` and reads its residuals
/// back, with the cycles it took.
fn residual_run(b: &mut Bench<'_>, half: u64, kind: u32, qp: i32, qpc: [i32; 2]) -> ([[i64; 16]; 24], u64) {
    b.set("half", half);
    b.set("kind", u64::from(kind));
    b.set("qp_y", u64::try_from(qp).expect("qp"));
    b.set("qp_cb", u64::try_from(qpc[0]).expect("qp"));
    b.set("qp_cr", u64::try_from(qpc[1]).expect("qp"));
    b.set("go", 1);
    b.cycle();
    b.set("go", 0);
    let start = b.cycles;
    loop {
        let finished = b.get("done") == 1;
        b.cycle();
        if finished {
            break;
        }
        assert!(b.cycles - start < 5000, "h264_residual never finished");
    }
    let spent = b.cycles - start;
    let zf = b.get(if half == 1 { "zf1" } else { "zf0" });
    let mut out = [[0i64; 16]; 24];
    for blk in 0..24u64 {
        if zf >> blk & 1 == 1 {
            continue;
        }
        for row in 0..4u64 {
            b.set("rr_addr", (half << 7) | (blk << 2) | row);
            b.cycle();
            let w = b.get("rr_data");
            for col in 0..4u64 {
                let v = i64::try_from((w >> (10 * col)) & 0x3FF).expect("ten bits");
                let v = if v >= 512 { v - 1024 } else { v };
                out[usize::try_from(blk).expect("small")][usize::try_from(4 * row + col).expect("small")] = v;
            }
        }
    }
    (out, spent)
}

/// `h264_residual` against §8.5 for every macroblock kind.
///
/// Each macroblock goes into one half of the coefficient memory over the
/// leftovers of the macroblock before it in the same half, which is the
/// point: the memory is never cleared, and a position or a block left
/// over from last time must read as zero.
///
/// Would catch: a wrong LevelScale entry or class, the rounding of
/// 8-337 or 8-322 off by a power of two, the Hadamard of 8-320 with the
/// DCs assigned to the wrong blocks (Figure 8-6), chroma DC with c01 and
/// c10 swapped, a chroma block scaled with the luma QP, the transform's
/// passes in the wrong order, stale coefficients, and a coded block
/// reported as zero. Would not catch a misreading the model shares.
#[test]
fn residual_matches_8_5_for_every_kind() {
    let design = design("h264_residual");
    let mut bench = Bench::new(
        &design,
        &[
            ("cw_en", 1),
            ("cw_addr", 10),
            ("cw_data", 16),
            ("mw_en", 1),
            ("mw_addr", 6),
            ("mw_mask", 16),
            ("clr_en", 1),
            ("clr_half", 1),
            ("go", 1),
            ("half", 1),
            ("kind", 3),
            ("qp_y", 6),
            ("qp_cb", 6),
            ("qp_cr", 6),
            ("rr_addr", 8),
        ],
        &["busy", "done", "rr_data", "zf0", "zf1"],
    );
    let mut rng = Rng(0x5EED_0001);
    let mut worst = 0u64;
    for case in 0..48u32 {
        let kind = case % 4;
        let qp = rng.range(0, 51);
        let offset = rng.range(-12, 12);
        let qpc = [chroma_qp(qp, offset), chroma_qp(qp, offset + rng.range(-1, 1))];
        let lv = draw_levels(&mut rng, kind, qp, qpc);
        let half = u64::from(case % 2);
        residual_load(&mut bench, half, &lv, kind == 3);
        let (got, spent) = residual_run(&mut bench, half, kind, qp, qpc);
        worst = worst.max(spent);
        let want = mb_residuals(&lv, kind, qp, qpc);
        for b in 0..24 {
            assert_eq!(
                got[b], want[b],
                "case {case}: kind {kind} qp {qp} qpc {qpc:?} block {b}\nlevels {:?}",
                lv.blocks[b]
            );
        }
    }
    println!("h264_residual: at most {worst} cycles a macroblock");
}

// ---------------------------------------------------------------------------
// The model: §8.3, intra prediction, and §8.5.14, picture construction
// ---------------------------------------------------------------------------

/// Macroblock kinds as the HDL numbers them.
const K_I4: u32 = 0;
const K_I16: u32 = 1;
const K_INTER: u32 = 2;
const K_PCM: u32 = 3;

/// One slice's parameters, as far as reconstruction cares.
#[derive(Clone, Debug)]
struct SliceDesc {
    first_mb: usize,
    chroma_qp_offset: i32,
    constrained_intra: bool,
    disable_deblocking: u32,
    offset_a: i32,
    offset_b: i32,
}

/// One macroblock, as the token stream describes it.
#[derive(Clone)]
struct MbDesc {
    kind: u32,
    i16_mode: u32,
    c_mode: u32,
    i4_modes: [u32; 16],
    qp: i32,
    levels: Levels,
    /// Inter prediction: 256 luma, 64 Cb, 64 Cr, each raster.
    pred: Vec<u8>,
    /// Per luma4x4BlkIdx: (mv x, mv y, ref_idx).
    mv: [(i32, i32, u32); 16],
}

/// A picture: its size in macroblocks, its slices, its macroblocks.
#[derive(Clone)]
struct PicDesc {
    w: usize,
    h: usize,
    slices: Vec<SliceDesc>,
    mbs: Vec<MbDesc>,
}

impl PicDesc {
    fn slice_of(&self, addr: usize) -> &SliceDesc {
        self.slices
            .iter()
            .rev()
            .find(|s| s.first_mb <= addr)
            .expect("every macroblock is in a slice")
    }

    fn is_intra(&self, addr: usize) -> bool {
        self.mbs[addr].kind != K_INTER
    }

    /// mbAddrA..D as 6.4.9 makes them available (same slice, inside the
    /// picture) — and, for an intra macroblock under
    /// constrained_intra_pred_flag, not inter (8.3.1.2 step 3).
    fn avail(&self, addr: usize, for_intra: bool) -> [bool; 4] {
        let w = self.w;
        let (mx, my) = (addr % w, addr / w);
        let first = self.slice_of(addr).first_mb;
        let cip = self.slice_of(addr).constrained_intra && for_intra;
        let ok = |n: usize| n >= first && (!cip || self.is_intra(n));
        [
            mx > 0 && ok(addr - 1),
            my > 0 && ok(addr - w),
            my > 0 && mx + 1 < w && ok(addr - w + 1),
            mx > 0 && my > 0 && ok(addr - w - 1),
        ]
    }
}

/// The three sample arrays of a picture.
#[derive(Clone, PartialEq)]
struct Planes {
    w: usize,
    h: usize,
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
}

impl Planes {
    fn new(w_mbs: usize, h_mbs: usize) -> Planes {
        Planes {
            w: w_mbs * 16,
            h: h_mbs * 16,
            y: vec![0; w_mbs * h_mbs * 256],
            cb: vec![0; w_mbs * h_mbs * 64],
            cr: vec![0; w_mbs * h_mbs * 64],
        }
    }

    /// Plane 0, 1 or 2, its width.
    fn plane(&self, c: usize) -> (&Vec<u8>, usize) {
        match c {
            0 => (&self.y, self.w),
            1 => (&self.cb, self.w / 2),
            _ => (&self.cr, self.w / 2),
        }
    }

    fn plane_mut(&mut self, c: usize) -> (&mut Vec<u8>, usize) {
        let w = self.w;
        match c {
            0 => (&mut self.y, w),
            1 => (&mut self.cb, w / 2),
            _ => (&mut self.cr, w / 2),
        }
    }

    fn at(&self, c: usize, x: usize, y: usize) -> i32 {
        let (p, w) = self.plane(c);
        i32::from(p[y * w + x])
    }

    fn put(&mut self, c: usize, x: usize, y: usize, v: i32) {
        let (p, w) = self.plane_mut(c);
        p[y * w + x] = u8::try_from(v).expect("a sample");
    }
}

/// Which neighbouring macroblock a location relative to the current one
/// falls in (6.4.12 for a frame): 0 A, 1 B, 2 C, 3 D, 4 the current one,
/// 5 none. `maxw` and `maxh` are the component's macroblock size.
fn neighbour_of(xn: i32, yn: i32, maxw: i32, maxh: i32) -> usize {
    match (xn, yn) {
        (x, y) if x < 0 && y < 0 => 3,
        (x, y) if x < 0 && (0..maxh).contains(&y) => 0,
        (x, y) if (0..maxw).contains(&x) && y < 0 => 1,
        (x, y) if (0..maxw).contains(&x) && (0..maxh).contains(&y) => 4,
        (x, y) if x >= maxw && y < 0 => 2,
        _ => 5,
    }
}

/// 8.3.1.2: the Intra_4x4 prediction of block `blk`, from the picture
/// as constructed so far.
fn pred_intra4x4(pl: &Planes, mx: usize, my: usize, blk: usize, mode: u32, av: [bool; 4]) -> [i32; 16] {
    let (bx, by) = blk_xy(blk);
    let (xo, yo) = (4 * bx as i32, 4 * by as i32);
    // p[x, y] for x = -1, y = -1..3 and x = 0..7, y = -1, or None.
    let fetch = |x: i32, y: i32| -> Option<i32> {
        let (xn, yn) = (xo + x, yo + y);
        let n = neighbour_of(xn, yn, 16, 16);
        let ok = match n {
            4 => true,
            5 => false,
            k => av[k],
        };
        if !ok || (x > 3 && (blk == 3 || blk == 11)) {
            return None;
        }
        let gx = usize::try_from(i32::try_from(mx * 16).expect("fits") + xn).expect("inside");
        let gy = usize::try_from(i32::try_from(my * 16).expect("fits") + yn).expect("inside");
        Some(pl.at(0, gx, gy))
    };
    let mut top: [Option<i32>; 8] = [None; 8];
    for (x, t) in top.iter_mut().enumerate() {
        *t = fetch(x as i32, -1);
    }
    // The substitution of p[3, -1] for p[4..7, -1].
    if top[4].is_none() && top[3].is_some() {
        let t3 = top[3];
        for t in top.iter_mut().skip(4) {
            *t = t3;
        }
    }
    let left: [Option<i32>; 4] = [fetch(-1, 0), fetch(-1, 1), fetch(-1, 2), fetch(-1, 3)];
    let corner = fetch(-1, -1);
    let p = |x: i32, y: i32| -> i32 {
        let v = if y == -1 && x >= 0 {
            top[x as usize]
        } else if x == -1 && y == -1 {
            corner
        } else {
            left[y as usize]
        };
        v.unwrap_or_else(|| panic!("mode {mode} of block {blk} reads p[{x}, {y}], which is not available"))
    };
    let mut out = [0i32; 16];
    for y in 0..4i32 {
        for x in 0..4i32 {
            let v = match mode {
                0 => p(x, -1),
                1 => p(-1, y),
                2 => {
                    let t = top[..4].iter().all(Option::is_some);
                    let l = left.iter().all(Option::is_some);
                    let st: i32 = if t { (0..4).map(|k| p(k, -1)).sum() } else { 0 };
                    let sl: i32 = if l { (0..4).map(|k| p(-1, k)).sum() } else { 0 };
                    match (t, l) {
                        (true, true) => (st + sl + 4) >> 3,
                        (false, true) => (sl + 2) >> 2,
                        (true, false) => (st + 2) >> 2,
                        _ => 128,
                    }
                }
                3 => {
                    if x == 3 && y == 3 {
                        (p(6, -1) + 3 * p(7, -1) + 2) >> 2
                    } else {
                        (p(x + y, -1) + 2 * p(x + y + 1, -1) + p(x + y + 2, -1) + 2) >> 2
                    }
                }
                4 => {
                    if x > y {
                        (p(x - y - 2, -1) + 2 * p(x - y - 1, -1) + p(x - y, -1) + 2) >> 2
                    } else if x < y {
                        (p(-1, y - x - 2) + 2 * p(-1, y - x - 1) + p(-1, y - x) + 2) >> 2
                    } else {
                        (p(0, -1) + 2 * p(-1, -1) + p(-1, 0) + 2) >> 2
                    }
                }
                5 => {
                    let z = 2 * x - y;
                    match z {
                        0 | 2 | 4 | 6 => (p(x - (y >> 1) - 1, -1) + p(x - (y >> 1), -1) + 1) >> 1,
                        1 | 3 | 5 => {
                            (p(x - (y >> 1) - 2, -1) + 2 * p(x - (y >> 1) - 1, -1) + p(x - (y >> 1), -1) + 2)
                                >> 2
                        }
                        -1 => (p(-1, 0) + 2 * p(-1, -1) + p(0, -1) + 2) >> 2,
                        _ => (p(-1, y - 1) + 2 * p(-1, y - 2) + p(-1, y - 3) + 2) >> 2,
                    }
                }
                6 => {
                    let z = 2 * y - x;
                    match z {
                        0 | 2 | 4 | 6 => (p(-1, y - (x >> 1) - 1) + p(-1, y - (x >> 1)) + 1) >> 1,
                        1 | 3 | 5 => {
                            (p(-1, y - (x >> 1) - 2) + 2 * p(-1, y - (x >> 1) - 1) + p(-1, y - (x >> 1)) + 2)
                                >> 2
                        }
                        -1 => (p(-1, 0) + 2 * p(-1, -1) + p(0, -1) + 2) >> 2,
                        _ => (p(x - 1, -1) + 2 * p(x - 2, -1) + p(x - 3, -1) + 2) >> 2,
                    }
                }
                7 => {
                    if y == 0 || y == 2 {
                        (p(x + (y >> 1), -1) + p(x + (y >> 1) + 1, -1) + 1) >> 1
                    } else {
                        (p(x + (y >> 1), -1) + 2 * p(x + (y >> 1) + 1, -1) + p(x + (y >> 1) + 2, -1) + 2) >> 2
                    }
                }
                _ => {
                    let z = x + 2 * y;
                    match z {
                        0 | 2 | 4 => (p(-1, y + (x >> 1)) + p(-1, y + (x >> 1) + 1) + 1) >> 1,
                        1 | 3 => {
                            (p(-1, y + (x >> 1)) + 2 * p(-1, y + (x >> 1) + 1) + p(-1, y + (x >> 1) + 2) + 2) >> 2
                        }
                        5 => (p(-1, 2) + 3 * p(-1, 3) + 2) >> 2,
                        _ => p(-1, 3),
                    }
                }
            };
            out[(4 * y + x) as usize] = v;
        }
    }
    out
}

/// The neighbours of a whole macroblock of component `c` for 8.3.3 and
/// 8.3.4: the row above, the column to the left and the corner, each
/// None when its macroblock is not available.
fn mb_neighbours(pl: &Planes, c: usize, mx: usize, my: usize, av: [bool; 4]) -> (Option<Vec<i32>>, Option<Vec<i32>>, Option<i32>) {
    let n = if c == 0 { 16 } else { 8 };
    let (ox, oy) = (mx * n, my * n);
    let top = av[1].then(|| (0..n).map(|x| pl.at(c, ox + x, oy - 1)).collect());
    let left = av[0].then(|| (0..n).map(|y| pl.at(c, ox - 1, oy + y)).collect());
    let corner = av[3].then(|| pl.at(c, ox - 1, oy - 1));
    (top, left, corner)
}

/// 8.3.3: an Intra_16x16 prediction, raster order.
fn pred_intra16x16(pl: &Planes, mx: usize, my: usize, mode: u32, av: [bool; 4]) -> Vec<i32> {
    let (top, left, corner) = mb_neighbours(pl, 0, mx, my, av);
    let mut out = vec![0i32; 256];
    match mode {
        0 => {
            let t = top.expect("Intra_16x16_Vertical needs the row above");
            for y in 0..16 {
                for x in 0..16 {
                    out[16 * y + x] = t[x];
                }
            }
        }
        1 => {
            let l = left.expect("Intra_16x16_Horizontal needs the column to the left");
            for y in 0..16 {
                for x in 0..16 {
                    out[16 * y + x] = l[y];
                }
            }
        }
        2 => {
            let v = match (&top, &left) {
                (Some(t), Some(l)) => (t.iter().sum::<i32>() + l.iter().sum::<i32>() + 16) >> 5,
                (None, Some(l)) => (l.iter().sum::<i32>() + 8) >> 4,
                (Some(t), None) => (t.iter().sum::<i32>() + 8) >> 4,
                (None, None) => 128,
            };
            out.iter_mut().for_each(|o| *o = v);
        }
        _ => {
            let t = top.expect("Plane needs the row above");
            let l = left.expect("Plane needs the column to the left");
            let d = corner.expect("Plane needs the corner");
            let pt = |x: i32| if x < 0 { d } else { t[x as usize] };
            let plf = |y: i32| if y < 0 { d } else { l[y as usize] };
            let hh: i32 = (0..8).map(|x| (x + 1) * (pt(8 + x) - pt(6 - x))).sum();
            let vv: i32 = (0..8).map(|y| (y + 1) * (plf(8 + y) - plf(6 - y))).sum();
            let a = 16 * (l[15] + t[15]);
            let b = (5 * hh + 32) >> 6;
            let c = (5 * vv + 32) >> 6;
            for y in 0..16i32 {
                for x in 0..16i32 {
                    out[(16 * y + x) as usize] = clip1((a + b * (x - 7) + c * (y - 7) + 16) >> 5);
                }
            }
        }
    }
    out
}

/// 8.3.4 for 4:2:0: one chroma component's prediction, raster order.
fn pred_chroma(pl: &Planes, comp: usize, mx: usize, my: usize, mode: u32, av: [bool; 4]) -> Vec<i32> {
    let (top, left, corner) = mb_neighbours(pl, comp, mx, my, av);
    let mut out = vec![0i32; 64];
    match mode {
        0 => {
            for blk in 0..4usize {
                let (xo, yo) = (4 * (blk % 2), 4 * (blk / 2));
                let st = top.as_ref().map(|t| t[xo..xo + 4].iter().sum::<i32>());
                let sl = left.as_ref().map(|l| l[yo..yo + 4].iter().sum::<i32>());
                let v = if (xo, yo) == (0, 0) || (xo > 0 && yo > 0) {
                    match (st, sl) {
                        (Some(t), Some(l)) => (t + l + 4) >> 3,
                        (None, Some(l)) => (l + 2) >> 2,
                        (Some(t), None) => (t + 2) >> 2,
                        _ => 128,
                    }
                } else if xo > 0 {
                    match (st, sl) {
                        (Some(t), _) => (t + 2) >> 2,
                        (None, Some(l)) => (l + 2) >> 2,
                        _ => 128,
                    }
                } else {
                    match (st, sl) {
                        (_, Some(l)) => (l + 2) >> 2,
                        (Some(t), None) => (t + 2) >> 2,
                        _ => 128,
                    }
                };
                for y in 0..4 {
                    for x in 0..4 {
                        out[8 * (yo + y) + xo + x] = v;
                    }
                }
            }
        }
        1 => {
            let l = left.expect("Intra_Chroma_Horizontal needs the left");
            for y in 0..8 {
                for x in 0..8 {
                    out[8 * y + x] = l[y];
                }
            }
        }
        2 => {
            let t = top.expect("Intra_Chroma_Vertical needs the top");
            for y in 0..8 {
                for x in 0..8 {
                    out[8 * y + x] = t[x];
                }
            }
        }
        _ => {
            let t = top.expect("Plane needs the row above");
            let l = left.expect("Plane needs the column to the left");
            let d = corner.expect("Plane needs the corner");
            let pt = |x: i32| if x < 0 { d } else { t[x as usize] };
            let plf = |y: i32| if y < 0 { d } else { l[y as usize] };
            let hh: i32 = (0..4).map(|x| (x + 1) * (pt(4 + x) - pt(2 - x))).sum();
            let vv: i32 = (0..4).map(|y| (y + 1) * (plf(4 + y) - plf(2 - y))).sum();
            let a = 16 * (l[7] + t[7]);
            let b = (34 * hh + 32) >> 6;
            let c = (34 * vv + 32) >> 6;
            for y in 0..8i32 {
                for x in 0..8i32 {
                    out[(8 * y + x) as usize] = clip1((a + b * (x - 3) + c * (y - 3) + 16) >> 5);
                }
            }
        }
    }
    out
}

/// §8.5.14 for a whole picture: each macroblock's prediction plus its
/// residual (24 blocks as `mb_residuals` gives them), clipped.
fn construct(pic: &PicDesc, residuals: &[[[i64; 16]; 24]]) -> Planes {
    let mut pl = Planes::new(pic.w, pic.h);
    for (addr, mb) in pic.mbs.iter().enumerate() {
        let (mx, my) = (addr % pic.w, addr / pic.w);
        let av = pic.avail(addr, mb.kind != K_INTER);
        let res = &residuals[addr];
        // Luma.
        if mb.kind == K_I4 {
            for blk in 0..16 {
                let pred = pred_intra4x4(&pl, mx, my, blk, mb.i4_modes[blk], av);
                let (bx, by) = blk_xy(blk);
                for i in 0..16 {
                    let v = clip1(pred[i] + i32::try_from(res[blk][i]).expect("small"));
                    pl.put(0, mx * 16 + 4 * bx + i % 4, my * 16 + 4 * by + i / 4, v);
                }
            }
        } else {
            let pred: Vec<i32> = match mb.kind {
                K_I16 => pred_intra16x16(&pl, mx, my, mb.i16_mode, av),
                K_INTER => mb.pred[..256].iter().map(|&v| i32::from(v)).collect(),
                _ => vec![0; 256],
            };
            for y in 0..16 {
                for x in 0..16 {
                    let blk = blk_at(x / 4, y / 4);
                    let r = i32::try_from(res[blk][4 * (y % 4) + x % 4]).expect("small");
                    pl.put(0, mx * 16 + x, my * 16 + y, clip1(pred[16 * y + x] + r));
                }
            }
        }
        // Chroma.
        for comp in 1..3usize {
            let pred: Vec<i32> = match mb.kind {
                K_I4 | K_I16 => pred_chroma(&pl, comp, mx, my, mb.c_mode, av),
                K_INTER => mb.pred[256 + 64 * (comp - 1)..256 + 64 * comp]
                    .iter()
                    .map(|&v| i32::from(v))
                    .collect(),
                _ => vec![0; 64],
            };
            for y in 0..8 {
                for x in 0..8 {
                    let blk = 16 + 4 * (comp - 1) + 2 * (y / 4) + x / 4;
                    let r = i32::try_from(res[blk][4 * (y % 4) + x % 4]).expect("small");
                    pl.put(comp, mx * 8 + x, my * 8 + y, clip1(pred[8 * y + x] + r));
                }
            }
        }
    }
    pl
}

/// luma4x4BlkIdx of the block at (x, y) in 4-sample units.
fn blk_at(x: usize, y: usize) -> usize {
    (x & 1) | ((y & 1) << 1) | ((x & 2) << 1) | ((y & 2) << 2)
}

// ---------------------------------------------------------------------------
// Drawing pictures the model and the HDL can both decode
// ---------------------------------------------------------------------------

/// Draws a prediction mode the stream would be allowed to use given
/// what is available (the "shall be used only when" of each mode).
fn draw_modes(rng: &mut Rng, mb: &mut MbDesc, av: [bool; 4]) {
    let [a, b, _, d] = av;
    let pick = |rng: &mut Rng, ok: &[u32]| ok[usize::try_from(rng.below(ok.len() as u64)).expect("small")];
    match mb.kind {
        K_I4 => {
            for blk in 0..16 {
                let (bx, by) = blk_xy(blk);
                let top = by > 0 || b;
                let left = bx > 0 || a;
                let corner = match (bx, by) {
                    (0, 0) => d,
                    (0, _) => a,
                    (_, 0) => b,
                    _ => true,
                };
                let mut ok = vec![2u32];
                if top {
                    ok.extend([0, 3, 7]);
                }
                if left {
                    ok.extend([1, 8]);
                }
                if top && left && corner {
                    ok.extend([4, 5, 6]);
                }
                mb.i4_modes[blk] = pick(rng, &ok);
            }
        }
        K_I16 => {
            let mut ok = vec![2u32];
            if b {
                ok.push(0);
            }
            if a {
                ok.push(1);
            }
            if a && b && d {
                ok.push(3);
            }
            mb.i16_mode = pick(rng, &ok);
        }
        _ => {}
    }
    if mb.kind == K_I4 || mb.kind == K_I16 {
        let mut ok = vec![0u32];
        if a {
            ok.push(1);
        }
        if b {
            ok.push(2);
        }
        if a && b && d {
            ok.push(3);
        }
        mb.c_mode = pick(rng, &ok);
    }
}

/// A random macroblock of `kind`, its modes drawn after the fact by
/// `draw_modes`.
fn blank_mb(rng: &mut Rng, kind: u32, qp: i32) -> MbDesc {
    let pred = if kind == K_INTER {
        (0..384).map(|_| u8::try_from(rng.below(256)).expect("a sample")).collect()
    } else {
        Vec::new()
    };
    let mut mv = [(0, 0, 0); 16];
    for m in mv.iter_mut() {
        *m = (rng.range(-8, 8), rng.range(-8, 8), u32::try_from(rng.below(2)).expect("small"));
    }
    MbDesc {
        kind,
        i16_mode: 2,
        c_mode: 0,
        i4_modes: [2; 16],
        qp,
        levels: Levels::zero(),
        pred,
        mv,
    }
}

/// Draws a picture of `w` x `h` macroblocks in `slices` slices, every
/// kind of macroblock and every mode its neighbours allow.
fn draw_picture(rng: &mut Rng, w: usize, h: usize, slices: usize, intra_only: bool) -> PicDesc {
    let n = w * h;
    let mut firsts = vec![0usize];
    while firsts.len() < slices {
        let f = 1 + usize::try_from(rng.below(u64::try_from(n - 1).expect("fits"))).expect("fits");
        if !firsts.contains(&f) {
            firsts.push(f);
        }
    }
    firsts.sort_unstable();
    let slices: Vec<SliceDesc> = firsts
        .iter()
        .map(|&first_mb| SliceDesc {
            first_mb,
            chroma_qp_offset: rng.range(-12, 12),
            constrained_intra: rng.below(2) == 1,
            disable_deblocking: [0u32, 0, 0, 1, 2][usize::try_from(rng.below(5)).expect("small")],
            offset_a: 2 * rng.range(-6, 6),
            offset_b: 2 * rng.range(-6, 6),
        })
        .collect();
    let mut pic = PicDesc {
        w,
        h,
        slices,
        mbs: Vec::new(),
    };
    for addr in 0..n {
        let kind = if intra_only {
            [K_I4, K_I4, K_I16, K_I16, K_PCM][usize::try_from(rng.below(5)).expect("small")]
        } else {
            [K_I4, K_I16, K_INTER, K_INTER, K_INTER, K_PCM][usize::try_from(rng.below(6)).expect("small")]
        };
        let qp = rng.range(0, 51);
        pic.mbs.push(blank_mb(rng, kind, qp));
        let av = pic.avail(addr, kind != K_INTER);
        let mut mb = pic.mbs[addr].clone();
        draw_modes(rng, &mut mb, av);
        pic.mbs[addr] = mb;
    }
    pic
}

/// Random residuals for the construction test: small mostly, some
/// large enough to clip, some blocks all zero, I_PCM's samples.
fn draw_residuals(rng: &mut Rng, mb: &MbDesc) -> ([[i64; 16]; 24], u32) {
    let mut r = [[0i64; 16]; 24];
    let mut zf = 0u32;
    for (b, blk) in r.iter_mut().enumerate() {
        if mb.kind == K_PCM {
            for v in blk.iter_mut() {
                *v = i64::try_from(rng.below(256)).expect("a sample");
            }
            continue;
        }
        match rng.below(4) {
            0 => zf |= 1 << b,
            1 => {
                for v in blk.iter_mut() {
                    *v = i64::from(rng.range(-300, 300));
                }
            }
            _ => {
                for v in blk.iter_mut() {
                    *v = i64::from(rng.range(-6, 6));
                }
            }
        }
    }
    (r, zf)
}

/// Packs a block row of four residuals as `h264_residual` stores it.
fn residual_word(r: &[i64; 16], row: usize) -> u64 {
    (0..4).fold(0u64, |acc, col| {
        let v = u64::try_from(r[4 * row + col].rem_euclid(1024)).expect("ten bits");
        acc | (v << (10 * col))
    })
}

/// Unpacks a slot of 96 words into a picture at (mx, my).
fn unpack_slot(pl: &mut Planes, mx: usize, my: usize, slot: &[u32; 96]) {
    for y in 0..16 {
        for x in 0..16 {
            let w = slot[4 * y + x / 4];
            pl.put(0, mx * 16 + x, my * 16 + y, i32::try_from((w >> (8 * (x % 4))) & 0xFF).expect("byte"));
        }
    }
    for comp in 1..3 {
        for y in 0..8 {
            for x in 0..8 {
                let w = slot[64 + 16 * (comp - 1) + 2 * y + x / 4];
                pl.put(comp, mx * 8 + x, my * 8 + y, i32::try_from((w >> (8 * (x % 4))) & 0xFF).expect("byte"));
            }
        }
    }
}

/// Packs a macroblock's inter prediction as `h264_inter` streams it:
/// luma 16 rows of 4 words, then Cb 8 rows of 2, then Cr.
fn pred_words(mb: &MbDesc) -> Vec<u32> {
    mb.pred
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// First differing sample of two pictures, for a message.
fn first_difference(got: &Planes, want: &Planes) -> String {
    for c in 0..3 {
        let (g, w) = (got.plane(c), want.plane(c));
        for (i, (a, b)) in g.0.iter().zip(w.0.iter()).enumerate() {
            if a != b {
                let (x, y) = (i % g.1, i / g.1);
                let n = if c == 0 { 16 } else { 8 };
                return format!(
                    "plane {c} ({x}, {y}), macroblock ({}, {}): got {a}, want {b}",
                    x / n,
                    y / n
                );
            }
        }
    }
    "none".to_owned()
}

/// `h264_intra` against §8.3 and §8.5.14 over whole pictures.
///
/// Every macroblock of a 5x4 picture in three slices, its kind drawn
/// from all four, its modes from all those its neighbours allow, so the
/// line buffer, the corner it saves, slice edges, constrained intra
/// prediction and the right-hand column all carry real data from one
/// macroblock to the next. Residuals are drawn directly — small, large
/// enough to clip, or flagged all zero — and the inter prediction is
/// noise, so what reaches the slot is the prediction's doing.
///
/// Would catch: any Intra_4x4 mode wrong at any (x, y), the above-right
/// substitution of 8.3.1.2 applied to the wrong blocks, a corner taken
/// before or after the block that writes it, the DC fallbacks of every
/// mode, plane prediction's rounding, chroma DC's per-block rules, a
/// left or top neighbour from the wrong macroblock, inter words in the
/// wrong order, and a residual added to the wrong sample. Would not
/// catch anything about residuals themselves, which
/// `residual_matches_8_5_for_every_kind` covers.
#[test]
fn intra_constructs_pictures_as_8_3_says() {
    let design = design("h264_intra");
    let mut b = Bench::new(
        &design,
        &[
            ("go", 1),
            ("mb_x", 7),
            ("kind", 3),
            ("i16_mode", 2),
            ("c_mode", 2),
            ("i4_modes", 64),
            ("avail_a", 1),
            ("avail_b", 1),
            ("avail_c", 1),
            ("avail_d", 1),
            ("half", 1),
            ("rr_data", 40),
            ("zf", 24),
            ("pred_valid", 1),
            ("pred_data", 32),
        ],
        &["busy", "done", "rr_addr", "pred_ready", "ow_en", "ow_addr", "ow_data"],
    );
    let mut rng = Rng(0x1A7A_0002);
    let mut worst = [0u64; 4];
    let mut used_i4 = [0u32; 9];
    let mut used_i16 = [0u32; 4];
    let mut used_c = [0u32; 4];
    for round in 0..6 {
        let pic = draw_picture(&mut rng, 5, 4, 1 + round % 3, round % 3 == 0);
        for mb in &pic.mbs {
            match mb.kind {
                K_I4 => mb.i4_modes.iter().for_each(|&m| used_i4[m as usize] += 1),
                K_I16 => used_i16[mb.i16_mode as usize] += 1,
                _ => {}
            }
            if mb.kind == K_I4 || mb.kind == K_I16 {
                used_c[mb.c_mode as usize] += 1;
            }
        }
        let mut residuals = Vec::new();
        let mut flags = Vec::new();
        for mb in &pic.mbs {
            let (r, z) = draw_residuals(&mut rng, mb);
            let mut r = r;
            for (k, blk) in r.iter_mut().enumerate() {
                if z >> k & 1 == 1 {
                    *blk = [0; 16];
                }
            }
            residuals.push(r);
            flags.push(z);
        }
        let want = construct(&pic, &residuals);
        let mut got = Planes::new(pic.w, pic.h);
        for (addr, mb) in pic.mbs.iter().enumerate() {
            let (mx, my) = (addr % pic.w, addr / pic.w);
            let av = pic.avail(addr, mb.kind != K_INTER);
            let half = u64::try_from(addr % 2).expect("small");
            b.set("mb_x", u64::try_from(mx).expect("small"));
            b.set("kind", u64::from(mb.kind));
            b.set("i16_mode", u64::from(mb.i16_mode));
            b.set("c_mode", u64::from(mb.c_mode));
            let modes = mb.i4_modes.iter().enumerate().fold(0u64, |acc, (k, &m)| acc | (u64::from(m) << (4 * k)));
            b.set("i4_modes", modes);
            for (k, name) in ["avail_a", "avail_b", "avail_c", "avail_d"].iter().enumerate() {
                b.set(name, u64::from(av[k]));
            }
            b.set("half", half);
            b.set("zf", u64::from(flags[addr]));
            let words = pred_words(mb);
            let mut sent = 0usize;
            let mut slot = [0u32; 96];
            b.set("go", 1);
            b.cycle();
            b.set("go", 0);
            let start = b.cycles;
            loop {
                let more = sent < words.len();
                b.set("pred_valid", u64::from(more));
                b.set("pred_data", u64::from(words.get(sent).copied().unwrap_or(0)));
                let addr_now = b.get("rr_addr");
                let took = more && b.get("pred_ready") == 1;
                if b.get("ow_en") == 1 {
                    let a = usize::try_from(b.get("ow_addr")).expect("small");
                    slot[a] = u32::try_from(b.get("ow_data")).expect("32 bits");
                }
                let finished = b.get("done") == 1;
                b.cycle();
                // The residual buffer answers a cycle after its address.
                let blk = usize::try_from((addr_now >> 2) & 31).expect("small");
                let row = usize::try_from(addr_now & 3).expect("small");
                let data = if blk < 24 { residual_word(&residuals[addr][blk], row) } else { 0 };
                b.set("rr_data", data);
                if took {
                    sent += 1;
                }
                if finished {
                    break;
                }
                assert!(b.cycles - start < 2000, "h264_intra stalled at macroblock {addr}");
            }
            let spent = b.cycles - start;
            let k = usize::try_from(mb.kind).expect("small");
            worst[k] = worst[k].max(spent);
            if mb.kind == K_INTER {
                assert_eq!(sent, 96, "macroblock {addr} took {sent} prediction words");
            }
            unpack_slot(&mut got, mx, my, &slot);
        }
        assert!(
            got == want,
            "round {round}: the constructed picture differs: {}",
            first_difference(&got, &want)
        );
    }
    println!("h264_intra: worst cycles a macroblock, I4 {} I16 {} inter {} PCM {}", worst[0], worst[1], worst[2], worst[3]);
    println!("modes used: Intra_4x4 {used_i4:?}, Intra_16x16 {used_i16:?}, chroma {used_c:?}");
    // Every mode, often enough that a wrong one would have shown.
    assert!(used_i4.iter().all(|&n| n >= 20), "Intra_4x4 modes {used_i4:?}");
    assert!(used_i16.iter().all(|&n| n >= 3), "Intra_16x16 modes {used_i16:?}");
    assert!(used_c.iter().all(|&n| n >= 3), "chroma modes {used_c:?}");
}

// ---------------------------------------------------------------------------
// The model: §8.7, the deblocking filter over a picture
// ---------------------------------------------------------------------------

/// The QPY the deblocking filter uses for a macroblock: 0 for I_PCM
/// (8.7.2, "qPz is set to 0").
fn dbf_qp(mb: &MbDesc) -> i32 {
    if mb.kind == K_PCM { 0 } else { mb.qp }
}

/// 8.7.2.1 for the luma samples p0 at `(px, py)` and q0 at `(qx, qy)`
/// (picture coordinates), for a frame with no field macroblocks.
fn boundary_strength(pic: &PicDesc, px: usize, py: usize, qx: usize, qy: usize) -> u32 {
    let w = pic.w;
    let pa = (py / 16) * w + px / 16;
    let qa = (qy / 16) * w + qx / 16;
    let (mp, mq) = (&pic.mbs[pa], &pic.mbs[qa]);
    let mb_edge = pa != qa;
    if mp.kind != K_INTER || mq.kind != K_INTER {
        return if mb_edge { 4 } else { 3 };
    }
    let bp = blk_at((px % 16) / 4, (py % 16) / 4);
    let bq = blk_at((qx % 16) / 4, (qy % 16) / 4);
    if mp.levels.coded[bp] || mq.levels.coded[bq] {
        return 2;
    }
    let (vp, vq) = (mp.mv[bp], mq.mv[bq]);
    if vp.2 != vq.2 || (vp.0 - vq.0).abs() >= 4 || (vp.1 - vq.1).abs() >= 4 {
        return 1;
    }
    0
}

/// §8.7 over a whole picture, in macroblock order, as the clause reads.
fn deblock_picture(pic: &PicDesc, input: &Planes) -> Planes {
    let mut s = input.clone();
    let w = pic.w;
    for addr in 0..pic.mbs.len() {
        let (mx, my) = (addr % w, addr / w);
        let slice = pic.slice_of(addr);
        let idc = slice.disable_deblocking;
        let same_slice = |n: usize| n >= slice.first_mb;
        let filter_left = mx > 0 && idc != 1 && !(idc == 2 && !same_slice(addr - 1));
        let filter_top = my > 0 && idc != 1 && !(idc == 2 && !same_slice(addr - w));
        let filter_inner = idc != 1;
        for comp in 0..3usize {
            let n = if comp == 0 { 16 } else { 8 };
            let (x0, y0) = (mx * n, my * n);
            // The edges, in order: vertical left to right, then horizontal.
            for vertical in [true, false] {
                let edges: Vec<usize> = if comp == 0 { vec![0, 4, 8, 12] } else { vec![0, 4] };
                for &e in &edges {
                    let on = if e == 0 {
                        if vertical { filter_left } else { filter_top }
                    } else {
                        filter_inner
                    };
                    if !on {
                        continue;
                    }
                    for k in 0..n {
                        // q0's position, then p_i and q_i.
                        let (qx, qy) = if vertical { (x0 + e, y0 + k) } else { (x0 + k, y0 + e) };
                        let at = |i: i32| -> (usize, usize) {
                            if vertical {
                                (usize::try_from(i32::try_from(qx).expect("fits") + i).expect("inside"), qy)
                            } else {
                                (qx, usize::try_from(i32::try_from(qy).expect("fits") + i).expect("inside"))
                            }
                        };
                        // The luma sample pair whose bS this line takes.
                        let (lqx, lqy) = if comp == 0 { (qx, qy) } else { (2 * qx, 2 * qy) };
                        let (lpx, lpy) = if vertical { (lqx - 1, lqy) } else { (lqx, lqy - 1) };
                        let bs = boundary_strength(pic, lpx, lpy, lqx, lqy);
                        // qPp, qPq.
                        let pa = (lpy / 16) * w + lpx / 16;
                        let qa = (lqy / 16) * w + lqx / 16;
                        let qz = |a: usize| -> i32 {
                            let q = dbf_qp(&pic.mbs[a]);
                            if comp == 0 { q } else { chroma_qp(q, slice.chroma_qp_offset) }
                        };
                        let qpav = (qz(pa) + qz(qa) + 1) >> 1;
                        let index_a = usize::try_from((qpav + slice.offset_a).clamp(0, 51)).expect("index");
                        let index_b = usize::try_from((qpav + slice.offset_b).clamp(0, 51)).expect("index");
                        let alpha = i32::from(ALPHA[index_a]);
                        let beta = i32::from(BETA[index_b]);
                        let tc0 = if (1..=3).contains(&bs) {
                            i32::from(TC0[usize::try_from(bs - 1).expect("small")][index_a])
                        } else {
                            0
                        };
                        let mut line = [0i32; 8];
                        for i in 0..4i32 {
                            let (x, y) = at(-i - 1);
                            line[usize::try_from(3 - i).expect("small")] = s.at(comp, x, y);
                            let (x, y) = at(i);
                            line[usize::try_from(4 + i).expect("small")] = if comp == 0 || i < 2 {
                                s.at(comp, x, y)
                            } else {
                                // A chroma line reads no further than q1; a
                                // position past the picture is never read.
                                let (pw, ph) = (s.w / 2, s.h / 2);
                                if x < pw && y < ph { s.at(comp, x, y) } else { 0 }
                            };
                        }
                        let out = filter_line(line, bs, comp != 0, alpha, beta, tc0);
                        for i in 0..3i32 {
                            let (x, y) = at(-i - 1);
                            s.put(comp, x, y, out[usize::try_from(3 - i).expect("small")]);
                            let (x, y) = at(i);
                            if comp == 0 || i < 2 {
                                s.put(comp, x, y, out[usize::try_from(4 + i).expect("small")]);
                            }
                        }
                    }
                }
            }
        }
    }
    s
}

/// The deblocking inputs of one macroblock, as `h264_deblock` takes them.
struct DbfParams {
    filt_left: bool,
    filt_top: bool,
    filt_inner: bool,
    bs: u128,
    qp: [i32; 9],
    off_a: i32,
    off_b: i32,
}

/// Works out a macroblock's deblocking inputs with the model's rules.
fn dbf_params(pic: &PicDesc, addr: usize) -> DbfParams {
    let w = pic.w;
    let (mx, my) = (addr % w, addr / w);
    let slice = pic.slice_of(addr);
    let idc = slice.disable_deblocking;
    let same_slice = |n: usize| n >= slice.first_mb;
    let mut bs = 0u128;
    let (x0, y0) = (mx * 16, my * 16);
    for e in 0..4usize {
        for k in 0..4usize {
            if mx > 0 || e > 0 {
                let v = boundary_strength(pic, x0 + 4 * e - 1 + usize::from(e == 0) * 0, y0 + 4 * k, x0 + 4 * e, y0 + 4 * k);
                bs |= u128::from(v) << (3 * (4 * e + k));
            }
            if my > 0 || e > 0 {
                let v = boundary_strength(pic, x0 + 4 * k, y0 + 4 * e - 1, x0 + 4 * k, y0 + 4 * e);
                bs |= u128::from(v) << (48 + 3 * (4 * e + k));
            }
        }
    }
    let q = |a: usize| dbf_qp(&pic.mbs[a]);
    let off = slice.chroma_qp_offset;
    let left = if mx > 0 { addr - 1 } else { addr };
    let top = if my > 0 { addr - w } else { addr };
    DbfParams {
        filt_left: mx > 0 && idc != 1 && !(idc == 2 && !same_slice(addr - 1)),
        filt_top: my > 0 && idc != 1 && !(idc == 2 && !same_slice(addr - w)),
        filt_inner: idc != 1,
        bs,
        qp: [
            q(addr),
            q(left),
            q(top),
            chroma_qp(q(addr), off),
            chroma_qp(q(left), off),
            chroma_qp(q(top), off),
            chroma_qp(q(addr), off),
            chroma_qp(q(left), off),
            chroma_qp(q(top), off),
        ],
        off_a: slice.offset_a,
        off_b: slice.offset_b,
    }
}

/// A macroblock of a picture as the 96 words of a slot.
fn pack_slot(pl: &Planes, mx: usize, my: usize) -> [u32; 96] {
    let mut slot = [0u32; 96];
    for y in 0..16 {
        for w in 0..4 {
            slot[4 * y + w] = (0..4).fold(0u32, |a, i| {
                a | (u32::try_from(pl.at(0, mx * 16 + 4 * w + i, my * 16 + y)).expect("byte") << (8 * i))
            });
        }
    }
    for comp in 1..3 {
        for y in 0..8 {
            for w in 0..2 {
                slot[64 + 16 * (comp - 1) + 2 * y + w] = (0..4).fold(0u32, |a, i| {
                    a | (u32::try_from(pl.at(comp, mx * 8 + 4 * w + i, my * 8 + y)).expect("byte") << (8 * i))
                });
            }
        }
    }
    slot
}

/// Collects `h264_deblock`'s (or the writer's) row records into a
/// picture, checking each sample arrives exactly once.
struct RecordSink {
    pl: Planes,
    seen: Vec<Vec<bool>>,
    header: Option<(usize, usize, usize, usize)>,
    words_left: usize,
    word_at: usize,
    eop: bool,
}

impl RecordSink {
    fn new(w: usize, h: usize) -> RecordSink {
        RecordSink {
            pl: Planes::new(w, h),
            seen: vec![vec![false; w * h * 256], vec![false; w * h * 64], vec![false; w * h * 64]],
            header: None,
            words_left: 0,
            word_at: 0,
            eop: false,
        }
    }

    fn take(&mut self, rec: u64) {
        let kind = rec >> 32;
        let p = rec & 0xFFFF_FFFF;
        match kind {
            1 => {
                assert_eq!(self.words_left, 0, "a header before the last record ended");
                let plane = usize::try_from((p >> 26) & 3).expect("small");
                let y = usize::try_from((p >> 15) & 0x7FF).expect("small");
                let x = usize::try_from((p >> 4) & 0x7FF).expect("small");
                let n = usize::try_from(p & 7).expect("small") + 1;
                self.header = Some((plane, x, y, n));
                self.words_left = n;
                self.word_at = 0;
            }
            0 => {
                let (plane, x, y, _) = self.header.expect("a word before any header");
                assert!(self.words_left > 0, "more words than the header said");
                for i in 0..4 {
                    let xx = x + 4 * self.word_at + i;
                    let v = i32::try_from((p >> (8 * i)) & 0xFF).expect("byte");
                    let pw = if plane == 0 { self.pl.w } else { self.pl.w / 2 };
                    let idx = y * pw + xx;
                    assert!(!self.seen[plane][idx], "plane {plane} ({xx}, {y}) sent twice");
                    self.seen[plane][idx] = true;
                    self.pl.put(plane, xx, y, v);
                }
                self.word_at += 1;
                self.words_left -= 1;
            }
            _ => {
                assert_eq!(self.words_left, 0, "the picture ended inside a record");
                self.eop = true;
            }
        }
    }

    fn complete(&self) -> bool {
        self.seen.iter().all(|p| p.iter().all(|&s| s))
    }
}

/// `h264_deblock` against §8.7 over whole pictures, and every sample
/// sent exactly once.
///
/// The pictures are drawn as for the construction test and constructed
/// by the model, then given to the HDL a macroblock at a time through
/// four emulated slots, as `h264_recon` will. The boundary strengths
/// are the model's, so this checks the filtering and the data movement,
/// not the derivation of bS (which `h264_tokin`'s test does).
///
/// Would catch: an edge filtered out of the order §8.7 sets, a band
/// boundary or the left columns written back to the wrong slot, a
/// chroma edge given the wrong luma bS, qPav taken across the wrong
/// edge, FilterOffsetA and B swapped, disable_deblocking_filter_idc 2
/// filtering across a slice edge, the line buffer holding the wrong
/// rows, and any sample sent twice, late or never.
#[test]
fn deblock_filters_pictures_as_8_7_says() {
    let design = design("h264_deblock");
    let mut b = Bench::new(
        &design,
        &[
            ("go", 1),
            ("mb_x", 7),
            ("mb_y", 7),
            ("last_col", 1),
            ("last_row", 1),
            ("frame", 4),
            ("filt_left", 1),
            ("filt_top", 1),
            ("filt_inner", 1),
            ("t8x8", 1),
            ("bs", 96),
            ("qp_y", 6),
            ("qp_y_left", 6),
            ("qp_y_top", 6),
            ("qp_cb", 6),
            ("qp_cb_left", 6),
            ("qp_cb_top", 6),
            ("qp_cr", 6),
            ("qp_cr_left", 6),
            ("qp_cr_top", 6),
            ("off_a", 5),
            ("off_b", 5),
            ("cur_rd", 32),
            ("left_rd", 32),
            ("op_room", 1),
        ],
        &[
            "busy", "done", "cur_ra", "cur_we", "cur_wa", "cur_wd", "left_ra", "left_we", "left_wa",
            "left_wd", "op_valid", "op_data",
        ],
    );
    let mut rng = Rng(0xDEB1_0003);
    let mut worst = 0u64;
    for round in 0..4 {
        let (w, h) = [(4, 3), (1, 2), (3, 1), (5, 2)][round];
        let pic = draw_picture(&mut rng, w, h, 1 + round % 3, false);
        let mut pic = pic;
        // Real streams code levels; here only `coded` matters, for bS 2.
        // The content is made smooth, with QPs high enough to filter, or
        // the thresholds would reject most lines.
        for mb in pic.mbs.iter_mut() {
            for k in 0..16 {
                mb.levels.coded[k] = rng.below(3) == 0;
            }
            mb.qp = rng.range(20, 51);
            if mb.kind == K_INTER {
                let base = rng.range(40, 200);
                let slope = rng.range(-2, 2);
                for (i, v) in mb.pred.iter_mut().enumerate() {
                    let x = i32::try_from(i % 16).expect("small");
                    *v = u8::try_from(clip1(base + slope * x + rng.range(-1, 1))).expect("byte");
                }
            }
        }
        let mut residuals = Vec::new();
        for mb in &pic.mbs {
            let mut r = draw_residuals(&mut rng, mb).0;
            if mb.kind != K_PCM {
                for blk in r.iter_mut() {
                    let step = i64::from(rng.range(-8, 8));
                    for v in blk.iter_mut() {
                        *v = step + i64::from(rng.range(-1, 1));
                    }
                }
            }
            residuals.push(r);
        }
        let before = construct(&pic, &residuals);
        let want = deblock_picture(&pic, &before);
        let changed = (0..3)
            .map(|c| before.plane(c).0.iter().zip(want.plane(c).0.iter()).filter(|(a, b)| a != b).count())
            .sum::<usize>();
        println!("round {round}: {changed} samples changed by the filter");
        let mut slots = [[0u32; 96]; 4];
        let mut sink = RecordSink::new(w, h);
        let frame = u64::try_from(round).expect("small");
        for addr in 0..pic.mbs.len() {
            let (mx, my) = (addr % w, addr / w);
            slots[addr % 4] = pack_slot(&before, mx, my);
            let prm = dbf_params(&pic, addr);
            b.set("mb_x", u64::try_from(mx).expect("small"));
            b.set("mb_y", u64::try_from(my).expect("small"));
            b.set("last_col", u64::from(mx + 1 == w));
            b.set("last_row", u64::from(my + 1 == h));
            b.set("frame", frame);
            b.set("filt_left", u64::from(prm.filt_left));
            b.set("filt_top", u64::from(prm.filt_top));
            b.set("filt_inner", u64::from(prm.filt_inner));
            b.set("bs", u64::try_from(prm.bs & u128::from(u64::MAX)).expect("low"));
            // `set` takes 64 bits; the top 32 of `bs` go through a wider Logic.
            let bs_net = top_net(&b.sim, "bs");
            b.sim.set(bs_net, logic_u128(96, prm.bs));
            for (k, name) in [
                "qp_y", "qp_y_left", "qp_y_top", "qp_cb", "qp_cb_left", "qp_cb_top", "qp_cr", "qp_cr_left",
                "qp_cr_top",
            ]
            .iter()
            .enumerate()
            {
                b.set(name, u64::try_from(prm.qp[k]).expect("qp"));
            }
            b.set("off_a", u64::try_from(prm.off_a & 31).expect("small"));
            b.set("off_b", u64::try_from(prm.off_b & 31).expect("small"));
            b.set("op_room", 1);
            b.set("go", 1);
            b.cycle();
            b.set("go", 0);
            let start = b.cycles;
            let (cur, left) = (addr % 4, (addr + 3) % 4);
            loop {
                // Back-pressure now and then.
                b.set("op_room", u64::from(rng.below(5) != 0));
                let cra = usize::try_from(b.get("cur_ra")).expect("small");
                let lra = usize::try_from(b.get("left_ra")).expect("small");
                if b.get("cur_we") == 1 {
                    let a = usize::try_from(b.get("cur_wa")).expect("small");
                    slots[cur][a] = u32::try_from(b.get("cur_wd")).expect("word");
                }
                if b.get("left_we") == 1 {
                    let a = usize::try_from(b.get("left_wa")).expect("small");
                    slots[left][a] = u32::try_from(b.get("left_wd")).expect("word");
                }
                if b.get("op_valid") == 1 {
                    let rec = b.get_wide("op_data");
                    sink.take(rec);
                }
                let finished = b.get("done") == 1;
                b.cycle();
                b.set("cur_rd", u64::from(slots[cur][cra.min(95)]));
                b.set("left_rd", u64::from(slots[left][lra.min(95)]));
                if finished {
                    break;
                }
                assert!(b.cycles - start < 20_000, "h264_deblock stalled at macroblock {addr}");
            }
            worst = worst.max(b.cycles - start);
        }
        assert!(sink.eop, "round {round}: no end of picture");
        assert!(sink.complete(), "round {round}: samples never sent");
        assert!(
            sink.pl == want,
            "round {round} ({w}x{h}): the deblocked picture differs: {}",
            first_difference(&sink.pl, &want)
        );
    }
    println!("h264_deblock: at most {worst} cycles a macroblock, with back-pressure");
}
