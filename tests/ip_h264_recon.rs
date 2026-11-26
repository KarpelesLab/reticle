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
use reticle::logic::Logic;
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
