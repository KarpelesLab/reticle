//! Exhaustive checking of a built network against a model, for the unit
//! tests of this module family.
//!
//! [`exhaustive`] builds a one-off module whose inputs are the widths it
//! is given, hands their bits to the architecture under test, and then
//! evaluates the resulting gate netlist for **every** input combination,
//! comparing each result against a closure written in Rust. At the
//! widths a unit test can afford — five bits or so — that is a complete
//! proof of the same kind `tests/synth_arith.rs` gets from a SAT solver
//! at sixteen, and it needs no feature beyond `synth`, so a change that
//! breaks an architecture fails immediately rather than in the
//! integration suite.
//!
//! Evaluating the netlist is simple because [`GateBuilder`] emits a cell
//! only once its inputs exist: the cells are already in dependency
//! order, so one pass over them in id order computes every net.

use super::bits::GateBuilder;
use crate::ir::builder::ModuleBuilder;
use crate::ir::{Bit, CellKind, Const, ExprId, ExprKind, Module, NetId, Type};
use crate::source::{SourceMap, Span};

/// A span pointing at nothing, for objects a test creates.
pub(crate) fn span() -> Span {
    let mut map = SourceMap::new();
    let file = map.add("arith-test", "").expect("a fresh source map");
    Span::new(file, 0, 0)
}

/// Builds `build` over inputs of the given widths and checks it against
/// `model` for every combination of input values.
///
/// `model` receives the inputs as unsigned words in the same order and
/// returns the expected result, of which only the built network's own
/// width is compared.
///
/// # Panics
///
/// Panics on the first disagreement, naming the inputs.
pub(crate) fn exhaustive(
    what: &str,
    widths: &[u32],
    build: impl FnOnce(&mut GateBuilder<'_>, &[Vec<ExprId>]) -> Vec<ExprId>,
    model: impl Fn(&[u64]) -> u64,
) {
    assert!(
        widths.iter().sum::<u32>() <= 20,
        "{what}: an exhaustive check of {} bits would take too long",
        widths.iter().sum::<u32>()
    );
    let mut b = ModuleBuilder::new("t", span());
    let nets: Vec<NetId> = widths
        .iter()
        .enumerate()
        .map(|(i, &w)| b.input(format!("i{i}"), Type::bits(w)))
        .collect();
    let reads: Vec<ExprId> = nets.iter().map(|&n| b.net(n)).collect();
    let (out, out_width) = {
        let mut cx = GateBuilder::new(&mut b, "u");
        let split: Vec<Vec<ExprId>> = reads
            .iter()
            .zip(widths)
            .map(|(&e, &w)| cx.split(e, w))
            .collect();
        let bits = build(&mut cx, &split);
        let width = u32::try_from(bits.len()).expect("a width fits in a u32");
        (cx.join(&bits), width)
    };
    let module = b.finish();

    let total: u64 = widths.iter().map(|&w| 1u64 << w).product();
    let mut values: Vec<Option<Const>> = vec![None; module.nets.len()];
    for code in 0..total {
        values.iter_mut().for_each(|v| *v = None);
        let mut inputs = Vec::with_capacity(widths.len());
        let mut rest = code;
        for (&net, &w) in nets.iter().zip(widths) {
            let value = rest % (1u64 << w);
            rest /= 1u64 << w;
            values[net.index()] = Some(Const::from_u64(value, w));
            inputs.push(value);
        }
        let got = run(&module, &mut values, out);
        let want = model(&inputs) & mask(out_width);
        assert_eq!(
            got, want,
            "{what}: inputs {inputs:?} gave {got} (expected {want})"
        );
    }
}

/// `2^width - 1`, saturating at the width of a `u64`.
fn mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

/// The value of one bit of a cell input.
///
/// A gate's operands are only ever a constant, a whole one-bit net or a
/// one-bit slice of an input word, which is what [`GateBuilder::split`]
/// produces, so three cases cover everything an architecture builds.
fn bit_of(module: &Module, expr: ExprId, values: &[Option<Const>]) -> bool {
    match &module.expr(expr).kind {
        ExprKind::Const(c) => c.bit(0) == Bit::One,
        ExprKind::Net(net) => {
            values[net.index()]
                .as_ref()
                .expect("a net evaluated before it is read")
                .bit(0)
                == Bit::One
        }
        ExprKind::Slice { base, hi, lo } => {
            assert_eq!(hi, lo, "a multi-bit slice on a gate input");
            let ExprKind::Net(net) = module.expr(*base).kind else {
                panic!("a slice of something other than a net");
            };
            values[net.index()]
                .as_ref()
                .expect("an input net with a value")
                .bit(*lo)
                == Bit::One
        }
        other => panic!("a gate input this harness does not understand: {other:?}"),
    }
}

/// Evaluates every cell in order, then assembles the output word.
fn run(module: &Module, values: &mut [Option<Const>], out: ExprId) -> u64 {
    for cell in module.cells.values() {
        let read = |port: &str| -> bool {
            let expr = cell.input(port).expect("a connected port");
            bit_of(module, expr, values)
        };
        let value = match cell.kind {
            CellKind::Not => !read("a"),
            CellKind::And => read("a") & read("b"),
            CellKind::Or => read("a") | read("b"),
            CellKind::Xor => read("a") ^ read("b"),
            CellKind::Mux => {
                if read("s") {
                    read("b")
                } else {
                    read("a")
                }
            }
            ref other => panic!("an architecture built a `{}` cell", other.keyword()),
        };
        let net = cell.output("y").expect("a driven output");
        values[net.index()] = Some(Const::from_bool(value));
    }
    // `GateBuilder::join` produces a concatenation, most significant
    // part first, or the bit itself when there is only one.
    let parts: Vec<ExprId> = match &module.expr(out).kind {
        ExprKind::Concat(parts) => parts.clone(),
        _ => vec![out],
    };
    let mut word = 0u64;
    for (i, part) in parts.iter().rev().enumerate() {
        if bit_of(module, *part, values) {
            word |= 1u64 << i;
        }
    }
    word
}

/// The value of an unsigned `width`-bit word read as a signed one.
pub(crate) fn signed(value: u64, width: u32) -> i64 {
    let v = i64::try_from(value).expect("a small word");
    if width < 64 && value >> (width - 1) & 1 == 1 {
        v - (1i64 << width)
    } else {
        v
    }
}

/// A signed value as the unsigned `width`-bit word holding it.
pub(crate) fn unsigned(value: i64, width: u32) -> u64 {
    let m = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    #[allow(clippy::cast_sign_loss)]
    // Two's complement is exactly the wrapping this masks back down.
    {
        (value as u64) & m
    }
}
