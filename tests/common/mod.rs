//! Shared support for the synthesis integration tests.
//!
//! Three pieces:
//!
//! - [`RandomModule`], a seeded generator of small combinational modules
//!   (random expression trees over 3–6 inputs of widths 1–8), used to
//!   cross-check the AIG and the mappers against the IR they came from.
//! - [`eval_assigns`] and [`CellEval`], tiny evaluators for the IR's
//!   expression trees and for the cell form, both on [`Logic`], which are
//!   the reference the mapped netlists are compared against.
//! - [`golden`], the golden-file helper (`UPDATE_EXPECT=1` rewrites).
//!
//! Everything here is deterministic: one seeded [`Rng`] drives generation
//! and the input vectors, so a failure reproduces exactly.

// The module is shared by several test binaries, and each uses a
// different part of it, so both lints fire on items the other one needs.
#![allow(dead_code)]
#![allow(unreachable_pub)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use reticle::ir::builder::ModuleBuilder;
use reticle::ir::cell::{Cell, CellKind};
use reticle::ir::design::{Module, NetId};
use reticle::ir::expr::{BinaryOp, ExprId, ExprKind, UnaryOp};
use reticle::ir::process::Lvalue;
use reticle::ir::types::Type;
use reticle::logic::{Bit, Logic};
use reticle::source::{SourceMap, Span};
use reticle::synth::aig::BitRef;
use reticle::synth::cells::GateLibrary;

/// A span for generated objects.
pub fn span() -> Span {
    let mut map = SourceMap::new();
    let id = map.add("synth-test", "").unwrap();
    Span::new(id, 0, 0)
}

/// xorshift64*, the same generator the AIG uses, so tests are
/// reproducible without pulling in a dependency.
pub struct Rng(u64);

impl Rng {
    /// A generator seeded with `seed`.
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n`.
    pub fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n).expect("n fits")).expect("fits")
    }

    /// A coin flip.
    pub fn flip(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

/// `n` random booleans.
pub fn random_bits(rng: &mut Rng, n: usize) -> Vec<bool> {
    let mut out = Vec::with_capacity(n);
    let mut word = 0u64;
    for i in 0..n {
        if i % 64 == 0 {
            word = rng.next_u64();
        }
        out.push((word >> (i % 64)) & 1 == 1);
    }
    out
}

/// Gathers per-bit values into whole-net [`Logic`] values.
pub fn net_values(module: &Module, inputs: &[BitRef], bits: &[bool]) -> BTreeMap<NetId, Logic> {
    let mut values: BTreeMap<NetId, Logic> = BTreeMap::new();
    for (i, input) in inputs.iter().enumerate() {
        let width = module.nets[input.net].ty.width().unwrap_or(1);
        let entry = values
            .entry(input.net)
            .or_insert_with(|| Logic::zero(width));
        entry.set_bit(input.bit, if bits[i] { Bit::One } else { Bit::Zero });
    }
    values
}

/// A random value for every input port of `module`.
pub fn random_input_values(module: &Module, rng: &mut Rng) -> BTreeMap<NetId, Logic> {
    let mut values = BTreeMap::new();
    for port in &module.ports {
        if port.dir != reticle::ir::PortDir::In {
            continue;
        }
        let Some(width) = module.nets[port.net].ty.width() else {
            continue;
        };
        values.insert(port.net, Logic::from_u64(rng.next_u64(), width));
    }
    values
}

// ---------------------------------------------------------------------------
// Evaluation of the IR
// ---------------------------------------------------------------------------

/// Evaluates an expression under the given net values; nets without a
/// value read as zero.
pub fn eval_expr(module: &Module, id: ExprId, values: &BTreeMap<NetId, Logic>) -> Logic {
    let expr = &module.exprs[id];
    let width = expr.ty.width().unwrap_or(1);
    let signed = expr.ty.is_signed();
    let result = match &expr.kind {
        ExprKind::Const(c) => c.clone(),
        ExprKind::Net(n) => values
            .get(n)
            .cloned()
            .unwrap_or_else(|| Logic::zero(module.nets[*n].ty.width().unwrap_or(1))),
        ExprKind::Slice { base, hi, lo } => eval_expr(module, *base, values).slice(*hi, *lo),
        ExprKind::Index { base, index } => {
            let b = eval_expr(module, *base, values);
            let i = eval_expr(module, *index, values);
            match i.to_u64() {
                Some(i) if i < u64::from(b.width()) => {
                    let i = u32::try_from(i).expect("index fits");
                    b.slice(i, i)
                }
                // The AIG reads an out-of-range select as zero.
                _ => Logic::zero(1),
            }
        }
        ExprKind::IndexedSlice {
            base,
            offset,
            width: w,
            up,
        } => {
            let b = eval_expr(module, *base, values);
            let off = eval_expr(module, *offset, values);
            let start = off.to_u64().unwrap_or(u64::MAX);
            let start = if *up {
                start
            } else {
                start.wrapping_sub(u64::from(*w - 1))
            };
            let mut out = Logic::zero(*w);
            for bit in 0..*w {
                let src = start.wrapping_add(u64::from(bit));
                let value = if src < u64::from(b.width()) {
                    b.bit(u32::try_from(src).expect("fits"))
                } else {
                    Bit::Zero
                };
                out.set_bit(bit, value);
            }
            out
        }
        ExprKind::Concat(parts) => {
            let mut out = Logic::zero(0);
            for part in parts {
                out = out.concat(&eval_expr(module, *part, values));
            }
            out
        }
        ExprKind::Replicate { count, expr } => eval_expr(module, *expr, values).replicate(*count),
        ExprKind::Unary { op, expr } => {
            let a = eval_expr(module, *expr, values);
            match op {
                UnaryOp::Not => a.not(),
                UnaryOp::Neg => a.neg(),
                UnaryOp::ReduceAnd => a.reduce_and(),
                UnaryOp::ReduceOr => a.reduce_or(),
                UnaryOp::ReduceXor => a.reduce_xor(),
                UnaryOp::ReduceNand => a.reduce_nand(),
                UnaryOp::ReduceNor => a.reduce_nor(),
                UnaryOp::ReduceXnor => a.reduce_xnor(),
                UnaryOp::LogicNot => a.logical_not(),
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let a = eval_expr(module, *lhs, values);
            let b = eval_expr(module, *rhs, values);
            match op {
                BinaryOp::And => a.and(&b),
                BinaryOp::Or => a.or(&b),
                BinaryOp::Xor => a.xor(&b),
                BinaryOp::Xnor => a.xnor(&b),
                BinaryOp::LogicAnd => a.logical_and(&b),
                BinaryOp::LogicOr => a.logical_or(&b),
                BinaryOp::Add => a.add(&b),
                BinaryOp::Sub => a.sub(&b),
                BinaryOp::Mul => a.mul(&b),
                BinaryOp::Div => a.div(&b),
                BinaryOp::Mod => a.rem(&b),
                BinaryOp::Pow => a.pow(&b),
                BinaryOp::Shl => a.shl_by(&b),
                BinaryOp::Shr => a.shr_by(&b),
                BinaryOp::Sshr => a.sshr_by(&b),
                BinaryOp::Eq => a.eq(&b),
                BinaryOp::Ne => a.ne(&b),
                BinaryOp::CaseEq => a.case_eq(&b),
                BinaryOp::CaseNe => a.case_ne(&b),
                BinaryOp::WildEq => a.wildcard_eq(&b),
                BinaryOp::Lt => a.lt(&b),
                BinaryOp::Le => a.le(&b),
                BinaryOp::Gt => a.gt(&b),
                BinaryOp::Ge => a.ge(&b),
            }
        }
        ExprKind::Ternary { cond, then_, else_ } => {
            let c = eval_expr(module, *cond, values);
            if c.bit(0) == Bit::One {
                eval_expr(module, *then_, values)
            } else {
                eval_expr(module, *else_, values)
            }
        }
        ExprKind::Resize {
            expr,
            width: w,
            signed: s,
        } => {
            // A `Resize` sign-extends only when it asks for a signed
            // result *and* the operand is itself signed; otherwise it
            // zero-extends. `Logic::resize` decides from the value's own
            // flag, so set that first and label the result afterwards.
            let a = eval_expr(module, *expr, values);
            let sign_extend = *s && a.is_signed();
            a.with_signed(sign_extend).resize(*w).with_signed(*s)
        }
        ExprKind::String(_) | ExprKind::MemRead { .. } | ExprKind::Call { .. } => {
            Logic::zero(width)
        }
    };
    result.resize(width).with_signed(signed)
}

/// Evaluates every continuous assignment of a module, in order, and
/// returns the values of the nets they drive. Assignments are expected to
/// be acyclic and already ordered (the generator emits them so).
pub fn eval_assigns(module: &Module, inputs: &BTreeMap<NetId, Logic>) -> BTreeMap<NetId, Logic> {
    let mut values = inputs.clone();
    // Repeat until nothing changes, so an assignment whose inputs come
    // later still settles.
    for _ in 0..module.assigns.len() + 1 {
        let mut changed = false;
        for assign in &module.assigns {
            let value = eval_expr(module, assign.value, &values);
            if let Lvalue::Net(net) = assign.target {
                let width = module.nets[net].ty.width().unwrap_or(value.width());
                let value = value.resize(width);
                if values.get(&net) != Some(&value) {
                    values.insert(net, value);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    values
}

/// A tiny evaluator for the cell form, used to check a mapped netlist
/// against the module it came from.
///
/// It understands the primitives the mappers emit plus, when a
/// [`GateLibrary`] is supplied, the `Blackbox` cells a standard-cell
/// mapping produces: their function comes from the library entry named by
/// the cell, with the cell's ports matched to the gate's pins by name.
pub struct CellEval<'m> {
    module: &'m Module,
    /// The module's cells by position, so the evaluation loop indexes
    /// them directly instead of walking the arena for each one.
    cells: Vec<&'m Cell>,
    /// Cells in an order where a cell's inputs are ready before it runs.
    order: Vec<usize>,
    /// The library that gives `Blackbox` cells their meaning.
    library: Option<&'m GateLibrary>,
}

impl<'m> CellEval<'m> {
    /// Orders the module's combinational cells topologically.
    pub fn new(module: &'m Module) -> CellEval<'m> {
        CellEval::build(module, None)
    }

    /// The same evaluator, resolving `Blackbox` cells through `library`.
    pub fn with_library(module: &'m Module, library: &'m GateLibrary) -> CellEval<'m> {
        CellEval::build(module, Some(library))
    }

    fn build(module: &'m Module, library: Option<&'m GateLibrary>) -> CellEval<'m> {
        let cells: Vec<&Cell> = module.cells.values().collect();
        // Which cell drives each net.
        let mut driver: BTreeMap<NetId, usize> = BTreeMap::new();
        for (i, cell) in module.cells.values().enumerate() {
            for (_, net) in &cell.outputs {
                driver.insert(*net, i);
            }
        }
        let mut order = Vec::new();
        let mut state = vec![0u8; module.cells.len()];
        for start in 0..module.cells.len() {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((i, expanded)) = stack.pop() {
                if expanded {
                    state[i] = 2;
                    order.push(i);
                    continue;
                }
                if state[i] != 0 {
                    continue;
                }
                state[i] = 1;
                stack.push((i, true));
                let cell = cells[i];
                if !evaluable(cell, library) {
                    continue;
                }
                for (_, e) in &cell.inputs {
                    for net in expr_nets(module, *e) {
                        if let Some(&d) = driver.get(&net)
                            && state[d] == 0
                        {
                            stack.push((d, false));
                        }
                    }
                }
            }
        }
        CellEval {
            module,
            cells,
            order,
            library,
        }
    }

    /// Evaluates the cells and the continuous assignments under `inputs`.
    pub fn eval(&self, inputs: &BTreeMap<NetId, Logic>) -> BTreeMap<NetId, Logic> {
        let mut values = inputs.clone();
        // Cells first, then the assignments that wire them to the ports;
        // repeated so either order settles.
        for _ in 0..3 {
            for &i in &self.order {
                let cell = self.cells[i];
                if !evaluable(cell, self.library) {
                    continue;
                }
                let Some(y) = cell.output("y") else {
                    continue;
                };
                let width = self.module.nets[y].ty.width().unwrap_or(1);
                let input = |port: &str| -> Logic {
                    cell.input(port)
                        .map(|e| eval_expr(self.module, e, &values))
                        .unwrap_or_else(|| Logic::zero(width))
                };
                let value = match &cell.kind {
                    CellKind::Not => input("a").not(),
                    CellKind::Buf => input("a"),
                    CellKind::And => input("a").and(&input("b")),
                    CellKind::Or => input("a").or(&input("b")),
                    CellKind::Xor => input("a").xor(&input("b")),
                    CellKind::Mux => {
                        if input("s").bit(0) == Bit::One {
                            input("b")
                        } else {
                            input("a")
                        }
                    }
                    CellKind::Lut { k, init } => {
                        let a = input("a");
                        let mut index = 0u32;
                        for bit in 0..*k {
                            if a.bit(bit) == Bit::One {
                                index |= 1 << bit;
                            }
                        }
                        Logic::from_bit(init.bit(index))
                    }
                    CellKind::Add => input("a").add(&input("b")),
                    CellKind::Sub => input("a").sub(&input("b")),
                    CellKind::Mul => input("a").mul(&input("b")),
                    CellKind::Eq => input("a").eq(&input("b")),
                    CellKind::Ne => input("a").ne(&input("b")),
                    CellKind::Lt => input("a").lt(&input("b")),
                    CellKind::Le => input("a").le(&input("b")),
                    CellKind::Gt => input("a").gt(&input("b")),
                    CellKind::Ge => input("a").ge(&input("b")),
                    CellKind::ReduceAnd => input("a").reduce_and(),
                    CellKind::ReduceOr => input("a").reduce_or(),
                    CellKind::ReduceXor => input("a").reduce_xor(),
                    CellKind::Blackbox(gate_name) => {
                        let Some(library) = self.library else {
                            continue;
                        };
                        let gate = library
                            .gate(gate_name.as_str())
                            .unwrap_or_else(|| panic!("`{gate_name}` is not in the library"));
                        // The cell's ports carry the gate's pin names.
                        let mut pattern = 0usize;
                        for (pin, name) in gate.pins.iter().enumerate() {
                            let value = cell
                                .input(name)
                                .map(|e| eval_expr(self.module, e, &values))
                                .unwrap_or_else(|| Logic::zero(1));
                            if value.bit(0) == Bit::One {
                                pattern |= 1 << pin;
                            }
                        }
                        Logic::from_bool(gate.function.bit(pattern))
                    }
                    _ => continue,
                };
                values.insert(y, value.resize(width));
            }
            for assign in &self.module.assigns {
                if let Lvalue::Net(net) = assign.target {
                    let width = self.module.nets[net].ty.width().unwrap_or(1);
                    let value = eval_expr(self.module, assign.value, &values).resize(width);
                    values.insert(net, value);
                }
            }
        }
        values
    }
}

/// Compares a two-state result against a four-state reference, bit by
/// bit, ignoring the bits the reference leaves unknown.
///
/// The AIG is a two-state representation: `x` and `z` read as zero, and
/// the operations that yield `x` in the IR's `Logic` semantics (division
/// or remainder by zero, above all) are given concrete two-state results
/// by the bit-blaster instead. Those bits are exactly where the two
/// legitimately disagree, so they are skipped rather than compared.
pub fn assert_known_bits_match(want: &Logic, got: &Logic, context: &str) {
    assert_eq!(want.width(), got.width(), "{context}: width");
    for bit in 0..want.width() {
        if !want.bit(bit).is_known() {
            continue;
        }
        assert_eq!(
            got.bit(bit),
            want.bit(bit),
            "{context}: bit {bit} of `{want}` vs `{got}`"
        );
    }
}

/// True when this evaluator can compute the cell's output.
///
/// `CellKind::Blackbox` is not combinational as far as the IR is
/// concerned — its contents are unknown — but a standard-cell mapping
/// emits library cells as black boxes, and those *are* combinational once
/// the library says what they compute.
fn evaluable(cell: &Cell, library: Option<&GateLibrary>) -> bool {
    match &cell.kind {
        CellKind::Blackbox(name) => library.is_some_and(|lib| lib.gate(name.as_str()).is_some()),
        kind => kind.is_combinational(),
    }
}

/// Every net an expression reads.
pub fn expr_nets(module: &Module, root: ExprId) -> Vec<NetId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(expr) = module.exprs.get(id) else {
            continue;
        };
        if let ExprKind::Net(n) = expr.kind {
            out.push(n);
        }
        stack.extend(reticle::ir::expr::operands(&expr.kind));
    }
    out
}

// ---------------------------------------------------------------------------
// Random modules
// ---------------------------------------------------------------------------

/// Generates small combinational modules to cross-check the passes.
pub struct RandomModule<'r> {
    rng: &'r mut Rng,
}

impl<'r> RandomModule<'r> {
    /// A generator drawing from `rng`.
    pub fn new(rng: &'r mut Rng) -> RandomModule<'r> {
        RandomModule { rng }
    }

    /// Builds module number `case`: 3–6 inputs of width 1–8 and a couple
    /// of outputs driven by random expression trees.
    pub fn build(&mut self, case: u64) -> Module {
        let span = span();
        let mut b = ModuleBuilder::new(format!("rand{case}"), span);
        let inputs = 3 + self.rng.below(4);
        let width = 1 + u32::try_from(self.rng.below(8)).expect("width");
        let signed = self.rng.flip();
        let ty = if signed {
            Type::sbits(width)
        } else {
            Type::bits(width)
        };
        let nets: Vec<_> = (0..inputs)
            .map(|i| b.input(format!("i{i}"), ty.clone()))
            .collect();
        let leaves: Vec<ExprId> = nets.iter().map(|&n| b.net(n)).collect();

        let outputs = 1 + self.rng.below(2);
        for o in 0..outputs {
            let depth = 2 + self.rng.below(3);
            let value = self.expr(&mut b, &leaves, width, signed, depth);
            let out = b.output(format!("o{o}"), ty.clone());
            b.assign(out, value);
        }
        // A single-bit output from a comparison or reduction, so the
        // predicate paths are covered too.
        let pred = self.predicate(&mut b, &leaves, width);
        let p = b.output("p", Type::bit());
        b.assign(p, pred);
        b.finish()
    }

    /// A random expression of the given width.
    fn expr(
        &mut self,
        b: &mut ModuleBuilder,
        leaves: &[ExprId],
        width: u32,
        signed: bool,
        depth: usize,
    ) -> ExprId {
        if depth == 0 {
            return if self.rng.below(5) == 0 {
                let value = self.rng.next_u64();
                if signed {
                    b.const_i64(width, i64::try_from(value % 256).expect("fits") - 128)
                } else {
                    b.const_u64(width, value)
                }
            } else {
                leaves[self.rng.below(leaves.len())]
            };
        }
        let lhs = self.expr(b, leaves, width, signed, depth - 1);
        let rhs = self.expr(b, leaves, width, signed, depth - 1);
        // Weighted towards the cheap operators; `Div`, `Mod` and `Pow`
        // appear rarely because they blow the AIG up.
        match self.rng.below(16) {
            0 => b.and(lhs, rhs),
            1 => b.or(lhs, rhs),
            2 => b.xor(lhs, rhs),
            3 => b.binary(BinaryOp::Xnor, lhs, rhs),
            4 | 5 => b.add(lhs, rhs),
            6 => b.sub(lhs, rhs),
            7 => b.mul(lhs, rhs),
            8 => b.not(lhs),
            9 => b.neg(lhs),
            10 => {
                let amount = self.expr(b, leaves, width, false, 0);
                b.shl(lhs, amount)
            }
            11 => {
                let amount = self.expr(b, leaves, width, false, 0);
                b.shr(lhs, amount)
            }
            12 => {
                let amount = self.expr(b, leaves, width, false, 0);
                b.binary(BinaryOp::Sshr, lhs, amount)
            }
            13 => {
                let cond = self.predicate(b, leaves, width);
                b.mux(cond, lhs, rhs)
            }
            14 => {
                // A slice put back to full width, so selections are
                // exercised.
                let hi = u32::try_from(self.rng.below(usize::try_from(width).expect("width")))
                    .expect("fits");
                let sliced = b.slice(lhs, hi, 0);
                b.resize(sliced, width, signed)
            }
            _ => {
                if width <= 4 && self.rng.below(3) == 0 {
                    b.binary(BinaryOp::Div, lhs, rhs)
                } else {
                    b.add(lhs, rhs)
                }
            }
        }
    }

    /// A random single-bit expression.
    fn predicate(&mut self, b: &mut ModuleBuilder, leaves: &[ExprId], width: u32) -> ExprId {
        let lhs = leaves[self.rng.below(leaves.len())];
        let rhs = leaves[self.rng.below(leaves.len())];
        let _ = width;
        match self.rng.below(9) {
            0 => b.eq(lhs, rhs),
            1 => b.ne(lhs, rhs),
            2 => b.lt(lhs, rhs),
            3 => b.le(lhs, rhs),
            4 => b.gt(lhs, rhs),
            5 => b.ge(lhs, rhs),
            6 => b.reduce_and(lhs),
            7 => b.reduce_or(lhs),
            _ => b.reduce_xor(lhs),
        }
    }
}

// ---------------------------------------------------------------------------
// Fixed designs and golden files
// ---------------------------------------------------------------------------

/// Optimisation settings whose result does not depend on which Cargo
/// features are enabled.
///
/// [`fraig`](reticle::synth::aig::fraig) proves candidate equivalences
/// with the SAT solver when the `formal` feature is on and only checks
/// small cones when it is off, so it legitimately merges more in one
/// build than the other. Golden files are produced with it disabled, so
/// they are the same whichever way the crate was compiled; the passes it
/// does run are still the bulk of the script.
pub fn portable_aig_options() -> reticle::synth::aig::AigOptions {
    reticle::synth::aig::AigOptions {
        fraig: false,
        ..reticle::synth::aig::AigOptions::default()
    }
}

/// The designs both integration tests map and report numbers for.
pub fn fixed_designs() -> Vec<(String, Module)> {
    vec![
        ("adder8".to_owned(), adder(8)),
        ("mul8".to_owned(), multiplier(8)),
        ("decoder4to16".to_owned(), decoder(4)),
        ("priority8".to_owned(), priority_encoder(8)),
        ("cmp32".to_owned(), comparator(32)),
    ]
}

/// An `n`-bit adder with a carry in and out.
pub fn adder(n: u32) -> Module {
    let mut b = ModuleBuilder::new(format!("adder{n}"), span());
    let a = b.input("a", Type::bits(n));
    let c = b.input("b", Type::bits(n));
    let cin = b.input("cin", Type::bit());
    let sum = b.output("sum", Type::bits(n));
    let cout = b.output("cout", Type::bit());
    let (an, cn, cinn) = (b.net(a), b.net(c), b.net(cin));
    // Widen by one bit so the carry out falls out of the top.
    let aw = b.zext(an, n + 1);
    let cw = b.zext(cn, n + 1);
    let cw2 = b.zext(cinn, n + 1);
    let t = b.add(aw, cw);
    let total = b.add(t, cw2);
    let low = b.slice(total, n - 1, 0);
    b.assign(sum, low);
    let top = b.slice(total, n, n);
    b.assign(cout, top);
    b.finish()
}

/// An `n` x `n` multiplier with a `2n`-bit product.
pub fn multiplier(n: u32) -> Module {
    let mut b = ModuleBuilder::new(format!("mul{n}"), span());
    let a = b.input("a", Type::bits(n));
    let c = b.input("b", Type::bits(n));
    let p = b.output("p", Type::bits(2 * n));
    let (an, cn) = (b.net(a), b.net(c));
    let aw = b.zext(an, 2 * n);
    let cw = b.zext(cn, 2 * n);
    let product = b.mul(aw, cw);
    b.assign(p, product);
    b.finish()
}

/// A `k`-to-`2^k` one-hot decoder with an enable.
pub fn decoder(k: u32) -> Module {
    let n = 1u32 << k;
    let mut b = ModuleBuilder::new(format!("decoder{k}to{n}"), span());
    let sel = b.input("sel", Type::bits(k));
    let en = b.input("en", Type::bit());
    let y = b.output("y", Type::bits(n));
    let (seln, enn) = (b.net(sel), b.net(en));
    // `y = en ? (1 << sel) : 0`, built as a concatenation of comparisons
    // so the mapper sees one cone per output bit.
    let mut parts = Vec::new();
    for i in (0..n).rev() {
        let value = b.const_u64(k, u64::from(i));
        let hit = b.eq(seln, value);
        let gated = b.land(hit, enn);
        parts.push(gated);
    }
    let cat = b.concat(parts);
    b.assign(y, cat);
    b.finish()
}

/// An `n`-bit priority encoder: the index of the highest set bit, plus a
/// valid flag.
pub fn priority_encoder(n: u32) -> Module {
    let bits = n.next_power_of_two().trailing_zeros().max(1);
    let mut b = ModuleBuilder::new(format!("priority{n}"), span());
    let d = b.input("d", Type::bits(n));
    let idx = b.output("idx", Type::bits(bits));
    let valid = b.output("valid", Type::bit());
    let dn = b.net(d);
    // A chain of muxes from the least significant bit up, so the highest
    // set bit wins.
    let mut result = b.const_u64(bits, 0);
    for i in 0..n {
        let bit = b.slice(dn, i, i);
        let value = b.const_u64(bits, u64::from(i));
        result = b.mux(bit, value, result);
    }
    b.assign(idx, result);
    let any = b.reduce_or(dn);
    b.assign(valid, any);
    b.finish()
}

/// An `n`-bit unsigned comparator with the three relations.
pub fn comparator(n: u32) -> Module {
    let mut b = ModuleBuilder::new(format!("cmp{n}"), span());
    let a = b.input("a", Type::bits(n));
    let c = b.input("b", Type::bits(n));
    let lt = b.output("lt", Type::bit());
    let eq = b.output("eq", Type::bit());
    let gt = b.output("gt", Type::bit());
    let (an, cn) = (b.net(a), b.net(c));
    let l = b.lt(an, cn);
    b.assign(lt, l);
    let e = b.eq(an, cn);
    b.assign(eq, e);
    let g = b.gt(an, cn);
    b.assign(gt, g);
    b.finish()
}

/// The path of a golden file under `testdata/synth/`.
pub fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/synth")
        .join(name)
}

/// Compares `actual` with the golden file `name`, rewriting it when
/// `UPDATE_EXPECT` is set.
pub fn golden(name: &str, actual: &str) {
    let path = golden_path(name);
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let existing = fs::read_to_string(&path).ok();
    if existing.as_deref() == Some(actual) {
        return;
    }
    if update {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create golden directory");
        }
        fs::write(&path, actual).expect("write golden file");
        return;
    }
    match existing {
        None => panic!(
            "golden file {} does not exist; run with UPDATE_EXPECT=1",
            path.display()
        ),
        Some(expected) => {
            let diff = first_difference(&expected, actual);
            panic!(
                "golden file {} is out of date (UPDATE_EXPECT=1 to rewrite)\n{diff}",
                path.display()
            );
        }
    }
}

/// The first differing line of two texts.
fn first_difference(expected: &str, actual: &str) -> String {
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            return format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!(
        "expected {} lines, got {}",
        expected.lines().count(),
        actual.lines().count()
    )
}
