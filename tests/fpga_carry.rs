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
//! The other families are checked here too, because the point of the
//! device-model extension is that it changed nothing for them: the iCE40
//! still maps its one-bit `SB_CARRY` and the ECP5 still declines.

#![cfg(all(feature = "fpga", feature = "synth"))]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{eval_expr, expr_nets};
use reticle::diag::Diagnostics;
use reticle::fpga::{self, BelRole, Constraints, FpgaOptions, MapOptions};
use reticle::ir::builder::ModuleBuilder;
use reticle::ir::validate::validate_module;
use reticle::ir::{
    AttrValue, Cell, CellId, CellKind, Design, ExprKind, Module, ModuleId, NetId, Type,
};
use reticle::logic::{Bit, Logic};
use reticle::source::{SourceMap, Span};

const XC7: &str = "xc7a35t-cpg236";

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
            Shape::Constant(k) => {
                let a = b.input("a", Type::bits(width));
                let y = b.output("y", Type::bits(width));
                let ae = b.net(a);
                let ke = b.const_u64(width, k);
                b.cell2("sum", CellKind::Add, ae, ke, y);
            }
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
            _ => width,
        }
    }

    /// What the module must answer for the given inputs.
    fn expected(self, width: u32, a: u64, b: u64, cin: u64) -> u64 {
        let mask = |w: u32| if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        match self {
            Shape::Plain => a.wrapping_add(b) & mask(width),
            Shape::CarryIn => a.wrapping_add(b).wrapping_add(cin) & mask(width),
            Shape::Widened => (a + b) & mask(width + 1),
            Shape::Constant(k) => a.wrapping_add(k) & mask(width),
            Shape::Subtract => a.wrapping_sub(b) & mask(width),
        }
    }

    /// The input ports, in the order the exhaustive loop counts them.
    fn inputs(self, width: u32) -> Vec<(&'static str, u32)> {
        match self {
            Shape::Constant(_) => vec![("a", width)],
            Shape::CarryIn => vec![("a", width), ("b", width), ("cin", 1)],
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
    let mut design = Design::new();
    let top = design.add_module(shape.module("top", width));
    design.top = Some(top);
    let device = fpga::target(device_name).expect("a built-in device");
    let constraints = Constraints::default();
    let mut diags = Diagnostics::new();
    let options = FpgaOptions {
        map,
        ..FpgaOptions::default()
    };
    let report = fpga::synthesize_for(&mut design, top, device, &constraints, &options, &mut diags)
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
            lut if lut.starts_with("LUT") && lut.len() == 4 => {
                let init = match cell.params.get("INIT") {
                    Some(AttrValue::Const(c)) => c.clone(),
                    other => panic!("`{}` has INIT {other:?}", cell.name),
                };
                let mut index = 0u32;
                for pin in 0..6u32 {
                    if cell.input(&format!("I{pin}")).is_some()
                        && one(&format!("I{pin}"), values) == Bit::One
                    {
                        index |= 1 << pin;
                    }
                }
                let (_, out) = cell.outputs.first().expect("a LUT has an output");
                values.insert(*out, Logic::from_bit(init.bit(index)));
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
            other => panic!("primitive `{other}` (cell `{}`) is not modelled", cell.name),
        }
    }
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
    let (design, top, report) = mapped(shape, width, XC7, map);
    let module = design.module(top);
    let netlist = Netlist::new(module);
    let ports = shape.inputs(width);
    let bits: u32 = ports.iter().map(|(_, w)| *w).sum();
    assert!(bits <= 20, "{bits} bits is too many to enumerate");
    let result = port_net(module, "y");
    let result_width = shape.result_width(width);
    for pattern in 0..(1u64 << bits) {
        let mut inputs = BTreeMap::new();
        let mut taken = 0;
        let mut values = [0u64; 3];
        for (index, (name, w)) in ports.iter().enumerate() {
            let mask = (1u64 << w) - 1;
            let value = (pattern >> taken) & mask;
            taken += w;
            values[index] = value;
            inputs.insert(port_net(module, name), Logic::from_u64(value, *w));
        }
        let (a, b, cin) = match shape {
            Shape::Constant(_) => (values[0], 0, 0),
            Shape::CarryIn => (values[0], values[1], values[2]),
            _ => (values[0], values[1], 0),
        };
        let got = netlist.eval(&inputs);
        let got = got
            .get(&result)
            .unwrap_or_else(|| panic!("`y` was not driven"))
            .clone();
        let want = shape.expected(width, a, b, cin);
        let want = Logic::from_u64(want, result_width);
        assert_eq!(
            got.to_u64(),
            want.to_u64(),
            "{shape:?} at {width} bits: a={a} b={b} cin={cin} gave {got}, wanted {want}\n\
             the netlist was {:?}",
            report.netlist
        );
    }
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

/// Subtraction does not go through the carry path on any family, and
/// still comes out right.
#[test]
fn subtraction_stays_in_luts_and_is_still_right() {
    for width in [4, 8] {
        check_exhaustively(Shape::Subtract, width, MapOptions::default());
        let (_, _, report) = mapped(Shape::Subtract, width, XC7, MapOptions::default());
        assert_eq!(
            report.count("CARRY4"),
            0,
            "a `sub` reached the carry chain, which nothing here has checked"
        );
    }
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

/// Nothing changed for the families that were already working: the iCE40
/// maps its one-bit element, and the ECP5 still declines with the note it
/// declined with before.
#[test]
fn the_one_bit_families_are_untouched() {
    let (_, _, report) = mapped(Shape::Plain, 8, "ice40-hx1k-tq144", MapOptions::default());
    assert_eq!(report.count("SB_CARRY"), 7, "one per bit but the last");
    assert_eq!(report.primitives.carry_chains[0].primitive, "SB_CARRY");

    let (_, _, report) = mapped(Shape::Plain, 8, "ecp5-45f-CABGA381", MapOptions::default());
    assert_eq!(report.count("CCU2C"), 0);
    assert!(report.primitives.carry_chains.is_empty());
    assert!(
        report
            .primitives
            .notes
            .iter()
            .any(|n| n.contains("without a (ci, i0, i1, co) port map")),
        "the ECP5 stopped saying why it declines: {:?}",
        report.primitives.notes
    );

    // And the two shapes are told apart by the device database itself.
    let ice40 = fpga::target("ice40-hx1k-tq144").unwrap();
    assert!(ice40.bel(BelRole::Carry).unwrap().wide_carry().is_none());
    let ecp5 = fpga::target("ecp5-45f-CABGA381").unwrap();
    assert!(ecp5.bel(BelRole::Carry).unwrap().wide_carry().is_none());
    let xc7 = fpga::target(XC7).unwrap();
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
    use reticle::ir::Name;

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
                    let mut pins = Vec::new();
                    for pin in (0..k).rev() {
                        let e = cell
                            .input(&format!("I{pin}"))
                            .unwrap_or_else(|| panic!("`{}` has no I{pin}", cell.name));
                        pins.push(e);
                    }
                    let a = b.concat(pins);
                    let (_, out) = cell.outputs[0];
                    b.cell(
                        format!("{}$model", cell.name),
                        CellKind::Lut { k, init },
                        vec![(Name::new("a"), a)],
                        vec![(Name::new("y"), out)],
                    );
                }
                "CARRY4" => model_carry4(&mut b, &cell),
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

    /// Proves the mapped netlist equivalent to the design it came from.
    fn prove(shape: Shape, width: u32) {
        let (design, top, _) = mapped(shape, width, XC7, MapOptions::default());
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

    /// The widths exhaustive simulation cannot reach.
    #[test]
    fn wide_adders_are_proved_equivalent() {
        for width in [16, 32] {
            prove(Shape::Plain, width);
        }
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
}
