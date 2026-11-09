//! The Xilinx 7-series carry chain: every mapped adder checked against
//! `CARRY4`'s own model.
//!
//! `src/fpga/devices/xc7.dev` maps adders onto `CARRY4`, four bits of
//! carry per instance, with a LUT per bit computing the propagate
//! `a ^ b` and the sums coming out of the element itself. An adder that
//! is subtly wrong is worse than an adder in LUTs, so this file checks
//! the wiring two independent ways, both against the behavioural model
//! the device file quotes from Yosys' `techlibs/xilinx/cells_sim.v`:
//!
//! ```text
//! O  = S ^ {CO[2:0], CI | CYINIT}
//! CO[0] = S[0] ? (CI | CYINIT) : DI[0]
//! CO[i] = S[i] ? CO[i-1]       : DI[i]      for i = 1, 2, 3
//! ```
//!
//! - **Exhaustive simulation** ([`Netlist`]): the netlist the whole flow
//!   produces — `IBUF`, `LUT6`, `CARRY4`, `OBUF` and the assignments
//!   between them — is evaluated on *every* input combination and
//!   compared with the arithmetic the source asked for. That covers 1 to
//!   8 bits, both carry-in values (the `a + b + cin` shape, which is two
//!   chained adders), a constant operand and the widened form that
//!   exposes the carry out.
//! - **SAT equivalence** (`mod proofs`): the same netlist with each
//!   primitive replaced by the model above becomes ordinary IR, and
//!   `formal::check_equivalent` proves it equivalent to the unmapped
//!   design. Combinationally that is a decision procedure, not a sample,
//!   which is what makes 16 and 32 bits provable at all.
//!
//! The two share nothing but the quoted equations: the simulator models
//! `CARRY4` in Rust over [`Logic`], the proof builds it out of `mux` and
//! `xor` cells, so a mistake in either shows up as a disagreement.
//!
//! # The ECP5's `CCU2C`, which is the third shape
//!
//! The same two checks cover `src/fpga/devices/ecp5.dev`'s `CCU2C`, which
//! takes the two operands rather than a propagate and answers the sums,
//! two bits of adder per instance. Its model, from the fields
//! `docs/fpga-trellis.md` reads out of `ecppack`'s own bitstreams:
//!
//! ```text
//! propagate[i] = INIT<i>[12 + 2b + a]   (the lookup table, C and D high)
//! generate[i]  = INIT<i>[2b + a]        (its low nibble, a table of its own)
//! S<i>         = propagate[i] ^ carry[i]
//! carry[0]     = CIN
//! carry[i+1]   = propagate[i] ? carry[i] : generate[i]
//! COUT         = carry[2]
//! ```
//!
//! That matters more here than it does for the 7 series, because **the
//! equivalence proof the rest of this tree relies on cannot see a carry
//! cell at all**: `every_block_maps_to_the_logic_it_was_mapped_from`
//! proves the *technology mapper* against its own AIG, and a carry cell is
//! emitted before that pass runs and is a blackbox the AIG never holds.
//! `mod proofs` below is the only place in the tree where a SAT solver is
//! shown the inside of one.
//!
//! The iCE40 is checked here too, because the point of each extension to
//! the device model is that it changed nothing for the families that were
//! already right: it still maps its one-bit `SB_CARRY`.

#![cfg(all(feature = "fpga", feature = "synth"))]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{eval_expr, expr_nets};
use reticle::diag::Diagnostics;
use reticle::fpga::{self, BelRole, Constraints, FpgaOptions, MapOptions};
use reticle::ir::builder::ModuleBuilder;
use reticle::ir::validate::validate_module;
use reticle::ir::{
    AttrValue, Cell, CellId, CellKind, Design, ExprKind, Module, ModuleId, Name, NetId, PortDir,
    Type,
};
use reticle::logic::{Bit, Logic};
use reticle::source::{SourceMap, Span};

const XC7: &str = "xc7a35t-cpg236";
const ECP5: &str = "ecp5-45f-CABGA381";

/// Mapping with no IO or clock buffers, which is how the ECP5 runs are
/// made: a `TRELLIS_IO` is a pad and a pad is not a wire this file can
/// evaluate, and what is under test is the carry chain.
fn bare() -> MapOptions {
    MapOptions {
        insert_io_buffers: false,
        insert_clock_buffers: false,
        ..MapOptions::default()
    }
}

fn span() -> Span {
    let mut map = SourceMap::new();
    let id = map.add("fpga-carry", "").unwrap();
    Span::new(id, 0, 0)
}

// ---------------------------------------------------------------------------
// The designs
// ---------------------------------------------------------------------------

/// What the module under test computes, and what the check compares
/// against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// `y = a + b`, `width` bits, wrapping.
    Plain,
    /// `y = a + b + cin`: two `add` cells, the second one carrying a
    /// one-bit operand, which is how a carry-in reaches an IR that has
    /// no carry-in port. Both values of `cin` are simulated.
    CarryIn,
    /// `y = a + b` on `width + 1` bits with both operands zero-extended,
    /// so the top bit of `y` is the carry out of the `width`-bit sum.
    Widened,
    /// `y = a + k`: one operand is a constant, which is what an
    /// incrementing counter looks like by the time it reaches mapping.
    Constant(u64),
    /// `y = a - b`, which no family lowers through the carry path; it is
    /// here to prove that it still comes out right.
    Subtract,
    /// `y = {0, a} - {0, b}` at three times the inputs' width: a
    /// difference whose upper operand bits are constant, which is what
    /// Verilog makes of `outside - take <= EXTRA` (the `EXTRA` is 32 bits
    /// wide) in the Husky Plus's slow FIFO.
    SubtractWidened,
    /// `y = (a - b) + cin`: a carry-in added to a difference that stays
    /// in lookup tables, so every lane `cin` does not reach has its
    /// generate equal to its propagate, a lookup table's output. That is
    /// the lane `legalise_carries` ties to zero, and the shape NewAE's
    /// Husky FIFO's `rused` has.
    SubtractThenCarryIn,
    /// `y = s ? (c ± d) : (a ± b)`, each side a subtraction when its flag
    /// says so: two adders behind a multiplexer, which `primitives` maps
    /// as one chain with its operands chosen instead, the carry in a
    /// subtraction needs riding in an extra low lane. NewAE's `esad` is
    /// `decision ? x - r : y + r` in each of 256 counters.
    Chosen { low_sub: bool, high_sub: bool },
    /// `y = s ? (c ^ e) - d : a + b`, `e` one bit on `c`'s lowest: the
    /// shared adders again, their carry in a signal (`s`), with bit 0's
    /// propagate reading six signals (`s`, `a`, `b`, `c`, `d`, `e`), more
    /// than a `LUT6_2` and its generate can share.
    ChosenXor,
    /// `y = a == b + 1`, one bit, which `primitives` compares on the chain
    /// at [`MapOptions::min_compare_carry_width`] bits and more, an operand
    /// being a sum: the carry out of `{0, L} + 1`, read as the top lane's
    /// sum alone.
    Equal,
    /// `y = a == b`, one bit, two signals: what `--synthesis vivado`
    /// compares on the chain from `flow::VIVADO_MIN_EQUALITY_CARRY_WIDTH`
    /// bits, in lanes of `flow::VIVADO_COMPARE_LANE_BITS` bit pairs.
    EqualPlain,
    /// `y = {0, a} == {0, b} - 2` at three times the inputs' width: an
    /// equality whose upper operand bits are constant, which is what
    /// Verilog makes of `presample_counter == presample_i - 2` (the `2`
    /// is 32 bits wide) in NewAE's capture.
    EqualWidened,
    /// `y = a < b`, `a <= b`, `a > b` or `a >= b`, one bit, the operands
    /// signed or not: what `primitives` compares on the chain in lanes of
    /// two bits at [`MapOptions::min_magnitude_carry_width`] bits and more.
    Order { op: Order, signed: bool },
    /// `y = a OP k`, `k` a constant of the operand's width and type: what
    /// `primitives` compares in kept lanes of a table's inputs each at
    /// [`MapOptions::min_constant_ordering_lane_width`] bits and more.
    OrderConst { op: Order, signed: bool, k: u64 },
}

/// Which ordering a [`Shape::Order`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    Lt,
    Le,
    Gt,
    Ge,
}

impl Order {
    const ALL: [Order; 4] = [Order::Lt, Order::Le, Order::Gt, Order::Ge];
}

impl Shape {
    /// The module, before any mapping.
    fn module(self, name: &str, width: u32) -> Module {
        let mut b = ModuleBuilder::new(name, span());
        match self {
            Shape::Plain | Shape::Subtract => {
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let y = b.output("y", Type::bits(width));
                let (ae, be) = (b.net(a), b.net(bn));
                let kind = if self == Shape::Plain {
                    CellKind::Add
                } else {
                    CellKind::Sub
                };
                b.cell2("sum", kind, ae, be, y);
            }
            Shape::SubtractThenCarryIn => {
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let cin = b.input("cin", Type::bit());
                let y = b.output("y", Type::bits(width));
                let (ae, be) = (b.net(a), b.net(bn));
                let partial = b.add_net("partial", Type::bits(width));
                b.cell2("difference", CellKind::Sub, ae, be, partial);
                let (pe, ce) = (b.net(partial), b.net(cin));
                let widened = b.zext(ce, width);
                b.cell2("bump", CellKind::Add, pe, widened, y);
            }
            Shape::CarryIn => {
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let cin = b.input("cin", Type::bit());
                let y = b.output("y", Type::bits(width));
                let (ae, be) = (b.net(a), b.net(bn));
                let partial = b.add_net("partial", Type::bits(width));
                b.cell2("sum", CellKind::Add, ae, be, partial);
                let (pe, ce) = (b.net(partial), b.net(cin));
                let widened = b.zext(ce, width);
                b.cell2("bump", CellKind::Add, pe, widened, y);
            }
            Shape::Widened => {
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let y = b.output("y", Type::bits(width + 1));
                let (ae, be) = (b.net(a), b.net(bn));
                let (za, zb) = (b.zext(ae, width + 1), b.zext(be, width + 1));
                b.cell2("sum", CellKind::Add, za, zb, y);
            }
            Shape::Equal => {
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let y = b.output("y", Type::bit());
                let (ae, be) = (b.net(a), b.net(bn));
                let one = b.const_u64(width, 1);
                let next = b.add_net("next", Type::bits(width));
                b.cell2("bump", CellKind::Add, be, one, next);
                let ne = b.net(next);
                b.cell2("same", CellKind::Eq, ae, ne, y);
            }
            Shape::EqualPlain => {
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let y = b.output("y", Type::bit());
                let (ae, be) = (b.net(a), b.net(bn));
                b.cell2("same", CellKind::Eq, ae, be, y);
            }
            Shape::SubtractWidened => {
                let wide = width * 3;
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let y = b.output("y", Type::bits(wide));
                let (ae, be) = (b.net(a), b.net(bn));
                let (za, zb) = (b.zext(ae, wide), b.zext(be, wide));
                b.cell2("difference", CellKind::Sub, za, zb, y);
            }
            Shape::EqualWidened => {
                let wide = width * 3;
                let a = b.input("a", Type::bits(width));
                let bn = b.input("b", Type::bits(width));
                let y = b.output("y", Type::bit());
                let (ae, be) = (b.net(a), b.net(bn));
                let (za, zb) = (b.zext(ae, wide), b.zext(be, wide));
                let two = b.const_u64(wide, 2);
                let less = b.add_net("less", Type::bits(wide));
                b.cell2("less", CellKind::Sub, zb, two, less);
                let le = b.net(less);
                b.cell2("same", CellKind::Eq, za, le, y);
            }
            Shape::Order { op, signed } => {
                let ty = if signed {
                    Type::sbits(width)
                } else {
                    Type::bits(width)
                };
                let a = b.input("a", ty.clone());
                let bn = b.input("b", ty);
                let y = b.output("y", Type::bit());
                let (ae, be) = (b.net(a), b.net(bn));
                let kind = match op {
                    Order::Lt => CellKind::Lt,
                    Order::Le => CellKind::Le,
                    Order::Gt => CellKind::Gt,
                    Order::Ge => CellKind::Ge,
                };
                b.cell2("order", kind, ae, be, y);
            }
            Shape::OrderConst { op, signed, k } => {
                let ty = if signed {
                    Type::sbits(width)
                } else {
                    Type::bits(width)
                };
                let a = b.input("a", ty.clone());
                let y = b.output("y", Type::bit());
                let ae = b.net(a);
                let mut value = reticle::ir::Const::from_u64(k, width);
                if signed {
                    value = value.with_signed(true);
                }
                let ke = b.constant(value);
                let kind = match op {
                    Order::Lt => CellKind::Lt,
                    Order::Le => CellKind::Le,
                    Order::Gt => CellKind::Gt,
                    Order::Ge => CellKind::Ge,
                };
                b.cell2("order", kind, ae, ke, y);
            }
            Shape::Constant(k) => {
                let a = b.input("a", Type::bits(width));
                let y = b.output("y", Type::bits(width));
                let ae = b.net(a);
                let ke = b.const_u64(width, k);
                b.cell2("sum", CellKind::Add, ae, ke, y);
            }
            Shape::ChosenXor => {}
            Shape::Chosen { low_sub, high_sub } => {
                let ports: Vec<NetId> = ["a", "b", "c", "d"]
                    .iter()
                    .map(|n| b.input(*n, Type::bits(width)))
                    .collect();
                let sel = b.input("s", Type::bit());
                let y = b.output("y", Type::bits(width));
                let kind = |sub: bool| if sub { CellKind::Sub } else { CellKind::Add };
                let low = b.add_net("low", Type::bits(width));
                let high = b.add_net("high", Type::bits(width));
                let (ae, be) = (b.net(ports[0]), b.net(ports[1]));
                b.cell2("low", kind(low_sub), ae, be, low);
                let (ce, de) = (b.net(ports[2]), b.net(ports[3]));
                b.cell2("high", kind(high_sub), ce, de, high);
                let (le, he, se) = (b.net(low), b.net(high), b.net(sel));
                b.cell(
                    "choose",
                    CellKind::Mux,
                    vec![
                        (Name::new("a"), le),
                        (Name::new("b"), he),
                        (Name::new("s"), se),
                    ],
                    vec![(Name::new("y"), y)],
                );
            }
        }
        if self == Shape::ChosenXor {
            let ports: Vec<NetId> = ["a", "b", "c", "d"]
                .iter()
                .map(|n| b.input(*n, Type::bits(width)))
                .collect();
            let e = b.input("e", Type::bit());
            let sel = b.input("s", Type::bit());
            let y = b.output("y", Type::bits(width));
            let low = b.add_net("low", Type::bits(width));
            let high = b.add_net("high", Type::bits(width));
            let (ae, be) = (b.net(ports[0]), b.net(ports[1]));
            b.cell2("low", CellKind::Add, ae, be, low);
            let (ce, ee) = (b.net(ports[2]), b.net(e));
            let widened = b.zext(ee, width);
            let flipped = b.xor(ce, widened);
            let de = b.net(ports[3]);
            b.cell2("high", CellKind::Sub, flipped, de, high);
            let (le, he, se) = (b.net(low), b.net(high), b.net(sel));
            b.cell(
                "choose",
                CellKind::Mux,
                vec![
                    (Name::new("a"), le),
                    (Name::new("b"), he),
                    (Name::new("s"), se),
                ],
                vec![(Name::new("y"), y)],
            );
        }
        let module = b.finish();
        let problems = validate_module(&module);
        assert!(
            !problems.has_errors(),
            "`{}` is not a valid module: {:?}",
            module.name,
            problems.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        module
    }

    /// The width of the result.
    fn result_width(self, width: u32) -> u32 {
        match self {
            Shape::Widened => width + 1,
            Shape::SubtractWidened => width * 3,
            Shape::Equal
            | Shape::EqualPlain
            | Shape::EqualWidened
            | Shape::Order { .. }
            | Shape::OrderConst { .. } => 1,
            _ => width,
        }
    }

    /// What the module must answer for the given inputs, in
    /// [`Shape::inputs`]' order.
    fn expected(self, width: u32, values: &[u64]) -> u64 {
        let mask = |w: u32| if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let at = |i: usize| values.get(i).copied().unwrap_or(0);
        let (a, b, cin) = match self {
            Shape::Constant(_) => (at(0), 0, 0),
            Shape::OrderConst { k, .. } => (at(0), k & mask(width), 0),
            _ => (at(0), at(1), at(2)),
        };
        match self {
            Shape::Chosen { low_sub, high_sub } => {
                let op = |sub: bool, x: u64, y: u64| {
                    if sub {
                        x.wrapping_sub(y)
                    } else {
                        x.wrapping_add(y)
                    }
                };
                let value = if at(4) == 1 {
                    op(high_sub, at(2), at(3))
                } else {
                    op(low_sub, a, b)
                };
                value & mask(width)
            }
            Shape::ChosenXor => {
                let value = if at(5) == 1 {
                    (at(2) ^ at(4)).wrapping_sub(at(3))
                } else {
                    a.wrapping_add(b)
                };
                value & mask(width)
            }
            Shape::Plain => a.wrapping_add(b) & mask(width),
            Shape::CarryIn => a.wrapping_add(b).wrapping_add(cin) & mask(width),
            Shape::Widened => (a + b) & mask(width + 1),
            Shape::Constant(k) => a.wrapping_add(k) & mask(width),
            Shape::Subtract => a.wrapping_sub(b) & mask(width),
            Shape::SubtractWidened => a.wrapping_sub(b) & mask(width * 3),
            Shape::SubtractThenCarryIn => a.wrapping_sub(b).wrapping_add(cin) & mask(width),
            Shape::Equal => u64::from(a == (b + 1) & mask(width)),
            Shape::EqualPlain => u64::from(a == b),
            Shape::EqualWidened => u64::from(a == b.wrapping_sub(2) & mask(width * 3)),
            Shape::Order { op, signed } | Shape::OrderConst { op, signed, .. } => {
                // Each operand as the number it is: sign-extended from its
                // own top bit when signed.
                let value = |x: u64| -> i128 {
                    if signed && (x >> (width - 1)) & 1 == 1 {
                        i128::from(x) - (1i128 << width)
                    } else {
                        i128::from(x)
                    }
                };
                let (x, y) = (value(a), value(b));
                u64::from(match op {
                    Order::Lt => x < y,
                    Order::Le => x <= y,
                    Order::Gt => x > y,
                    Order::Ge => x >= y,
                })
            }
        }
    }

    /// The input ports, in the order the exhaustive loop counts them.
    fn inputs(self, width: u32) -> Vec<(&'static str, u32)> {
        match self {
            Shape::Constant(_) | Shape::OrderConst { .. } => vec![("a", width)],
            Shape::CarryIn | Shape::SubtractThenCarryIn => {
                vec![("a", width), ("b", width), ("cin", 1)]
            }
            Shape::Chosen { .. } => vec![
                ("a", width),
                ("b", width),
                ("c", width),
                ("d", width),
                ("s", 1),
            ],
            Shape::ChosenXor => vec![
                ("a", width),
                ("b", width),
                ("c", width),
                ("d", width),
                ("e", 1),
                ("s", 1),
            ],
            _ => vec![("a", width), ("b", width)],
        }
    }
}

/// Runs the whole FPGA flow over one shape and answers the mapped
/// design, the module and the report.
fn mapped(
    shape: Shape,
    width: u32,
    device_name: &str,
    map: MapOptions,
) -> (Design, ModuleId, fpga::FlowReport) {
    let options = FpgaOptions {
        map,
        ..FpgaOptions::default()
    };
    mapped_with(shape, width, device_name, &options)
}

/// The flow's options as `reticle fpga` gives them to the 7 series by
/// default: [`FpgaOptions::vivado_synthesis`] on.
fn vivado() -> FpgaOptions {
    FpgaOptions {
        vivado_synthesis: true,
        ..FpgaOptions::default()
    }
}

/// [`mapped`] with the whole of the flow's options.
fn mapped_with(
    shape: Shape,
    width: u32,
    device_name: &str,
    options: &FpgaOptions,
) -> (Design, ModuleId, fpga::FlowReport) {
    let mut design = Design::new();
    let top = design.add_module(shape.module("top", width));
    design.top = Some(top);
    let device = fpga::target(device_name).expect("a built-in device");
    let constraints = Constraints::default();
    let mut diags = Diagnostics::new();
    let report = fpga::synthesize_for(&mut design, top, device, &constraints, options, &mut diags)
        .expect("the flow finished");
    assert!(
        !diags.has_errors(),
        "the flow reported errors: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let problems = validate_module(design.module(top));
    assert!(
        !problems.has_errors(),
        "the mapped module is not valid: {:?}",
        problems.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    (design, top, report)
}

// ---------------------------------------------------------------------------
// Simulating the mapped netlist
// ---------------------------------------------------------------------------

/// One thing to evaluate: a cell, or a continuous assignment by index.
#[derive(Clone, Copy, Debug)]
enum Step {
    Cell(CellId),
    Assign(usize),
}

/// A combinational netlist of device primitives, ordered once and then
/// evaluated on as many input vectors as the caller likes.
///
/// It models exactly the primitives the 7-series flow emits — the buffers,
/// `LUT1`..`LUT6` and `CARRY4` — and the handful of generic cells that
/// survive when the caller turns the rewrite to device cells off. Anything
/// else is a panic rather than a silent zero: a netlist holding a
/// primitive this file does not model is one this file cannot vouch for.
struct Netlist<'m> {
    module: &'m Module,
    steps: Vec<Step>,
}

impl<'m> Netlist<'m> {
    fn new(module: &'m Module) -> Netlist<'m> {
        // Nets a step needs, and nets it produces.
        let mut pending: Vec<(Step, Vec<NetId>, Vec<NetId>)> = Vec::new();
        for (id, cell) in module.cells.iter() {
            let mut needs = Vec::new();
            for (_, e) in &cell.inputs {
                needs.extend(expr_nets(module, *e));
            }
            let makes = cell.outputs.iter().map(|(_, n)| *n).collect();
            pending.push((Step::Cell(id), needs, makes));
        }
        for (index, assign) in module.assigns.iter().enumerate() {
            let target = match assign.target {
                reticle::ir::Lvalue::Net(net) => net,
                _ => panic!("the mapped netlist assigns to something that is not a net"),
            };
            let needs = expr_nets(module, assign.value);
            pending.push((Step::Assign(index), needs, vec![target]));
        }
        // Everything a port or a constant supplies is ready to begin with.
        let mut ready: BTreeSet<NetId> = module
            .ports
            .iter()
            .filter(|p| p.dir != reticle::ir::PortDir::Out)
            .map(|p| p.net)
            .collect();
        let mut steps = Vec::with_capacity(pending.len());
        let mut done = vec![false; pending.len()];
        loop {
            let mut progress = false;
            for (index, (step, needs, makes)) in pending.iter().enumerate() {
                if done[index] || !needs.iter().all(|n| ready.contains(n)) {
                    continue;
                }
                done[index] = true;
                progress = true;
                steps.push(*step);
                ready.extend(makes.iter().copied());
            }
            if !progress {
                break;
            }
        }
        let stuck: Vec<String> = pending
            .iter()
            .zip(&done)
            .filter(|(_, done)| !**done)
            .map(|((step, _, _), _)| match step {
                Step::Cell(id) => format!("cell `{}`", module.cells[*id].name),
                Step::Assign(index) => format!("assign #{index}"),
            })
            .collect();
        assert!(
            stuck.is_empty(),
            "the netlist has a combinational loop, or a net nothing drives: {stuck:?}"
        );
        Netlist { module, steps }
    }

    /// Evaluates every step under `inputs`, which must give a value for
    /// each input port.
    fn eval(&self, inputs: &BTreeMap<NetId, Logic>) -> BTreeMap<NetId, Logic> {
        let mut values = inputs.clone();
        for step in &self.steps {
            match *step {
                Step::Assign(index) => {
                    let assign = &self.module.assigns[index];
                    let reticle::ir::Lvalue::Net(target) = assign.target else {
                        unreachable!("checked when ordering")
                    };
                    let width = self.module.nets[target].ty.width().unwrap_or(1);
                    let value = eval_expr(self.module, assign.value, &values).resize(width);
                    values.insert(target, value);
                }
                Step::Cell(id) => {
                    let cell = &self.module.cells[id];
                    self.eval_cell(cell, &mut values);
                }
            }
        }
        values
    }

    fn eval_cell(&self, cell: &Cell, values: &mut BTreeMap<NetId, Logic>) {
        let input = |port: &str, values: &BTreeMap<NetId, Logic>| -> Option<Logic> {
            cell.input(port).map(|e| eval_expr(self.module, e, values))
        };
        let one = |port: &str, values: &BTreeMap<NetId, Logic>| -> Bit {
            input(port, values).map_or(Bit::Zero, |v| v.bit(0))
        };
        let name = match &cell.kind {
            CellKind::Blackbox(name) => name.as_str().to_owned(),
            _ => String::new(),
        };
        // The generic cells that survive when the caller asks for no
        // device cells; the mapper's own XOR is one of them.
        if name.is_empty() {
            let Some(y) = cell.output("y") else {
                panic!("cell `{}` has no `y` output", cell.name);
            };
            let width = self.module.nets[y].ty.width().unwrap_or(1);
            let value = match &cell.kind {
                CellKind::Not => input("a", values).expect("`a`").not(),
                CellKind::Buf => input("a", values).expect("`a`"),
                CellKind::And => input("a", values)
                    .expect("`a`")
                    .and(&input("b", values).expect("`b`")),
                CellKind::Or => input("a", values)
                    .expect("`a`")
                    .or(&input("b", values).expect("`b`")),
                CellKind::Xor => input("a", values)
                    .expect("`a`")
                    .xor(&input("b", values).expect("`b`")),
                CellKind::Mux => {
                    if one("s", values) == Bit::One {
                        input("b", values).expect("`b`")
                    } else {
                        input("a", values).expect("`a`")
                    }
                }
                CellKind::Lut { k, init } => {
                    let a = input("a", values).expect("`a`");
                    let mut index = 0u32;
                    for bit in 0..*k {
                        if a.bit(bit) == Bit::One {
                            index |= 1 << bit;
                        }
                    }
                    Logic::from_bit(init.bit(index))
                }
                other => panic!("cell `{}` of kind `{other:?}` is not modelled", cell.name),
            };
            values.insert(y, value.resize(width));
            return;
        }
        match name.as_str() {
            // A buffer passes its one input through; which pin is which
            // differs, so the model is "the only input reaches the only
            // output".
            "IBUF" | "OBUF" | "BUFG" => {
                let (_, e) = cell.inputs.first().expect("a buffer has an input");
                let value = eval_expr(self.module, *e, values);
                let (_, out) = cell.outputs.first().expect("a buffer has an output");
                let width = self.module.nets[*out].ty.width().unwrap_or(1);
                values.insert(*out, value.resize(width));
            }
            // A LUT of whatever width, indexed by **its own input ports in
            // declaration order**: the 7 series calls them `I0`..`I5` and
            // an ECP5 calls them `A`..`D`, and the constant drivers
            // `techcells::drive_constant_data` builds are ECP5 `LUT4`s.
            lut if lut.starts_with("LUT") && lut.len() == 4 => {
                let init = match cell.params.get("INIT") {
                    Some(AttrValue::Const(c)) => c.clone(),
                    other => panic!("`{}` has INIT {other:?}", cell.name),
                };
                let mut index = 0u32;
                for (pin, (port, _)) in cell.inputs.iter().enumerate() {
                    if one(port.as_str(), values) == Bit::One {
                        index |= 1 << pin;
                    }
                }
                let (_, out) = cell.outputs.first().expect("a LUT has an output");
                values.insert(*out, Logic::from_bit(init.bit(index)));
            }
            // Two functions of one table (UG474's `LUT6_2`): `O6` reads
            // the whole of `INIT`, `O5` its lower half whatever `I5` is.
            "LUT6_2" => {
                let init = match cell.params.get("INIT") {
                    Some(AttrValue::Const(c)) => c.clone(),
                    other => panic!("`{}` has INIT {other:?}", cell.name),
                };
                let mut index = 0u32;
                for pin in 0..6u32 {
                    if one(&format!("I{pin}"), values) == Bit::One {
                        index |= 1 << pin;
                    }
                }
                for (port, at) in [("O6", index), ("O5", index & 31)] {
                    if let Some(out) = cell.output(port) {
                        values.insert(out, Logic::from_bit(init.bit(at)));
                    }
                }
            }
            // CARRY4, straight from the equations in `cells_sim.v` that
            // `xc7.dev` quotes.
            "CARRY4" => {
                let s = input("S", values).expect("`S`");
                let di = input("DI", values).expect("`DI`");
                let ci = one("CI", values);
                let cyinit = one("CYINIT", values);
                let mut carry = if ci == Bit::One || cyinit == Bit::One {
                    Bit::One
                } else {
                    Bit::Zero
                };
                let (mut o, mut co) = (Logic::zero(4), Logic::zero(4));
                for bit in 0..4 {
                    let propagate = s.bit(bit) == Bit::One;
                    let sum = if propagate {
                        !matches!(carry, Bit::One)
                    } else {
                        matches!(carry, Bit::One)
                    };
                    o.set_bit(bit, if sum { Bit::One } else { Bit::Zero });
                    carry = if propagate { carry } else { di.bit(bit) };
                    co.set_bit(bit, carry);
                }
                let out = |port: &str| -> NetId {
                    cell.output(port)
                        .unwrap_or_else(|| panic!("`{}` has no `{port}`", cell.name))
                };
                values.insert(out("O"), o);
                values.insert(out("CO"), co);
            }
            // CCU2C, from the equations in this file's own header.
            "CCU2C" => {
                let init = |lane: u32| -> reticle::ir::Const {
                    match cell.params.get(&format!("INIT{lane}")) {
                        Some(AttrValue::Const(c)) => c.clone(),
                        other => panic!("`{}` has INIT{lane} {other:?}", cell.name),
                    }
                };
                let mut carry = one("CIN", values);
                for lane in 0..2u32 {
                    let init = init(lane);
                    // A lane with no operand pins is a zero of the chain
                    // and its INIT says so, so which value an absent pin
                    // is modelled with cannot matter. `src/fpga/
                    // primitives.rs`'s own check varies it both ways.
                    let index = u32::from(one(&format!("B{lane}"), values) == Bit::One) * 2
                        + u32::from(one(&format!("A{lane}"), values) == Bit::One);
                    let propagate = init.bit(12 + index) == Bit::One;
                    let generate = init.bit(index);
                    let sum = if propagate {
                        !matches!(carry, Bit::One)
                    } else {
                        matches!(carry, Bit::One)
                    };
                    if let Some(out) = cell.output(&format!("S{lane}")) {
                        values.insert(out, Logic::from_bit(if sum { Bit::One } else { Bit::Zero }));
                    }
                    carry = if propagate { carry } else { generate };
                }
                if let Some(out) = cell.output("COUT") {
                    values.insert(out, Logic::from_bit(carry));
                }
            }
            other => panic!("primitive `{other}` (cell `{}`) is not modelled", cell.name),
        }
    }
}

/// The other part a simulation cannot see: **a carry in from the fabric has
/// `AX` to itself.** `CYINIT` is reached from `AX` alone, and so is a generate
/// from the fabric, by the bypass input; simulated, the two are just two nets.
/// So every `CARRY4` whose `CYINIT` is a signal takes bit 0's generate off the
/// `O5` of the `LUT6_2` whose `O6` is bit 0's propagate, or a constant, which
/// comes off `O5` too.
fn carry_ins_have_ax_to_themselves(module: &Module) {
    use reticle::ir::emit::{BitView, SigBit};
    let view = BitView::new(module).expect("the legalised module is structural");
    let mut driver: std::collections::HashMap<SigBit, (CellId, String)> =
        std::collections::HashMap::new();
    for (id, cell) in module.cells.iter() {
        for (port, net) in &cell.outputs {
            for bit in view.net_bits(*net, cell.span).expect("a bit vector") {
                driver.insert(view.canonical(bit), (id, port.as_str().to_owned()));
            }
        }
    }
    let bits = |e| -> Vec<SigBit> {
        view.expr_bits(e)
            .expect("a structural pin")
            .into_iter()
            .map(|b| view.canonical(b))
            .collect()
    };
    for (_, cell) in module.cells.iter() {
        if !matches!(&cell.kind, CellKind::Blackbox(n) if n.as_str() == "CARRY4") {
            continue;
        }
        let Some(init) = cell.input("CYINIT") else {
            continue;
        };
        if !matches!(bits(init).first(), Some(SigBit::Slot(_))) {
            continue;
        }
        let s = bits(cell.input("S").expect("`S`"))[0];
        let di = bits(cell.input("DI").expect("`DI`"))[0];
        if matches!(di, SigBit::Const(_)) {
            continue;
        }
        let (dual, port) = driver
            .get(&di)
            .unwrap_or_else(|| panic!("`{}`'s bit 0 generate has no driver", cell.name));
        assert!(
            matches!(&module.cells[*dual].kind, CellKind::Blackbox(n) if n.as_str() == "LUT6_2")
                && port == "O5"
                && driver.get(&s) == Some(&(*dual, "O6".to_owned())),
            "`{}` takes its carry in on `CYINIT` and bit 0's generate from `{}`'s `{port}`, \
             not its propagate's `O5`: both on `AX`",
            cell.name,
            module.cells[*dual].name
        );
    }
}

/// The part of a packed chain a simulation cannot see: **which pin** of a
/// `LUT6_2` each carry input is wired to. Simulated, a net has its value
/// whichever output drives it, so a pairing with its halves swapped
/// answers every sum correctly; on the slice `S` is the table's `O6` and
/// nothing else, and a generate from the table is its `O5` through
/// `<L>CY0`. So: every lane whose generate a `LUT6_2` drives takes it off
/// that cell's `O5`, and takes its propagate off the same cell's `O6`.
fn generates_come_off_their_own_o5(module: &Module) {
    use reticle::ir::emit::{BitView, SigBit};
    let view = BitView::new(module).expect("the packed module is structural");
    let mut driver: std::collections::HashMap<SigBit, (CellId, String)> =
        std::collections::HashMap::new();
    for (id, cell) in module.cells.iter() {
        for (port, net) in &cell.outputs {
            for bit in view.net_bits(*net, cell.span).expect("a bit vector") {
                driver.insert(view.canonical(bit), (id, port.as_str().to_owned()));
            }
        }
    }
    let bits = |e| -> Vec<SigBit> {
        view.expr_bits(e)
            .expect("a structural pin")
            .into_iter()
            .map(|b| view.canonical(b))
            .collect()
    };
    let mut lanes = 0;
    for (_, cell) in module.cells.iter() {
        if !matches!(&cell.kind, CellKind::Blackbox(n) if n.as_str() == "CARRY4") {
            continue;
        }
        let s = bits(cell.input("S").expect("`S`"));
        let di = bits(cell.input("DI").expect("`DI`"));
        for (s, di) in s.iter().zip(&di) {
            let Some((dual, port)) = driver.get(di) else {
                continue;
            };
            if !matches!(&module.cells[*dual].kind, CellKind::Blackbox(n) if n.as_str() == "LUT6_2")
            {
                continue;
            }
            assert_eq!(port, "O5", "a generate off a `LUT6_2`'s `{port}`");
            assert_eq!(
                driver.get(s),
                Some(&(*dual, "O6".to_owned())),
                "a generate off one table's `O5` and a propagate off another's"
            );
            lanes += 1;
        }
    }
    assert!(lanes > 0, "no lane took its generate off `O5`");
}

/// The net of a port, by name.
fn port_net(module: &Module, name: &str) -> NetId {
    module
        .ports
        .iter()
        .find(|p| p.name.as_str() == name)
        .unwrap_or_else(|| panic!("no port `{name}`"))
        .net
}

/// Simulates every input combination of `shape` at `width` bits and
/// compares with the arithmetic it stands for.
fn check_exhaustively(shape: Shape, width: u32, map: MapOptions) {
    check_full(shape, width, XC7, map, false, false);
}

/// The same, on a named device.
fn check_on(shape: Shape, width: u32, device: &str, map: MapOptions) {
    check_full(shape, width, device, map, false, false);
}

/// [`check_exhaustively`], optionally after `fpga::xray::legalise_carries`
/// has given every propagate bit a lookup table of its own.
fn check(shape: Shape, width: u32, map: MapOptions, legalise: bool) {
    check_full(shape, width, XC7, map, legalise, false);
}

/// [`check`] with the chain legalised and then the lookup tables paired
/// into `LUT6_2` by `fpga::xray::pack_lut_pairs`, which is the netlist
/// the 7-series bitstream is built from: a lane's generate then comes off
/// its propagate table's `O5`.
fn check_packed(shape: Shape, width: u32) {
    check_full(shape, width, XC7, MapOptions::default(), true, true);
}

/// The one simulation the wrappers above all reach: map `shape` at
/// `width` bits for `device`, legalise the chain for a slice if asked,
/// pair its lookup tables if asked, then evaluate every input combination.
fn check_full(shape: Shape, width: u32, device: &str, map: MapOptions, legalise: bool, pack: bool) {
    let options = FpgaOptions {
        map,
        ..FpgaOptions::default()
    };
    check_full_with(shape, width, device, &options, legalise, pack);
}

/// [`check_full`] with the whole of the flow's options.
fn check_full_with(
    shape: Shape,
    width: u32,
    device: &str,
    options: &FpgaOptions,
    legalise: bool,
    pack: bool,
) -> Option<fpga::xray::CarryPacking> {
    assert!(
        !legalise || device == XC7,
        "`legalise_carries` is the 7 series' own pass and there is nothing to ask it on {device}"
    );
    let (mut design, top, report) = mapped_with(shape, width, device, options);
    let mut packing = None;
    if legalise {
        let device = fpga::target(device).expect("a built-in device");
        // As `reticle fpga` legalises: with Vivado's synthesis, a constant
        // generate on its propagate's own table where it fits.
        let carry = fpga::xray::CarryOptions {
            generates_on_their_tables: options.vivado_synthesis,
        };
        packing = Some(
            fpga::xray::legalise_carries_with(&mut design, top, device, &carry)
                .expect("the chain legalises"),
        );
        carry_ins_have_ax_to_themselves(design.module(top));
        if pack {
            let pairing = fpga::xray::pack_lut_pairs(
                &mut design,
                top,
                device,
                &fpga::xray::PairOptions::default(),
            )
            .expect("the lookup tables pair");
            assert!(pairing.pairs > 0, "nothing paired: {pairing:?}");
            generates_come_off_their_own_o5(design.module(top));
        }
        let problems = validate_module(design.module(top));
        assert!(
            !problems.has_errors(),
            "the legalised module is not valid: {:?}",
            problems.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
    let module = design.module(top);
    let netlist = Netlist::new(module);
    let ports = shape.inputs(width);
    let bits: u32 = ports.iter().map(|(_, w)| *w).sum();
    assert!(bits <= 20, "{bits} bits is too many to enumerate");
    let result = port_net(module, "y");
    let result_width = shape.result_width(width);
    // Every pattern is checked, the range cut into one run per core: the
    // patterns are independent, and a 17-bit shape is 131,072 of them
    // through an interpreter. A run that fails panics with its own
    // message, and the scope then fails the test.
    let total = 1u64 << bits;
    let runs = std::thread::available_parallelism()
        .map_or(1, |n| n.get() as u64)
        .clamp(1, 16)
        .min(total);
    let checked = std::sync::atomic::AtomicU64::new(0);
    let check_range = |range: std::ops::Range<u64>| {
        for pattern in range {
            checked.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut inputs = BTreeMap::new();
            let mut taken = 0;
            let mut values = [0u64; 6];
            for (index, (name, w)) in ports.iter().enumerate() {
                let mask = (1u64 << w) - 1;
                let value = (pattern >> taken) & mask;
                taken += w;
                values[index] = value;
                inputs.insert(port_net(module, name), Logic::from_u64(value, *w));
            }
            let got = netlist.eval(&inputs);
            let got = got
                .get(&result)
                .unwrap_or_else(|| panic!("`y` was not driven"))
                .clone();
            let want = shape.expected(width, &values[..ports.len()]);
            let want = Logic::from_u64(want, result_width);
            assert_eq!(
                got.to_u64(),
                want.to_u64(),
                "{shape:?} at {width} bits: inputs {:?} gave {got}, wanted {want}\n\
                 the netlist was {:?}",
                &values[..ports.len()],
                report.netlist
            );
        }
    };
    std::thread::scope(|scope| {
        for run in 0..runs {
            let range = total * run / runs..total * (run + 1) / runs;
            let check_range = &check_range;
            scope.spawn(move || check_range(range));
        }
    });
    // The runs cover the range once: none of it was left out.
    assert_eq!(checked.into_inner(), total, "{shape:?} at {width} bits");
    packing
}

// ---------------------------------------------------------------------------
// The exhaustive checks
// ---------------------------------------------------------------------------

/// Four, five and eight bits, every input combination, every shape: one
/// CARRY4 exactly, one and a bit, and two.
#[test]
fn every_input_of_a_narrow_adder_is_simulated() {
    for width in [4, 5, 8] {
        for shape in [Shape::Plain, Shape::Widened, Shape::Constant(1)] {
            check_exhaustively(shape, width, MapOptions::default());
        }
    }
    // The carry-in shape doubles the input space, so it stops at five.
    for width in [4, 5] {
        check_exhaustively(Shape::CarryIn, width, MapOptions::default());
    }
}

/// The awkward widths: one bit, widths that are not a multiple of four,
/// and the widths either side of a whole instance.
#[test]
fn every_input_of_an_awkward_width_is_simulated() {
    let one = MapOptions {
        min_carry_width: 1,
        ..MapOptions::default()
    };
    for width in [1, 2, 3, 6, 7] {
        check_exhaustively(Shape::Plain, width, one.clone());
        check_exhaustively(Shape::Widened, width, one.clone());
    }
}

/// **Legalising a chain for a 7-series slice changes no sum.**
///
/// `fpga::xray::legalise_carries` puts a buffer lookup table in front of
/// every propagate bit that has none, a constant one in front of every
/// constant bit of a lane something reads, and `x` on the lanes nothing
/// reads. The widened shape is the one that exercises the constants: its
/// top lane adds two zeros and its sum is the carry out, so that lane's
/// propagate *and* generate are constants, and its lookup table carries
/// the generate in the lower half of its truth table for `O5` with `I5`
/// tied to one.
///
/// Would catch: a buffer reading the wrong input, a truth table whose
/// upper half is not the propagate, `S` and `DI` reassembled in the wrong
/// lane order, and a lane wrongly judged dead. Would not catch: `O5`
/// reading something other than the lower half on silicon — this model
/// reads `O6` only, and the lower half is the database's reading of the
/// `CARRY4.<L>CY0` mux, quoted in `docs/fpga-xray.md`.
#[test]
fn legalising_a_chain_for_a_slice_changes_no_sum() {
    for width in [4, 5, 8] {
        for shape in [
            Shape::Plain,
            Shape::Widened,
            Shape::Constant(1),
            Shape::Constant(5),
        ] {
            check(shape, width, MapOptions::default(), true);
        }
    }
    check(Shape::CarryIn, 5, MapOptions::default(), true);
    // A generate that is its own lane's propagate, from a lookup table,
    // tied to zero: would catch the identity misapplied.
    check(Shape::SubtractThenCarryIn, 5, MapOptions::default(), true);
    let one = MapOptions {
        min_carry_width: 1,
        ..MapOptions::default()
    };
    for width in [1, 3, 6] {
        check(Shape::Widened, width, one.clone(), true);
    }
}

/// **No lane of a legalised chain puts out both its sum and its carry.**
///
/// A slice has one output per lane for the two, `<L>MUX`, so a lane whose
/// sum and carry out the fabric both reads cannot be routed: the router
/// gave up on four of the language corpus's builds for it and the flow
/// wrote a bitstream with nothing routed. `{0, a} + {0, b}` with its top
/// bit taken off the carry out (`MapOptions::carry_ends`) is that shape,
/// since the port reads the top sum too. Vivado takes the carry out off the
/// lane above, a pass-through (measured: `docs/fpga-vivado-parity.md`), and
/// so does `legalise_carries` now: from the lane above in the same `CARRY4`
/// at 5 to 7 bits, and from a `CARRY4` of its own above the chain at 4 and
/// 8. Every input is simulated after.
///
/// Would catch: the carry out left beside its sum (the structural check),
/// moved onto a lane that does not pass the carry through, or onto the
/// wrong lane (the simulation). Would not catch: a lane above that
/// something reads, which is left alone and has not been seen, nor
/// anything about the slice's output multiplexers themselves.
#[test]
fn no_lane_puts_out_both_its_sum_and_its_carry() {
    let ends = MapOptions {
        carry_ends: true,
        min_carry_width: 1,
        ..MapOptions::default()
    };
    for width in [4, 5, 6, 7, 8] {
        let options = FpgaOptions {
            map: ends.clone(),
            ..FpgaOptions::default()
        };
        let packing =
            check_full_with(Shape::Widened, width, XC7, &options, true, false).expect("legalised");
        assert_eq!(packing.carries_out_moved, 1, "{width} bits: {packing:?}");
        let (mut design, top, _) = mapped_with(Shape::Widened, width, XC7, &options);
        let device = fpga::target(XC7).expect("a built-in device");
        fpga::xray::legalise_carries(&mut design, top, device).expect("the chain legalises");
        let blocks = sums_and_carries_apart(design.module(top));
        assert_eq!(
            blocks,
            usize::try_from(width.div_ceil(4) + u32::from(width % 4 == 0)).unwrap(),
            "{width} bits"
        );
    }
}

/// Fails if any `CARRY4` lane has its sum and its carry out both read by
/// anything but the next instance's `CI`; answers how many instances.
fn sums_and_carries_apart(module: &Module) -> usize {
    use reticle::ir::emit::{BitView, SigBit};
    let view = BitView::new(module).expect("the legalised module is structural");
    let canon = |bits: Vec<SigBit>| -> Vec<SigBit> {
        bits.into_iter().map(|b| view.canonical(b)).collect()
    };
    // Who reads each bit: the cell and pin, or `None` for a port.
    let mut readers: std::collections::HashMap<SigBit, Vec<Option<(CellId, String)>>> =
        std::collections::HashMap::new();
    for (id, cell) in module.cells.iter() {
        for (port, e) in &cell.inputs {
            for bit in canon(view.expr_bits(*e).expect("a structural pin")) {
                readers
                    .entry(bit)
                    .or_default()
                    .push(Some((id, port.as_str().to_owned())));
            }
        }
    }
    for port in &module.ports {
        if port.dir == PortDir::Out {
            for bit in canon(view.net_bits(port.net, port.span).expect("a bit vector")) {
                readers.entry(bit).or_default().push(None);
            }
        }
    }
    let carry = |id: CellId| matches!(&module.cells[id].kind, CellKind::Blackbox(n) if n.as_str() == "CARRY4");
    let mut blocks = 0;
    for (_, cell) in module.cells.iter() {
        if !matches!(&cell.kind, CellKind::Blackbox(n) if n.as_str() == "CARRY4") {
            continue;
        }
        blocks += 1;
        let out = |port: &str| {
            canon(
                view.net_bits(cell.output(port).expect("an output"), cell.span)
                    .expect("a bit vector"),
            )
        };
        let (sums, carries) = (out("O"), out("CO"));
        for lane in 0..4 {
            let sum_read = readers.get(&sums[lane]).is_some_and(|r| !r.is_empty());
            let carry_out = readers.get(&carries[lane]).is_some_and(|r| {
                r.iter()
                    .any(|r| !(lane == 3 && matches!(r, Some((c, p)) if carry(*c) && p == "CI")))
            });
            assert!(
                !(sum_read && carry_out),
                "`{}` puts out lane {lane}'s sum and its carry both",
                cell.name
            );
        }
    }
    blocks
}

/// **Two adders behind a multiplexer share one chain, and each answer is
/// still its own.** Every input of all four combinations of adding and
/// subtracting, at four bits: mapped, then legalised, then with the
/// lookup tables paired, which is where each lane's generate moves onto
/// its propagate table's `O5`.
///
/// Would catch: a carry in of the wrong polarity for the side that
/// subtracts, the complemented operand on the wrong side, the extra low
/// lane's sum leaking into the answer, and a paired table whose halves
/// are swapped or whose `O5` is not the generate it replaced. Would not
/// catch: `CARRY4.<L>CY0` selecting something other than `O5` on
/// silicon, which is prjxray's fuzzer, quoted; the netlist says what the
/// slice is believed to do, and a routed bitstream's every bit decoding
/// says only that the bit was meant.
#[test]
fn a_chosen_sum_shares_a_chain_and_is_still_right() {
    for low_sub in [false, true] {
        for high_sub in [false, true] {
            let shape = Shape::Chosen { low_sub, high_sub };
            check_exhaustively(shape, 4, MapOptions::default());
            check_packed(shape, 4);
            let (_, _, report) = mapped(shape, 8, XC7, MapOptions::default());
            assert_eq!(
                report.primitives.carry_chains.len(),
                1,
                "{shape:?}: one chain for the two"
            );
        }
    }
}

/// **A subtraction goes onto the 7-series carry chain and comes out
/// right.** It used to stay in lookup tables on every family, which this
/// test pinned; a 32-bit decrement was then 38 tables seven deep, and on
/// the ChipWhisperer-Husky Plus a compare against one sixteen tables in a
/// row. With `min_subtract_carry_width` at or below its width, `a - b` is
/// the adder `{a, 1} + {~b, 1}` without its low bit: one `CARRY4` per four
/// bits of the one-wider sum. By default the threshold is `u32::MAX` and a
/// subtraction stays in tables — see the option for the part's reason —
/// and is right there too. With the threshold lowered, the chain is
/// simulated on every input at 4 and 8 bits here, and `mod proofs` proves
/// it at 16 and 32. Would catch the complement or
/// the carry in lost (every difference off by one or more), the low lane
/// kept, or the threshold ignored; would not catch an ECP5 subtraction,
/// which still goes through lookup tables (`the_ecp5_chain_...`).
#[test]
fn a_subtraction_goes_onto_the_chain_and_is_still_right() {
    let low = MapOptions {
        min_subtract_carry_width: 1,
        ..MapOptions::default()
    };
    for width in [4, 8] {
        check_exhaustively(Shape::Subtract, width, MapOptions::default());
        let (_, _, report) = mapped(Shape::Subtract, width, XC7, MapOptions::default());
        assert_eq!(
            report.count("CARRY4"),
            0,
            "{width} bits is below the threshold"
        );

        check_exhaustively(Shape::Subtract, width, low.clone());
        let (_, _, report) = mapped(Shape::Subtract, width, XC7, low.clone());
        assert_eq!(
            report.count("CARRY4"),
            usize::try_from((width + 1).div_ceil(4)).unwrap(),
            "a `sub` of {width} bits is a chain of {} bits",
            width + 1
        );
    }
    let (_, _, report) = mapped(Shape::Subtract, 16, XC7, MapOptions::default());
    assert_eq!(report.count("CARRY4"), 0, "by default none is on the chain");
    let (_, _, report) = mapped(Shape::Subtract, 16, XC7, low.clone());
    assert_eq!(
        report.count("CARRY4"),
        5,
        "sixteen bits on the chain is five"
    );
    // And legalised for a slice, as the flow builds it. (Paired too would
    // pair nothing: its propagates have no generate tables to share with.)
    for width in [4, 5, 8] {
        check_full(Shape::Subtract, width, XC7, low.clone(), true, false);
    }
}

/// **A subtraction of narrower operands is a chain of their width.**
/// `{0, a} - {0, b}` at three times the width: the operands differ by
/// less than `2^width`, so the chain is `width + 1` bits of difference
/// (one `CARRY4` per four bits of the one-wider sum) and the bits above
/// copy its top one. At the full width the Husky Plus's
/// `outside - take <= EXTRA` was nine `CARRY4`s in a row for six bits
/// and one. And the threshold is against that narrower width: a
/// subtraction whose operands are below it stays in tables however wide
/// its result. Every input at 4 and 5 bits, legalised as the flow builds
/// it. Would catch the upper bits filled with zeros rather than the sign
/// (every negative difference wrong), the chain left at the full width,
/// or the threshold read off the result; would not catch a signed
/// operand narrowed, which is left at the full width by the code and not
/// exercised here.
#[test]
fn a_subtraction_of_narrower_operands_is_a_chain_of_their_width() {
    let low = MapOptions {
        min_subtract_carry_width: 1,
        ..MapOptions::default()
    };
    for width in [4, 5] {
        check_full(Shape::SubtractWidened, width, XC7, low.clone(), true, false);
        let (_, _, report) = mapped(Shape::SubtractWidened, width, XC7, low.clone());
        assert_eq!(
            report.count("CARRY4"),
            usize::try_from((width + 2).div_ceil(4)).unwrap(),
            "{width} bits of operand"
        );
        let above = MapOptions {
            min_subtract_carry_width: width + 2,
            ..MapOptions::default()
        };
        check_full(
            Shape::SubtractWidened,
            width,
            XC7,
            above.clone(),
            true,
            false,
        );
        let (_, _, report) = mapped(Shape::SubtractWidened, width, XC7, above);
        assert_eq!(report.count("CARRY4"), 0, "below the threshold");
    }
}

/// **An equality compared on the chain is right once legalised for a
/// slice.** `a == b` is the carry out of `{0, L} + 1`, read as the top
/// lane's sum alone, so every lane below matters only through its carry.
/// `legalise_carries` called a lane nothing read dead, and left its
/// propagate to whatever lookup table the placer seated there, when the
/// only thing read was a sum above it: on the Husky Plus three builds
/// whose capture compares were on the chain never ended a capture. Every
/// input at 9 bits, where the read sum is the fourth lane of the one
/// `CARRY4` and the three below it are read through their carries alone,
/// and at 10, where the chain continues into a second. Would catch a lane
/// below a read sum left dead again; would not catch a wider chain's
/// fault that only shows past twenty input bits, which cannot be
/// enumerated here and is proved instead.
#[test]
fn an_equality_on_the_chain_is_right_once_legalised() {
    let low = MapOptions {
        min_compare_carry_width: 4,
        ..MapOptions::default()
    };
    for width in [9, 10] {
        let (_, _, report) = mapped(Shape::Equal, width, XC7, low.clone());
        assert!(report.count("CARRY4") > 0, "{width} bits is on the chain");
        check_full(Shape::Equal, width, XC7, low.clone(), true, false);
    }
    // And one whose upper operand bits are constant, as NewAE's widened
    // compares are: 15 bits compared, 5 of each operand live.
    for width in [4, 5] {
        let (_, _, report) = mapped(Shape::EqualWidened, width, XC7, low.clone());
        assert!(
            report.count("CARRY4") > 0,
            "{width} bits widened is on the chain"
        );
        check_full(Shape::EqualWidened, width, XC7, low.clone(), true, false);
    }
}

/// **Under `--synthesis vivado` an equality's constant generates go on its
/// lanes' own tables, and it is still right.** Its lanes are two bit pairs,
/// a table of four inputs each, so the chain's generate of zero is that
/// table's `O5` (`xray::CarryOptions::generates_on_their_tables`) and there
/// is no buffer table between the lane and the `CARRY4`, where with lanes of
/// three every lane had one. Every input of `a == b` at 9 and 10 bits
/// (`b` compared as it is) and of the widened shape, legalised; then the
/// counts: every live lane folded and no buffer carrying a generate; and the
/// legacy synthesis's lanes of three, which have no room, as they were.
///
/// Would catch the constant in the wrong half (every unequal pair read as
/// equal in a lane's slot: the simulation reads `O5` as the lower half), `I5`
/// left low (the lane reads its own constant), and the fold leaking into the
/// legacy flow; would not catch the slice's `<L>CY0` reading `O5` other than
/// as the database says, which is quoted.
#[test]
fn an_equalitys_generates_go_on_its_lanes_own_tables() {
    let vivado = vivado();
    for width in [9, 10] {
        let packing = check_full_with(Shape::EqualPlain, width, XC7, &vivado, true, false)
            .expect("legalised");
        // Lane 0 adds the `+ 1` to its own equality, so its generate is that
        // equality, a signal on the bypass input; every lane above has the
        // generate zero, and every one is folded, with no buffer.
        let lanes = usize::try_from(width.div_ceil(2)).unwrap();
        assert_eq!(
            packing.generates_folded,
            lanes - 1,
            "{width} bits: {packing:?}"
        );
        assert_eq!(packing.buffers, 0, "{width} bits: {packing:?}");
    }
    let packing =
        check_full_with(Shape::EqualWidened, 4, XC7, &vivado, true, false).expect("legalised");
    assert!(packing.generates_folded > 0, "widened: {packing:?}");
    // Legacy: lanes of three, and buffers, as before, even for the top lane
    // of ten bits, one bit pair, whose table would have room.
    let legacy = FpgaOptions {
        map: MapOptions {
            min_compare_carry_width: 4,
            ..MapOptions::default()
        },
        ..FpgaOptions::default()
    };
    let packing = check_full_with(Shape::Equal, 10, XC7, &legacy, true, false).expect("legalised");
    assert_eq!(packing.generates_folded, 0, "legacy: {packing:?}");
    assert!(packing.generates > 0, "legacy: {packing:?}");
}

/// **An equality compared in kept lanes is right.** `compare_in_lanes`
/// compares a wide equality against a sum as lanes of three bit pairs,
/// each a kept net, and the AND of them: lookup tables only.
/// Every input of both equality shapes, the widened one's upper lanes
/// comparing constant zeros. Would catch a lane's slice or the AND's
/// order wrong; would not catch the lanes dissolving into the operand's
/// logic again, which changes depth and not the answer.
#[test]
fn an_equality_in_kept_lanes_is_right() {
    let lanes = MapOptions {
        min_compare_lane_width: 4,
        ..MapOptions::default()
    };
    for width in [9, 10] {
        check_exhaustively(Shape::Equal, width, lanes.clone());
    }
    for width in [4, 5] {
        check_exhaustively(Shape::EqualWidened, width, lanes.clone());
    }
}

/// **An ordering compared on the chain is right, signed or not, once
/// legalised and paired.** Under `--synthesis vivado` (the 7 series'
/// default), `a < b` and its three siblings from seven bits are lanes of
/// two bits, each lane's `>` the generate and its `>=` the other operand
/// of an addition whose carry out is the answer, `>=` starting from a one
/// on `CYINIT`. Every input at 7, 8 and 9 bits (a short top lane, a whole
/// `CARRY4`, and one lane into a second), each of the four, signed and
/// unsigned, mapped, legalised and paired into `LUT6_2` as the bitstream
/// is built.
///
/// Would catch: the operands of `<` not swapped (every answer of `<` and
/// `>` exchanged), `>=` started from zero (every equal pair wrong), the
/// sign bits left unflipped (every pair of opposite signs wrong), a lane
/// of the wrong two bits, and the carry out read off the wrong lane of the
/// last `CARRY4` or off a sum. Would not catch an ordering of twenty bits
/// and more wrong in a way these widths cannot show, which `mod proofs`
/// proves at 16 and 32.
#[test]
fn an_ordering_goes_onto_the_chain_and_is_still_right() {
    for op in Order::ALL {
        for signed in [false, true] {
            let shape = Shape::Order { op, signed };
            for width in [7, 8, 9] {
                check_full_with(shape, width, XC7, &vivado(), true, true);
                let (_, _, report) = mapped_with(shape, width, XC7, &vivado());
                assert_eq!(
                    report.count("CARRY4"),
                    usize::try_from(width.div_ceil(2).div_ceil(4)).unwrap(),
                    "{shape:?} at {width} bits: a CARRY4 for each four lanes of two bits"
                );
            }
            // Below the threshold, and with the legacy synthesis, tables.
            check_full_with(shape, 6, XC7, &vivado(), true, false);
            let (_, _, report) = mapped_with(shape, 6, XC7, &vivado());
            assert_eq!(report.count("CARRY4"), 0, "{shape:?} at 6 bits");
            let (_, _, report) = mapped(shape, 16, XC7, MapOptions::default());
            assert_eq!(report.count("CARRY4"), 0, "{shape:?}, legacy");
        }
    }
}

/// **An ordering against a constant compared in lanes is right.** Under
/// `--synthesis vivado`, `a < k` and its siblings from seven bits are lanes
/// of six bits, each lane's `>` and equality a kept table, and the chain
/// over the lanes; no `CARRY4`. Every input at 7, 12, 13 and 16 bits, each
/// of the four, signed and not, against constants at the edges of the range
/// and inside it, then the depth: sixteen bits against a constant, two
/// tables. Would catch the operands of `<` not swapped, the lowest lane's
/// `>=` taken for `>` (every equal input wrong), a lane of the wrong bits,
/// the sign bits unflipped and the chain over the lanes in the wrong order;
/// `mod proofs` proves 32 bits.
#[test]
fn an_ordering_against_a_constant_is_right_in_lanes() {
    for op in Order::ALL {
        for signed in [false, true] {
            for (width, k) in [
                (7, 0),
                (7, 127),
                (12, 0x5a5),
                (13, 1),
                (16, 6),
                (16, 0xfffe),
            ] {
                let shape = Shape::OrderConst { op, signed, k };
                check_full_with(shape, width, XC7, &vivado(), true, false);
                let (_, _, report) = mapped_with(shape, width, XC7, &vivado());
                assert_eq!(report.count("CARRY4"), 0, "{shape:?} at {width} bits");
            }
        }
        let shape = Shape::OrderConst {
            op,
            signed: false,
            k: 6,
        };
        let (_, _, report) = mapped_with(shape, 16, XC7, &vivado());
        assert_eq!(report.lut_depth, 2, "{shape:?} at 16 bits");
        let (_, _, report) = mapped(shape, 16, XC7, MapOptions::default());
        assert_eq!(report.lut_depth, 3, "{shape:?} at 16 bits, legacy");
    }
}

/// **Every shape is still right with its carry in on `CYINIT` and its
/// carry out off `CO`.** `--synthesis vivado` takes a constant low bit both
/// operands share (a subtraction's `+ 1`, a `>=`'s) as the first
/// `CARRY4`'s carry in, and the top sum of `{0, a} + {0, b}` off
/// the last lane's carry out, so neither is a lane of its own. Every input
/// of each shape, mapped, legalised and, where the chain has generate
/// tables to share, paired; and the counts that say the lanes went.
///
/// Would catch: the carry in left on a lane as well (every sum off by
/// it), the shared bit's zero sum dropped from the result (every bit one
/// place low), the carry out of the wrong lane, and a shared carry in that
/// is a signal put on `CYINIT` (the shared adders' is, and
/// `legalise_carries` refuses it beside bit 0's generate on the one `AX`).
/// Would not catch `PRECYINIT` taking its constant other than as prjxray's
/// fuzzer says (`C1`), quoted, as `CARRY4.<L>CY0` is.
#[test]
fn a_chain_takes_its_carry_in_and_out_on_its_own_pins() {
    let shapes = [
        (Shape::Plain, 8, false),
        (Shape::Widened, 4, false),
        (Shape::Widened, 8, false),
        (Shape::Subtract, 6, false),
        (Shape::Subtract, 8, false),
        (Shape::SubtractWidened, 4, false),
        (Shape::SubtractThenCarryIn, 6, false),
        (Shape::Equal, 9, false),
        (Shape::EqualWidened, 4, false),
        (Shape::CarryIn, 5, false),
    ];
    for (shape, width, pack) in shapes {
        check_full_with(shape, width, XC7, &vivado(), true, pack);
    }
    for low_sub in [false, true] {
        for high_sub in [false, true] {
            let shape = Shape::Chosen { low_sub, high_sub };
            check_full_with(shape, 4, XC7, &vivado(), true, true);
        }
    }
    // The lanes that went: a carry in and a carry out are no CARRY4 of
    // their own. Eight bits of difference are eight lanes, two CARRY4s,
    // where `{a, 1} + {~b, 1}` with the one a lane was nine and three.
    let (_, _, report) = mapped_with(Shape::Subtract, 8, XC7, &vivado());
    assert_eq!(report.count("CARRY4"), 2, "an 8-bit difference");
    let (_, _, report) = mapped_with(Shape::Widened, 8, XC7, &vivado());
    assert_eq!(report.count("CARRY4"), 2, "an 8-bit sum and its carry out");
    let (_, _, report) = mapped(Shape::Widened, 8, XC7, MapOptions::default());
    assert_eq!(report.count("CARRY4"), 3, "legacy: the carry out is a lane");
}

/// **A carry in that is a signal goes on `CYINIT`, bit 0's generate on
/// `O5`, and both are still right.** The shared adders' carry in
/// (`s ? c - d : a + b`) is a signal both operands carry as a low bit;
/// `--synthesis vivado` puts it on the first `CARRY4`'s `CYINIT`, which a
/// slice reaches from `AX` alone, so bit 0's generate cannot take `AX` too:
/// `legalise_carries` puts it on the lane's `O5`, one `LUT6_2` with the
/// propagate, as Vivado does (`SharedAx::Pin`), or, when the two read more
/// than five signals between them (`Shape::ChosenXor`), the carry in on a
/// `CARRY4` of its own below (`SharedAx::Below`). Every input of each,
/// legalised, and paired too.
///
/// Would catch: the generate's table folded in wrongly (its half of the
/// `LUT6_2` the propagate's, or its inputs in another order), the carry in
/// passed up from the wrong lane of the `CARRY4` below or that lane's
/// propagate not a zero (every sum off by the carry), either path not taken
/// where it should be (the counts), and, through
/// `carry_ins_have_ax_to_themselves` after every legalisation, a carry in
/// left on `CYINIT` beside a generate that is not its propagate's `O5`,
/// which is also what catches the chain's own `CYINIT` left on the signal
/// beside the `CI` from below (that simulates the same, `CI | CYINIT` of
/// one signal). Would not catch a generate's table folded in
/// while something else reads it, which simulates the same and would put
/// two signals on the lane's output mux; nor the `O5` routed to `DI` by
/// another way than `CARRY4.ACY0`, which `tests/fpga_xray_carry_in_vivado.rs`
/// compares with Vivado's bits.
#[test]
fn a_signal_carry_in_takes_cyinit_and_puts_bit_0s_generate_on_o5() {
    let mut on_o5 = 0;
    let mut below = 0;
    for pack in [false, true] {
        for low_sub in [false, true] {
            for high_sub in [false, true] {
                let shape = Shape::Chosen { low_sub, high_sub };
                let packing =
                    check_full_with(shape, 4, XC7, &vivado(), true, pack).expect("legalised");
                on_o5 += packing.generates_on_o5;
                below += packing.carries_below;
            }
        }
        let packing =
            check_full_with(Shape::ChosenXor, 4, XC7, &vivado(), true, pack).expect("legalised");
        assert_eq!(
            (packing.generates_on_o5, packing.carries_below),
            (0, 1),
            "{packing:?}"
        );
    }
    assert!(
        on_o5 > 0,
        "no carry in went on `CYINIT` beside an `O5` generate"
    );
    assert_eq!(below, 0, "a shared adder's two tables fit one `LUT6_2`");
    // Two lanes fewer than with the carry in a lane of its own: an eight-bit
    // shared adder is two `CARRY4`s where it was three.
    let (_, _, report) = mapped_with(
        Shape::Chosen {
            low_sub: false,
            high_sub: true,
        },
        8,
        XC7,
        &vivado(),
    );
    assert_eq!(report.count("CARRY4"), 2, "an eight-bit shared adder");
}

// ---------------------------------------------------------------------------
// What the mapping costs
// ---------------------------------------------------------------------------

/// The shape of the result: one instance per four bits, one LUT per bit,
/// and no more.
#[test]
fn an_adder_costs_one_carry4_per_four_bits() {
    for (width, instances) in [(4u32, 1usize), (5, 2), (8, 2), (9, 3), (16, 4), (32, 8)] {
        let (_, _, report) = mapped(Shape::Plain, width, XC7, MapOptions::default());
        assert_eq!(
            report.count("CARRY4"),
            instances,
            "{width} bits should be {instances} instance(s)"
        );
        assert_eq!(
            report.count("LUT6"),
            usize::try_from(width).unwrap(),
            "{width} bits should be one propagate LUT per bit"
        );
        assert_eq!(report.primitives.carry_chains.len(), 1);
        assert_eq!(
            report.primitives.carry_chains[0].primitives,
            u32::try_from(instances).unwrap()
        );
    }
}

/// The wiring rule simulation cannot see.
///
/// `CI | CYINIT` means the two carry inputs are interchangeable to a
/// simulator and to the SAT proof, but they are not interchangeable on
/// the part: `CI` comes from the slice below and `CYINIT` from the
/// fabric, and exactly one of them may be driven. So this is checked
/// structurally instead — the first instance takes its carry in on
/// `CYINIT` with `CI` at zero, every later one takes it on `CI` from the
/// instance below with `CYINIT` at zero, and no instance has a live
/// signal on both.
#[test]
fn the_chain_uses_ci_and_the_start_uses_cyinit() {
    let (design, top, _) = mapped(Shape::Plain, 12, XC7, MapOptions::default());
    let module = design.module(top);
    let mut instances: Vec<&Cell> = module
        .cells
        .values()
        .filter(|c| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "CARRY4"))
        .collect();
    instances.sort_by_key(|c| c.name.as_str().to_owned());
    assert_eq!(instances.len(), 3);
    let is_zero = |cell: &Cell, port: &str| -> bool {
        let e = cell.input(port).unwrap_or_else(|| panic!("no `{port}`"));
        matches!(module.expr(e).kind, ExprKind::Const(ref c) if c.is_zero())
    };
    for (index, cell) in instances.iter().enumerate() {
        let ci_tied = is_zero(cell, "CI");
        let cyinit_tied = is_zero(cell, "CYINIT");
        assert!(
            ci_tied != cyinit_tied || index == 0,
            "`{}` drives both carry inputs, or neither",
            cell.name
        );
        if index == 0 {
            // The chain's own carry in is a constant zero, so both pins
            // are tied; what matters is that neither carries a signal.
            assert!(
                ci_tied && cyinit_tied,
                "`{}` starts from a signal",
                cell.name
            );
        } else {
            assert!(cyinit_tied, "`{}` drives CYINIT mid-chain", cell.name);
            assert!(!ci_tied, "`{}` does not take the chain on CI", cell.name);
            // And what it takes is the top carry out of the instance
            // below, not one of the lower ones.
            let e = cell.input("CI").expect("`CI`");
            let ExprKind::Slice { base, hi, lo } = module.expr(e).kind else {
                panic!("`{}` takes CI from {:?}", cell.name, module.expr(e).kind)
            };
            assert_eq!((hi, lo), (3, 3), "`{}` takes the wrong carry", cell.name);
            let ExprKind::Net(net) = module.expr(base).kind else {
                panic!("`{}` takes CI from an expression", cell.name)
            };
            let below = instances[index - 1].output("CO").expect("`CO`");
            assert_eq!(net, below, "`{}` is chained out of order", cell.name);
        }
    }
}

/// An adder narrower than the threshold is left in LUTs, as it was
/// before: one CARRY4 is four bits, so four bits is where it starts.
#[test]
fn an_adder_below_the_threshold_stays_generic() {
    for width in [1, 2, 3] {
        let (_, _, report) = mapped(Shape::Plain, width, XC7, MapOptions::default());
        assert_eq!(report.count("CARRY4"), 0, "{width} bits reached the chain");
    }
}

// ---------------------------------------------------------------------------
// The other families
// ---------------------------------------------------------------------------

/// Every input combination of an ECP5 adder, on the element's own
/// equation.
///
/// Four, five and eight bits of two-operand add, plus the shape with a
/// carry in (which is two chained adders) and the one with a constant
/// operand. The widths that are not odd multiples of the element's two
/// lanes matter here in a way they do not on the 7 series, because this
/// chain spends a lane entering itself: an 8-bit add is nine lanes, so
/// five instances with one lane over, and a 7-bit add is eight lanes
/// with none.
#[test]
fn every_input_of_an_ecp5_adder_is_simulated() {
    for width in [4, 5, 7, 8] {
        for shape in [Shape::Plain, Shape::Widened, Shape::Constant(1)] {
            check_on(shape, width, ECP5, bare());
        }
    }
    for width in [4, 5] {
        check_on(Shape::CarryIn, width, ECP5, bare());
    }
    // And the awkward narrow widths, with the threshold out of the way.
    let one = MapOptions {
        min_carry_width: 1,
        ..bare()
    };
    for width in [1, 2, 3, 6] {
        check_on(Shape::Plain, width, ECP5, one.clone());
        check_on(Shape::Widened, width, ECP5, one.clone());
    }
}

/// An ECP5 adder is `(width + 1) / 2` instances rounded up and **no
/// soft logic at all**, which is the whole difference from the other two
/// shapes.
#[test]
fn an_ecp5_adder_costs_one_ccu2c_per_two_bits_and_no_lookup_table() {
    for (width, instances) in [(4u32, 3usize), (8, 5), (16, 9), (32, 17), (31, 16)] {
        let (_, _, report) = mapped(Shape::Plain, width, ECP5, bare());
        assert_eq!(
            report.count("CCU2C"),
            instances,
            "{width} bits is {} lanes",
            width + 1
        );
        assert_eq!(
            report.count("LUT4"),
            0,
            "{width} bits: the element is the propagate and the generate, \
             so nothing outside it computes either"
        );
        assert_eq!(report.luts, 0, "and the technology mapper covered nothing");
        assert_eq!(report.lut_depth, 0);
        let chain = &report.primitives.carry_chains[0];
        assert_eq!(chain.primitive, "CCU2C");
        assert_eq!(
            (chain.width, chain.primitives),
            (width, u32::try_from(instances).unwrap())
        );
    }
    // A subtract still goes through lookup tables. The element can do one
    // — `ecppack` writes INIT 0x999A for a subtract bit, with the
    // propagate `a ~^ b` — and this flow does not ask it to.
    let (_, _, report) = mapped(Shape::Subtract, 8, ECP5, bare());
    assert_eq!(report.count("CCU2C"), 0);
}

/// Nothing changed for the families that were already working: the iCE40
/// maps its one-bit element.
#[test]
fn the_one_bit_families_are_untouched() {
    let (_, _, report) = mapped(Shape::Plain, 8, "ice40-hx1k-tq144", MapOptions::default());
    assert_eq!(report.count("SB_CARRY"), 7, "one per bit but the last");
    assert_eq!(report.primitives.carry_chains[0].primitive, "SB_CARRY");

    // And the three shapes are told apart by the device database
    // itself, which is what decides which path maps an adder.
    let ice40 = fpga::target("ice40-hx1k-tq144").unwrap();
    assert!(ice40.bel(BelRole::Carry).unwrap().wide_carry().is_none());
    assert!(ice40.bel(BelRole::Carry).unwrap().operand_carry().is_none());
    let ecp5 = fpga::target(ECP5).unwrap();
    assert!(ecp5.bel(BelRole::Carry).unwrap().wide_carry().is_none());
    let operand = ecp5
        .bel(BelRole::Carry)
        .unwrap()
        .operand_carry()
        .expect("the ECP5's is the operand shape");
    assert_eq!(operand.width, 2);
    assert_eq!((operand.a, operand.b), ("A0,A1", "B0,B1"));
    assert_eq!(operand.sum, "S0,S1");
    assert_eq!((operand.carry_in, operand.carry_out), ("CIN", "COUT"));
    assert_eq!(operand.init, None, "nothing can drive this chain's CIN");
    let xc7 = fpga::target(XC7).unwrap();
    assert!(xc7.bel(BelRole::Carry).unwrap().operand_carry().is_none());
    let wide = xc7.bel(BelRole::Carry).unwrap().wide_carry().expect("wide");
    assert_eq!(wide.width, 4);
    assert_eq!(wide.propagate, "S");
    assert_eq!(wide.data, "DI");
    assert_eq!(wide.sum, "O");
    assert_eq!(wide.carry_out, "CO");
    assert_eq!(wide.carry_in, Some("CI"));
    assert_eq!(wide.init, Some("CYINIT"));
}

// ---------------------------------------------------------------------------
// The proofs
// ---------------------------------------------------------------------------

#[cfg(feature = "formal")]
mod proofs {
    use super::*;
    use reticle::formal::{EquivOptions, EquivOutcome, check_equivalent};
    // `ExprId` is only needed by `model_ccu2c`, which is in this module, so
    // it is imported here: at the top of the file it is an unused import in
    // a build with `fpga` and without `formal`, which the gate's per-feature
    // clippy loop refuses.
    use reticle::ir::{Const, ExprId, Name};

    /// Replaces every primitive of the mapped module with the model it
    /// stands for, leaving ordinary IR the bit-blaster can read.
    ///
    /// This is the second, independent statement of `CARRY4`'s
    /// behaviour: `mux` and `xor` cells rather than Rust over [`Logic`].
    fn model_primitives(module: Module) -> Module {
        let mut b = ModuleBuilder::from_module(module, span());
        let ids: Vec<CellId> = b.module().cells.iter().map(|(id, _)| id).collect();
        let mut replaced = Vec::new();
        for id in ids {
            let cell = b.module().cells[id].clone();
            let CellKind::Blackbox(name) = &cell.kind else {
                continue;
            };
            match name.as_str() {
                "IBUF" | "OBUF" | "BUFG" => {
                    let (_, e) = cell.inputs[0];
                    let (_, out) = cell.outputs[0];
                    b.cell(
                        format!("{}$model", cell.name),
                        CellKind::Buf,
                        vec![(Name::new("a"), e)],
                        vec![(Name::new("y"), out)],
                    );
                }
                lut if lut.starts_with("LUT") && lut.len() == 4 => {
                    let AttrValue::Const(init) = cell
                        .params
                        .get("INIT")
                        .unwrap_or_else(|| panic!("`{}` has no INIT", cell.name))
                    else {
                        panic!("`{}` has a non-constant INIT", cell.name)
                    };
                    let init = init.clone();
                    let k = init.width().trailing_zeros();
                    assert_eq!(
                        cell.inputs.len(),
                        k as usize,
                        "`{}` has {} input(s) for a {k}-input table",
                        cell.name,
                        cell.inputs.len()
                    );
                    // Its own ports in declaration order, for the same
                    // reason the Rust model uses them: `I0`..`I5` on the
                    // 7 series, `A`..`D` on an ECP5.
                    let mut pins: Vec<ExprId> = cell.inputs.iter().map(|(_, e)| *e).collect();
                    pins.reverse();
                    let a = b.concat(pins);
                    let (_, out) = cell.outputs[0];
                    b.cell(
                        format!("{}$model", cell.name),
                        CellKind::Lut { k, init },
                        vec![(Name::new("a"), a)],
                        vec![(Name::new("y"), out)],
                    );
                }
                // UG474's two functions of one table: `O6` the whole of it
                // over `I0`..`I5`, `O5` its lower half over `I0`..`I4`.
                "LUT6_2" => {
                    let AttrValue::Const(init) = cell
                        .params
                        .get("INIT")
                        .unwrap_or_else(|| panic!("`{}` has no INIT", cell.name))
                    else {
                        panic!("`{}` has a non-constant INIT", cell.name)
                    };
                    let init = init.clone();
                    let mut lower = Const::zero(32);
                    for bit in 0..32 {
                        lower.set_bit(bit, init.bit(bit));
                    }
                    let pin = |n: &str| {
                        cell.input(n)
                            .unwrap_or_else(|| panic!("`{}` has no `{n}`", cell.name))
                    };
                    let six: Vec<ExprId> = (0..6).rev().map(|i| pin(&format!("I{i}"))).collect();
                    let five: Vec<ExprId> = (0..5).rev().map(|i| pin(&format!("I{i}"))).collect();
                    let six = b.concat(six);
                    let five = b.concat(five);
                    if let Some(o6) = cell.output("O6") {
                        b.cell(
                            format!("{}$model6", cell.name),
                            CellKind::Lut { k: 6, init },
                            vec![(Name::new("a"), six)],
                            vec![(Name::new("y"), o6)],
                        );
                    }
                    if let Some(o5) = cell.output("O5") {
                        b.cell(
                            format!("{}$model5", cell.name),
                            CellKind::Lut { k: 5, init: lower },
                            vec![(Name::new("a"), five)],
                            vec![(Name::new("y"), o5)],
                        );
                    }
                }
                "CARRY4" => model_carry4(&mut b, &cell),
                "CCU2C" => model_ccu2c(&mut b, &cell),
                other => panic!("primitive `{other}` has no model"),
            }
            replaced.push(id);
        }
        b.module_mut().cells.retain(|id, _| !replaced.contains(&id));
        let module = b.finish();
        let problems = validate_module(&module);
        assert!(
            !problems.has_errors(),
            "the modelled module is not valid: {:?}",
            problems.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        module
    }

    /// `CARRY4` out of `mux` and `xor`, written from the equations in
    /// `xc7.dev`:
    ///
    /// ```text
    /// carry[0]   = CI | CYINIT
    /// O[i]       = S[i] ^ carry[i]
    /// carry[i+1] = CO[i] = S[i] ? carry[i] : DI[i]
    /// ```
    fn model_carry4(b: &mut ModuleBuilder, cell: &Cell) {
        let name = cell.name.as_str().to_owned();
        let s = cell.input("S").expect("`S`");
        let di = cell.input("DI").expect("`DI`");
        let ci = cell.input("CI").expect("`CI`");
        let cyinit = cell.input("CYINIT").expect("`CYINIT`");
        let start = b.add_net(format!("{name}$model$ci"), Type::bit());
        b.cell2(format!("{name}$model$or"), CellKind::Or, ci, cyinit, start);
        let mut carry = b.net(start);
        let mut sums = Vec::new();
        let mut carries = Vec::new();
        for bit in 0..4u32 {
            let s_bit = b.slice(s, bit, bit);
            let di_bit = b.slice(di, bit, bit);
            let sum = b.add_net(format!("{name}$model$o{bit}"), Type::bit());
            b.cell2(
                format!("{name}$model$xor{bit}"),
                CellKind::Xor,
                s_bit,
                carry,
                sum,
            );
            sums.push(b.net(sum));
            let next = b.add_net(format!("{name}$model$co{bit}"), Type::bit());
            // `mux(a, b, s)` picks `b` when `s` is one, so the propagate
            // selects the carry and its complement selects `DI`.
            b.cell(
                format!("{name}$model$mux{bit}"),
                CellKind::Mux,
                vec![
                    (Name::new("a"), di_bit),
                    (Name::new("b"), carry),
                    (Name::new("s"), s_bit),
                ],
                vec![(Name::new("y"), next)],
            );
            carry = b.net(next);
            carries.push(carry);
        }
        sums.reverse();
        carries.reverse();
        let o = b.concat(sums);
        let co = b.concat(carries);
        b.assign(cell.output("O").expect("`O`"), o);
        b.assign(cell.output("CO").expect("`CO`"), co);
    }

    /// `CCU2C` out of `lut`, `xor` and `mux`, written from the
    /// equations in this file's header.
    ///
    /// The two lookup tables are the halves of the element's own `INIT`
    /// that the silicon reads: bits 15..12 are the propagate, read with
    /// `C` and `D` forced high, and bits 3..0 are the generate, which is
    /// a two-input table in its own right. Writing them as two `lut`
    /// cells over `{b, a}` is the whole claim, and it is a different
    /// statement of it from the Rust above.
    fn model_ccu2c(b: &mut ModuleBuilder, cell: &Cell) {
        let name = cell.name.as_str().to_owned();
        let zero = b.constant(Const::zero(1));
        let mut carry = cell.input("CIN").unwrap_or(zero);
        for lane in 0..2u32 {
            let AttrValue::Const(init) = cell
                .params
                .get(&format!("INIT{lane}"))
                .unwrap_or_else(|| panic!("`{name}` has no INIT{lane}"))
            else {
                panic!("`{name}` has a non-constant INIT{lane}")
            };
            let nibble = |shift: u32| -> Const {
                let mut out = Const::zero(4);
                for bit in 0..4 {
                    out.set_bit(bit, init.bit(shift + bit));
                }
                out
            };
            let pin = |role: &str| cell.input(&format!("{role}{lane}")).unwrap_or(zero);
            let inputs = b.concat(vec![pin("B"), pin("A")]);
            let table = |b: &mut ModuleBuilder, what: &str, init: Const| {
                let out = b.add_net(format!("{name}$model${what}{lane}"), Type::bit());
                b.cell(
                    format!("{name}$model${what}{lane}$lut"),
                    CellKind::Lut { k: 2, init },
                    vec![(Name::new("a"), inputs)],
                    vec![(Name::new("y"), out)],
                );
                b.net(out)
            };
            let propagate = table(b, "p", nibble(12));
            let generate = table(b, "g", nibble(0));
            if let Some(out) = cell.output(&format!("S{lane}")) {
                let sum = b.add_net(format!("{name}$model$s{lane}"), Type::bit());
                b.cell2(
                    format!("{name}$model$xor{lane}"),
                    CellKind::Xor,
                    propagate,
                    carry,
                    sum,
                );
                let sum = b.net(sum);
                b.assign(out, sum);
            }
            let next = b.add_net(format!("{name}$model$c{lane}"), Type::bit());
            b.cell(
                format!("{name}$model$mux{lane}"),
                CellKind::Mux,
                vec![
                    (Name::new("a"), generate),
                    (Name::new("b"), carry),
                    (Name::new("s"), propagate),
                ],
                vec![(Name::new("y"), next)],
            );
            carry = b.net(next);
        }
        if let Some(out) = cell.output("COUT") {
            b.assign(out, carry);
        }
    }

    /// Proves the mapped netlist equivalent to the design it came from.
    fn prove(shape: Shape, width: u32) {
        prove_on(shape, width, XC7, MapOptions::default());
    }

    /// The same, on a named device.
    fn prove_on(shape: Shape, width: u32, device: &str, map: MapOptions) {
        let (design, top, _) = mapped(shape, width, device, map);
        let modelled = model_primitives(design.module(top).clone());
        let mut proof = Design::new();
        let mut reference = shape.module("reference", width);
        reference.name = Name::new("reference");
        let a = proof.add_module(modelled);
        let b = proof.add_module(reference);
        let options = EquivOptions::default();
        let report = check_equivalent(&proof, a, b, &options);
        assert!(
            matches!(report.outcome, EquivOutcome::Equivalent(_)),
            "{shape:?} at {width} bits is not proved equivalent:\n{}",
            report.render("mapped", "reference")
        );
    }

    /// Proves the netlist the 7 series' default synthesis maps.
    fn prove_vivado(shape: Shape, width: u32) {
        let (design, top, _) = mapped_with(shape, width, XC7, &vivado());
        let modelled = model_primitives(design.module(top).clone());
        let mut proof = Design::new();
        let mut reference = shape.module("reference", width);
        reference.name = Name::new("reference");
        let a = proof.add_module(modelled);
        let b = proof.add_module(reference);
        let report = check_equivalent(&proof, a, b, &EquivOptions::default());
        assert!(
            matches!(report.outcome, EquivOutcome::Equivalent(_)),
            "{shape:?} at {width} bits under --synthesis vivado is not proved equivalent:\n{}",
            report.render("mapped", "reference")
        );
    }

    /// **An equality on the chain, legalised as `reticle fpga` legalises it
    /// under `--synthesis vivado`, proved** at 16 and 32 bits, which
    /// simulation cannot enumerate: its lanes' constant generates on their
    /// tables' `O5` (`xray::CarryOptions::generates_on_their_tables`), the
    /// `LUT6_2`s modelled from UG474's reading of the table, the `CARRY4` from
    /// its equations. Would catch what `an_equalitys_generates_go_on_its_lanes_own_tables`
    /// would, at any width; would not catch the slice reading `O5` other than
    /// as that model says.
    #[test]
    fn an_equality_with_its_generates_on_its_tables_is_proved() {
        for width in [16, 32] {
            let (mut design, top, _) = mapped_with(Shape::EqualPlain, width, XC7, &vivado());
            let device = fpga::target(XC7).expect("a built-in device");
            let carry = fpga::xray::CarryOptions {
                generates_on_their_tables: true,
            };
            let packing = fpga::xray::legalise_carries_with(&mut design, top, device, &carry)
                .expect("the chain legalises");
            assert!(packing.generates_folded > 0, "{width} bits: {packing:?}");
            let modelled = model_primitives(design.module(top).clone());
            let mut proof = Design::new();
            let mut reference = Shape::EqualPlain.module("reference", width);
            reference.name = Name::new("reference");
            let a = proof.add_module(modelled);
            let b = proof.add_module(reference);
            let report = check_equivalent(&proof, a, b, &EquivOptions::default());
            assert!(
                matches!(report.outcome, EquivOutcome::Equivalent(_)),
                "`a == b` at {width} bits, legalised, is not proved equivalent:\n{}",
                report.render("legalised", "reference")
            );
        }
    }

    /// **The orderings, and every shape with its carry ends on the
    /// `CARRY4`'s own pins, proved** at the widths simulation cannot
    /// enumerate: 16 and 32 bits of each ordering, signed and not, and the
    /// shapes whose carry in or out moved. Would catch what
    /// `an_ordering_goes_onto_the_chain_and_is_still_right` would, at any
    /// width; would not catch what that test says it would not, the
    /// slice's own reading of its pins.
    #[test]
    fn orderings_and_carry_ends_are_proved_equivalent() {
        for op in Order::ALL {
            for signed in [false, true] {
                for width in [16, 32] {
                    prove_vivado(Shape::Order { op, signed }, width);
                }
            }
        }
        for width in [13, 32] {
            prove_vivado(Shape::Subtract, width);
            prove_vivado(Shape::Widened, width);
            prove_vivado(Shape::Plain, width);
        }
        prove_vivado(Shape::SubtractThenCarryIn, 13);
        for op in Order::ALL {
            for signed in [false, true] {
                prove_vivado(
                    Shape::OrderConst {
                        op,
                        signed,
                        k: 0x8000_1234,
                    },
                    32,
                );
            }
        }
        prove_vivado(Shape::Equal, 16);
        prove_vivado(Shape::CarryIn, 16);
        for low_sub in [false, true] {
            for high_sub in [false, true] {
                prove_vivado(Shape::Chosen { low_sub, high_sub }, 13);
            }
        }
    }

    /// The widths exhaustive simulation cannot reach.
    #[test]
    fn wide_adders_are_proved_equivalent() {
        for width in [16, 32] {
            prove(Shape::Plain, width);
        }
    }

    /// A subtraction on the chain, and one feeding a second adder's carry
    /// in, at the widths simulation cannot reach.
    #[test]
    fn wide_subtractions_are_proved_equivalent() {
        // On the chain, at a lowered threshold, and in tables by default.
        let low = MapOptions {
            min_subtract_carry_width: 1,
            ..MapOptions::default()
        };
        for width in [16, 32] {
            prove_on(Shape::Subtract, width, XC7, low.clone());
            prove(Shape::Subtract, width);
        }
        prove_on(Shape::SubtractThenCarryIn, 13, XC7, low);
        prove(Shape::SubtractThenCarryIn, 13);
    }

    /// The same proof over the awkward shapes, at a width that is not a
    /// multiple of four and with a constant operand.
    #[test]
    fn the_awkward_shapes_are_proved_equivalent() {
        // Nine, ten and seventeen bits: one lane over a whole instance,
        // two over, and one over on a chain long enough to matter.
        prove(Shape::Plain, 9);
        prove(Shape::Plain, 10);
        prove(Shape::Widened, 9);
        prove(Shape::Plain, 17);
        prove(Shape::Widened, 16);
        prove(Shape::Constant(1), 16);
        prove(Shape::Constant(0xdead), 16);
        prove(Shape::CarryIn, 16);
    }

    /// Two adders behind a multiplexer, sharing one chain, at a width
    /// simulation cannot enumerate: every combination of adding and
    /// subtracting on each side.
    #[test]
    fn a_chosen_sum_is_proved_equivalent() {
        for low_sub in [false, true] {
            for high_sub in [false, true] {
                prove(Shape::Chosen { low_sub, high_sub }, 13);
            }
        }
    }

    /// The same proof for the ECP5's `CCU2C`, at the widths simulation
    /// cannot reach and at the shapes where the lane the chain spends on
    /// entering itself lands differently.
    ///
    /// **This is the only check in the tree that shows a solver the
    /// inside of a carry cell**; see this file's header for why
    /// `every_block_maps_to_the_logic_it_was_mapped_from` cannot.
    #[test]
    fn the_ecp5_chain_is_proved_equivalent() {
        for width in [16, 32] {
            prove_on(Shape::Plain, width, ECP5, bare());
        }
        for width in [9, 10, 17] {
            prove_on(Shape::Plain, width, ECP5, bare());
            prove_on(Shape::Widened, width, ECP5, bare());
        }
        prove_on(Shape::Constant(1), 16, ECP5, bare());
        prove_on(Shape::Constant(0xdead), 16, ECP5, bare());
        prove_on(Shape::CarryIn, 16, ECP5, bare());
    }
}
