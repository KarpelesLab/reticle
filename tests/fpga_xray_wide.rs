//! A wide status word: a concatenation of registers and constants, muxed
//! against a wide constant into a 128-bit shift register, emitted four
//! bits at a time.
//!
//! This is the shape `examples/basys3/iso7816_terminal.v` builds its
//! status word with, and on a Basys 3 it came back with each field's bits
//! moved: `CARD_DIV16`, a `localparam [15:0]` holding 14, read `0x00A4`,
//! and `etu_div`, holding 5208, read `0x14D0` — while the **pure
//! constant** pushed through the same shift register and the same emitter
//! came back byte for byte exact. `docs/fpga-xray.md` has the reading and
//! the arithmetic that fits it.
//!
//! Nothing in the tree covered the construct. `tests/fpga_xray_carry.rs`
//! has the one wide register the 7-series path ever placed and it is a
//! plain counter; `tests/fpga_flow.rs` maps a concatenation into a wide
//! register and never places it; and the equivalence the flow can prove
//! (`reticle fpga --verify`, `synth::techmap::verify`) cuts at the AIG
//! boundary, so it says nothing about a `FDRE`'s set/reset carrying a
//! constant leg, nor about a slice, nor about a route.
//!
//! So the two tests here sit either side of that gap:
//!
//! 1. [`the_mapped_netlist_loads_shifts_and_emits_the_word_the_source_does`]
//!    **simulates the mapped netlist** — the device primitives, with each
//!    `LUT6`'s own `INIT` and each flip-flop's own `INIT` and set/reset —
//!    over the whole load-and-emit sequence, and compares the 32 nibbles
//!    with what the source says they are. It needs no chip database.
//! 2. [`the_placed_design_keeps_every_pin_on_the_signal_the_netlist_names`]
//!    places and routes it on the real fabric and asks three things of the
//!    result that nothing else asked: that every sink is reached **from
//!    its own signal's driver**, that no node carries two signals, and
//!    that no slice holds flip-flops disagreeing about the things a slice
//!    has **one** of.
//!
//! **What they would and would not catch.** The first would catch a wide
//! mux whose bit index is miscomputed, a constant leg folded into the
//! wrong flip-flop's `R`/`S`, a truth table built for the wrong bit, and a
//! shift that moves by the wrong amount — the whole family the reading
//! looks like. It ran **green on the exact design that was measured**, so
//! it would *not* have caught the Basys 3 defect: that defect is not in
//! the netlist. The second would catch a route that lands on another
//! signal's metal and a slice packed with flip-flops the silicon cannot
//! configure together; both are also green on that design, so neither is
//! the defect either. What remains unchecked is everything between the
//! routing graph and the part: the bits a slice's shared mode fields
//! carry, and timing, which nothing here models.

#![cfg(all(feature = "fpga", feature = "synth", feature = "verilog"))]

mod common;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use common::{eval_expr, expr_nets};
use reticle::diag::Diagnostics;
use reticle::fpga::arch::NodeId;
use reticle::fpga::place::{PlaceOptions, place};
use reticle::fpga::xray::{XrayDatabase, XrayOptions, legalise_carries};
use reticle::fpga::{
    Constraints, FpgaOptions, Netlist, RouteOptions, route, synthesize_for, target,
};
use reticle::ir::memfile::FileProvider;
use reticle::ir::{AttrValue, Cell, CellId, CellKind, Design, Module, ModuleId, NetId};
use reticle::logic::{Bit, Logic};
use reticle::source::SourceMap;

const DEVICE: &str = "xc7a35t-cpg236";

/// The known constant, every nibble distinct, which the design loads
/// instead of the status word so that a reading can be told from the thing
/// it reads. The same value `iso7816_terminal.v` uses.
const KNOWN: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;

/// `etu_div`'s initial value: 372 elementary time units of a 14-cycle card
/// clock, which is what the terminal starts at.
const ETU_DIV: u64 = 5208;
/// What a PPS switches it to, and what the status word's comparison bit
/// compares against.
const FAST_DIV: u64 = 56;
/// The `localparam` whose sixteen bits reach the word as constants only.
const CARD_DIV: u64 = 14;

/// The four register fields' initial values, high field first. Distinct and
/// asymmetric so that a permutation of the word cannot look like the word.
const FIELDS: [u64; 4] = [0x1234, 0x5A5A, 0x0F1E, 0xBEEF];

/// The construct, with everything the terminal does around it removed: four
/// registers, a register that a command reloads, a three-bit state, a
/// sixteen-bit `localparam`, two constant bytes and a comparison, loaded
/// into a 128-bit shift register against a 128-bit constant and shifted out
/// four bits at a time.
///
/// Two deliberate choices. The registers are given their values by a
/// `preset` input rather than by a declaration, because a declared initial
/// value **does not reach the flip-flop\'s `INIT`** on this path — see
/// [`a_registers_declared_initial_value_reaches_the_flip_flops_init`] — and
/// a test of the status word should fail for its own reason and not for
/// that one. And the four fields rotate rather than count, because a
/// counter would bring a `CARRY4` into the netlist and the carry chain
/// already has `tests/fpga_xray_carry.rs` and `tests/fpga_carry.rs` to
/// itself.
const VERILOG: &str = r#"
module wide_status (
    input  wire       clk,
    input  wire       preset,
    input  wire       load_status,
    input  wire       load_known,
    input  wire       shift,
    input  wire       fast,
    input  wire       tick,
    output wire [3:0] nibble
);
    localparam [15:0] CARD_DIV16 = 16'd14;
    localparam [15:0] FAST_DIV16 = 16'd56;

    reg [15:0]  f3, f2, f1, f0;
    reg [15:0]  etu_div;
    reg [2:0]   state;
    reg [127:0] sh;

    wire [127:0] status = {
        f3, f2, f1, f0,
        etu_div, CARD_DIV16,
        {7'd0, etu_div == FAST_DIV16}, {5'd0, state},
        8'ha5,
        8'h07
    };

    always @(posedge clk) begin
        if (preset) begin
            f3      <= 16'h1234;
            f2      <= 16'h5a5a;
            f1      <= 16'h0f1e;
            f0      <= 16'hbeef;
            etu_div <= 16'd5208;
            state   <= 3'd1;
        end else begin
            if (fast) etu_div <= FAST_DIV16;
            if (tick) begin
                f3    <= {f3[14:0], f3[15]};
                f2    <= {f2[14:0], f2[15]};
                f1    <= {f1[14:0], f1[15]};
                f0    <= {f0[14:0], f0[15]};
                state <= {state[1:0], state[2]};
            end
        end
    end

    always @(posedge clk) begin
        if (preset)           sh <= 128'd0;
        else if (load_known)  sh <= 128'h0123456789abcdeffedcba9876543210;
        else if (load_status) sh <= status;
        else if (shift)       sh <= {sh[123:0], 4'd0};
    end

    assign nibble = sh[127:124];
endmodule
"#;

/// The pins, all on balls the Digilent Basys 3 master constraints name:
/// the oscillator, five switches and four LEDs.
const RCF: &str = "\
set_io -io_standard LVCMOS33 clk W5
set_io -io_standard LVCMOS33 preset V17
set_io -io_standard LVCMOS33 load_status V16
set_io -io_standard LVCMOS33 load_known W16
set_io -io_standard LVCMOS33 shift W17
set_io -io_standard LVCMOS33 fast W15
set_io -io_standard LVCMOS33 tick R2
set_io -io_standard LVCMOS33 nibble[0] U16
set_io -io_standard LVCMOS33 nibble[1] E19
set_io -io_standard LVCMOS33 nibble[2] U19
set_io -io_standard LVCMOS33 nibble[3] V19
create_clock -name sys -period 10.0 clk
clock_domain sys clk
";

/// The word the source says `status` is, with `etu_div` holding `etu`.
fn expected_status(etu: u64) -> u128 {
    let mut word: u128 = 0;
    for (index, field) in FIELDS.iter().enumerate() {
        word |= u128::from(*field) << (112 - 16 * index);
    }
    word |= u128::from(etu) << 48;
    word |= u128::from(CARD_DIV) << 32;
    word |= u128::from(u64::from(etu == FAST_DIV)) << 24;
    word |= 1 << 16; // `state`, whose initial value is one
    word |= 0xA5 << 8;
    word |= 0x07;
    word
}

// ---------------------------------------------------------------------------
// Building the design
// ---------------------------------------------------------------------------

/// Elaborates [`VERILOG`] and maps it for the 7 series, returning the
/// design, its top and the constraints the pins came from.
fn mapped() -> (Design, ModuleId, Constraints) {
    let mut map = SourceMap::new();
    let source = map.add("wide_status.v", VERILOG).unwrap();
    let rcf = map.add("wide_status.rcf", RCF).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::SystemVerilog,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let mut design = reticle::verilog::elaborate_file(
        &ast,
        &reticle::verilog::ElabOptions::default(),
        &mut diags,
    )
    .expect("the design elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let top = design.top.expect("one root module");
    let device = target(DEVICE).expect("a built-in device");
    let mut constraints = Constraints::parse(RCF, rcf, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the design maps");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    (design, top, constraints)
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

/// A clocked netlist of device primitives: the combinational part ordered
/// once, and every flip-flop's state carried from cycle to cycle.
///
/// `tests/fpga_carry.rs` has the combinational half of this and no
/// flip-flops, because a carry chain needs none. The shape under test here
/// is nothing *but* flip-flops, so the two cannot share: a flip-flop's
/// output is ready before its input, which is the whole difference.
///
/// Every primitive the 7-series flow emits for this design is modelled and
/// anything else is a panic. A netlist holding a primitive this file does
/// not model is one this file cannot vouch for.
struct Clocked<'m> {
    module: &'m Module,
    /// The combinational steps, in an order where every input is ready.
    steps: Vec<Step>,
    /// Every flip-flop: its cell, and the net its `Q` drives.
    flops: Vec<(CellId, NetId)>,
    /// Each flip-flop's state, by the same index.
    state: Vec<Bit>,
}

impl<'m> Clocked<'m> {
    fn new(module: &'m Module) -> Clocked<'m> {
        let is_flop = |cell: &Cell| -> bool {
            matches!(&cell.kind, CellKind::Blackbox(name) if name.as_str().starts_with("FD"))
        };
        let mut flops = Vec::new();
        let mut state = Vec::new();
        let mut pending: Vec<(Step, Vec<NetId>, Vec<NetId>)> = Vec::new();
        // A flip-flop's `Q` is ready at the start of a cycle, so it is a
        // source and not a step. Its `INIT` is the state it starts in.
        let mut ready: BTreeSet<NetId> = module
            .ports
            .iter()
            .filter(|p| p.dir != reticle::ir::PortDir::Out)
            .map(|p| p.net)
            .collect();
        for (id, cell) in module.cells.iter() {
            if is_flop(cell) {
                let q = cell.output("Q").expect("a flip-flop drives `Q`");
                ready.insert(q);
                flops.push((id, q));
                state.push(match cell.params.get("INIT") {
                    Some(AttrValue::Const(c)) => c.bit(0),
                    Some(AttrValue::Int(n)) => {
                        if *n == 0 {
                            Bit::Zero
                        } else {
                            Bit::One
                        }
                    }
                    None => Bit::Zero,
                    other => panic!("`{}` has INIT {other:?}", cell.name),
                });
                continue;
            }
            let mut needs = Vec::new();
            for (_, e) in &cell.inputs {
                needs.extend(expr_nets(module, *e));
            }
            pending.push((
                Step::Cell(id),
                needs,
                cell.outputs.iter().map(|(_, n)| *n).collect(),
            ));
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
        Clocked {
            module,
            steps,
            flops,
            state,
        }
    }

    /// The combinational values of one cycle, with `inputs` on the input
    /// ports and the flip-flops holding what they hold.
    fn settle(&self, inputs: &BTreeMap<NetId, Logic>) -> BTreeMap<NetId, Logic> {
        let mut values = inputs.clone();
        for (index, (_, q)) in self.flops.iter().enumerate() {
            values.insert(*q, Logic::from_bit(self.state[index]));
        }
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

    /// One rising edge: settle, then let every flip-flop take its new
    /// value. Returns the settled values, so a caller can read an output
    /// *before* the edge it is about to apply.
    fn tick(&mut self, inputs: &BTreeMap<NetId, Logic>) -> BTreeMap<NetId, Logic> {
        let values = self.settle(inputs);
        let bit = |e: Option<reticle::ir::ExprId>, default: Bit| -> Bit {
            e.map_or(default, |e| eval_expr(self.module, e, &values).bit(0))
        };
        let mut next = self.state.clone();
        for (index, (id, _)) in self.flops.iter().enumerate() {
            let cell = &self.module.cells[*id];
            let CellKind::Blackbox(name) = &cell.kind else {
                unreachable!("only blackboxes are collected")
            };
            let name = name.as_str();
            assert!(
                !name.ends_with("_1"),
                "`{}` is a negative-edge flip-flop and this model has one edge",
                cell.name
            );
            // The enable, which every 7-series flip-flop has, and which
            // `xc7.dev` calls `CE`. Absent means tied high.
            if bit(cell.input("CE"), Bit::One) != Bit::One {
                continue;
            }
            // The set or reset, synchronous for `FDRE`/`FDSE` and
            // asynchronous for `FDCE`/`FDPE` — both act on this edge in a
            // model with no delays, and the difference is invisible here.
            // Which value it forces is the primitive's own: `R`/`CLR` to
            // zero, `S`/`PRE` to one. It has priority over `D`.
            let (reset_pin, to) = match name {
                "FDRE" => ("R", Bit::Zero),
                "FDSE" => ("S", Bit::One),
                "FDCE" => ("CLR", Bit::Zero),
                "FDPE" => ("PRE", Bit::One),
                other => panic!("flip-flop `{other}` (cell `{}`) is not modelled", cell.name),
            };
            if bit(cell.input(reset_pin), Bit::Zero) == Bit::One {
                next[index] = to;
                continue;
            }
            next[index] = bit(cell.input("D"), Bit::Zero);
        }
        self.state = next;
        values
    }

    fn eval_cell(&self, cell: &Cell, values: &mut BTreeMap<NetId, Logic>) {
        let CellKind::Blackbox(name) = &cell.kind else {
            panic!(
                "cell `{}` of kind `{:?}` is not a device primitive",
                cell.name, cell.kind
            );
        };
        let name = name.as_str();
        match name {
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
            // A LUT of whatever width, indexed by its own input ports in
            // declaration order: on the 7 series `I0`..`I5`, `I0` least
            // significant, which is what `xc7.dev` declares and what
            // `cells_sim.v` says.
            lut if lut.starts_with("LUT") && lut.len() == 4 => {
                let init = match cell.params.get("INIT") {
                    Some(AttrValue::Const(c)) => c.clone(),
                    other => panic!("`{}` has INIT {other:?}", cell.name),
                };
                let mut index = 0u32;
                for (pin, (port, e)) in cell.inputs.iter().enumerate() {
                    let _ = port;
                    if eval_expr(self.module, *e, values).bit(0) == Bit::One {
                        index |= 1 << pin;
                    }
                }
                let (_, out) = cell.outputs.first().expect("a LUT has an output");
                values.insert(*out, Logic::from_bit(init.bit(index)));
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

/// Loads the shift register from whichever leg `load` names and shifts the
/// whole 128 bits out four at a time, assembling them the way the terminal's
/// host does: first nibble most significant.
///
/// `fast` first reloads `etu_div`, so the same emitter can be asked for two
/// different status words.
fn emit(module: &Module, load: &str, fast: bool) -> u128 {
    let mut sim = Clocked::new(module);
    let pins = [
        "clk",
        "preset",
        "load_status",
        "load_known",
        "shift",
        "fast",
        "tick",
    ];
    let nets: BTreeMap<&str, NetId> = pins.iter().map(|p| (*p, port_net(module, p))).collect();
    let nibble = port_net(module, "nibble");
    let drive = |held: &[&str]| -> BTreeMap<NetId, Logic> {
        let mut inputs = BTreeMap::new();
        for pin in pins {
            let one = held.contains(&pin);
            inputs.insert(
                nets[pin],
                Logic::from_bit(if one { Bit::One } else { Bit::Zero }),
            );
        }
        inputs
    };
    // `preset` puts the registers where the test wants them, which is a
    // cycle of the design and not a back door into the model.
    sim.tick(&drive(&["preset"]));
    if fast {
        sim.tick(&drive(&["fast"]));
    }
    sim.tick(&drive(&[load]));
    let mut word: u128 = 0;
    for _ in 0..32 {
        let values = sim.tick(&drive(&["shift"]));
        let seen = values
            .get(&nibble)
            .expect("`nibble` is driven")
            .to_u64()
            .expect("`nibble` holds no unknown bit");
        word = (word << 4) | u128::from(seen & 0xF);
    }
    word
}

/// **The mapped netlist loads, shifts and emits the word the source does.**
///
/// Four emissions: the status word, the status word after a command has
/// reloaded `etu_div`, and the known constant before and after the same
/// command — the constant must not notice. The netlist is the device's own
/// primitives, each `LUT6` evaluated through its `INIT` and each flip-flop
/// through its `INIT` and its set or reset, so a constant leg that the
/// mapper folded into a `FDSE`'s `S` is exercised as such.
///
/// Would catch: a wide mux whose bit index is off, a constant whose bits
/// land on the wrong flip-flops, a truth table built for a neighbouring
/// bit, a shift of the wrong distance, and an initial value lost on the way
/// to `INIT`. Would not catch: anything about placement, routing or the
/// bits a slice's shared fields carry — and, measured, it does not catch
/// the Basys 3 reading this file's header quotes, which is the point of
/// saying so.
#[test]
fn the_mapped_netlist_loads_shifts_and_emits_the_word_the_source_does() {
    let (design, top, _) = mapped();
    let module = design.module(top);
    assert_eq!(
        emit(module, "load_status", false),
        expected_status(ETU_DIV),
        "the status word came back altered"
    );
    assert_eq!(
        emit(module, "load_status", true),
        expected_status(FAST_DIV),
        "the status word came back altered after the fast reload"
    );
    for fast in [false, true] {
        assert_eq!(
            emit(module, "load_known", fast),
            KNOWN,
            "the known constant came back altered (fast = {fast})"
        );
    }
}

/// **A register's declared initial value reaches the flip-flop's `INIT`.**
///
/// It does not, and this test says so rather than leaving it to be found on
/// a board. `reg [15:0] r = 16'h1234;` with no reset becomes sixteen
/// `FDRE #(INIT=1'd0)`: the IR's [`reticle::ir::CellKind::Dff`] has no
/// field for a power-up value at all, so `techcells::set_params` copies the
/// *device file's declared default* for the variant it picked and nothing
/// else — `FDRE` is declared `INIT=1'b0` and `FDSE` `INIT=1'b1` in
/// `src/fpga/devices/xc7.dev`.
///
/// On this fabric that is not a cosmetic loss. `<letter>FF.ZINI` is an
/// **inverted** field: the bit has to be *set* for a flip-flop to power up
/// holding zero, so `INIT` is what decides the state the part comes out of
/// configuration in. Two consequences, both measured off the netlist:
///
/// - a register with a declared value powers up at **zero** instead, except
///   in the bits its set/reset drives to one, which come up at **one** —
///   `reg [15:0] etu_div = 16'd5208` with `if (fast) etu_div <= 16'd56`
///   maps to flip-flops whose `INIT`s spell **56**, the reset value;
/// - a register declared zero that is *set* somewhere comes up holding
///   **one**, because the bit took `FDSE`'s default.
///
/// Fixing it means giving `CellKind::Dff` an initial value and carrying it
/// through elaboration, generic synthesis, the equivalence checker and
/// every backend, which is more than this test's change. It is
/// `#[ignore]`d so that the gate stays green while the reproducer stays in
/// the tree; `cargo test --all-features --test fpga_xray_wide -- --ignored`
/// is the one command that shows it.
///
/// Would catch: exactly this, and any later regression of it. Would not
/// catch: whether `ZINI` is the right way up — that is a reading quoted
/// from prjxray's `011-clb-ffconfig` fuzzer in
/// `src/fpga/xray/sites.rs`, and only a part settles it.
#[test]
#[ignore = "known defect: a register's declared initial value never reaches INIT"]
fn a_registers_declared_initial_value_reaches_the_flip_flops_init() {
    const SOURCE: &str = r#"
module held (input wire clk, input wire tick, output wire [15:0] q);
    reg [15:0] r = 16'h1234;
    always @(posedge clk) if (tick) r <= {r[14:0], r[15]};
    assign q = r;
endmodule
"#;
    let mut map = SourceMap::new();
    let source = map.add("held.v", SOURCE).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::SystemVerilog,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let mut design = reticle::verilog::elaborate_file(
        &ast,
        &reticle::verilog::ElabOptions::default(),
        &mut diags,
    )
    .expect("the design elaborates");
    let top = design.top.expect("one root module");
    let device = target(DEVICE).expect("a built-in device");
    synthesize_for(
        &mut design,
        top,
        device,
        &Constraints::default(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the design maps");
    let module = design.module(top);
    let mut got: u64 = 0;
    let mut flops = 0;
    for (_, cell) in module.cells.iter() {
        let CellKind::Blackbox(name) = &cell.kind else {
            continue;
        };
        if !name.as_str().starts_with("FD") {
            continue;
        }
        let bit: u32 = cell
            .name
            .as_str()
            .rsplit("$ff")
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("`{}` is not named `…$ff<bit>`", cell.name));
        let init = match cell.params.get("INIT") {
            Some(AttrValue::Const(c)) => c.bit(0),
            other => panic!("`{}` has INIT {other:?}", cell.name),
        };
        if init == Bit::One {
            got |= 1 << bit;
        }
        flops += 1;
    }
    assert_eq!(flops, 16, "a 16-bit register is sixteen flip-flops");
    assert_eq!(
        got, 0x1234,
        "the flip-flops' INIT spells {got:#06x}, not the declared 0x1234"
    );
}

// ---------------------------------------------------------------------------
// Placing and routing it on the real fabric
// ---------------------------------------------------------------------------

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing. The
/// same search the other `fpga_xray*` tests make.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a50t/tilegrid.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but `{root}/{probe}` is not there");
        return None;
    }
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")));
    let dir = cache.map(|c| format!("{c}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3"));
    match dir {
        Some(dir) if Path::new(&format!("{dir}/{probe}")).is_file() => Some(dir),
        _ => {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` \
                 or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// **Every pin is reached from its own signal's driver, no node carries two
/// signals, and no slice holds flip-flops the silicon cannot configure
/// together.**
///
/// The first is [`reticle::fpga::Routing::verify`]'s question asked the
/// other way round: `verify` walks a sink back through *that signal's* pips
/// and reports a sink it cannot reach, so a pin the right signal reaches
/// **and another signal also drives** is invisible to it. This builds the
/// reverse map over every route of the design at once and walks back
/// through all of them, so the signal a pin arrives from is named rather
/// than assumed.
///
/// The third is the gap `src/fpga/xray/sites.rs` documents and nothing
/// enforced: `FFSYNC`, `CLKINV` and `LATCH` are **one field for the whole
/// slice**, and this model gives every flip-flop a bel of its own, so a
/// slice holding an `FDRE` and an `FDCE` has its mode bits OR-ed together
/// rather than refused. `xc7.dev` says `shared_reset`, and
/// [`reticle::fpga::FfFeatures::reset_is_shared`] is parsed and read by
/// nothing. The clock, the enable and the set/reset are *wires* the slice
/// shares, which the placer's own site rules do enforce — so this test
/// passes today on the strength of that, and the assertion is here to fail
/// the day a packer makes the mode fields reachable.
///
/// Would catch: a route delivering another signal's value to a pin, two
/// signals welded onto one node, and a slice packed with flip-flops that
/// disagree about its one clock, its one enable, its one set/reset or its
/// one synchronous-or-not field. Would not catch: a bel pin whose *wire*
/// `xray::sites` names wrongly — this check reads the same table the router
/// did, so it is blind to it in exactly the way `verify` is.
#[test]
fn the_placed_design_keeps_every_pin_on_the_signal_the_netlist_names() {
    let Some(root) = chipdb() else { return };
    let (mut design, top, constraints) = mapped();
    let device = target(DEVICE).expect("a built-in device");
    legalise_carries(&mut design, top, device).expect("there is no chain to legalise");

    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let pins: Vec<String> = constraints.pins.iter().map(|p| p.pin.clone()).collect();
    let region = db
        .region_for_pins(&DiskFiles, &pins, 12)
        .unwrap()
        .expect("the constrained pins name sites");
    let region = db
        .region_with_site_type(&DiskFiles, region, "BUFGCTRL")
        .unwrap()
        .unwrap_or(region);
    let mut options = XrayOptions::new();
    options.region = Some(region);
    let fabric = db.load(&DiskFiles, &options).unwrap();
    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    let (placement, _) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    let (routing, _) = route(&netlist, &graph, &placement, &RouteOptions::default())
        .expect("the shift register routes");
    let problems = routing.verify(&netlist, &graph, &placement);
    assert!(problems.is_empty(), "{problems:#?}");

    // Every node any route drives, and the signal whose pip drives it,
    // over the whole design at once.
    let mut driven: HashMap<NodeId, Vec<usize>> = HashMap::new();
    let mut from: HashMap<(usize, NodeId), NodeId> = HashMap::new();
    let mut source_of: HashMap<NodeId, usize> = HashMap::new();
    for signal in netlist.routable() {
        let route = routing.route(signal).expect("verify said every one routes");
        source_of.insert(route.source, signal);
        for pip in &route.pips {
            let pip = graph.pip(*pip);
            driven.entry(pip.to).or_default().push(signal);
            from.insert((signal, pip.to), pip.from);
        }
    }
    let mut shorted: BTreeSet<String> = BTreeSet::new();
    for (node, signals) in &driven {
        if signals.len() > 1 {
            shorted.insert(format!(
                "{} is driven by {:?}",
                graph.wire(*node).full_name(),
                signals
                    .iter()
                    .map(|s| netlist.signals[*s].name.clone())
                    .collect::<Vec<_>>()
            ));
        }
    }
    let mut wrong: Vec<String> = Vec::new();
    for signal in netlist.routable() {
        for pin in &netlist.signals[signal].sinks {
            let pin = &netlist.pins[*pin];
            let Some(site) = placement.site_of(pin.instance) else {
                continue;
            };
            let name = format!(
                "{}.{}[{}]",
                netlist.instances[pin.instance].name, pin.port, pin.bit
            );
            for start in graph.sites[site].pin_nodes(&pin.role) {
                let mut node = start;
                let mut steps = 0usize;
                loop {
                    if let Some(found) = source_of.get(&node) {
                        if *found != signal {
                            wrong.push(format!(
                                "{name} wants `{}` and is reached from `{}`",
                                netlist.signals[signal].name, netlist.signals[*found].name
                            ));
                        }
                        break;
                    }
                    let Some(next) = from.get(&(signal, node)) else {
                        wrong.push(format!(
                            "{name} wants `{}` and stops at {}, which that signal does not drive",
                            netlist.signals[signal].name,
                            graph.wire(node).full_name()
                        ));
                        break;
                    };
                    node = *next;
                    steps += 1;
                    assert!(steps <= graph.nodes.len(), "{name}: a loop in the route");
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert!(shorted.is_empty(), "{shorted:#?}");

    // What a slice has one of. The flip-flops are grouped by the slice
    // their bel belongs to, which is the bel name with the `<letter>FF`
    // suffix taken off.
    let signal_on = |instance: usize, role: &str| -> String {
        netlist.instances[instance]
            .pins
            .iter()
            .map(|p| &netlist.pins[*p])
            .find(|p| p.role == role)
            .map_or_else(
                || "absent".to_owned(),
                |p| match p.signal {
                    Some(s) => netlist.signals[s].name.clone(),
                    None => format!("tied to {:?}", p.constant),
                },
            )
    };
    let synchronous =
        |primitive: &str| -> bool { matches!(primitive, "FDRE" | "FDSE" | "FDRE_1" | "FDSE_1") };
    let mut slices: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, instance) in netlist.instances.iter().enumerate() {
        if instance.kind != "ff" {
            continue;
        }
        let Some(site) = placement.site_of(index) else {
            continue;
        };
        let site = &graph.sites[site];
        let slice = site
            .bel
            .rsplit_once('_')
            .map_or_else(|| site.bel.clone(), |(head, _)| head.to_owned());
        slices
            .entry(format!("{:?}/{slice}", site.tile))
            .or_default()
            .push(index);
    }
    assert!(!slices.is_empty(), "128 flip-flops went somewhere");
    let mut packed: Vec<String> = Vec::new();
    for (slice, instances) in &slices {
        let mut disagree = Vec::new();
        for role in ["clk", "en", "rst"] {
            let first = signal_on(instances[0], role);
            if instances.iter().any(|i| signal_on(*i, role) != first) {
                disagree.push(role);
            }
        }
        let kinds: Vec<&str> = instances
            .iter()
            .map(|i| netlist.instances[*i].primitive.as_str())
            .collect();
        if kinds.iter().any(|k| synchronous(k)) && kinds.iter().any(|k| !synchronous(k)) {
            disagree.push("synchronous set/reset (FFSYNC)");
        }
        if !disagree.is_empty() {
            packed.push(format!(
                "{slice} holds {kinds:?}, disagreeing about {disagree:?}"
            ));
        }
    }
    assert!(packed.is_empty(), "{packed:#?}");
}
