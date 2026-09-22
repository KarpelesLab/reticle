//! Arithmetic lowering: every architecture proved, then measured.
//!
//! `reticle::synth::arith` offers a choice of adder, multiplier,
//! comparator, shifter and divider architectures. A choice is only
//! worth having if each option is *right* and if the differences
//! between them are real, so this file does two things.
//!
//! **Proof.** For every architecture and a spread of widths — 1, 2, 3,
//! 7, 8, 13 and 16, which covers one bit, powers of two, one below a
//! power of two and two odd widths — a module is built twice: once out
//! of the generic cells the pass replaces (`add`, `mul`, `lt`, `shl`,
//! `div`, …) and once out of the architecture. The two are then proved
//! equivalent with `formal::check_equivalent`, which for combinational
//! modules is a complete decision procedure, not a sample. Signed and
//! unsigned are separate cases wherever the operation has them. The
//! carry in and carry out of the adders, and the borrow of the
//! subtractors, are ports of both modules, so the chaining interface is
//! proved too and not merely the sum.
//!
//! Two exceptions, both stated rather than hidden. The **multipliers**
//! are proved at 1, 2, 3, 7 and 8 bits in every run (the full `2n`-bit
//! product to 6); at 13 and 16 the miter of two structurally different
//! multipliers is the textbook hard instance for a CDCL solver, so that
//! proof is a separate `#[ignore]`d test and those widths are covered in
//! the ordinary run by
//! `wide_multipliers_match_over_random_vectors`, which is a check and
//! not a proof. The **dividers** are proved to 8 bits, which is the
//! default width threshold: above it the pass leaves the cell generic,
//! so there is nothing there to prove.
//!
//! **Measurement.** The same builders produce the table in
//! `docs/arithmetic.md`: cells and estimated combinational depth, from
//! `synth::report`, for each architecture at 8, 16 and 32 bits. The
//! table is compared against the document byte for byte, so it cannot
//! drift; run with `UPDATE_EXPECT=1` to rewrite it after an intended
//! change and read the diff. `architectures_are_shallower_than_ripple`
//! then asserts the shape of those numbers: a Kogge-Stone adder that is
//! not shallower than a ripple carry means something is wrong, and it
//! is better to fail here than to publish a table saying so.
//!
//! The equivalence proofs need the `formal` feature and live in
//! `mod equivalence`; everything else needs only `synth`.

#![cfg(feature = "synth")]

mod common;

use std::fmt::Write as _;

use reticle::diag::Diagnostics;
use reticle::ir::builder::ModuleBuilder;
use reticle::ir::validate::validate_module;
use reticle::ir::{CellKind, Design, Module, Name, Type};
use reticle::source::{SourceMap, Span};
use reticle::synth::Pass;
use reticle::synth::arith::{
    AdderArch, AdderOptions, ArithLower, ArithOptions, CompareArch, GateBuilder, MultiplierArch,
    MultiplierOptions, ShiftKind, ShifterArch, WidthThresholds, adder, cmp, div, dsp_candidates,
    mul, shift,
};
use reticle::synth::report::Report;

/// The widths every architecture is proved at.
#[cfg(feature = "formal")]
const WIDTHS: [u32; 7] = [1, 2, 3, 7, 8, 13, 16];

/// The widths the multipliers are proved at in every run; see
/// `equivalence::wide_multipliers_match_the_generic_cell` for the other
/// two and why they are apart.
#[cfg(feature = "formal")]
const NARROW: [u32; 5] = [1, 2, 3, 7, 8];

/// The widths the published table reports.
const TABLE_WIDTHS: [u32; 3] = [8, 16, 32];

fn span() -> Span {
    let mut map = SourceMap::new();
    let file = map.add("arith", "").unwrap();
    Span::new(file, 0, 0)
}

/// Runs `f` with a gate builder over `b`.
fn with_gates<R>(
    b: &mut ModuleBuilder,
    prefix: &str,
    f: impl FnOnce(&mut GateBuilder<'_>) -> R,
) -> R {
    let mut cx = GateBuilder::new(b, prefix);
    f(&mut cx)
}

/// Checks a built module against the IR rules; a pass that produces an
/// invalid module is a bug even if it happens to be equivalent.
fn checked(module: Module) -> Module {
    let problems = validate_module(&module);
    assert!(
        !problems.has_errors(),
        "`{}` is not a valid module:\n{}",
        module.name,
        problems
            .iter()
            .map(|d| format!("  {}", d.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
    module
}

// ---------------------------------------------------------------------------
// Adders and subtractors
// ---------------------------------------------------------------------------

/// `{cout, sum} = a + b + cin`, out of generic `add` cells.
#[cfg(feature = "formal")]
fn adder_reference(name: &str, w: u32) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", Type::bits(w));
    let bn = b.input("b", Type::bits(w));
    let cin = b.input("cin", Type::bit());
    let sum = b.output("sum", Type::bits(w));
    let cout = b.output("cout", Type::bit());
    let wide = w + 1;
    let (av, bv, cv) = (b.net(a), b.net(bn), b.net(cin));
    let (za, zb, zc) = (b.zext(av, wide), b.zext(bv, wide), b.zext(cv, wide));
    let t = b.add_net("t", Type::bits(wide));
    b.cell2("add0", CellKind::Add, za, zb, t);
    let tv = b.net(t);
    let u = b.add_net("u", Type::bits(wide));
    b.cell2("add1", CellKind::Add, tv, zc, u);
    let uv = b.net(u);
    let (lo, hi) = (b.slice(uv, w - 1, 0), b.slice(uv, w, w));
    b.assign(sum, lo);
    b.assign(cout, hi);
    checked(b.finish())
}

/// The same interface, built by one of the adder architectures.
fn adder_built(name: &str, w: u32, options: AdderOptions) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", Type::bits(w));
    let bn = b.input("b", Type::bits(w));
    let cin = b.input("cin", Type::bit());
    let sum = b.output("sum", Type::bits(w));
    let cout = b.output("cout", Type::bit());
    let (av, bv, cv) = (b.net(a), b.net(bn), b.net(cin));
    let (sum_expr, cout_expr) = with_gates(&mut b, "u", |cx| {
        let (ab, bb) = (cx.split(av, w), cx.split(bv, w));
        let s = adder::add(cx, &options, &ab, &bb, cv);
        (cx.join(&s.bits), s.carry_out)
    });
    b.assign(sum, sum_expr);
    b.assign(cout, cout_expr);
    checked(b.finish())
}

/// `{borrow, diff} = a - b - bin`, out of generic `sub` cells.
#[cfg(feature = "formal")]
fn subtractor_reference(name: &str, w: u32) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", Type::bits(w));
    let bn = b.input("b", Type::bits(w));
    let bin = b.input("bin", Type::bit());
    let diff = b.output("diff", Type::bits(w));
    let borrow = b.output("borrow", Type::bit());
    let wide = w + 1;
    let (av, bv, cv) = (b.net(a), b.net(bn), b.net(bin));
    let (za, zb, zc) = (b.zext(av, wide), b.zext(bv, wide), b.zext(cv, wide));
    let t = b.add_net("t", Type::bits(wide));
    b.cell2("sub0", CellKind::Sub, za, zb, t);
    let tv = b.net(t);
    let u = b.add_net("u", Type::bits(wide));
    b.cell2("sub1", CellKind::Sub, tv, zc, u);
    let uv = b.net(u);
    let (lo, hi) = (b.slice(uv, w - 1, 0), b.slice(uv, w, w));
    b.assign(diff, lo);
    b.assign(borrow, hi);
    checked(b.finish())
}

/// The same interface, built by one of the adder architectures.
fn subtractor_built(name: &str, w: u32, options: AdderOptions) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", Type::bits(w));
    let bn = b.input("b", Type::bits(w));
    let bin = b.input("bin", Type::bit());
    let diff = b.output("diff", Type::bits(w));
    let borrow = b.output("borrow", Type::bit());
    let (av, bv, cv) = (b.net(a), b.net(bn), b.net(bin));
    let (diff_expr, borrow_expr) = with_gates(&mut b, "u", |cx| {
        let (ab, bb) = (cx.split(av, w), cx.split(bv, w));
        let d = adder::subtract(cx, &options, &ab, &bb, cv);
        (cx.join(&d.bits), d.borrow_out)
    });
    b.assign(diff, diff_expr);
    b.assign(borrow, borrow_expr);
    checked(b.finish())
}

// ---------------------------------------------------------------------------
// Multipliers
// ---------------------------------------------------------------------------

fn operand_type(w: u32, signed: bool) -> Type {
    if signed {
        Type::sbits(w)
    } else {
        Type::bits(w)
    }
}

/// `y = a * b` at `out` bits, out of a generic `mul` cell: the operands
/// are resized to the output width first, which is what makes the
/// product a full one when `out` is `2w`.
#[cfg(feature = "formal")]
fn multiplier_reference(name: &str, w: u32, signed: bool, out: u32) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty);
    let y = b.output("y", Type::bits(out));
    let (av, bv) = (b.net(a), b.net(bn));
    let (ra, rb) = (b.resize(av, out, signed), b.resize(bv, out, signed));
    let p = b.add_net("p", Type::bits(out));
    b.cell2("mul0", CellKind::Mul, ra, rb, p);
    let pv = b.net(p);
    b.assign(y, pv);
    checked(b.finish())
}

/// The same interface, built by one of the multiplier architectures.
fn multiplier_built(
    name: &str,
    w: u32,
    signed: bool,
    out: u32,
    options: MultiplierOptions,
) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty);
    let y = b.output("y", Type::bits(out));
    let (av, bv) = (b.net(a), b.net(bn));
    let product = with_gates(&mut b, "u", |cx| {
        let (ab, bb) = (cx.split(av, w), cx.split(bv, w));
        let bits = mul::multiply(
            cx,
            &options,
            &ab,
            &bb,
            signed,
            usize::try_from(out).expect("a width fits in a usize"),
        );
        cx.join(&bits)
    });
    b.assign(y, product);
    checked(b.finish())
}

// ---------------------------------------------------------------------------
// Comparators
// ---------------------------------------------------------------------------

/// The comparison operators, with the cell each is proved against.
#[cfg(feature = "formal")]
const COMPARISONS: [(&str, CellKind); 6] = [
    ("eq", CellKind::Eq),
    ("ne", CellKind::Ne),
    ("lt", CellKind::Lt),
    ("le", CellKind::Le),
    ("gt", CellKind::Gt),
    ("ge", CellKind::Ge),
];

#[cfg(feature = "formal")]
fn comparator_reference(name: &str, w: u32, signed: bool, kind: CellKind) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty);
    let y = b.output("y", Type::bit());
    let (av, bv) = (b.net(a), b.net(bn));
    let t = b.add_net("t", Type::bit());
    b.cell2("cmp0", kind, av, bv, t);
    let tv = b.net(t);
    b.assign(y, tv);
    checked(b.finish())
}

fn comparator_built(
    name: &str,
    w: u32,
    signed: bool,
    kind: &CellKind,
    arch: CompareArch,
) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty);
    let y = b.output("y", Type::bit());
    let (av, bv) = (b.net(a), b.net(bn));
    let verdict = with_gates(&mut b, "u", |cx| {
        let (ab, bb) = (cx.split(av, w), cx.split(bv, w));
        match kind {
            CellKind::Eq => cmp::equal(cx, arch, &ab, &bb),
            CellKind::Ne => {
                let e = cmp::equal(cx, arch, &ab, &bb);
                cx.not(e)
            }
            CellKind::Lt => cmp::less_than(cx, arch, &ab, &bb, signed),
            CellKind::Gt => cmp::less_than(cx, arch, &bb, &ab, signed),
            CellKind::Le => {
                let gt = cmp::less_than(cx, arch, &bb, &ab, signed);
                cx.not(gt)
            }
            CellKind::Ge => {
                let lt = cmp::less_than(cx, arch, &ab, &bb, signed);
                cx.not(lt)
            }
            other => panic!("not a comparison: {other:?}"),
        }
    });
    b.assign(y, verdict);
    checked(b.finish())
}

/// Both verdicts of [`cmp::compare`] at once, which is the entry point
/// a caller that needs `<` and `==` together uses.
fn compare_pair_built(name: &str, w: u32, signed: bool, arch: CompareArch) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty);
    let lt = b.output("lt", Type::bit());
    let eq = b.output("eq", Type::bit());
    let le = b.output("le", Type::bit());
    let (av, bv) = (b.net(a), b.net(bn));
    let (lt_e, eq_e, le_e) = with_gates(&mut b, "u", |cx| {
        let (ab, bb) = (cx.split(av, w), cx.split(bv, w));
        let c = cmp::compare(cx, arch, &ab, &bb, signed);
        (c.lt, c.eq, c.le(cx))
    });
    b.assign(lt, lt_e);
    b.assign(eq, eq_e);
    b.assign(le, le_e);
    checked(b.finish())
}

#[cfg(feature = "formal")]
fn compare_pair_reference(name: &str, w: u32, signed: bool) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty);
    let lt = b.output("lt", Type::bit());
    let eq = b.output("eq", Type::bit());
    let le = b.output("le", Type::bit());
    let (av, bv) = (b.net(a), b.net(bn));
    for (name, kind, port) in [
        ("c0", CellKind::Lt, lt),
        ("c1", CellKind::Eq, eq),
        ("c2", CellKind::Le, le),
    ] {
        let t = b.add_net(format!("{name}_t"), Type::bit());
        b.cell2(name, kind, av, bv, t);
        let tv = b.net(t);
        b.assign(port, tv);
    }
    checked(b.finish())
}

// ---------------------------------------------------------------------------
// Shifters
// ---------------------------------------------------------------------------

/// The shift operators, with the cell each is proved against and
/// whether the left operand has to be signed for it.
const SHIFTS: [(&str, CellKind, ShiftKind, bool); 3] = [
    ("shl", CellKind::Shl, ShiftKind::Left, false),
    ("shr", CellKind::Shr, ShiftKind::RightLogical, false),
    ("sshr", CellKind::Sshr, ShiftKind::RightArithmetic, true),
];

#[cfg(feature = "formal")]
fn shifter_reference(name: &str, w: u32, aw: u32, signed: bool, kind: CellKind) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", operand_type(w, signed));
    let amt = b.input("amt", Type::bits(aw));
    let y = b.output("y", Type::bits(w));
    let (av, amv) = (b.net(a), b.net(amt));
    let t = b.add_net("t", operand_type(w, signed));
    b.cell2("sh0", kind, av, amv, t);
    let tv = b.net(t);
    b.assign(y, tv);
    checked(b.finish())
}

fn shifter_built(
    name: &str,
    w: u32,
    aw: u32,
    signed: bool,
    kind: ShiftKind,
    arch: ShifterArch,
) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", operand_type(w, signed));
    let amt = b.input("amt", Type::bits(aw));
    let y = b.output("y", Type::bits(w));
    let (av, amv) = (b.net(a), b.net(amt));
    let result = with_gates(&mut b, "u", |cx| {
        let ab = cx.split(av, w);
        let amb = cx.split(amv, aw);
        let bits = shift::shift(cx, arch, &ab, &amb, kind);
        cx.join(&bits)
    });
    b.assign(y, result);
    checked(b.finish())
}

/// `y = ({hi, lo} >> amt)[w-1:0]` (or the top window of a left shift),
/// out of a generic shift cell over the doubled word.
#[cfg(feature = "formal")]
fn funnel_reference(name: &str, w: u32, aw: u32, left: bool) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let hi = b.input("hi", Type::bits(w));
    let lo = b.input("lo", Type::bits(w));
    let amt = b.input("amt", Type::bits(aw));
    let y = b.output("y", Type::bits(w));
    let (hv, lv, amv) = (b.net(hi), b.net(lo), b.net(amt));
    let wide = b.concat(vec![hv, lv]);
    let t = b.add_net("t", Type::bits(2 * w));
    let kind = if left { CellKind::Shl } else { CellKind::Shr };
    b.cell2("sh0", kind, wide, amv, t);
    let tv = b.net(t);
    let window = if left {
        b.slice(tv, 2 * w - 1, w)
    } else {
        b.slice(tv, w - 1, 0)
    };
    b.assign(y, window);
    checked(b.finish())
}

#[cfg(feature = "formal")]
fn funnel_built(name: &str, w: u32, aw: u32, left: bool, radix: u32) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let hi = b.input("hi", Type::bits(w));
    let lo = b.input("lo", Type::bits(w));
    let amt = b.input("amt", Type::bits(aw));
    let y = b.output("y", Type::bits(w));
    let (hv, lv, amv) = (b.net(hi), b.net(lo), b.net(amt));
    let result = with_gates(&mut b, "u", |cx| {
        let hb = cx.split(hv, w);
        let lb = cx.split(lv, w);
        let amb = cx.split(amv, aw);
        let bits = if left {
            shift::funnel_left(cx, &hb, &lb, &amb, radix)
        } else {
            shift::funnel_right(cx, &hb, &lb, &amb, radix)
        };
        cx.join(&bits)
    });
    b.assign(y, result);
    checked(b.finish())
}

/// `y = (a << amt) | (a >> (w - amt))` with the subtraction masked, the
/// textbook rotate, out of generic cells. Only used at power-of-two
/// widths, where `amt` covers exactly `0 .. w`.
#[cfg(feature = "formal")]
fn rotate_reference(name: &str, w: u32, aw: u32, left: bool) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", Type::bits(w));
    let amt = b.input("amt", Type::bits(aw));
    let y = b.output("y", Type::bits(w));
    let (av, amv) = (b.net(a), b.net(amt));
    let zamt = b.zext(amv, w);
    let width_const = b.const_u64(w, u64::from(w));
    let mask = b.const_u64(w, u64::from(w - 1));
    let complement = b.add_net("complement", Type::bits(w));
    b.cell2("sub0", CellKind::Sub, width_const, zamt, complement);
    let cv = b.net(complement);
    let masked = b.add_net("masked", Type::bits(w));
    b.cell2("and0", CellKind::And, cv, mask, masked);
    let mv = b.net(masked);
    let up = b.add_net("up", Type::bits(w));
    let down = b.add_net("down", Type::bits(w));
    let (first, second) = if left { (zamt, mv) } else { (mv, zamt) };
    b.cell2("shl0", CellKind::Shl, av, first, up);
    b.cell2("shr0", CellKind::Shr, av, second, down);
    let (uv, dv) = (b.net(up), b.net(down));
    let both = b.add_net("both", Type::bits(w));
    b.cell2("or0", CellKind::Or, uv, dv, both);
    let bv = b.net(both);
    b.assign(y, bv);
    checked(b.finish())
}

fn rotate_built(name: &str, w: u32, aw: u32, left: bool, radix: u32) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let a = b.input("a", Type::bits(w));
    let amt = b.input("amt", Type::bits(aw));
    let y = b.output("y", Type::bits(w));
    let (av, amv) = (b.net(a), b.net(amt));
    let result = with_gates(&mut b, "u", |cx| {
        let ab = cx.split(av, w);
        let amb = cx.split(amv, aw);
        let bits = if left {
            shift::rotate_left(cx, &ab, &amb, radix)
        } else {
            shift::rotate_right(cx, &ab, &amb, radix)
        };
        cx.join(&bits)
    });
    b.assign(y, result);
    checked(b.finish())
}

// ---------------------------------------------------------------------------
// Dividers
// ---------------------------------------------------------------------------

#[cfg(feature = "formal")]
fn divider_reference(name: &str, w: u32, signed: bool) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty.clone());
    let q = b.output("q", Type::bits(w));
    let r = b.output("r", Type::bits(w));
    let (av, bv) = (b.net(a), b.net(bn));
    let qn = b.add_net("qn", ty.clone());
    let rn = b.add_net("rn", ty);
    b.cell2("div0", CellKind::Div, av, bv, qn);
    b.cell2("mod0", CellKind::Mod, av, bv, rn);
    let (qv, rv) = (b.net(qn), b.net(rn));
    b.assign(q, qv);
    b.assign(r, rv);
    checked(b.finish())
}

fn divider_built(name: &str, w: u32, signed: bool, options: AdderOptions) -> Module {
    let mut b = ModuleBuilder::new(name, span());
    let ty = operand_type(w, signed);
    let a = b.input("a", ty.clone());
    let bn = b.input("b", ty);
    let q = b.output("q", Type::bits(w));
    let r = b.output("r", Type::bits(w));
    let (av, bv) = (b.net(a), b.net(bn));
    let (qe, re) = with_gates(&mut b, "u", |cx| {
        let (ab, bb) = (cx.split(av, w), cx.split(bv, w));
        let d = div::divide(cx, &options, &ab, &bb, signed);
        (cx.join(&d.quotient), cx.join(&d.remainder))
    });
    b.assign(q, qe);
    b.assign(r, re);
    checked(b.finish())
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// Cells and estimated combinational depth of a module.
fn measure(module: &Module) -> (usize, usize) {
    let report = Report::of_module(module);
    let cells = report.cells.iter().map(|(_, n)| n).sum();
    (cells, report.depth)
}

/// One row of the published table: a name and a measurement per width.
fn row(label: &str, mut build: impl FnMut(u32) -> Module) -> String {
    let mut line = format!("| {label} |");
    for w in TABLE_WIDTHS {
        let (cells, depth) = measure(&build(w));
        let _ = write!(line, " {cells} | {depth} |");
    }
    line.push('\n');
    line
}

fn table_header() -> String {
    let mut head = String::from("| Architecture |");
    for w in TABLE_WIDTHS {
        let _ = write!(head, " {w}-bit cells | depth |");
    }
    head.push_str("\n|---|");
    for _ in TABLE_WIDTHS {
        head.push_str("---|---|");
    }
    head.push('\n');
    head
}

/// The whole generated section of `docs/arithmetic.md`.
fn measurement_tables() -> String {
    let mut out = String::new();

    out.push_str("#### Adders (`a + b + cin`, with the carry out)\n\n");
    out.push_str(&table_header());
    for arch in AdderArch::ALL {
        out.push_str(&row(arch.name(), |w| {
            adder_built("m", w, AdderOptions::new(arch))
        }));
    }

    out.push_str("\n#### Subtractors (`a - b - bin`, with the borrow out)\n\n");
    out.push_str(&table_header());
    for arch in AdderArch::ALL {
        out.push_str(&row(arch.name(), |w| {
            subtractor_built("m", w, AdderOptions::new(arch))
        }));
    }

    out.push_str("\n#### Multipliers, truncated (`n x n -> n`, the `mul` cell)\n\n");
    out.push_str(&table_header());
    for arch in MultiplierArch::ALL {
        out.push_str(&row(arch.name(), |w| {
            multiplier_built("m", w, false, w, MultiplierOptions::new(arch))
        }));
    }

    out.push_str("\n#### Multipliers, full signed product (`n x n -> 2n`)\n\n");
    out.push_str(&table_header());
    for arch in MultiplierArch::ALL {
        out.push_str(&row(arch.name(), |w| {
            multiplier_built("m", w, true, 2 * w, MultiplierOptions::new(arch))
        }));
    }

    out.push_str("\n#### Comparators\n\n");
    out.push_str(&table_header());
    for arch in CompareArch::ALL {
        for (label, kind) in [("==", CellKind::Eq), ("<", CellKind::Lt)] {
            out.push_str(&row(&format!("{} `{label}`", arch.name()), |w| {
                comparator_built("m", w, false, &kind, arch)
            }));
        }
        out.push_str(&row(&format!("{} `<` and `==`", arch.name()), |w| {
            compare_pair_built("m", w, false, arch)
        }));
    }

    out.push_str("\n#### Shifters (amount as wide as the operand)\n\n");
    out.push_str(&table_header());
    for arch in ShifterArch::ALL {
        for (label, _, kind, signed) in SHIFTS {
            out.push_str(&row(&format!("{} `{label}`", arch.name()), |w| {
                shifter_built("m", w, w, signed, kind, arch)
            }));
        }
    }

    out.push_str("\n#### Rotates (funnel network, amount masked to the width)\n\n");
    out.push_str(&table_header());
    for (label, left) in [("rotate left", true), ("rotate right", false)] {
        out.push_str(&row(label, |w| {
            rotate_built("m", w, w.trailing_zeros().max(1), left, 2)
        }));
    }

    out.push_str("\n#### Dividers (restoring array, `q` and `r` together)\n\n");
    out.push_str("| Architecture | 4-bit cells | depth | 8-bit cells | depth |\n");
    out.push_str("|---|---|---|---|---|\n");
    for arch in [AdderArch::Ripple, AdderArch::KoggeStone] {
        for signed in [false, true] {
            let label = format!(
                "{} inner adder, {}",
                arch.name(),
                if signed { "signed" } else { "unsigned" }
            );
            let mut line = format!("| {label} |");
            for w in [4u32, 8] {
                let m = divider_built("m", w, signed, AdderOptions::new(arch));
                let (cells, depth) = measure(&m);
                let _ = write!(line, " {cells} | {depth} |");
            }
            line.push('\n');
            out.push_str(&line);
        }
    }
    out
}

/// Where the generated tables live in the document.
const TABLE_BEGIN: &str = "<!-- measurements: generated by tests/synth_arith.rs -->\n";
const TABLE_END: &str = "<!-- end measurements -->\n";

#[test]
fn measurements_match_the_documentation() {
    use std::fs;
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/arithmetic.md");
    let doc = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n");
    let (head, rest) = doc
        .split_once(TABLE_BEGIN)
        .unwrap_or_else(|| panic!("{} has no measurement marker", path.display()));
    let (found, tail) = rest
        .split_once(TABLE_END)
        .unwrap_or_else(|| panic!("{} has no end marker", path.display()));

    let tables = measurement_tables();
    if found == tables {
        return;
    }
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        let updated = format!("{head}{TABLE_BEGIN}{tables}{TABLE_END}{tail}");
        fs::write(&path, updated).expect("rewrite the document");
        return;
    }
    let first = found
        .lines()
        .zip(tables.lines())
        .find(|(a, b)| a != b)
        .map(|(a, b)| format!("expected: {a}\nactual:   {b}"))
        .unwrap_or_else(|| "the tables differ in length".to_owned());
    panic!(
        "the tables in {} are out of date (UPDATE_EXPECT=1 to rewrite)\n{first}",
        path.display()
    );
}

/// The point of offering a fast architecture is that it is faster. A
/// table that said otherwise would mean a broken network, not a
/// surprising result, so the shape of the numbers is asserted here.
#[test]
fn architectures_are_shallower_than_ripple() {
    for w in [8u32, 16, 32] {
        let (ripple_cells, ripple_depth) =
            measure(&adder_built("m", w, AdderOptions::new(AdderArch::Ripple)));
        for arch in [
            AdderArch::CarrySelect,
            AdderArch::CarryLookahead,
            AdderArch::KoggeStone,
            AdderArch::BrentKung,
        ] {
            let (cells, depth) = measure(&adder_built("m", w, AdderOptions::new(arch)));
            // Carry-select and Kogge-Stone win at every width. The
            // lookahead and Brent-Kung adders pay for a hierarchy whose
            // levels cost, in two-input gates, about what eight bits of
            // ripple carry do, so they only win from sixteen bits up —
            // which `docs/arithmetic.md` says, and which is the reason
            // the default is neither of them.
            let hierarchical = matches!(arch, AdderArch::CarryLookahead | AdderArch::BrentKung);
            if w >= 16 || !hierarchical {
                assert!(
                    depth < ripple_depth,
                    "{} is {depth} deep at {w} bits, ripple carry is {ripple_depth}",
                    arch.name()
                );
            }
            assert!(
                cells > ripple_cells,
                "{} costs {cells} cells at {w} bits, ripple carry costs {ripple_cells}; \
                 a faster adder that is also smaller would be suspicious",
                arch.name()
            );
        }
        // Kogge-Stone is the shallowest of the family, and Brent-Kung
        // buys most of the depth for far fewer cells.
        let ks = measure(&adder_built(
            "m",
            w,
            AdderOptions::new(AdderArch::KoggeStone),
        ));
        let bk = measure(&adder_built(
            "m",
            w,
            AdderOptions::new(AdderArch::BrentKung),
        ));
        assert!(ks.1 <= bk.1, "Kogge-Stone {} vs Brent-Kung {}", ks.1, bk.1);
        assert!(ks.0 > bk.0, "Kogge-Stone {} vs Brent-Kung {}", ks.0, bk.0);

        // The prefix comparator is shallower than the ripple one, and
        // unlike the adder it is not much bigger.
        let rc = measure(&comparator_built(
            "m",
            w,
            false,
            &CellKind::Lt,
            CompareArch::Ripple,
        ));
        let pc = measure(&comparator_built(
            "m",
            w,
            false,
            &CellKind::Lt,
            CompareArch::Prefix,
        ));
        assert!(
            pc.1 < rc.1,
            "prefix `<` is {} deep, ripple is {}",
            pc.1,
            rc.1
        );

        // A tree multiplier is shallower than the row-at-a-time array.
        let array = measure(&multiplier_built(
            "m",
            w,
            false,
            w,
            MultiplierOptions::new(MultiplierArch::Array),
        ));
        for arch in [MultiplierArch::Wallace, MultiplierArch::Dadda] {
            let tree = measure(&multiplier_built(
                "m",
                w,
                false,
                w,
                MultiplierOptions::new(arch),
            ));
            assert!(
                tree.1 < array.1,
                "{} is {} deep at {w} bits, the array is {}",
                arch.name(),
                tree.1,
                array.1
            );
        }
        // Dadda places no adder Wallace does not need, so it is never
        // the more expensive of the two.
        let wallace = measure(&multiplier_built(
            "m",
            w,
            false,
            w,
            MultiplierOptions::new(MultiplierArch::Wallace),
        ));
        let dadda = measure(&multiplier_built(
            "m",
            w,
            false,
            w,
            MultiplierOptions::new(MultiplierArch::Dadda),
        ));
        assert!(
            dadda.0 <= wallace.0,
            "Dadda costs {} cells at {w} bits, Wallace costs {}",
            dadda.0,
            wallace.0
        );
    }
}

/// The same netlist comes out every time, which the golden tables
/// depend on and the `.rtl` round trip would catch late.
#[test]
fn lowering_is_deterministic() {
    for arch in AdderArch::ALL {
        let a = adder_built("m", 12, AdderOptions::new(arch));
        let b = adder_built("m", 12, AdderOptions::new(arch));
        assert_eq!(to_text(&a), to_text(&b), "{}", arch.name());
    }
    let one = lowered_alu(ArithOptions::default());
    let two = lowered_alu(ArithOptions::default());
    assert_eq!(to_text(&one), to_text(&two));
}

/// A small lowered design, pinned as `.rtl` so a change to what the
/// pass emits shows up as a readable diff, and so the output is known
/// to round-trip through the text format.
#[test]
fn lowered_netlists_match_the_golden_files() {
    for (label, options) in [
        ("default", ArithOptions::default()),
        ("fast", ArithOptions::fast()),
    ] {
        let mut module = alu(4);
        let mut diags = Diagnostics::new();
        ArithLower::new(options).run(&mut module, &mut diags);
        assert!(diags.is_empty());
        let text = to_text(&checked(module));
        common::golden(&format!("arith/alu4.{label}.rtl"), &text);

        let mut map = SourceMap::new();
        let file = map.add("alu4.rtl", text.clone()).unwrap();
        let parsed = Design::parse_text(&text, file)
            .unwrap_or_else(|d| panic!("the lowered ALU does not parse:\n{}", d.render(&map)));
        assert_eq!(parsed.to_text(), text, "the `.rtl` does not round-trip");
    }
}

fn to_text(module: &Module) -> String {
    let mut design = Design::new();
    let id = design.add_module(module.clone());
    design.top = Some(id);
    design.to_text()
}

// ---------------------------------------------------------------------------
// Wide multipliers, by vector rather than by proof
// ---------------------------------------------------------------------------

/// Feeds `module` every corner case and `count` random vectors,
/// comparing the output against `model`.
///
/// This is a *check*, not a proof, and exists for the two widths where
/// `check_equivalent` is impractical; see
/// `equivalence::wide_multipliers_match_the_generic_cell`.
fn check_over_vectors(
    what: &str,
    module: &Module,
    w: u32,
    out: u32,
    count: usize,
    model: impl Fn(u64, u64) -> u64,
) {
    use std::collections::BTreeMap;

    use common::{CellEval, Rng};
    use reticle::logic::Logic;

    let mask = |width: u32| -> u64 {
        if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        }
    };
    let a_net = module.net_by_name("a").expect("an `a` port");
    let b_net = module.net_by_name("b").expect("a `b` port");
    let y_net = module.net_by_name("y").expect("a `y` port");
    let evaluator = CellEval::new(module);

    // Zero, one, all ones, and the two signed extremes, crossed.
    let corners = [0u64, 1, mask(w), 1u64 << (w - 1), mask(w) >> 1];
    let mut cases: Vec<(u64, u64)> = Vec::new();
    for &a in &corners {
        for &b in &corners {
            cases.push((a, b));
        }
    }
    let mut rng = Rng::new(0x5EED_A71C);
    for _ in 0..count {
        cases.push((rng.next_u64() & mask(w), rng.next_u64() & mask(w)));
    }

    for (a, b) in cases {
        let mut inputs = BTreeMap::new();
        inputs.insert(a_net, Logic::from_u64(a, w));
        inputs.insert(b_net, Logic::from_u64(b, w));
        let values = evaluator.eval(&inputs);
        let got = values
            .get(&y_net)
            .and_then(Logic::to_u64)
            .expect("a known result");
        assert_eq!(got, model(a, b) & mask(out), "{what}: {a} by {b}");
    }
}

/// The widths `check_equivalent` cannot reach for a multiplier, checked
/// over vectors instead so a regression at those widths is still
/// caught by an ordinary test run.
#[test]
fn wide_multipliers_match_over_random_vectors() {
    for arch in MultiplierArch::ALL {
        for w in [13u32, 16] {
            let options = MultiplierOptions::new(arch);
            check_over_vectors(
                &format!("{} multiplier, {w} bits, truncated", arch.name()),
                &multiplier_built("m", w, false, w, options),
                w,
                w,
                200,
                |a, b| a.wrapping_mul(b),
            );
            let signed_value = move |v: u64| -> i64 {
                let x = i64::try_from(v).expect("a small word");
                if v >> (w - 1) & 1 == 1 {
                    x - (1i64 << w)
                } else {
                    x
                }
            };
            check_over_vectors(
                &format!("{} multiplier, {w} bits, full signed", arch.name()),
                &multiplier_built("m", w, true, 2 * w, options),
                w,
                2 * w,
                200,
                move |a, b| {
                    #[allow(clippy::cast_sign_loss)]
                    // Two's complement: the bit pattern is the answer.
                    {
                        (signed_value(a) * signed_value(b)) as u64
                    }
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The pass
// ---------------------------------------------------------------------------

/// A small design with one of each arithmetic cell.
fn alu(width: u32) -> Module {
    let mut b = ModuleBuilder::new("alu", span());
    let a = b.input("a", Type::bits(width));
    let bn = b.input("b", Type::bits(width));
    let sum = b.output("sum", Type::bits(width));
    let diff = b.output("diff", Type::bits(width));
    let prod = b.output("prod", Type::bits(width));
    let shifted = b.output("shifted", Type::bits(width));
    let below = b.output("below", Type::bit());
    let (av, bv) = (b.net(a), b.net(bn));
    for (name, kind, out) in [
        ("add0", CellKind::Add, sum),
        ("sub0", CellKind::Sub, diff),
        ("mul0", CellKind::Mul, prod),
        ("shl0", CellKind::Shl, shifted),
    ] {
        let t = b.add_net(format!("{name}_t"), Type::bits(width));
        b.cell2(name, kind, av, bv, t);
        let tv = b.net(t);
        b.assign(out, tv);
    }
    let t = b.add_net("lt_t", Type::bit());
    b.cell2("lt0", CellKind::Lt, av, bv, t);
    let tv = b.net(t);
    b.assign(below, tv);
    checked(b.finish())
}

fn lowered_alu(options: ArithOptions) -> Module {
    let mut module = alu(8);
    let mut diags = Diagnostics::new();
    ArithLower::new(options).run(&mut module, &mut diags);
    assert!(!diags.has_errors(), "{:?}", diags.iter().next());
    checked(module)
}

#[test]
fn the_pass_replaces_every_arithmetic_cell() {
    let module = lowered_alu(ArithOptions::default());
    for cell in module.cells.values() {
        assert!(
            matches!(
                cell.kind,
                CellKind::And | CellKind::Or | CellKind::Xor | CellKind::Not | CellKind::Mux
            ),
            "`{}` is still a `{}`",
            cell.name,
            cell.kind.keyword()
        );
    }
    // Every output is still driven, now by an assignment from the
    // network the pass built.
    assert_eq!(module.assigns.len(), 10);
}

#[test]
fn the_pass_reports_what_it_did() {
    let mut module = alu(8);
    let mut diags = Diagnostics::new();
    let stats = ArithLower::new(ArithOptions::default()).run(&mut module, &mut diags);
    assert!(stats.changed);
    assert_eq!(stats.get("lowered"), 5);
    assert!(stats.get("cells") > 100, "{}", stats.get("cells"));
    // Nothing left to do the second time.
    let stats = ArithLower::new(ArithOptions::default()).run(&mut module, &mut diags);
    assert!(!stats.changed);
}

/// Two adders and nothing else, so an adder's depth is the module's.
fn adder_pair(width: u32) -> Module {
    let mut b = ModuleBuilder::new("pair", span());
    let a = b.input("a", Type::bits(width));
    let bn = b.input("b", Type::bits(width));
    let x = b.output("x", Type::bits(width));
    let y = b.output("y", Type::bits(width));
    let (av, bv) = (b.net(a), b.net(bn));
    for (name, out) in [("add0", x), ("add1", y)] {
        let t = b.add_net(format!("{name}_t"), Type::bits(width));
        b.cell2(name, CellKind::Add, av, bv, t);
        let tv = b.net(t);
        b.assign(out, tv);
    }
    checked(b.finish())
}

/// One `add` and nothing else, so the module's depth is the adder's.
fn one_adder(width: u32) -> Module {
    let mut b = ModuleBuilder::new("one", span());
    let a = b.input("a", Type::bits(width));
    let bn = b.input("b", Type::bits(width));
    let y = b.output("y", Type::bits(width));
    let (av, bv) = (b.net(a), b.net(bn));
    let t = b.add_net("t", Type::bits(width));
    b.cell2("add0", CellKind::Add, av, bv, t);
    let tv = b.net(t);
    b.assign(y, tv);
    checked(b.finish())
}

fn lowered(mut module: Module, options: ArithOptions) -> Module {
    let mut diags = Diagnostics::new();
    ArithLower::new(options).run(&mut module, &mut diags);
    assert!(diags.is_empty(), "{:?}", diags.iter().next());
    checked(module)
}

/// One adder can be asked to be fast while the rest stay small, which
/// is the whole point of the per-cell attribute.
#[test]
fn a_cell_attribute_overrides_the_options() {
    let mut fast = one_adder(16);
    let id = fast.cell_by_name("add0").expect("the adder");
    fast.cells[id].attrs.set("arith", "kogge_stone");
    let (fast_cells, fast_depth) = measure(&lowered(fast, ArithOptions::default()));
    let (plain_cells, plain_depth) = measure(&lowered(one_adder(16), ArithOptions::default()));
    assert!(fast_depth < plain_depth, "{fast_depth} vs {plain_depth}");
    assert!(fast_cells > plain_cells, "{fast_cells} vs {plain_cells}");

    // In a module of two, only the attributed one changes.
    let mut mixed = adder_pair(16);
    let id = mixed.cell_by_name("add0").expect("the adder");
    mixed.cells[id].attrs.set("arith", "kogge_stone");
    let mixed_cells = measure(&lowered(mixed, ArithOptions::default())).0;
    let pair_cells = measure(&lowered(adder_pair(16), ArithOptions::default())).0;
    assert_eq!(mixed_cells - pair_cells, fast_cells - plain_cells);
}

#[test]
fn a_cell_attribute_can_opt_out() {
    let mut module = alu(8);
    for name in ["add0", "sub0", "mul0", "shl0", "lt0"] {
        let id = module.cell_by_name(name).expect("a cell");
        module.cells[id].attrs.set("arith", "none");
    }
    let mut diags = Diagnostics::new();
    let stats = ArithLower::new(ArithOptions::default()).run(&mut module, &mut diags);
    assert!(!stats.changed);
    assert_eq!(module.cells.len(), 5);
}

/// A module-wide `arith` is a default for the cells it fits, and is
/// silently not for the others: a module with an adder and a shifter in
/// it can still say `arith = "brent_kung"`.
#[test]
fn a_module_attribute_is_the_default() {
    let mut module = alu(16);
    module.attrs.set("arith", "brent_kung");
    let mut diags = Diagnostics::new();
    ArithLower::new(ArithOptions::default()).run(&mut module, &mut diags);
    assert!(diags.is_empty(), "{:?}", diags.iter().next());

    let mut pair = adder_pair(16);
    pair.attrs.set("arith", "brent_kung");
    let attributed = measure(&lowered(pair, ArithOptions::default()));
    let plain = measure(&lowered(adder_pair(16), ArithOptions::default()));
    assert!(attributed.1 < plain.1, "{attributed:?} vs {plain:?}");
}

#[test]
fn an_unknown_architecture_is_a_warning() {
    let mut module = alu(8);
    let id = module.cell_by_name("add0").expect("the adder");
    module.cells[id].attrs.set("arith", "wallace");
    let mut diags = Diagnostics::new();
    let stats = ArithLower::new(ArithOptions::default()).run(&mut module, &mut diags);
    assert!(stats.changed);
    assert_eq!(diags.iter().next().and_then(|d| d.code), Some("S0041"));
    assert_eq!(diags.warning_count(), 1);
}

#[test]
fn wide_division_is_left_generic_with_a_reason() {
    let mut b = ModuleBuilder::new("wide", span());
    let a = b.input("a", Type::bits(16));
    let bn = b.input("b", Type::bits(16));
    let q = b.output("q", Type::bits(16));
    let (av, bv) = (b.net(a), b.net(bn));
    let t = b.add_net("t", Type::bits(16));
    b.cell2("div0", CellKind::Div, av, bv, t);
    let tv = b.net(t);
    b.assign(q, tv);
    let mut module = checked(b.finish());

    let mut diags = Diagnostics::new();
    let stats = ArithLower::new(ArithOptions::default()).run(&mut module, &mut diags);
    assert!(!stats.changed);
    let d = diags.iter().next().expect("a diagnostic");
    assert_eq!(d.code, Some("S0043"));
    assert!(d.message.contains("pipelined") || d.notes.iter().any(|n| n.contains("pipelined")));
    assert_eq!(module.cells.len(), 1);

    // Raising the threshold expands it after all.
    let options = ArithOptions {
        width_thresholds: WidthThresholds {
            divider: 16,
            ..WidthThresholds::default()
        },
        ..ArithOptions::default()
    };
    let mut diags = Diagnostics::new();
    let stats = ArithLower::new(options).run(&mut module, &mut diags);
    assert!(stats.changed);
    assert!(diags.is_empty());
}

#[test]
fn a_wide_multiply_is_left_generic_with_a_note() {
    let mut b = ModuleBuilder::new("wide", span());
    let a = b.input("a", Type::bits(64));
    let bn = b.input("b", Type::bits(64));
    let y = b.output("y", Type::bits(64));
    let (av, bv) = (b.net(a), b.net(bn));
    let t = b.add_net("t", Type::bits(64));
    b.cell2("mul0", CellKind::Mul, av, bv, t);
    let tv = b.net(t);
    b.assign(y, tv);
    let mut module = checked(b.finish());
    let mut diags = Diagnostics::new();
    let stats = ArithLower::new(ArithOptions::default()).run(&mut module, &mut diags);
    assert!(!stats.changed);
    assert_eq!(diags.iter().next().and_then(|d| d.code), Some("S0042"));
}

#[test]
fn narrow_cells_can_be_left_to_the_aig() {
    let options = ArithOptions {
        width_thresholds: WidthThresholds {
            min_width: 9,
            ..WidthThresholds::default()
        },
        ..ArithOptions::default()
    };
    let mut module = alu(8);
    let mut diags = Diagnostics::new();
    let stats = ArithLower::new(options).run(&mut module, &mut diags);
    assert!(!stats.changed);
}

// ---------------------------------------------------------------------------
// DSP candidates
// ---------------------------------------------------------------------------

/// `p <= a * b` with both operands registered, plus an accumulator
/// downstream of a second multiply.
fn dsp_design() -> Module {
    let mut b = ModuleBuilder::new("dsp", span());
    let clk = b.input("clk", Type::bit());
    let x = b.input("x", Type::sbits(16));
    let y = b.input("y", Type::sbits(16));
    let out = b.output("out", Type::sbits(32));
    let acc_out = b.output("acc_out", Type::sbits(32));

    let clkv = b.net(clk);

    // Registered operands.
    let xr = b.add_net("xr", Type::sbits(16));
    let yr = b.add_net("yr", Type::sbits(16));
    let (xv, yv) = (b.net(x), b.net(y));
    for (name, d, q) in [("ffx", xv, xr), ("ffy", yv, yr)] {
        b.cell(
            name,
            dff_kind(),
            vec![(Name::new("clk"), clkv), (Name::new("d"), d)],
            vec![(Name::new("q"), q)],
        );
    }
    let (xrv, yrv) = (b.net(xr), b.net(yr));
    let (xw, yw) = (b.resize(xrv, 32, true), b.resize(yrv, 32, true));
    let p = b.add_net("p", Type::sbits(32));
    b.cell2("mul0", CellKind::Mul, xw, yw, p);
    let pv = b.net(p);
    let pr = b.add_net("pr", Type::sbits(32));
    b.cell(
        "ffp",
        dff_kind(),
        vec![(Name::new("clk"), clkv), (Name::new("d"), pv)],
        vec![(Name::new("q"), pr)],
    );
    let prv = b.net(pr);
    b.assign(out, prv);

    // A multiply-accumulate: acc <= acc + x * y, over the unregistered
    // inputs, so `xr` and `yr` feed the first multiply alone.
    let (xw2, yw2) = (b.resize(xv, 32, true), b.resize(yv, 32, true));
    let p2 = b.add_net("p2", Type::sbits(32));
    b.cell2("mul1", CellKind::Mul, xw2, yw2, p2);
    let p2v = b.net(p2);
    let acc = b.add_net("acc", Type::sbits(32));
    let accv = b.net(acc);
    let s = b.add_net("s", Type::sbits(32));
    b.cell2("add0", CellKind::Add, p2v, accv, s);
    let sv = b.net(s);
    b.cell(
        "ffacc",
        dff_kind(),
        vec![(Name::new("clk"), clkv), (Name::new("d"), sv)],
        vec![(Name::new("q"), acc)],
    );
    b.assign(acc_out, accv);
    checked(b.finish())
}

/// A plain rising-edge flip-flop with no enable and no reset.
fn dff_kind() -> CellKind {
    CellKind::Dff {
        clk_pos: true,
        has_enable: false,
        reset: None,
    }
}

#[test]
fn dsp_candidates_describe_the_multiplies() {
    let module = dsp_design();
    let found = dsp_candidates(&module);
    assert_eq!(found.len(), 2);

    let plain = &found[0];
    assert_eq!(plain.kind.name(), "multiply");
    assert!(plain.signed);
    assert_eq!((plain.a.width, plain.b.width), (32, 32));
    assert_eq!(plain.product_width, 32);
    // Both operands come through flip-flops that feed nothing else, and
    // the product goes into one: two stages a block could absorb.
    assert!(plain.a.register.is_some() && plain.b.register.is_some());
    assert!(plain.output_register.is_some());
    assert_eq!(plain.absorbable_stages(), 2);
    assert!(plain.absorbs(plain.mul));

    let mac = &found[1];
    assert_eq!(mac.kind.name(), "multiply-accumulate");
    assert!(mac.add.is_some());
    assert!(mac.addend.is_some());
    assert!(!mac.negated);
    assert_eq!(mac.absorbable_stages(), 1);
    assert!(
        mac.describe(&module)
            .contains("multiply-accumulate signed 32x32"),
        "{}",
        mac.describe(&module)
    );
}

#[test]
fn a_shared_product_is_not_a_multiply_add() {
    // `mul0`'s product feeds a register and nothing else, so it stays a
    // plain multiply even though an adder is in the module.
    let module = dsp_design();
    let found = dsp_candidates(&module);
    assert!(found[0].add.is_none());
}

// ---------------------------------------------------------------------------
// Equivalence
// ---------------------------------------------------------------------------

#[cfg(feature = "formal")]
mod equivalence {
    use super::*;

    use reticle::formal::{EquivOptions, check_equivalent};

    /// Proves `built` computes what `reference` does, for every input.
    ///
    /// Both modules are combinational, so the check is a single SAT
    /// call on the miter: `Equivalent` means *proved*, over all
    /// `2^inputs` vectors, not sampled.
    fn assert_equivalent(what: &str, reference: Module, built: Module) {
        let mut design = Design::new();
        let a = design.add_module(reference);
        let b = design.add_module(built);
        let report = check_equivalent(&design, a, b, &EquivOptions::default());
        assert!(
            report.equivalent(),
            "{what}: {}",
            report.render("generic", "built")
        );
    }

    #[test]
    fn adders_match_the_generic_cell() {
        for arch in AdderArch::ALL {
            for w in WIDTHS {
                let options = AdderOptions::new(arch);
                assert_equivalent(
                    &format!("{} adder, {w} bits", arch.name()),
                    adder_reference("generic", w),
                    adder_built("built", w, options),
                );
            }
        }
    }

    /// The block and group sizes are a knob, so they are proved at more
    /// than their default.
    #[test]
    fn adder_block_sizes_match_the_generic_cell() {
        for size in [1u32, 2, 3, 5, 8] {
            for w in [1u32, 7, 8, 13] {
                for arch in [AdderArch::CarrySelect, AdderArch::CarryLookahead] {
                    let options = AdderOptions {
                        arch,
                        block_size: size,
                        group_size: size.max(2),
                    };
                    assert_equivalent(
                        &format!("{} adder, {w} bits, block {size}", arch.name()),
                        adder_reference("generic", w),
                        adder_built("built", w, options),
                    );
                }
            }
        }
    }

    #[test]
    fn subtractors_match_the_generic_cell() {
        for arch in AdderArch::ALL {
            for w in WIDTHS {
                assert_equivalent(
                    &format!("{} subtractor, {w} bits", arch.name()),
                    subtractor_reference("generic", w),
                    subtractor_built("built", w, AdderOptions::new(arch)),
                );
            }
        }
    }

    #[test]
    fn truncated_multipliers_match_the_generic_cell() {
        for arch in MultiplierArch::ALL {
            for w in NARROW {
                for signed in [false, true] {
                    assert_equivalent(
                        &format!("{} multiplier, {w} bits, signed {signed}", arch.name()),
                        multiplier_reference("generic", w, signed, w),
                        multiplier_built("built", w, signed, w, MultiplierOptions::new(arch)),
                    );
                }
            }
        }
    }

    /// The two remaining widths of [`WIDTHS`] for the multipliers.
    ///
    /// The miter of two *structurally different* multipliers is the
    /// textbook hard instance for a CDCL solver: the tree architectures
    /// share no internal equivalence with the reference's row-at-a-time
    /// array, so there is nothing for the solver to propagate and it
    /// falls back on case splitting. At eight bits every pairing here
    /// is proved in seconds; at thirteen and sixteen it is minutes to
    /// hours, which is why this test is ignored by default rather than
    /// deleted. Run it with
    ///
    /// ```sh
    /// cargo test --features synth,formal --test synth_arith -- --ignored
    /// ```
    ///
    /// The same widths are covered in every run by
    /// `wide_multipliers_match_over_random_vectors`, which is a check
    /// and not a proof and says so.
    #[test]
    #[ignore = "a 13- or 16-bit multiplier miter can take hours of SAT"]
    fn wide_multipliers_match_the_generic_cell() {
        for arch in MultiplierArch::ALL {
            for signed in [false, true] {
                for w in [7u32, 8] {
                    assert_equivalent(
                        &format!("{} full multiplier, {w} bits, signed {signed}", arch.name()),
                        multiplier_reference("generic", w, signed, 2 * w),
                        multiplier_built("built", w, signed, 2 * w, MultiplierOptions::new(arch)),
                    );
                }
                for w in [13u32, 16] {
                    assert_equivalent(
                        &format!("{} multiplier, {w} bits, signed {signed}", arch.name()),
                        multiplier_reference("generic", w, signed, w),
                        multiplier_built("built", w, signed, w, MultiplierOptions::new(arch)),
                    );
                }
            }
        }
    }

    /// The full product is where signedness stops being free: the
    /// Baugh-Wooley correction and the Booth recoding are both proved
    /// here. A full product is twice as wide as a truncated one, so its
    /// miter is the hard case sooner: up to six bits here, and seven and
    /// eight in `wide_multipliers_match_the_generic_cell`.
    #[test]
    fn full_multipliers_match_the_generic_cell() {
        for arch in MultiplierArch::ALL {
            for w in [1u32, 2, 3, 5, 6] {
                for signed in [false, true] {
                    assert_equivalent(
                        &format!("{} full multiplier, {w} bits, signed {signed}", arch.name()),
                        multiplier_reference("generic", w, signed, 2 * w),
                        multiplier_built("built", w, signed, 2 * w, MultiplierOptions::new(arch)),
                    );
                }
            }
        }
    }

    /// Every final adder is proved with every reduction, since the two
    /// halves of a multiplier are chosen independently.
    #[test]
    fn multiplier_final_adders_match_the_generic_cell() {
        for arch in MultiplierArch::ALL {
            for adder in AdderArch::ALL {
                let options = MultiplierOptions {
                    arch,
                    adder: AdderOptions::new(adder),
                };
                assert_equivalent(
                    &format!("{} multiplier over a {} adder", arch.name(), adder.name()),
                    multiplier_reference("generic", 6, true, 12),
                    multiplier_built("built", 6, true, 12, options),
                );
            }
        }
    }

    #[test]
    fn comparators_match_the_generic_cells() {
        for arch in CompareArch::ALL {
            for w in WIDTHS {
                for signed in [false, true] {
                    for (label, kind) in COMPARISONS {
                        assert_equivalent(
                            &format!("{} `{label}`, {w} bits, signed {signed}", arch.name()),
                            comparator_reference("generic", w, signed, kind.clone()),
                            comparator_built("built", w, signed, &kind, arch),
                        );
                    }
                    assert_equivalent(
                        &format!("{} pair, {w} bits, signed {signed}", arch.name()),
                        compare_pair_reference("generic", w, signed),
                        compare_pair_built("built", w, signed, arch),
                    );
                }
            }
        }
    }

    #[test]
    fn shifters_match_the_generic_cells() {
        for arch in ShifterArch::ALL {
            for w in WIDTHS {
                for (label, cell, kind, signed) in SHIFTS {
                    // The amount is proved both as wide as the operand
                    // (so every out-of-range amount occurs) and narrow
                    // (so the in-range ones are exhaustive).
                    for aw in [w, 3] {
                        assert_equivalent(
                            &format!("{} `{label}`, {w} bits, amount {aw}", arch.name()),
                            shifter_reference("generic", w, aw, signed, cell.clone()),
                            shifter_built("built", w, aw, signed, kind, arch),
                        );
                    }
                }
            }
        }
    }

    /// An unsigned left operand makes `sshr` a logical shift, which is
    /// what the bit-blaster does and therefore what the lowering must.
    #[test]
    fn an_unsigned_arithmetic_shift_is_logical() {
        for w in [1u32, 7, 8] {
            assert_equivalent(
                &format!("unsigned `sshr`, {w} bits"),
                shifter_reference("generic", w, w, false, CellKind::Sshr),
                shifter_built(
                    "built",
                    w,
                    w,
                    false,
                    ShiftKind::RightLogical,
                    ShifterArch::Barrel,
                ),
            );
        }
    }

    #[test]
    fn funnel_shifters_match_a_shift_of_the_doubled_word() {
        for radix in [2u32, 4] {
            for w in WIDTHS {
                for left in [false, true] {
                    for aw in [w, 2] {
                        assert_equivalent(
                            &format!(
                                "funnel {}, {w} bits, amount {aw}, radix {radix}",
                                if left { "left" } else { "right" }
                            ),
                            funnel_reference("generic", w, aw, left),
                            funnel_built("built", w, aw, left, radix),
                        );
                    }
                }
            }
        }
    }

    /// Rotates are proved at the power-of-two widths, where an amount
    /// of `log2(w)` bits covers every rotation and nothing else.
    #[test]
    fn rotates_match_a_shift_pair() {
        for w in [2u32, 4, 8, 16] {
            let aw = w.trailing_zeros();
            for left in [false, true] {
                for radix in [2u32, 4] {
                    assert_equivalent(
                        &format!(
                            "rotate {}, {w} bits, radix {radix}",
                            if left { "left" } else { "right" }
                        ),
                        rotate_reference("generic", w, aw, left),
                        rotate_built("built", w, aw, left, radix),
                    );
                }
            }
        }
    }

    #[test]
    fn dividers_match_the_generic_cells() {
        for arch in [
            AdderArch::Ripple,
            AdderArch::KoggeStone,
            AdderArch::BrentKung,
        ] {
            for w in [1u32, 2, 3, 7, 8] {
                for signed in [false, true] {
                    assert_equivalent(
                        &format!("{} divider, {w} bits, signed {signed}", arch.name()),
                        divider_reference("generic", w, signed),
                        divider_built("built", w, signed, AdderOptions::new(arch)),
                    );
                }
            }
        }
    }

    /// The pass itself: an ALU lowered with each set of options still
    /// computes what it did before.
    #[test]
    fn the_pass_preserves_behaviour() {
        for options in [ArithOptions::default(), ArithOptions::fast()] {
            for width in [1u32, 3, 8] {
                let mut before = alu(width);
                let mut after = alu(width);
                let mut diags = Diagnostics::new();
                ArithLower::new(options).run(&mut after, &mut diags);
                assert!(!diags.has_errors());
                before.name = Name::new("generic");
                after.name = Name::new("built");
                assert_equivalent(&format!("the pass at {width} bits"), before, after);
            }
        }
    }
}
