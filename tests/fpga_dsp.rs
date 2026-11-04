//! The Xilinx 7-series `DSP48E1`: every multiply the flow puts on one
//! checked against the block's own model, and every one it refuses
//! checked to have been refused.
//!
//! `src/fpga/devices/xc7.dev` declares `DSP48E1` as a 25x18
//! two's-complement multiplier with its mode pins tied and its registers
//! off, transcribed from Yosys' `xc7_dsp_map.v`. A multiplier that is
//! subtly wrong — an operand zero-extended where it should have been
//! sign-extended, a 25-bit operand on the 18-bit port, a result read past
//! the 43 bits the product is exact in — is worse than a multiplier in
//! LUTs, so this file checks the result two independent ways, both against
//! the behaviour of the DSP48E1 model in Yosys' `techlibs/xilinx/cells_sim.v`
//! **as configured by the pins and parameters the instance actually
//! carries**:
//!
//! ```text
//! A_MULT = INMODE[1] ? 0 : A[24:0]          (signed; USE_DPORT = "FALSE")
//! M      = A_MULT * B[17:0]                 (signed, 43 bits)
//! X      = OPMODE[1:0] == 01 ? M : 0        (and OPMODE[3:2] == 01 with it)
//! Y      = OPMODE[3:2] == 11 ? C : 0
//! Z      = OPMODE[6:4] == 011 ? C : 0
//! P      = Z + X + Y + CARRYIN              (ALUMODE = 0000, CARRYINSEL = 000)
//! Z      = OPMODE[6:4] == 101 ? PCIN >>> 17 : Z   (a split multiply's cascade)
//! ```
//!
//! Neither model hard-codes the multiply: each reads `OPMODE`, `ALUMODE`,
//! `INMODE`, `CARRYINSEL` and `CARRYIN` off the instance and refuses any
//! setting or register parameter outside the table above, so a wrong tie
//! in the device file (an `OPMODE` of zero, say, which makes `P` zero)
//! fails here rather than in a person's hands.
//!
//! - **Simulation** (`Netlist`): the whole flow's netlist — `IBUF`,
//!   `DSP48E1`, `OBUF` — evaluated in Rust over [`Logic`] and compared
//!   with the product computed in `i128`. Exhaustively up to 16 input
//!   bits; past that, every corner (zero, one, all ones, the most
//!   negative and most positive value of each operand) against every
//!   other, and then seeded random vectors. The random part is a sample
//!   and is called one.
//! - **Proof** (`mod proofs`): the netlist with each primitive replaced
//!   by the model above built out of IR cells, proved equivalent to the
//!   unmapped multiply by `formal::check_equivalent`. A multiplier miter
//!   is hard for SAT, so these are the narrow cases, where the checker's
//!   exhaustive simulation stage decides — which is a decision, not a
//!   sample.
//!
//! And what does **not** go on a block is checked to stay off it, with the
//! note saying why, and what is wider than one block is checked over several.
//! `tests/fpga_dsp_vivado.rs` has registers, and what Vivado says of all of it.

#![cfg(all(feature = "fpga", feature = "synth"))]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Rng, eval_expr, expr_nets};
use reticle::diag::Diagnostics;
use reticle::fpga::{self, Constraints, FpgaOptions};
use reticle::ir::builder::ModuleBuilder;
use reticle::ir::validate::validate_module;
use reticle::ir::{AttrValue, Cell, CellId, CellKind, Design, Module, ModuleId, NetId, Type};
use reticle::logic::Logic;
use reticle::source::{SourceMap, Span};

const XC7: &str = "xc7a35t-cpg236";

fn span() -> Span {
    let mut map = SourceMap::new();
    let id = map.add("fpga-dsp", "").unwrap();
    Span::new(id, 0, 0)
}

// ---------------------------------------------------------------------------
// The designs
// ---------------------------------------------------------------------------

/// `y = a * b`, with `a` and `b` extended to `result` bits first — which
/// is exactly what the Verilog frontend produces for `assign y = a * b`
/// with a `result`-bit `y` (checked: `reticle synth` on such a module
/// prints `mul (a=resize(%a, u16), b=resize(%b, u16))`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mul {
    a: u32,
    b: u32,
    result: u32,
    signed: bool,
}

const fn mul(a: u32, b: u32, result: u32, signed: bool) -> Mul {
    Mul {
        a,
        b,
        result,
        signed,
    }
}

impl Mul {
    fn module(self, name: &str) -> Module {
        let mut b = ModuleBuilder::new(name, span());
        let ty = |w: u32| {
            if self.signed {
                Type::sbits(w)
            } else {
                Type::bits(w)
            }
        };
        let a = b.input("a", ty(self.a));
        let x = b.input("b", ty(self.b));
        let y = b.output("y", ty(self.result));
        let (a_e, x_e) = (b.net(a), b.net(x));
        let a_r = b.resize(a_e, self.result, self.signed);
        let x_r = b.resize(x_e, self.result, self.signed);
        b.cell2("mul", CellKind::Mul, a_r, x_r, y);
        b.finish()
    }

    /// The value of an input pattern as the source reads it.
    fn value(self, bits: u64, width: u32) -> i128 {
        let raw = i128::from(bits & mask(width));
        if self.signed && width > 0 && (raw >> (width - 1)) & 1 == 1 {
            raw - (1i128 << width)
        } else {
            raw
        }
    }

    /// `a * b` modulo `2^result`, as an unsigned bit pattern.
    fn expected(self, a: u64, b: u64) -> u64 {
        let product = self.value(a, self.a) * self.value(b, self.b);
        low_bits(product, self.result)
    }
}

/// The low `width` bits of a two's-complement integer, as a pattern.
fn low_bits(value: i128, width: u32) -> u64 {
    let modulus = 1i128 << width;
    u64::try_from(value.rem_euclid(modulus)).expect("at most 64 bits")
}

fn mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

/// The flow's options with every multiply that fits on a block, however
/// small. A block's result can only be proved or walked exhaustively while
/// its operands are narrow, and narrow is what the flow now leaves in logic,
/// so the checks below ask for the block regardless; `small_multiplies_stay_
/// in_logic_as_vivado_leaves_them` pins what it does when not asked.
fn every_size() -> FpgaOptions {
    let mut options = FpgaOptions::default();
    options.map.dsp_smallest = false;
    options
}

/// Runs the whole FPGA flow over one multiply.
fn mapped(case: Mul) -> (Design, ModuleId, fpga::FlowReport) {
    let mut design = Design::new();
    let top = design.add_module(case.module("top"));
    design.top = Some(top);
    let device = fpga::target(XC7).expect("a built-in device");
    let mut diags = Diagnostics::new();
    let report = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &Constraints::default(),
        &every_size(),
        &mut diags,
    )
    .expect("the flow finished");
    assert!(
        !diags.has_errors(),
        "{case:?}: the flow reported errors: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let problems = validate_module(design.module(top));
    assert!(
        !problems.has_errors(),
        "{case:?}: the mapped module is not valid: {:?}",
        problems.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    (design, top, report)
}

// ---------------------------------------------------------------------------
// What the instance is configured to do
// ---------------------------------------------------------------------------

/// The registers every `DSP48E1` instance must have switched off for the
/// block to be combinational. The library default of each is 1. `CREG` is not
/// here: the multiply does not read `C`, and Vivado leaves it on when no other
/// register is, for no effect (`testdata/fpga/vivado/dsp/inference.txt`).
const REGISTERS: [&str; 13] = [
    "ACASCREG",
    "ADREG",
    "ALUMODEREG",
    "AREG",
    "BCASCREG",
    "BREG",
    "CARRYINREG",
    "CARRYINSELREG",
    "DREG",
    "INMODEREG",
    "MREG",
    "OPMODEREG",
    "PREG",
];

/// Panics unless the instance's parameters are inside what both models
/// cover: no register, no pre-adder, a multiplier, one 48-bit ALU.
fn check_parameters(cell: &Cell) {
    for name in REGISTERS {
        let value = cell
            .params
            .get(name)
            .and_then(AttrValue::as_int)
            .unwrap_or_else(|| {
                panic!(
                    "`{}` leaves {name} at its library default, which is a register",
                    cell.name
                )
            });
        assert_eq!(value, 0, "`{}` has {name}={value}", cell.name);
    }
    let string = |name: &str| match cell.params.get(name) {
        Some(AttrValue::String(s)) => s.clone(),
        other => panic!("`{}` has {name} = {other:?}", cell.name),
    };
    assert_eq!(string("USE_MULT"), "MULTIPLY");
    assert_eq!(string("USE_DPORT"), "FALSE");
    assert_eq!(string("USE_SIMD"), "ONE48");
    assert_eq!(string("A_INPUT"), "DIRECT");
    assert_eq!(string("B_INPUT"), "DIRECT");
}

/// The control pins' values, as constants: the models refuse a control
/// pin a signal drives, because neither covers a block that changes mode.
struct Controls {
    opmode: u64,
    alumode: u64,
    inmode: u64,
    carryinsel: u64,
    carryin: u64,
}

fn controls(module: &Module, cell: &Cell) -> Controls {
    let get = |pin: &str| -> u64 {
        let e = cell
            .input(pin)
            .unwrap_or_else(|| panic!("`{}` leaves {pin} unconnected", cell.name));
        assert!(
            expr_nets(module, e).is_empty(),
            "`{}` drives {pin} from a signal",
            cell.name
        );
        eval_expr(module, e, &BTreeMap::new())
            .to_u64()
            .unwrap_or_else(|| panic!("`{}` has an unknown bit on {pin}", cell.name))
    };
    Controls {
        opmode: get("OPMODE"),
        alumode: get("ALUMODE"),
        inmode: get("INMODE"),
        carryinsel: get("CARRYINSEL"),
        carryin: get("CARRYIN"),
    }
}

impl Controls {
    /// Panics on any setting outside the table in this file's header.
    fn check(&self, name: &str) {
        let (x, y, z) = (self.opmode & 3, (self.opmode >> 2) & 3, self.opmode >> 4);
        assert!(
            matches!((x, y), (0, 0) | (1, 1) | (0, 3)),
            "`{name}`: OPMODE {:07b} has an X/Y selection the model does not cover",
            self.opmode
        );
        assert!(
            matches!(z, 0 | 3 | 5),
            "`{name}`: OPMODE {:07b} has a Z selection the model does not cover",
            self.opmode
        );
        assert_eq!(self.alumode, 0, "`{name}`: only ALUMODE 0000 is modelled");
        assert_eq!(
            self.carryinsel, 0,
            "`{name}`: only CARRYINSEL 000 is modelled"
        );
        assert_eq!(
            self.inmode & 0b1_1101,
            0,
            "`{name}`: INMODE {:05b} selects the D port or a register the model does not cover",
            self.inmode
        );
    }
}

// ---------------------------------------------------------------------------
// Simulating the mapped netlist
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Step {
    Cell(CellId),
    Assign(usize),
}

/// A combinational netlist of the primitives this flow emits for a
/// multiply, ordered once and then evaluated on many vectors. A primitive
/// it does not model is a panic, never a silent zero.
struct Netlist<'m> {
    module: &'m Module,
    steps: Vec<Step>,
}

impl<'m> Netlist<'m> {
    fn new(module: &'m Module) -> Netlist<'m> {
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
            let reticle::ir::Lvalue::Net(target) = assign.target else {
                panic!("the mapped netlist assigns to something that is not a net");
            };
            pending.push((
                Step::Assign(index),
                expr_nets(module, assign.value),
                vec![target],
            ));
        }
        let mut ready: BTreeSet<NetId> = module
            .ports
            .iter()
            .filter(|p| p.dir != reticle::ir::PortDir::Out)
            .map(|p| p.net)
            .collect();
        let mut steps = Vec::new();
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
        assert!(
            done.iter().all(|d| *d),
            "the netlist has a loop or a net nothing drives"
        );
        Netlist { module, steps }
    }

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
                Step::Cell(id) => self.eval_cell(&self.module.cells[id], &mut values),
            }
        }
        values
    }

    fn eval_cell(&self, cell: &Cell, values: &mut BTreeMap<NetId, Logic>) {
        // The adder a split multiply's columns are summed in, while it is still
        // one; the carry chain it becomes is `fpga::primitives`' own question.
        if cell.kind == CellKind::Add {
            let read = |port: &str| -> u64 {
                let e = cell.input(port).expect("an adder's operand");
                eval_expr(self.module, e, values)
                    .to_u64()
                    .expect("no unknown bits")
            };
            let out = cell.output("y").expect("an adder drives y");
            let width = self.module.nets[out].ty.width().unwrap();
            let sum = read("a").wrapping_add(read("b")) & mask(width);
            values.insert(out, Logic::from_u64(sum, width));
            return;
        }
        let CellKind::Blackbox(name) = &cell.kind else {
            panic!("cell `{}` ({:?}) is not a primitive", cell.name, cell.kind);
        };
        match name.as_str() {
            "IBUF" | "OBUF" => {
                let (_, e) = cell.inputs[0];
                let (_, out) = cell.outputs[0];
                let width = self.module.nets[out].ty.width().unwrap_or(1);
                let value = eval_expr(self.module, e, values).resize(width);
                values.insert(out, value);
            }
            "DSP48E1" => {
                check_parameters(cell);
                let c = controls(self.module, cell);
                c.check(cell.name.as_str());
                let pin = |name: &str, width: u32| -> i128 {
                    let value = cell
                        .input(name)
                        .map_or_else(|| Logic::zero(width), |e| eval_expr(self.module, e, values));
                    let bits = value
                        .to_u64()
                        .unwrap_or_else(|| panic!("`{}` has an unknown bit on {name}", cell.name))
                        & mask(width);
                    signed(bits, width)
                };
                let a_mult = if c.inmode & 2 != 0 { 0 } else { pin("A", 25) };
                let m = a_mult * pin("B", 18);
                let x = if c.opmode & 3 == 1 { m } else { 0 };
                let cin = pin("C", 48);
                let y = if (c.opmode >> 2) & 3 == 3 { cin } else { 0 };
                let z = match c.opmode >> 4 {
                    3 => cin,
                    5 => pin("PCIN", 48) >> 17,
                    _ => 0,
                };
                let p = low_bits(z + x + y + i128::from(c.carryin & 1), 48);
                let out = cell.output("P").expect("a DSP48E1 drives P");
                assert_eq!(self.module.nets[out].ty.width(), Some(48));
                values.insert(out, Logic::from_u64(p, 48));
                if let Some(cascade) = cell.output("PCOUT") {
                    values.insert(cascade, Logic::from_u64(p, 48));
                }
            }
            other => panic!("primitive `{other}` (cell `{}`) is not modelled", cell.name),
        }
    }
}

/// The two's-complement value of a `width`-bit pattern.
fn signed(bits: u64, width: u32) -> i128 {
    let raw = i128::from(bits);
    if (raw >> (width - 1)) & 1 == 1 {
        raw - (1i128 << width)
    } else {
        raw
    }
}

fn port_net(module: &Module, name: &str) -> NetId {
    module
        .ports
        .iter()
        .find(|p| p.name.as_str() == name)
        .unwrap_or_else(|| panic!("no port `{name}`"))
        .net
}

/// Every pattern the check drives for one operand of `width` bits:
/// every value when that is at most 256 of them, the corners otherwise.
fn corners(width: u32) -> Vec<u64> {
    if width <= 8 {
        return (0..1u64 << width).collect();
    }
    let top = 1u64 << (width - 1);
    let mut v = vec![0, 1, 2, top - 1, top, top + 1, mask(width) - 1, mask(width)];
    v.sort_unstable();
    v.dedup();
    v
}

/// Maps `case`, asserts it went on exactly `blocks` DSP48E1 instances,
/// and compares the netlist with the arithmetic on every corner pair and
/// `random` seeded random pairs.
fn check(case: Mul, random: usize) {
    let (design, top, report) = mapped(case);
    assert_eq!(
        report.count("DSP48E1"),
        1,
        "{case:?} should be one DSP48E1; the netlist was {:?}, the notes {:?}",
        report.netlist,
        report.primitives.notes
    );
    assert_eq!(report.luts, 0, "{case:?} left logic beside the block");
    let module = design.module(top);
    let netlist = Netlist::new(module);
    let (a_net, b_net, y_net) = (
        port_net(module, "a"),
        port_net(module, "b"),
        port_net(module, "y"),
    );
    let run = |a: u64, b: u64| {
        let mut inputs = BTreeMap::new();
        inputs.insert(a_net, Logic::from_u64(a & mask(case.a), case.a));
        inputs.insert(b_net, Logic::from_u64(b & mask(case.b), case.b));
        let got = netlist.eval(&inputs)[&y_net].to_u64();
        let want = case.expected(a, b);
        assert_eq!(
            got,
            Some(want),
            "{case:?}: a={a:#x} b={b:#x} gave {got:?}, wanted {want:#x}"
        );
    };
    for a in corners(case.a) {
        for b in corners(case.b) {
            run(a, b);
        }
    }
    let mut rng = Rng::new(0xD5B4_8E10 ^ u64::from(case.a * 64 + case.b));
    for _ in 0..random {
        run(rng.next_u64(), rng.next_u64());
    }
}

/// The primitive-mapping step over `case`, asserting it went on exactly `blocks`
/// blocks, and the mapped netlist against the arithmetic on every corner pair and
/// `random` seeded random pairs.
fn check_split(case: Mul, blocks: usize, random: usize) {
    let mut design = Design::new();
    let top = design.add_module(case.module("top"));
    design.top = Some(top);
    let mut diags = Diagnostics::new();
    let options = fpga::MapOptions {
        map_carry: false,
        dsp_smallest: false,
        ..fpga::MapOptions::default()
    };
    let report = fpga::map(
        &mut design,
        top,
        fpga::target(XC7).unwrap(),
        &Constraints::default(),
        &options,
        &mut diags,
    );
    assert!(!diags.has_errors(), "{case:?}");
    let module = design.module(top);
    let found = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "DSP48E1"))
        .count();
    assert_eq!(found, blocks, "{case:?}: {:?}", report.notes);
    let netlist = Netlist::new(module);
    let (a_net, b_net, y_net) = (
        port_net(module, "a"),
        port_net(module, "b"),
        port_net(module, "y"),
    );
    let run = |a: u64, b: u64| {
        let mut inputs = BTreeMap::new();
        inputs.insert(a_net, Logic::from_u64(a & mask(case.a), case.a));
        inputs.insert(b_net, Logic::from_u64(b & mask(case.b), case.b));
        let got = netlist.eval(&inputs)[&y_net].to_u64();
        let want = case.expected(a, b);
        assert_eq!(
            got,
            Some(want),
            "{case:?}: a={a:#x} b={b:#x} gave {got:?}, wanted {want:#x}"
        );
    };
    for a in corners(case.a) {
        for b in corners(case.b) {
            run(a, b);
        }
    }
    let mut rng = Rng::new(0xD5B4_8E11 ^ u64::from(case.a * 64 + case.b));
    for _ in 0..random {
        run(rng.next_u64(), rng.next_u64());
    }
}

/// Runs only the primitive-mapping step over `case` and asserts the
/// multiply did **not** go on a DSP48E1, and that the report says so.
/// (The whole flow would then cover it in LUTs, which is the slow part
/// and not what is under test.)
fn check_refused(case: Mul) {
    let mut design = Design::new();
    let top = design.add_module(case.module("top"));
    design.top = Some(top);
    let mut diags = Diagnostics::new();
    let report = fpga::map(
        &mut design,
        top,
        fpga::target(XC7).unwrap(),
        &Constraints::default(),
        &fpga::MapOptions::default(),
        &mut diags,
    );
    assert!(
        report.dsps.is_empty(),
        "{case:?} cannot be one DSP48E1 and was put on one"
    );
    assert!(
        report.notes.iter().any(|n| n.contains("fits no DSP block")),
        "{case:?} was refused without a note: {:?}",
        report.notes
    );
    let still_a_mul = design
        .module(top)
        .cells
        .iter()
        .any(|(_, c)| c.kind == CellKind::Mul);
    assert!(
        still_a_mul,
        "{case:?}: the multiply is left for the LUT mapper"
    );
}

// ---------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------

/// Small unsigned and signed multiplies, every input pattern.
#[test]
fn every_input_of_a_small_multiply_is_simulated() {
    for case in [
        mul(8, 8, 16, false),
        mul(8, 8, 16, true),
        mul(3, 7, 10, true),
        mul(7, 3, 10, false),
        // Same-width: the result is as wide as the operands, so the
        // operands go on as they stand and the product is only right
        // modulo 2^8 — which is all that is asked.
        mul(8, 8, 8, false),
        mul(8, 8, 8, true),
        // A result narrower than the operands' product.
        mul(8, 8, 5, true),
    ] {
        check(case, 0);
    }
}

/// The widest operands one block takes, both ways round, signed and
/// unsigned: an unsigned operand needs a bit of the port for a sign that
/// stays zero, so 24x17 is the unsigned limit and 25x18 the signed one.
#[test]
fn the_widest_operands_fit_and_multiply() {
    for case in [
        mul(25, 18, 43, true),
        mul(18, 25, 43, true),
        mul(24, 17, 41, false),
        mul(17, 24, 41, false),
        mul(16, 16, 32, false),
        mul(18, 18, 36, true),
        mul(17, 17, 34, false),
        mul(12, 12, 24, true),
        // Same width, at the widest the 18-bit port allows.
        mul(18, 18, 18, false),
        mul(18, 18, 18, true),
    ] {
        check(case, 2000);
    }
}

/// A result wider than the 48-bit `P` port: the product of two exact
/// operands is exact in 43 bits, so the bits above 48 are copies of its
/// sign, and mapping must extend it rather than pad it with zeros.
#[test]
fn a_result_wider_than_p_is_sign_extended() {
    check(mul(16, 16, 64, true), 2000);
    check(mul(25, 18, 60, true), 2000);
    check(mul(16, 16, 64, false), 2000);
}

/// **What Vivado leaves in logic stays in logic**, by the table in
/// `testdata/fpga/vivado/dsp/inference.txt`'s `t_` lines, and what it puts on
/// a block goes on one. Would catch the limit moved or read against the wrong
/// operand, or an unsigned operand counted without the bit its sign takes:
/// `u9x9` is left and `u10x10` is not. Says nothing of whether either is right
/// to do, only that it is what the vendor's tool does.
#[test]
fn the_wider_operand_goes_on_the_wider_port() {
    let widths = |a: u32, b: u32| {
        let mut design = Design::new();
        let top = design.add_module(mul(a, b, a + b, true).module("top"));
        design.top = Some(top);
        let mut diags = Diagnostics::new();
        let report = fpga::map(
            &mut design,
            top,
            fpga::target(XC7).unwrap(),
            &Constraints::default(),
            &fpga::MapOptions::default(),
            &mut diags,
        );
        (report.dsps[0].a_width, report.dsps[0].b_width)
    };
    // `t_s6x12` in `testdata/fpga/vivado/dsp/inference.txt`, and its mirror.
    assert_eq!(widths(6, 12), (12, 6));
    assert_eq!(widths(12, 6), (12, 6));
    assert_eq!(widths(16, 16), (16, 16));
}

#[test]
fn small_multiplies_stay_in_logic_as_vivado_leaves_them() {
    let blocks = |case: Mul| -> usize {
        let mut design = Design::new();
        let top = design.add_module(case.module("top"));
        design.top = Some(top);
        let mut diags = Diagnostics::new();
        let report = fpga::map(
            &mut design,
            top,
            fpga::target(XC7).unwrap(),
            &Constraints::default(),
            &fpga::MapOptions::default(),
            &mut diags,
        );
        report.dsps.len()
    };
    for (a, b, signed, vivado) in [
        (9, 9, true, 0),
        (10, 10, true, 0),
        (11, 11, true, 1),
        (5, 18, true, 0),
        (6, 18, true, 1),
        (6, 10, true, 0),
        (6, 12, true, 1),
        (8, 10, true, 0),
        (8, 12, true, 1),
        (4, 18, true, 0),
        (3, 25, true, 0),
        (9, 9, false, 0),
        (10, 10, false, 1),
        (8, 8, false, 0),
        (4, 17, false, 0),
        (6, 16, false, 1),
        (8, 12, false, 1),
    ] {
        let case = mul(a, b, a + b, signed);
        assert_eq!(blocks(case), vivado, "{case:?}");
    }
}

/// **What one block cannot take goes over several**, as Vivado's does
/// (`testdata/fpga/vivado/dsp/inference.txt`), right to the last bit on every
/// corner and a run of random pairs. This test pinned the opposite until
/// 2026-11-04, that a multiply too wide for one block stays in logic, which was a
/// choice and not a fact: "splitting is its own correctness problem", it said, and
/// this is that problem being checked. Would catch a piece read with the wrong
/// sign, `PCIN` shifted logically, a column put at the wrong place in the sum and a
/// product left out that the result needs; a corner is what finds most of those,
/// since each goes wrong only where a piece has its top bit set.
#[test]
fn what_one_block_cannot_take_goes_over_several() {
    for (case, blocks) in [
        (mul(18, 18, 36, false), 2),
        (mul(25, 17, 42, false), 2),
        (mul(26, 4, 30, true), 2),
        (mul(19, 19, 38, true), 2),
        (mul(20, 20, 20, false), 2),
        (mul(26, 18, 44, true), 2),
        (mul(26, 26, 52, true), 4),
        (mul(32, 32, 64, false), 4),
        (mul(32, 32, 64, true), 4),
        (mul(32, 32, 32, true), 3),
        (mul(32, 32, 32, false), 3),
        (mul(35, 35, 64, true), 4),
        (mul(48, 48, 64, true), 8),
        (mul(48, 48, 64, false), 8),
        (mul(48, 16, 64, true), 3),
        (mul(16, 48, 64, false), 3),
        // A result too short to read past the first block's piece.
        (mul(32, 32, 17, true), 1),
    ] {
        check_split(case, blocks, 3000);
    }
}

/// **A part with no block left keeps the rest in logic**, and says so, rather than
/// stopping in the placer with a design that fitted before there was a block to run
/// out of. Would catch `count` read for the wrong device or not counted at all, and
/// a split or a sum taking more than are left.
#[test]
fn a_part_out_of_blocks_keeps_the_rest_in_logic() {
    let mut b = ModuleBuilder::new("top", span());
    let a = b.input("a", Type::sbits(18));
    let x = b.input("b", Type::sbits(18));
    let (a_e, x_e) = (b.net(a), b.net(x));
    let a_r = b.resize(a_e, 36, true);
    let x_r = b.resize(x_e, 36, true);
    for index in 0..95 {
        let y = b.output(format!("y{index}"), Type::sbits(36));
        b.cell2(format!("mul{index}"), CellKind::Mul, a_r, x_r, y);
    }
    let mut design = Design::new();
    let top = design.add_module(b.finish());
    design.top = Some(top);
    let mut diags = Diagnostics::new();
    let report = fpga::map(
        &mut design,
        top,
        fpga::target(XC7).unwrap(),
        &Constraints::default(),
        &fpga::MapOptions::default(),
        &mut diags,
    );
    assert_eq!(report.dsps.len(), 90, "the part's count");
    let left: Vec<_> = report
        .notes
        .iter()
        .filter(|n| n.contains("stays in logic"))
        .collect();
    assert_eq!(left.len(), 5, "{left:?}");
    let still = design
        .module(top)
        .cells
        .iter()
        .filter(|(_, c)| c.kind == CellKind::Mul)
        .count();
    assert_eq!(still, 5);
}

/// A multiply with a narrow side is not worth a cascade, however wide the other:
/// Vivado keeps `5x25` in logic (`t_s5x25`) and this keeps `26x4`.
#[test]
fn a_narrow_side_stays_in_logic_however_wide_the_other() {
    check_refused(mul(26, 4, 30, true));
    check_refused(mul(40, 5, 45, true));
}

/// The instance the flow writes is the one Yosys' `xc7_dsp_map.v` writes,
/// pin for pin, except that `C` is all ones where Yosys writes zeros: this is
/// what Vivado reads. `C` is what Vivado itself ties, and on this fabric the
/// zero would cost 48 routes for an input nothing selects.
#[test]
fn the_instance_carries_the_plain_multiply_configuration() {
    let (design, top, _) = mapped(mul(8, 8, 16, true));
    let module = design.module(top);
    let (_, cell) = module
        .cells
        .iter()
        .find(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "DSP48E1"))
        .expect("one DSP48E1");
    check_parameters(cell);
    let c = controls(module, cell);
    assert_eq!(c.opmode, 0b000_0101);
    assert_eq!(c.alumode, 0);
    assert_eq!(c.inmode, 0);
    assert_eq!(c.carryinsel, 0);
    assert_eq!(c.carryin, 0);
    let width = |pin: &str| {
        let e = cell.input(pin).unwrap_or_else(|| panic!("no {pin}"));
        module.exprs[e].ty.width()
    };
    assert_eq!(width("A"), Some(30), "A is connected at its full width");
    assert_eq!(width("B"), Some(18));
    let c_pin = cell.input("C").expect("C");
    assert_eq!(
        eval_expr(module, c_pin, &BTreeMap::new()).to_u64(),
        Some((1u64 << 48) - 1),
        "C is tied to ones"
    );
    for (pin, bits) in [("D", 25), ("ACIN", 30), ("BCIN", 18), ("PCIN", 48)] {
        assert_eq!(width(pin), Some(bits), "{pin}");
        let e = cell.input(pin).unwrap();
        assert!(
            eval_expr(module, e, &BTreeMap::new()).is_zero(),
            "{pin} is tied to zero"
        );
    }
    assert!(
        cell.params.get("A_WIDTH").is_none(),
        "A_WIDTH is not a DSP48E1 parameter and Vivado would refuse it"
    );
}

/// A multiply feeding an adder goes on a block with the adder in it. This test
/// pinned the adder staying outside until 2026-11-04, when the line had no way to
/// say otherwise, and Vivado's own block takes it (`f_add`, `fpga::dsp_fold`). What
/// it was written to catch, that failing to fold the adder in left the multiply in
/// logic as well, is still what the block count catches.
#[test]
fn a_multiply_feeding_an_adder_goes_on_a_block_with_it() {
    let mut b = ModuleBuilder::new("top", span());
    let a = b.input("a", Type::bits(12));
    let x = b.input("b", Type::bits(12));
    let c = b.input("c", Type::bits(24));
    let y = b.output("y", Type::bits(24));
    let p = b.add_net("p", Type::bits(24));
    let (a_e, x_e) = (b.net(a), b.net(x));
    let a_r = b.zext(a_e, 24);
    let x_r = b.zext(x_e, 24);
    b.cell2("mul", CellKind::Mul, a_r, x_r, p);
    let (p_e, c_e) = (b.net(p), b.net(c));
    b.cell2("add", CellKind::Add, p_e, c_e, y);
    let mut design = Design::new();
    let top = design.add_module(b.finish());
    design.top = Some(top);
    let mut diags = Diagnostics::new();
    let report = fpga::synthesize_for(
        &mut design,
        top,
        fpga::target(XC7).unwrap(),
        &Constraints::default(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap();
    assert!(!diags.has_errors());
    assert_eq!(report.count("DSP48E1"), 1, "{:?}", report.primitives.notes);
    let item = &report.primitives.dsps[0];
    assert!(item.multiply_add, "the adder is the block's");
}

/// The other families are untouched: their `dsp` lines carry no `signed`,
/// no ties and no parameters, so they still get the generic `A_WIDTH` and
/// `B_WIDTH` and the old width rule.
#[test]
fn the_unsigned_families_are_untouched() {
    let ecp5 = fpga::target("ecp5-25f-CABGA381").unwrap();
    let dsp = &ecp5.dsps[0];
    assert!(!dsp.signed && dsp.ties.is_empty() && dsp.params.is_empty());
    let xc7 = fpga::target(XC7).unwrap();
    let dsp = &xc7.dsps[0];
    assert_eq!(dsp.name, "DSP48E1");
    assert!(dsp.signed);
    assert_eq!((dsp.a_width, dsp.b_width, dsp.p_width), (25, 18, 48));
    assert_eq!(dsp.pin_width("A"), Some(30));
}

// ---------------------------------------------------------------------------
// The proofs
// ---------------------------------------------------------------------------

#[cfg(feature = "formal")]
mod proofs {
    use super::*;
    use reticle::formal::{EquivOptions, EquivOutcome, check_equivalent};
    use reticle::ir::Name;

    /// Replaces every primitive with IR: buffers with `buf`, a `DSP48E1`
    /// with a `mul` of its sign-extended `A[24:0]` and `B[17:0]` plus the
    /// `C` and carry terms its controls select — the second, independent
    /// statement of the model, in cells rather than in Rust.
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
                "IBUF" | "OBUF" => {
                    let (_, e) = cell.inputs[0];
                    let (_, out) = cell.outputs[0];
                    b.cell(
                        format!("{}$model", cell.name),
                        CellKind::Buf,
                        vec![(Name::new("a"), e)],
                        vec![(Name::new("y"), out)],
                    );
                }
                "DSP48E1" => model_dsp(&mut b, &cell),
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

    /// `bits[hi:0]` sign-extended to 48 bits, out of a concatenation.
    fn sext48(b: &mut ModuleBuilder, e: reticle::ir::ExprId, hi: u32) -> reticle::ir::ExprId {
        let low = b.slice(e, hi, 0);
        let sign = b.slice(e, hi, hi);
        let mut parts = vec![sign; (47 - hi) as usize];
        parts.push(low);
        b.concat(parts)
    }

    fn model_dsp(b: &mut ModuleBuilder, cell: &Cell) {
        check_parameters(cell);
        let c = controls(b.module(), cell);
        c.check(cell.name.as_str());
        let name = cell.name.as_str().to_owned();
        let a = cell.input("A").expect("`A`");
        let a = if c.inmode & 2 != 0 {
            b.const_u64(48, 0)
        } else {
            sext48(b, a, 24)
        };
        let bb = cell.input("B").expect("`B`");
        let bb = sext48(b, bb, 17);
        let m = b.add_net(format!("{name}$model$m"), Type::bits(48));
        b.cell2(format!("{name}$model$mul"), CellKind::Mul, a, bb, m);
        let zero = b.const_u64(48, 0);
        let x = if c.opmode & 3 == 1 { b.net(m) } else { zero };
        let cin = cell.input("C").expect("`C`");
        let y = if (c.opmode >> 2) & 3 == 3 { cin } else { zero };
        let z = if c.opmode >> 4 == 3 { cin } else { zero };
        let carry = b.const_u64(48, c.carryin & 1);
        let mut sum = z;
        for (index, term) in [x, y, carry].into_iter().enumerate() {
            let next = b.add_net(format!("{name}$model$sum{index}"), Type::bits(48));
            b.cell2(
                format!("{name}$model$add{index}"),
                CellKind::Add,
                sum,
                term,
                next,
            );
            sum = b.net(next);
        }
        b.assign(cell.output("P").expect("`P`"), sum);
    }

    fn prove(case: Mul) {
        let (design, top, report) = mapped(case);
        assert_eq!(report.count("DSP48E1"), 1, "{case:?}");
        let modelled = model_primitives(design.module(top).clone());
        let mut proof = Design::new();
        let mut reference = case.module("reference");
        reference.name = Name::new("reference");
        let a = proof.add_module(modelled);
        let b = proof.add_module(reference);
        let report = check_equivalent(&proof, a, b, &EquivOptions::default());
        assert!(
            matches!(report.outcome, EquivOutcome::Equivalent(_)),
            "{case:?} is not proved equivalent:\n{}",
            report.render("mapped", "reference")
        );
    }

    /// Unsigned, signed, mixed widths, the same-width form and the
    /// sign-extended result above 48 bits, each proved.
    #[test]
    fn narrow_multiplies_are_proved_equivalent() {
        for case in [
            mul(4, 4, 8, false),
            mul(4, 4, 8, true),
            mul(3, 6, 9, true),
            mul(6, 3, 9, false),
            mul(6, 6, 6, false),
            mul(6, 6, 6, true),
            mul(4, 4, 50, true),
            mul(4, 4, 50, false),
        ] {
            prove(case);
        }
    }
}

// ---------------------------------------------------------------------------
// The fabric
// ---------------------------------------------------------------------------

mod fabric {
    use std::path::Path;

    use reticle::fpga::xray::{GridRegion, XrayDatabase, XrayOptions};
    use reticle::ir::memfile::FileProvider;

    struct DiskFiles;

    impl FileProvider for DiskFiles {
        fn read_file(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(path).ok()
        }
    }

    fn chipdb() -> Option<String> {
        let probe = "artix7/xc7a50t/tilegrid.json";
        if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
            if Path::new(&format!("{root}/{probe}")).is_file() {
                return Some(root);
            }
            eprintln!("skipped: RETICLE_CHIPDB is `{root}` but has no `{probe}`");
            return None;
        }
        let home = std::env::var("XDG_CACHE_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|d| format!("{d}/reticle"))
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| format!("{h}/.cache/reticle"))
            })?;
        let dir = format!("{home}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3");
        if Path::new(&format!("{dir}/{probe}")).is_file() {
            Some(dir)
        } else {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` \
                 or set RETICLE_CHIPDB"
            );
            None
        }
    }

    /// **What the loader makes of a `DSP_R` tile, and what the database still
    /// does not hold for one.**
    ///
    /// This test used to pin the opposite: one site called `DSP48`, of kind
    /// `other`, with no pins, and it was named
    /// `the_fabric_has_no_dsp_site_and_the_database_no_use_mult`, the reasons
    /// `fpga::xray::dsp_refusal` gave for refusing the block. The loader's half
    /// of that was a thing to fix and is fixed, and the database's half is still
    /// true, and is now understood: `USE_MULT` is missing from it because the
    /// mode that multiplies sets no bit (`fpga::xray::dsp`). So it pins both:
    /// two sites with every pin, and no `USE_MULT` feature in the file, whose
    /// arrival would mean the derived `USE_MULT_NONE` has a rival to agree with.
    ///
    /// Would catch a block's pins lost to a renamed wire, or the two blocks
    /// merged again. Whether each pin is the right wire is
    /// `the_pin_table_is_the_databases`.
    #[test]
    fn the_fabric_has_two_dsp_sites_and_the_database_no_use_mult() {
        let Some(root) = chipdb() else { return };
        let db = XrayDatabase::open(&DiskFiles, &root, "xc7a35t-cpg236", &XrayOptions::new())
            .expect("the database opens");
        // Columns 24 to 32 of the bottom rows: slices either side of the
        // `DSP_R` column at grid x 28, whose tiles start at y 5.
        let mut options = XrayOptions::new();
        options.region = Some(GridRegion::new(24, 0, 32, 12));
        let fabric = db.load(&DiskFiles, &options).expect("the region loads");
        let graph = fabric.arch.build_graph();
        let blocks: Vec<_> = graph.sites.iter().filter(|s| s.kind == "dsp").collect();
        assert!(!blocks.is_empty(), "the region holds the DSP column");
        assert_eq!(blocks.len() % 2, 0, "two blocks a tile");
        for site in &blocks {
            assert!(
                site.bel == "DSP_0" || site.bel == "DSP_1",
                "{}: `{}`",
                site.name,
                site.bel
            );
            // 263 inputs and 154 outputs, which is the 417 the site's own list
            // in `tile_type_DSP_R.json` has.
            assert_eq!(site.pins.len(), 417, "{}", site.name);
        }
        let merged = graph.sites.iter().filter(|s| s.bel == "DSP48").count();
        assert_eq!(merged, 0, "no tile has the one merged site any more");
        for side in ["l", "r"] {
            let path = format!("{root}/artix7/segbits_dsp_{side}.db");
            let text = std::fs::read_to_string(&path).expect("the DSP segbits are fetched");
            assert!(
                text.contains(".ZMREG[0] "),
                "{path} documents the registers"
            );
            assert!(
                !text.contains("USE_MULT"),
                "{path} documents USE_MULT now; compare it with `dsp::derived_features`"
            );
            let ppips = format!("{root}/artix7/ppips_dsp_{side}.db");
            let text = std::fs::read_to_string(&ppips).expect("the DSP ppips are fetched");
            assert!(text.contains(".DSP_0_A0.DSP_IMUX23_0 always"), "{ppips}");
        }
    }

    /// **The wires this flow's tables name are the database's.** Every pin of
    /// both blocks is a wire the tile type's own site list gives it, and the
    /// three-way muxes `fpga::xray::dsp` lists are the ones the tile type's pip
    /// list has, with the interconnect wire it says. Would catch a typing error
    /// in either table; it reads the same file the tables were taken from, so
    /// it would not catch a mistake the file makes.
    #[test]
    fn the_pin_table_is_the_databases() {
        let Some(root) = chipdb() else { return };
        for side in ["L", "R"] {
            let path = format!("{root}/artix7/tile_type_DSP_{side}.json");
            // A copy fetched before the tile types were in the manifest
            // (2026-10-23) lacks them; `reticle fetch prjxray-db` adds them.
            let Ok(text) = std::fs::read_to_string(&path) else {
                eprintln!("skipped: no `{path}`; `reticle fetch prjxray-db` fetches it");
                return;
            };
            let json = reticle::json::Json::parse(&text).expect("the file is JSON");
            let pips = json.get("pips").and_then(|p| p.as_object()).expect("pips");
            let mut mux: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            for (_, pip) in pips {
                let to = pip.get("dst_wire").and_then(|w| w.as_str()).unwrap();
                let from = pip.get("src_wire").and_then(|w| w.as_str()).unwrap();
                mux.entry(to.to_owned()).or_default().push(from.to_owned());
            }
            let tables = reticle::fpga::xray::dsp_local_muxes();
            let mut seen = 0;
            for (to, from) in &tables {
                let inputs = mux.get(to).unwrap_or_else(|| panic!("no `{to}` in {path}"));
                assert!(inputs.contains(from), "`{from}` -> `{to}`: {inputs:?}");
                assert_eq!(inputs.len(), 3, "`{to}`: {inputs:?}");
                seen += 1;
            }
            let in_file = mux.values().filter(|v| v.len() == 3).count();
            assert_eq!(seen, in_file, "a mux the table has not got");
            let sites = json.get("sites").and_then(|s| s.as_array()).expect("sites");
            let blocks: Vec<_> = sites
                .iter()
                .filter(|s| s.get("type").and_then(|t| t.as_str()) == Some("DSP48E1"))
                .collect();
            assert_eq!(blocks.len(), 2);
            for (block, site) in blocks.iter().enumerate() {
                let pins = site
                    .get("site_pins")
                    .and_then(|p| p.as_object())
                    .expect("site_pins");
                let mut wires: Vec<String> = pins
                    .iter()
                    .map(|(_, pin)| pin.get("wire").and_then(|w| w.as_str()).unwrap().to_owned())
                    .collect();
                wires.sort();
                let mut ours = reticle::fpga::xray::dsp_pin_wires(u32::try_from(block).unwrap());
                ours.sort();
                assert_eq!(ours, wires, "{path}, block {block}");
            }
        }
    }
}
