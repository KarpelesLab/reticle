//! A model of the iCE40 UltraPlus DSP block, `SB_MAC16`, and what
//! `testdata/fpga/ice40up/dsp_selftest.v` should print when it is right.
//!
//! The model is a reading of the iCE Technology Library's cell as Yosys's
//! `techlibs/ice40/cells_sim.v` (module `SB_MAC16`) writes it, cycle by
//! cycle: the four input registers `C`, `A`, `B`, `D`; the four 8 x 8
//! partial products and their registers `F`, `J`, `K`, `G`; their 32-bit
//! sum `L` and its register `H`; and two 16-bit adder/accumulators, the top
//! one (`X + W`, register `Q`) carrying in from the bottom one (`Z + Y`,
//! register `S`). It was written from that file's structure and checked
//! against nothing but the silicon: what the board prints is compared with
//! it, so a disagreement is either the model or the bits, and the board
//! said neither (`docs/fpga-icestorm.md`).
//!
//! `dsp_selftest.v` feeds all eight blocks one stream of vectors and
//! prints, per step, the vector and every block's outputs; the host replays
//! the stream through the model with each block's parameters and wiring
//! ([`BLOCKS`]) and wants every digit equal.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// One block's parameters: every one of `MAC16_PARAMS`, zero when absent.
pub(crate) type Params = BTreeMap<&'static str, u64>;

/// The inputs of one block for one step.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Inputs {
    pub(crate) a: u16,
    pub(crate) b: u16,
    pub(crate) c: u16,
    pub(crate) d: u16,
    pub(crate) ahold: bool,
    pub(crate) bhold: bool,
    pub(crate) chold: bool,
    pub(crate) dhold: bool,
    pub(crate) irsttop: bool,
    pub(crate) irstbot: bool,
    pub(crate) orsttop: bool,
    pub(crate) orstbot: bool,
    pub(crate) oloadtop: bool,
    pub(crate) oloadbot: bool,
    pub(crate) addsubtop: bool,
    pub(crate) addsubbot: bool,
    pub(crate) oholdtop: bool,
    pub(crate) oholdbot: bool,
    pub(crate) ci: bool,
    pub(crate) accumci: bool,
    pub(crate) signextin: bool,
}

/// The registers of one block. They start at zero, which on the board the
/// first step's resets make sure of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    c: u16,
    a: u16,
    b: u16,
    d: u16,
    f: u16,
    j: u16,
    k: u16,
    g: u16,
    h: u32,
    q: u16,
    s: u16,
}

/// What one step of one block gives: the outputs while the inputs are
/// presented, and the registers after one clock with `CE` high.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) o: u32,
    pub(crate) co: bool,
    pub(crate) next: State,
}

/// One step of the block with parameters `p` from registers `st`.
pub(crate) fn step(p: &Params, st: &State, i: &Inputs) -> Step {
    let on = |n: &str| p.get(n).copied().unwrap_or(0) != 0;
    let val = |n: &str| p.get(n).copied().unwrap_or(0);
    // The asynchronous resets act at once.
    let mut st = *st;
    if i.irsttop {
        (st.c, st.a, st.f, st.j) = (0, 0, 0, 0);
    }
    if i.irstbot {
        (st.b, st.d, st.k, st.g, st.h) = (0, 0, 0, 0, 0);
    }
    if i.orsttop {
        st.q = 0;
    }
    if i.orstbot {
        st.s = 0;
    }
    let ic = if on("C_REG") { st.c } else { i.c };
    let ia = if on("A_REG") { st.a } else { i.a };
    let ib = if on("B_REG") { st.b } else { i.b };
    let id = if on("D_REG") { st.d } else { i.d };
    // The multiplier: four 8 x 8 products of 16-bit operands, each half
    // sign-extended or not.
    let (a_signed, b_signed, mode8) = (on("A_SIGNED"), on("B_SIGNED"), on("MODE_8x8"));
    let ext = |byte: u16, signed: bool| -> u16 {
        if signed && byte & 0x80 != 0 {
            0xff00 | byte
        } else {
            byte
        }
    };
    let ah = ext(ia >> 8, a_signed);
    let al = ext(ia & 0xff, a_signed && mode8);
    let bh = ext(ib >> 8, b_signed);
    let bl = ext(ib & 0xff, b_signed && mode8);
    let p_ahbh = ah.wrapping_mul(bh);
    let p_albh = (al & 0xff).wrapping_mul(bh);
    let p_ahbl = ah.wrapping_mul(bl & 0xff);
    let p_albl = al.wrapping_mul(bl);
    let i_f = if on("TOP_8x8_MULT_REG") { st.f } else { p_ahbh };
    let i_j = if on("PIPELINE_16x16_MULT_REG1") {
        st.j
    } else {
        p_albh
    };
    let i_k = if on("PIPELINE_16x16_MULT_REG1") {
        st.k
    } else {
        p_ahbl
    };
    let i_g = if on("BOT_8x8_MULT_REG") { st.g } else { p_albl };
    let ext24 = |v: u16, signed: bool| -> u32 {
        if signed && v & 0x8000 != 0 {
            0x00ff_0000 | u32::from(v)
        } else {
            u32::from(v)
        }
    };
    let i_l = u32::from(i_g)
        .wrapping_add(ext24(i_k, a_signed) << 8)
        .wrapping_add(ext24(i_j, b_signed) << 8)
        .wrapping_add(u32::from(i_f) << 16);
    let i_h = if on("PIPELINE_16x16_MULT_REG2") {
        st.h
    } else {
        i_l
    };
    // The bottom adder first: the top one may carry in from it.
    let mask = |sub: bool| if sub { 0xffffu32 } else { 0 };
    let i_y = if on("BOTADDSUB_UPPERINPUT") { id } else { st.s };
    let i_z = match val("BOTADDSUB_LOWERINPUT") {
        0 => ib,
        1 => i_g,
        2 => (i_h & 0xffff) as u16,
        _ => {
            if i.signextin {
                0xffff
            } else {
                0
            }
        }
    };
    let lci = match val("BOTADDSUB_CARRYSELECT") {
        0 => false,
        1 => true,
        2 => i.accumci,
        _ => i.ci,
    };
    let sum_bot = u32::from(i_z) + (u32::from(i_y) ^ mask(i.addsubbot)) + u32::from(lci);
    let lco = sum_bot >> 16 & 1 != 0;
    let yz = sum_bot & 0xffff;
    let i_r = if i.oloadbot {
        id
    } else {
        ((yz ^ mask(i.addsubbot)) & 0xffff) as u16
    };
    let i_w = if on("TOPADDSUB_UPPERINPUT") { ic } else { st.q };
    let i_x = match val("TOPADDSUB_LOWERINPUT") {
        0 => ia,
        1 => i_f,
        2 => (i_h >> 16) as u16,
        _ => {
            if i_z & 0x8000 != 0 {
                0xffff
            } else {
                0
            }
        }
    };
    let hci = match val("TOPADDSUB_CARRYSELECT") {
        0 => false,
        1 => true,
        2 => lco,
        _ => lco ^ i.addsubbot,
    };
    let sum_top = u32::from(i_x) + (u32::from(i_w) ^ mask(i.addsubtop)) + u32::from(hci);
    let accumco = sum_top >> 16 & 1 != 0;
    let xw = sum_top & 0xffff;
    let i_p = if i.oloadtop {
        ic
    } else {
        ((xw ^ mask(i.addsubtop)) & 0xffff) as u16
    };
    let oh = match val("TOPOUTPUT_SELECT") {
        0 => i_p,
        1 => st.q,
        2 => i_f,
        _ => (i_h >> 16) as u16,
    };
    let ol = match val("BOTOUTPUT_SELECT") {
        0 => i_r,
        1 => st.s,
        2 => i_g,
        _ => (i_h & 0xffff) as u16,
    };
    // One clock with CE high.
    let mut next = st;
    if !i.irsttop {
        if !i.chold {
            next.c = i.c;
        }
        if !i.ahold {
            next.a = i.a;
        }
        next.f = p_ahbh;
        if !mode8 {
            next.j = p_albh;
        }
    }
    if !i.irstbot {
        if !i.bhold {
            next.b = i.b;
        }
        if !i.dhold {
            next.d = i.d;
        }
        if !mode8 {
            next.k = p_ahbl;
            next.h = i_l;
        }
        next.g = p_albl;
    }
    if !i.orsttop && !i.oholdtop {
        next.q = i_p;
    }
    if !i.orstbot && !i.oholdbot {
        next.s = i_r;
    }
    Step {
        o: u32::from(oh) << 16 | u32::from(ol),
        co: accumco ^ i.addsubtop,
        next,
    }
}

/// The control byte of one step: `{3'b0, ci, hold, addsub, oload, rst}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Vector {
    pub(crate) k: u8,
    pub(crate) a: u16,
    pub(crate) b: u16,
    pub(crate) c: u16,
    pub(crate) d: u16,
}

/// How one block of `dsp_selftest.v` is wired to the step's vector.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wiring {
    /// `C` and `D` connected (otherwise zero).
    pub(crate) cd: bool,
    /// `OLOADTOP` and `OLOADBOT` on `oload`.
    pub(crate) oload: bool,
    /// `ADDSUBTOP` on `addsub`.
    pub(crate) addsub_top: bool,
    /// `ADDSUBBOT` on `addsub`.
    pub(crate) addsub_bot: bool,
    /// `AHOLD` and `OHOLDBOT` on `hold`.
    pub(crate) hold: bool,
    /// `CI` on `ci`.
    pub(crate) ci: bool,
}

/// A block of `dsp_selftest.v`: its instance name, parameters, wiring.
pub(crate) type Block = (&'static str, &'static [(&'static str, u64)], Wiring);

const W: Wiring = Wiring {
    cd: false,
    oload: false,
    addsub_top: false,
    addsub_bot: false,
    hold: false,
    ci: false,
};

/// The eight blocks of `dsp_selftest.v`, in the order it prints them; this
/// must say what the RTL's `#(...)` and port lists say.
pub(crate) const BLOCKS: [Block; 8] = [
    (
        "mul_u",
        &[("TOPOUTPUT_SELECT", 3), ("BOTOUTPUT_SELECT", 3)],
        W,
    ),
    (
        "mul_s",
        &[
            ("TOPOUTPUT_SELECT", 3),
            ("BOTOUTPUT_SELECT", 3),
            ("A_SIGNED", 1),
            ("B_SIGNED", 1),
        ],
        W,
    ),
    (
        "mul_p",
        &[
            ("TOPOUTPUT_SELECT", 3),
            ("BOTOUTPUT_SELECT", 3),
            ("A_SIGNED", 1),
            ("A_REG", 1),
            ("B_REG", 1),
            ("TOP_8x8_MULT_REG", 1),
            ("BOT_8x8_MULT_REG", 1),
            ("PIPELINE_16x16_MULT_REG1", 1),
            ("PIPELINE_16x16_MULT_REG2", 1),
        ],
        W,
    ),
    (
        "mul_8",
        &[
            ("MODE_8x8", 1),
            ("TOPOUTPUT_SELECT", 2),
            ("BOTOUTPUT_SELECT", 2),
            ("A_SIGNED", 1),
            ("TOP_8x8_MULT_REG", 1),
        ],
        W,
    ),
    (
        "mad_8",
        &[
            ("MODE_8x8", 1),
            ("D_REG", 1),
            ("TOPADDSUB_LOWERINPUT", 1),
            ("TOPADDSUB_UPPERINPUT", 1),
            ("BOTADDSUB_LOWERINPUT", 1),
            ("BOTADDSUB_UPPERINPUT", 1),
        ],
        Wiring { cd: true, ..W },
    ),
    (
        "mac",
        &[
            ("A_REG", 1),
            ("B_REG", 1),
            ("C_REG", 1),
            ("TOPOUTPUT_SELECT", 1),
            ("BOTOUTPUT_SELECT", 1),
            ("TOPADDSUB_LOWERINPUT", 2),
            ("BOTADDSUB_LOWERINPUT", 2),
            ("TOPADDSUB_CARRYSELECT", 2),
        ],
        Wiring {
            cd: true,
            oload: true,
            ..W
        },
    ),
    (
        "mac_s",
        &[
            ("A_SIGNED", 1),
            ("B_SIGNED", 1),
            ("A_REG", 1),
            ("B_REG", 1),
            ("C_REG", 1),
            ("D_REG", 1),
            ("TOPOUTPUT_SELECT", 1),
            ("BOTOUTPUT_SELECT", 1),
            ("TOPADDSUB_LOWERINPUT", 2),
            ("BOTADDSUB_LOWERINPUT", 2),
            ("TOPADDSUB_CARRYSELECT", 3),
            ("BOTADDSUB_CARRYSELECT", 1),
        ],
        Wiring {
            cd: true,
            oload: true,
            addsub_top: true,
            addsub_bot: true,
            hold: true,
            ci: false,
        },
    ),
    (
        "add32",
        &[
            ("TOPADDSUB_UPPERINPUT", 1),
            ("BOTADDSUB_UPPERINPUT", 1),
            ("TOPADDSUB_CARRYSELECT", 2),
            ("BOTADDSUB_CARRYSELECT", 3),
        ],
        Wiring {
            cd: true,
            addsub_top: true,
            ci: true,
            ..W
        },
    ),
];

/// A block's parameters as a map.
pub(crate) fn params(block: &Block) -> Params {
    block.1.iter().copied().collect()
}

/// The 64 vectors `dsp_selftest.v` presents, as its RTL computes them.
pub(crate) fn vectors() -> Vec<Vector> {
    let lfsr = |s: u32| {
        if s & 1 != 0 {
            (s >> 1) ^ 0x8020_0003
        } else {
            s >> 1
        }
    };
    let edges: [u32; 8] = [
        0x0000_0000,
        0xffff_ffff,
        0x8000_8000,
        0xffff_8000,
        0x7fff_8000,
        0x0001_ffff,
        0x00ff_ff00,
        0x8001_7fff,
    ];
    let (mut lab, mut lcd) = (0x1234_5678u32, 0x9abc_def1u32);
    let mut out = Vec::new();
    for step in 0..64u32 {
        let ab = if step < 8 { edges[step as usize] } else { lab };
        let rst = step == 0;
        let oload = step % 8 == 1;
        let addsub = lcd & 1 != 0;
        let hold = lcd >> 5 & 1 != 0 && lcd >> 9 & 1 != 0;
        let ci = lab >> 7 & 1 != 0;
        let k = u8::from(rst)
            | u8::from(oload) << 1
            | u8::from(addsub) << 2
            | u8::from(hold) << 3
            | u8::from(ci) << 4;
        out.push(Vector {
            k,
            a: (ab >> 16) as u16,
            b: (ab & 0xffff) as u16,
            c: (lcd >> 16) as u16,
            d: (lcd & 0xffff) as u16,
        });
        lab = lfsr(lab);
        lcd = lfsr(lcd);
    }
    out
}

/// One block's inputs for one vector.
pub(crate) fn inputs(v: &Vector, w: &Wiring) -> Inputs {
    let bit = |n: u32| v.k >> n & 1 != 0;
    let (rst, oload, addsub, hold, ci) = (bit(0), bit(1), bit(2), bit(3), bit(4));
    Inputs {
        a: v.a,
        b: v.b,
        c: if w.cd { v.c } else { 0 },
        d: if w.cd { v.d } else { 0 },
        irsttop: rst,
        irstbot: rst,
        orsttop: rst,
        orstbot: rst,
        oloadtop: w.oload && oload,
        oloadbot: w.oload && oload,
        addsubtop: w.addsub_top && addsub,
        addsubbot: w.addsub_bot && addsub,
        ahold: w.hold && hold,
        oholdbot: w.hold && hold,
        ci: w.ci && ci,
        ..Inputs::default()
    }
}

/// What one block prints over the whole stream: `(O, CO)` per step.
pub(crate) fn replay(p: &Params, w: &Wiring, vs: &[Vector]) -> Vec<(u32, bool)> {
    let mut st = State::default();
    let mut out = Vec::new();
    for v in vs {
        let s = step(p, &st, &inputs(v, w));
        out.push((s.o, s.co));
        st = s.next;
    }
    out
}

/// The 64 lines `dsp_selftest.v` should print, then `END`.
pub(crate) fn expected_report() -> String {
    let vs = vectors();
    let results: Vec<Vec<(u32, bool)>> = BLOCKS
        .iter()
        .map(|b| replay(&params(b), &b.2, &vs))
        .collect();
    let mut out = String::new();
    for (n, v) in vs.iter().enumerate() {
        out.push_str(&format!(
            "{:02x}{:04x}{:04x}{:04x}{:04x}",
            v.k, v.a, v.b, v.c, v.d
        ));
        let mut co = 0u8;
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!("{:08x}", r[n].0));
            co |= u8::from(r[n].1) << i;
        }
        out.push_str(&format!("{co:02x}\r\n"));
    }
    out.push_str("END\r\n");
    out
}

/// Compares a report from the board with [`expected_report`], naming every
/// field that differs: the vector, a block's `O` or the carry outs.
pub(crate) fn check_report(text: &str) -> Vec<String> {
    let want = expected_report();
    let want: Vec<&str> = want.split("\r\n").filter(|l| !l.is_empty()).collect();
    let got: Vec<&str> = text.split(['\r', '\n']).filter(|l| !l.is_empty()).collect();
    let mut problems = Vec::new();
    if got.len() != want.len() {
        problems.push(format!(
            "{} line(s) where {} were expected",
            got.len(),
            want.len()
        ));
    }
    for (n, (g, w)) in got.iter().zip(&want).enumerate() {
        if g == w {
            continue;
        }
        if g.len() != w.len() {
            problems.push(format!("line {n}: {g:?} is not {} characters", w.len()));
            continue;
        }
        let field = |s: &str, from: usize, len: usize| s[from..from + len].to_owned();
        if g[..18] != w[..18] {
            problems.push(format!(
                "line {n}: the vector reads {} for {}",
                field(g, 0, 18),
                field(w, 0, 18)
            ));
        }
        for (i, block) in BLOCKS.iter().enumerate() {
            let at = 18 + 8 * i;
            if g[at..at + 8] != w[at..at + 8] {
                problems.push(format!(
                    "line {n}: {} gives {} where the model gives {}",
                    block.0,
                    field(g, at, 8),
                    field(w, at, 8)
                ));
            }
        }
        if g[82..] != w[82..] {
            problems.push(format!(
                "line {n}: the carry outs read {} for {}",
                field(g, 82, 2),
                field(w, 82, 2)
            ));
        }
    }
    problems
}
