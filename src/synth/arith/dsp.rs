//! DSP block inference: the recognition half.
//!
//! An FPGA's DSP block is a hard multiplier with an adder behind it and
//! registers in front, between and after. Using one is two decisions:
//! *which* piece of a netlist could go into one, and *how* it maps onto
//! a particular device's primitive. This module answers only the first.
//! The second belongs to the target, and for the families Reticle knows
//! it already lives in `src/fpga/primitives.rs`; nothing here mentions a
//! device, a primitive name or a port.
//!
//! [`candidates`] walks a module and returns one [`DspCandidate`] per
//! multiplier, describing:
//!
//! - the shape — a bare multiply, a multiply-add, or a
//!   multiply-accumulate where the adder's own output comes back
//!   through a register;
//! - each operand's width and signedness, which is what decides whether
//!   a block is wide enough and whether two have to be cascaded;
//! - the registers that could be *absorbed* into the block's pipeline
//!   stages. A register is only listed when the block could really
//!   swallow it: its output must feed nothing but the multiplier (or be
//!   fed by nothing but it), because a register someone else reads has
//!   to stay where it is.
//!
//! Recognition is deliberately syntactic and deliberately conservative.
//! It runs on the cell form after [`crate::synth::cellify`], reports
//! candidates in cell order so the result is deterministic, and never
//! rewrites anything: a caller is free to use one candidate and ignore
//! the next.
//!
//! # What is not recognised
//!
//! A product that is subtracted (`A*B - C`), a pre-adder feeding the
//! multiplier (`(A + D) * B`), and cascades between blocks are all
//! shapes real DSP blocks support and this module does not report. They
//! are additions to make when a target asks for them; leaving them out
//! costs a missed inference, never a wrong one.

use std::collections::HashMap;

use crate::ir::{CellId, CellKind, ExprId, ExprKind, Module, NetId};
use crate::source::Span;

/// What shape a candidate has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DspKind {
    /// `a * b`.
    Multiply,
    /// `a * b + c`, or `c - a * b`, with `c` coming from elsewhere.
    MultiplyAdd,
    /// `acc <= acc + a * b`: the adder's addend is the register the
    /// adder itself drives.
    MultiplyAccumulate,
}

impl DspKind {
    /// A short name for reports.
    pub fn name(self) -> &'static str {
        match self {
            DspKind::Multiply => "multiply",
            DspKind::MultiplyAdd => "multiply-add",
            DspKind::MultiplyAccumulate => "multiply-accumulate",
        }
    }
}

/// One input of a candidate.
#[derive(Clone, Debug)]
pub struct DspOperand {
    /// The expression feeding the operand.
    pub expr: ExprId,
    /// Its width in bits.
    pub width: u32,
    /// Whether it is a signed value.
    pub signed: bool,
    /// A flip-flop driving this operand and nothing else, which the
    /// block's input pipeline stage could absorb.
    pub register: Option<CellId>,
}

/// A multiply a DSP block could implement.
#[derive(Clone, Debug)]
pub struct DspCandidate {
    /// The shape.
    pub kind: DspKind,
    /// The `mul` cell at the centre of it.
    pub mul: CellId,
    /// The multiplicand.
    pub a: DspOperand,
    /// The multiplier.
    pub b: DspOperand,
    /// The `add` or `sub` cell behind the multiplier, if any.
    pub add: Option<CellId>,
    /// What is added to the product.
    pub addend: Option<DspOperand>,
    /// True when the product is subtracted from the addend
    /// (`c - a * b`) rather than added to it.
    pub negated: bool,
    /// True when the multiplication is signed, that is when both
    /// operands are.
    pub signed: bool,
    /// The width of the product as the netlist computes it.
    pub product_width: u32,
    /// The net the candidate's result drives.
    pub output: NetId,
    /// A flip-flop the result feeds and nothing else does, which the
    /// block's output pipeline stage could absorb. For
    /// [`DspKind::MultiplyAccumulate`] this is the accumulator itself.
    pub output_register: Option<CellId>,
    /// Where the multiply came from.
    pub span: Span,
}

impl DspCandidate {
    /// How many register stages a block could absorb: one for the
    /// inputs when both are registered, and one for the output.
    ///
    /// A DSP block's input stage registers both operands together, so a
    /// single registered operand buys nothing and is not counted.
    pub fn absorbable_stages(&self) -> u32 {
        let inputs = u32::from(self.a.register.is_some() && self.b.register.is_some());
        inputs + u32::from(self.output_register.is_some())
    }

    /// A one-line description, for reports and tests.
    pub fn describe(&self, module: &Module) -> String {
        format!(
            "{} {}{}x{} -> {} in `{}`, {} register stage{}",
            self.kind.name(),
            if self.signed { "signed " } else { "" },
            self.a.width,
            self.b.width,
            self.product_width,
            module.cells[self.mul].name,
            self.absorbable_stages(),
            if self.absorbable_stages() == 1 {
                ""
            } else {
                "s"
            }
        )
    }
}

/// Every multiply in `module` that a DSP block could implement, in cell
/// order.
pub fn candidates(module: &Module) -> Vec<DspCandidate> {
    let use_count = net_uses(module);
    let drivers = net_drivers(module);
    let mut out = Vec::new();
    for (id, cell) in module.cells.iter() {
        if cell.kind != CellKind::Mul {
            continue;
        }
        let (Some(ia), Some(ib), Some(y)) = (cell.input("a"), cell.input("b"), cell.output("y"))
        else {
            continue;
        };
        let a = operand(module, ia, &use_count, &drivers);
        let b = operand(module, ib, &use_count, &drivers);
        let signed = a.signed && b.signed;
        let product_width = module.nets[y].ty.width().unwrap_or(0);
        let mut candidate = DspCandidate {
            kind: DspKind::Multiply,
            mul: id,
            a,
            b,
            add: None,
            addend: None,
            negated: false,
            signed,
            product_width,
            output: y,
            output_register: None,
            span: cell.span,
        };
        if let Some((add_id, addend_expr, negated)) = post_adder(module, y, &use_count) {
            candidate.kind = DspKind::MultiplyAdd;
            candidate.add = Some(add_id);
            candidate.addend = Some(operand(module, addend_expr, &use_count, &drivers));
            candidate.negated = negated;
            candidate.output = module.cells[add_id].output("y").unwrap_or(candidate.output);
        }
        candidate.output_register = absorbable_register(module, candidate.output, &use_count);
        if candidate.kind == DspKind::MultiplyAdd
            && let (Some(reg), Some(addend)) = (candidate.output_register, &candidate.addend)
            && let Some(q) = module.cells[reg].output("q")
            && root_net(module, addend.expr) == Some(q)
        {
            candidate.kind = DspKind::MultiplyAccumulate;
        }
        out.push(candidate);
    }
    out
}

/// Describes one operand, noting a flip-flop that drives it alone.
fn operand(
    module: &Module,
    expr: ExprId,
    use_count: &HashMap<NetId, usize>,
    drivers: &HashMap<NetId, CellId>,
) -> DspOperand {
    let ty = &module.expr(expr).ty;
    let register = root_net(module, expr)
        .filter(|net| use_count.get(net).copied().unwrap_or(0) == 1)
        .filter(|net| module.port_of_net(*net).is_none())
        .and_then(|net| drivers.get(&net).copied())
        .filter(|id| matches!(module.cells[*id].kind, CellKind::Dff { .. }));
    DspOperand {
        expr,
        width: ty.width().unwrap_or(0),
        signed: ty.is_signed(),
        register,
    }
}

/// The `add` or `sub` cell that consumes the product alone, with the
/// other operand and whether the product is the subtrahend.
fn post_adder(
    module: &Module,
    product: NetId,
    use_count: &HashMap<NetId, usize>,
) -> Option<(CellId, ExprId, bool)> {
    if use_count.get(&product).copied().unwrap_or(0) != 1 {
        return None;
    }
    if module.port_of_net(product).is_some() {
        return None;
    }
    for (id, cell) in module.cells.iter() {
        let (a, b) = match (cell.input("a"), cell.input("b")) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };
        let on_a = root_net(module, a) == Some(product);
        let on_b = root_net(module, b) == Some(product);
        if !on_a && !on_b {
            continue;
        }
        match cell.kind {
            CellKind::Add => return Some((id, if on_a { b } else { a }, false)),
            // `c - a * b` is a shape a DSP block has; `a * b - c` is
            // not, so only the product as subtrahend is recognised.
            CellKind::Sub if on_b => return Some((id, a, true)),
            _ => return None,
        }
    }
    None
}

/// A flip-flop `net` feeds and nothing else does.
fn absorbable_register(
    module: &Module,
    net: NetId,
    use_count: &HashMap<NetId, usize>,
) -> Option<CellId> {
    if use_count.get(&net).copied().unwrap_or(0) != 1 || module.port_of_net(net).is_some() {
        return None;
    }
    module.cells.iter().find_map(|(id, cell)| {
        if !matches!(cell.kind, CellKind::Dff { .. }) {
            return None;
        }
        let d = cell.input("d")?;
        (root_net(module, d) == Some(net)).then_some(id)
    })
}

/// The net an expression ultimately reads, through resizes.
///
/// A resize is a width change the block's own operand width absorbs, so
/// peeling it finds the register behind a sign extension.
fn root_net(module: &Module, mut expr: ExprId) -> Option<NetId> {
    loop {
        match &module.expr(expr).kind {
            ExprKind::Net(net) => return Some(*net),
            ExprKind::Resize { expr: inner, .. } => expr = *inner,
            _ => return None,
        }
    }
}

/// How many places read each net: cell inputs, assignments, instance
/// connections and processes all count.
fn net_uses(module: &Module) -> HashMap<NetId, usize> {
    let mut counts: HashMap<NetId, usize> = HashMap::new();
    let bump = |m: &Module, expr: ExprId, counts: &mut HashMap<NetId, usize>| {
        for net in expr_nets(m, expr) {
            *counts.entry(net).or_insert(0) += 1;
        }
    };
    for cell in module.cells.values() {
        for (_, expr) in &cell.inputs {
            bump(module, *expr, &mut counts);
        }
    }
    for assign in &module.assigns {
        bump(module, assign.value, &mut counts);
    }
    for instance in module.instances.values() {
        for (_, expr) in &instance.connections {
            bump(module, *expr, &mut counts);
        }
    }
    // A module with processes left in it has not been through the whole
    // pipeline; count every net any of them mentions as read, so no
    // register is called absorbable that a process still uses.
    if !module.processes.is_empty() {
        for expr in module.exprs.ids() {
            if let ExprKind::Net(net) = module.expr(expr).kind {
                *counts.entry(net).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// Which cell drives each net.
fn net_drivers(module: &Module) -> HashMap<NetId, CellId> {
    let mut out = HashMap::new();
    for (id, cell) in module.cells.iter() {
        for (_, net) in &cell.outputs {
            out.insert(*net, id);
        }
    }
    out
}

/// Every net an expression tree reads.
fn expr_nets(module: &Module, root: ExprId) -> Vec<NetId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(expr) = module.exprs.get(id) else {
            continue;
        };
        if let ExprKind::Net(net) = expr.kind {
            out.push(net);
        }
        stack.extend(crate::ir::expr::operands(&expr.kind));
    }
    out
}

/// Renders candidates as text, one per line, for a report.
pub fn render(module: &Module, candidates: &[DspCandidate]) -> String {
    let mut out = String::new();
    for c in candidates {
        out.push_str("  ");
        out.push_str(&c.describe(module));
        out.push('\n');
    }
    out
}

impl DspCandidate {
    /// True when `cell` is one this candidate would swallow, so a caller
    /// that maps it knows what to remove from the netlist.
    pub fn absorbs(&self, cell: CellId) -> bool {
        self.mul == cell
            || self.add == Some(cell)
            || self.output_register == Some(cell)
            || self.a.register == Some(cell)
            || self.b.register == Some(cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{Name, Type};
    use crate::synth::arith::testkit::span;

    /// `y = c - a * b`, the one subtracting shape a DSP block has.
    fn subtracting() -> Module {
        let mut b = ModuleBuilder::new("m", span());
        let x = b.input("x", Type::sbits(8));
        let y = b.input("y", Type::sbits(8));
        let c = b.input("c", Type::sbits(8));
        let out = b.output("out", Type::sbits(8));
        let (xv, yv, cv) = (b.net(x), b.net(y), b.net(c));
        let p = b.add_net("p", Type::sbits(8));
        b.cell2("mul0", CellKind::Mul, xv, yv, p);
        let pv = b.net(p);
        let d = b.add_net("d", Type::sbits(8));
        b.cell2("sub0", CellKind::Sub, cv, pv, d);
        let dv = b.net(d);
        b.assign(out, dv);
        b.finish()
    }

    #[test]
    fn a_subtracted_product_is_a_multiply_add() {
        let m = subtracting();
        let found = candidates(&m);
        assert_eq!(found.len(), 1);
        let c = &found[0];
        assert_eq!(c.kind, DspKind::MultiplyAdd);
        assert!(c.negated, "the product is the subtrahend");
        assert!(c.signed);
        assert_eq!((c.a.width, c.b.width, c.product_width), (8, 8, 8));
        assert_eq!(c.absorbable_stages(), 0);
        assert!(c.absorbs(c.mul));
        assert!(c.add.is_some_and(|id| c.absorbs(id)));
        assert_eq!(
            c.describe(&m),
            "multiply-add signed 8x8 -> 8 in `mul0`, 0 register stages"
        );
        assert_eq!(render(&m, &found), format!("  {}\n", c.describe(&m)));
    }

    /// `a * b - c` is the other way round and is deliberately not
    /// recognised, so the candidate stays a plain multiply.
    #[test]
    fn a_product_that_is_subtracted_from_is_not() {
        let mut b = ModuleBuilder::new("m", span());
        let x = b.input("x", Type::bits(8));
        let y = b.input("y", Type::bits(8));
        let c = b.input("c", Type::bits(8));
        let out = b.output("out", Type::bits(8));
        let (xv, yv, cv) = (b.net(x), b.net(y), b.net(c));
        let p = b.add_net("p", Type::bits(8));
        b.cell2("mul0", CellKind::Mul, xv, yv, p);
        let pv = b.net(p);
        let d = b.add_net("d", Type::bits(8));
        b.cell2("sub0", CellKind::Sub, pv, cv, d);
        let dv = b.net(d);
        b.assign(out, dv);
        let m = b.finish();
        let found = candidates(&m);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DspKind::Multiply);
        assert!(found[0].add.is_none());
        assert!(
            !found[0].signed,
            "unsigned operands make an unsigned product"
        );
    }

    /// A product read twice cannot be folded into an adder, because the
    /// block would have to produce the bare product as well.
    #[test]
    fn a_product_with_two_readers_stays_bare() {
        let mut b = ModuleBuilder::new("m", span());
        let x = b.input("x", Type::bits(8));
        let y = b.input("y", Type::bits(8));
        let o1 = b.output("o1", Type::bits(8));
        let o2 = b.output("o2", Type::bits(8));
        let (xv, yv) = (b.net(x), b.net(y));
        let p = b.add_net("p", Type::bits(8));
        b.cell2("mul0", CellKind::Mul, xv, yv, p);
        let pv = b.net(p);
        let s = b.add_net("s", Type::bits(8));
        b.cell2("add0", CellKind::Add, pv, xv, s);
        let sv = b.net(s);
        b.assign(o1, sv);
        b.assign(o2, pv);
        let m = b.finish();
        let found = candidates(&m);
        assert_eq!(found[0].kind, DspKind::Multiply);
        assert!(found[0].add.is_none());
        assert!(found[0].output_register.is_none());
    }

    /// A module with no multiply has no candidates, and a `Name` import
    /// that is otherwise unused keeps the helper honest.
    #[test]
    fn nothing_to_infer() {
        let mut b = ModuleBuilder::new("m", span());
        let x = b.input("x", Type::bits(4));
        let out = b.output("out", Type::bits(4));
        let xv = b.net(x);
        let t = b.add_net("t", Type::bits(4));
        b.cell(
            "n0",
            CellKind::Not,
            vec![(Name::new("a"), xv)],
            vec![(Name::new("y"), t)],
        );
        let tv = b.net(t);
        b.assign(out, tv);
        let m = b.finish();
        assert!(candidates(&m).is_empty());
        assert_eq!(render(&m, &[]), "");
    }
}
