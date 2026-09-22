//! Technology mapping: covering an AIG with LUTs or standard cells.
//!
//! Mapping chooses, for every node the outputs need, one *cut* to
//! implement it with, and turns the chosen cuts into cells. The
//! implementation follows Mishchenko, Chatterjee and Brayton,
//! "Improvements to technology mapping for LUT-based FPGAs" (FPGA 2007 /
//! TCAD 2007), whose three ideas are what make a cut-based mapper
//! competitive:
//!
//! - **Priority cuts.** Rather than enumerating every cut of every node,
//!   each node keeps a bounded, sorted list of the most promising cuts,
//!   merged from its fanins' lists. The list size and the cut size bound
//!   the work, and the quality lost is small.
//! - **Area flow.** The area of a cut is estimated as its own area plus
//!   the *area flow* of its leaves, where a leaf's area flow is divided by
//!   its fanout. This spreads the cost of shared logic over its users and
//!   is a far better global estimate than local area.
//! - **Exact area with references.** Once a mapping exists, the true
//!   number of cells a cut adds can be measured by dereferencing the
//!   current choice and referencing the candidate, exactly as the AIG
//!   rewriting does with its MFFCs. The final passes use this to recover
//!   area without hurting depth.
//!
//! The flow is: a depth-oriented pass first (minimising arrival time,
//! breaking ties on area flow), which fixes the achievable depth; then
//! area recovery passes (area flow, then exact area) that may not push any
//! node's arrival time past what that first pass achieved, so depth is
//! preserved while area falls. Holding every node to its own arrival time
//! is a conservative stand-in for propagating true required times backward
//! from the outputs: it never lengthens a path, but it also declines some
//! area recovery that slack on a non-critical path would allow.
//!
//! # Entry points
//!
//! | Function | Result |
//! |----------|--------|
//! | [`lut_map`] | A [`LutNetwork`] of `k`-input LUTs, `k` in `2..=8` |
//! | [`gate_map`] | A [`GateNetwork`] over a [`GateLibrary`] |
//! | [`map_module`] | Extract, optimise, map and rewrite a whole [`Module`] |
//!
//! # LUT init ordering
//!
//! A LUT's `init` constant has one bit per input pattern: **bit `i` is the
//! output for the pattern whose bit `j` is the value of input `j`**, with
//! input 0 the least significant. That is the same convention as
//! [`CellKind::Lut`] and as the AIG's
//! [`truth`](super::aig::truth) tables, so a cut's function is written out
//! unchanged. Input 0 of a mapped LUT is the least significant bit of the
//! expression connected to its `a` port.

use std::collections::HashMap;

use super::aig::emit::{NetNode, Netlist, Signal, write_back};
use super::aig::truth::TruthTable;
use super::aig::{Aig, AigOptions, AigStats};
use super::cells::{Gate, GateLibrary};
use crate::ir::attr::Attrs;
use crate::ir::design::Module;
use crate::ir::types::Const;
use crate::ir::{CellKind, Name};
use crate::logic::Bit;

mod cuts;

pub use cuts::{Cut, CutOptions, PriorityCuts};

/// What to map onto.
pub enum Target<'a> {
    /// `k`-input lookup tables, `k` in `2..=8`.
    Lut(u32),
    /// Standard cells from a library.
    Gates(&'a GateLibrary),
}

/// Options for [`map_module`].
pub struct MapOptions<'a> {
    /// What to map onto.
    pub target: Target<'a>,
    /// How hard to optimise the AIG before mapping.
    pub aig_opt: AigOptions,
    /// Number of area recovery passes after the depth-oriented one.
    pub area_passes: u32,
    /// Cut enumeration limits.
    pub cuts: CutOptions,
}

impl Default for MapOptions<'_> {
    fn default() -> Self {
        MapOptions {
            target: Target::Lut(4),
            aig_opt: AigOptions::default(),
            area_passes: 2,
            cuts: CutOptions::default(),
        }
    }
}

impl<'a> MapOptions<'a> {
    /// Options mapping to `k`-input LUTs with everything else default.
    pub fn lut(k: u32) -> MapOptions<'a> {
        MapOptions {
            target: Target::Lut(k),
            ..MapOptions::default()
        }
    }

    /// Options mapping to `library` with everything else default.
    pub fn gates(library: &'a GateLibrary) -> MapOptions<'a> {
        MapOptions {
            target: Target::Gates(library),
            ..MapOptions::default()
        }
    }
}

/// What [`map_module`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapStats {
    /// The AIG extracted from the module, before optimisation.
    pub before: AigStats,
    /// The AIG handed to the mapper.
    pub after: AigStats,
    /// Cells the mapping produced.
    pub cells: usize,
    /// Depth of the mapped network, in cells.
    pub depth: u32,
    /// Total area, for a gate mapping; the cell count for a LUT mapping.
    pub area: f64,
}

/// One mapped LUT.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    /// Its inputs, input 0 first.
    pub inputs: Vec<Signal>,
    /// The function, `2^inputs.len()` bits, bit `i` for pattern `i`.
    pub init: Const,
}

/// A network of LUTs over the AIG's inputs.
#[derive(Clone, Debug, Default)]
pub struct LutNetwork {
    /// The LUTs, fanins before fanouts.
    pub luts: Vec<Lut>,
    /// One signal per AIG output, in order.
    pub outputs: Vec<Signal>,
    /// Depth in LUTs.
    pub depth: u32,
}

impl LutNetwork {
    /// Number of LUTs.
    pub fn len(&self) -> usize {
        self.luts.len()
    }

    /// True when nothing was mapped.
    pub fn is_empty(&self) -> bool {
        self.luts.is_empty()
    }

    /// How many LUTs have each input count, indexed by arity.
    pub fn histogram(&self) -> Vec<usize> {
        let max = self.luts.iter().map(|l| l.inputs.len()).max().unwrap_or(0);
        let mut h = vec![0usize; max + 1];
        for lut in &self.luts {
            h[lut.inputs.len()] += 1;
        }
        h
    }

    /// The network as a [`Netlist`] of `Lut` cells.
    pub fn to_netlist(&self) -> Netlist {
        Netlist {
            nodes: self
                .luts
                .iter()
                .map(|lut| NetNode {
                    kind: CellKind::Lut {
                        k: u32::try_from(lut.inputs.len()).expect("arity"),
                        init: lut.init.clone(),
                    },
                    ports: vec![Name::new("a")],
                    fanins: lut.inputs.clone(),
                    attrs: Attrs::new(),
                    prefix: "lut",
                })
                .collect(),
            outputs: self.outputs.clone(),
        }
    }

    /// Writes the network into `module` in place of the logic `mapping`
    /// absorbed.
    pub fn to_module(&self, mapping: &super::aig::Mapping, module: &mut Module) {
        write_back(&self.to_netlist(), mapping, module);
    }
}

/// One mapped standard cell.
#[derive(Clone, Debug)]
pub struct MappedGate {
    /// The library cell.
    pub gate: String,
    /// The IR primitive to emit, when the cell is exactly one.
    pub primitive: Option<CellKind>,
    /// The cell's input pin names, in pin order.
    pub pins: Vec<Name>,
    /// The signal feeding each pin, in pin order.
    pub fanins: Vec<Signal>,
    /// The cell's area.
    pub area: f64,
}

/// A network of standard cells over the AIG's inputs.
#[derive(Clone, Debug, Default)]
pub struct GateNetwork {
    /// The cells, fanins before fanouts.
    pub gates: Vec<MappedGate>,
    /// One signal per AIG output, in order.
    pub outputs: Vec<Signal>,
    /// Total area.
    pub area: f64,
    /// Depth in cells.
    pub depth: u32,
}

impl GateNetwork {
    /// Number of cells.
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// True when nothing was mapped.
    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    /// How many cells of each library gate the network uses, sorted by
    /// name.
    pub fn histogram(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for g in &self.gates {
            *counts.entry(g.gate.as_str()).or_default() += 1;
        }
        let mut out: Vec<(String, usize)> =
            counts.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        out.sort();
        out
    }

    /// The network as a [`Netlist`]: a primitive cell where the library
    /// gate is exactly one, a `Blackbox` carrying a `lib_cell` attribute
    /// otherwise.
    pub fn to_netlist(&self) -> Netlist {
        Netlist {
            nodes: self
                .gates
                .iter()
                .map(|g| {
                    let mut attrs = Attrs::new();
                    attrs.set("lib_cell", g.gate.as_str());
                    match &g.primitive {
                        Some(kind) => NetNode {
                            kind: kind.clone(),
                            ports: primitive_ports(kind),
                            fanins: g.fanins.clone(),
                            attrs,
                            prefix: "g",
                        },
                        None => NetNode {
                            kind: CellKind::Blackbox(Name::new(g.gate.clone())),
                            ports: g.pins.clone(),
                            fanins: g.fanins.clone(),
                            attrs,
                            prefix: "g",
                        },
                    }
                })
                .collect(),
            outputs: self.outputs.clone(),
        }
    }

    /// Writes the network into `module` in place of the logic `mapping`
    /// absorbed.
    pub fn to_module(&self, mapping: &super::aig::Mapping, module: &mut Module) {
        write_back(&self.to_netlist(), mapping, module);
    }
}

/// The IR port names of a primitive, in the order the mapper wires them.
fn primitive_ports(kind: &CellKind) -> Vec<Name> {
    kind.input_ports().into_iter().map(Name::new).collect()
}

/// A `Blackbox` cell has no declared ports, so a gate's pins are used as
/// written.
fn gate_pins(gate: &Gate) -> Vec<Name> {
    gate.pins.iter().map(Name::new).collect()
}

/// Maps `aig` onto `k`-input LUTs.
///
/// # Panics
///
/// If `k` is outside `2..=8`.
pub fn lut_map(aig: &Aig, k: u32) -> LutNetwork {
    lut_map_with(aig, k, &CutOptions::default(), 2)
}

/// [`lut_map`] with explicit cut limits and area recovery passes.
pub fn lut_map_with(aig: &Aig, k: u32, options: &CutOptions, area_passes: u32) -> LutNetwork {
    assert!((2..=8).contains(&k), "LUT size {k} is outside 2..=8");
    let size = usize::try_from(k).expect("LUT size");
    let cost = LutCost;
    let mapping = run(aig, size, options, area_passes, &cost);
    let mut emitter = LutEmitter { luts: Vec::new() };
    let outputs = mapping.emit(aig, &mut emitter);
    LutNetwork {
        luts: emitter.luts,
        outputs,
        depth: mapping.depth,
    }
}

/// Collects the LUTs of a mapping.
struct LutEmitter {
    luts: Vec<Lut>,
}

impl LutEmitter {
    fn push(&mut self, lut: Lut) -> u32 {
        self.luts.push(lut);
        u32::try_from(self.luts.len() - 1).expect("lut index")
    }
}

impl Emitter for LutEmitter {
    fn cell(&mut self, function: &TruthTable, inputs: &[Signal]) -> Option<u32> {
        // A LUT implements any function of its inputs.
        Some(self.push(Lut {
            inputs: inputs.to_vec(),
            init: truth_to_const(function, inputs.len()),
        }))
    }

    fn inverter(&mut self, input: Signal) -> u32 {
        // A one-input LUT computing `!a`: output 1 for pattern 0.
        self.push(Lut {
            inputs: vec![input],
            init: Const::from_u64(0b01, 2),
        })
    }
}

/// Maps `aig` onto the cells of `library`.
pub fn gate_map(aig: &Aig, library: &GateLibrary) -> GateNetwork {
    gate_map_with(aig, library, &CutOptions::default(), 2)
}

/// [`gate_map`] with explicit cut limits and area recovery passes.
pub fn gate_map_with(
    aig: &Aig,
    library: &GateLibrary,
    options: &CutOptions,
    area_passes: u32,
) -> GateNetwork {
    assert!(!library.is_empty(), "the gate library is empty");
    assert!(
        library.inverter().is_some(),
        "the gate library has no inverter"
    );
    let size = library.max_arity().clamp(1, 4);
    let cost = GateCost { library };
    let mapping = run(aig, size, options, area_passes, &cost);
    let mut emitter = GateEmitter {
        library,
        gates: Vec::new(),
        area: 0.0,
    };
    let outputs = mapping.emit(aig, &mut emitter);
    GateNetwork {
        gates: emitter.gates,
        outputs,
        area: emitter.area,
        depth: mapping.depth,
    }
}

/// Collects the standard cells of a mapping.
struct GateEmitter<'a> {
    library: &'a GateLibrary,
    gates: Vec<MappedGate>,
    area: f64,
}

impl GateEmitter<'_> {
    fn push(&mut self, gate: &Gate, fanins: Vec<Signal>) -> u32 {
        self.area += gate.area;
        self.gates.push(MappedGate {
            gate: gate.name.clone(),
            primitive: gate.primitive.clone(),
            pins: gate_pins(gate),
            fanins,
            area: gate.area,
        });
        u32::try_from(self.gates.len() - 1).expect("gate index")
    }
}

impl Emitter for GateEmitter<'_> {
    fn cell(&mut self, function: &TruthTable, inputs: &[Signal]) -> Option<u32> {
        let (gate, m) = self.library.match_function(function)?;
        let gate = gate.clone();
        // `m.wiring[v]` is the pin that cut variable `v` feeds, through an
        // inverter when `m.invert` says so.
        let mut fanins = vec![Signal::Const(false); gate.arity()];
        for (v, &pin) in m.wiring.iter().enumerate() {
            let mut signal = inputs[v];
            if (m.invert >> v) & 1 == 1 {
                signal = Signal::Node(self.inverter(signal));
            }
            fanins[pin] = signal;
        }
        Some(self.push(&gate, fanins))
    }

    fn inverter(&mut self, input: Signal) -> u32 {
        let inverter = self
            .library
            .inverter()
            .expect("checked by gate_map_with")
            .clone();
        self.push(&inverter, vec![input])
    }
}

/// The truth table of a cut as a LUT `init` constant.
fn truth_to_const(function: &TruthTable, inputs: usize) -> Const {
    let width = 1u32 << inputs;
    let mut init = Const::zero(width);
    for pattern in 0..(1usize << inputs) {
        if function.bit(pattern) {
            init.set_bit(u32::try_from(pattern).expect("pattern"), Bit::One);
        }
    }
    init
}

/// How a target turns a chosen cut into a cell.
///
/// Both methods return the index of the cell created, which becomes a
/// [`Signal::Node`].
trait Emitter {
    /// Creates the cell implementing `function` over `inputs`, or `None`
    /// when the target has nothing for that function (a gate library need
    /// not contain every function; a LUT always does).
    fn cell(&mut self, function: &TruthTable, inputs: &[Signal]) -> Option<u32>;

    /// Creates a cell computing the complement of `input`.
    fn inverter(&mut self, input: Signal) -> u32;
}

/// How a target prices a cut.
trait Cost {
    /// The area of one cell implementing `function` over `inputs` leaves,
    /// or `None` when the target cannot implement it at all.
    fn area(&self, function: &TruthTable, inputs: usize) -> Option<f64>;

    /// The delay a cell adds; LUTs use one level per cell.
    fn delay(&self, function: &TruthTable, inputs: usize) -> f64;

    /// True when a cut of `inputs` leaves is worth considering at all.
    fn accepts(&self, inputs: usize) -> bool;
}

/// LUT area: one LUT per cut regardless of size, one level of delay.
struct LutCost;

impl Cost for LutCost {
    fn area(&self, _function: &TruthTable, _inputs: usize) -> Option<f64> {
        Some(1.0)
    }

    fn delay(&self, _function: &TruthTable, _inputs: usize) -> f64 {
        1.0
    }

    fn accepts(&self, _inputs: usize) -> bool {
        true
    }
}

/// Gate area: whatever the library's cheapest match costs, if there is
/// one.
struct GateCost<'a> {
    library: &'a GateLibrary,
}

impl Cost for GateCost<'_> {
    fn area(&self, function: &TruthTable, _inputs: usize) -> Option<f64> {
        // The inverters an input negation needs are part of the price.
        self.library.match_cost(function)
    }

    fn delay(&self, function: &TruthTable, _inputs: usize) -> f64 {
        match self.library.match_function(function) {
            Some((gate, m)) => {
                let inv = if m.inverters() > 0 {
                    self.library.inverter().map_or(0.0, Gate::max_delay)
                } else {
                    0.0
                };
                gate.max_delay() + inv
            }
            None => 1.0,
        }
    }

    fn accepts(&self, inputs: usize) -> bool {
        inputs <= 4
    }
}

/// The chosen cut of every node, plus the figures the passes maintain.
struct Mapping {
    /// The chosen cut per node, `None` for inputs, the constant and nodes
    /// nothing uses.
    best: Vec<Option<Cut>>,
    /// The arrival time of each node under the current choice.
    arrival: Vec<f64>,
    /// Area flow per node.
    area_flow: Vec<f64>,
    /// How many chosen cuts use each node, plus the outputs.
    refs: Vec<u32>,
    /// Depth of the mapped network, in cells.
    depth: u32,
}

impl Mapping {
    /// Walks the chosen cuts from the outputs and has `emitter` build a
    /// cell for each, in topological order; returns the signal of every
    /// AIG output.
    ///
    /// An output edge may be complemented while the cut functions account
    /// for polarity only *inside* their cones. Rather than always adding
    /// an inverter, a node that nothing else uses is emitted with its
    /// function complemented, which costs nothing: for a LUT that is the
    /// complemented `init`, for a gate library the matching cell of the
    /// complementary function. An inverter is added only when the node is
    /// genuinely needed in both polarities, or when the target has no cell
    /// for the complemented function.
    fn emit(&self, aig: &Aig, emitter: &mut dyn Emitter) -> Vec<Signal> {
        // The covered nodes, fanins before fanouts.
        let order = self.cover_order(aig);

        // Which polarity of each node is actually used.
        let mut needs_plain = vec![false; aig.len()];
        let mut needs_comp = vec![false; aig.len()];
        for &id in &order {
            // A cut's leaves are always read in plain polarity.
            for &leaf in &self.best[id as usize].as_ref().expect("covered").leaves {
                needs_plain[leaf as usize] = true;
            }
        }
        for &e in aig.outputs() {
            if e.is_complement() {
                needs_comp[e.index()] = true;
            } else {
                needs_plain[e.index()] = true;
            }
        }

        let mut plain: Vec<Option<Signal>> = vec![None; aig.len()];
        let mut complemented: Vec<Option<Signal>> = vec![None; aig.len()];
        plain[0] = Some(Signal::Const(false));
        complemented[0] = Some(Signal::Const(true));
        for (pos, &pi) in aig.inputs().iter().enumerate() {
            plain[pi as usize] = Some(Signal::Input(u32::try_from(pos).expect("input index")));
        }

        for &id in &order {
            let cut = self.best[id as usize].as_ref().expect("covered");
            let inputs: Vec<Signal> = cut
                .leaves
                .iter()
                .map(|&l| plain[l as usize].expect("leaf emitted"))
                .collect();
            if needs_plain[id as usize] {
                let index = emitter
                    .cell(&cut.function, &inputs)
                    .expect("the cover only chose implementable cuts");
                plain[id as usize] = Some(Signal::Node(index));
            }
            if !needs_comp[id as usize] {
                continue;
            }
            // Free complement: the node is used in one polarity only.
            if !needs_plain[id as usize]
                && let Some(index) = emitter.cell(&cut.function.not(), &inputs)
            {
                complemented[id as usize] = Some(Signal::Node(index));
                continue;
            }
            let base = match plain[id as usize] {
                Some(s) => s,
                None => {
                    let index = emitter
                        .cell(&cut.function, &inputs)
                        .expect("the cover only chose implementable cuts");
                    let s = Signal::Node(index);
                    plain[id as usize] = Some(s);
                    s
                }
            };
            complemented[id as usize] = Some(Signal::Node(emitter.inverter(base)));
        }

        aig.outputs()
            .iter()
            .map(|&e| {
                let slot = if e.is_complement() {
                    &complemented
                } else {
                    &plain
                };
                match (slot[e.index()], e.is_complement()) {
                    (Some(s), _) => s,
                    // An input or the constant read complemented.
                    (None, true) => {
                        let base = plain[e.index()].expect("input emitted");
                        Signal::Node(emitter.inverter(base))
                    }
                    (None, false) => unreachable!("plain output was not emitted"),
                }
            })
            .collect()
    }

    /// The covered AND nodes, fanins before fanouts.
    fn cover_order(&self, aig: &Aig) -> Vec<u32> {
        let mut order = Vec::new();
        let mut seen = vec![false; aig.len()];
        let mut stack: Vec<(u32, bool)> = Vec::new();
        for &out in aig.outputs() {
            if aig.is_and(out.node()) {
                stack.push((out.node(), false));
            }
        }
        while let Some((id, expanded)) = stack.pop() {
            if expanded {
                order.push(id);
                continue;
            }
            if seen[id as usize] {
                continue;
            }
            seen[id as usize] = true;
            stack.push((id, true));
            for &leaf in &self.best[id as usize]
                .as_ref()
                .expect("covered node")
                .leaves
            {
                if aig.is_and(leaf) && !seen[leaf as usize] {
                    stack.push((leaf, false));
                }
            }
        }
        order
    }
}

/// Runs the mapping passes and returns the chosen cover.
fn run(aig: &Aig, size: usize, options: &CutOptions, area_passes: u32, cost: &dyn Cost) -> Mapping {
    let n = aig.len();
    let mut m = Mapping {
        best: vec![None; n],
        arrival: vec![0.0; n],
        area_flow: vec![0.0; n],
        refs: vec![0; n],
        depth: 0,
    };
    let cuts = PriorityCuts::compute(aig, size, options);

    // Pass 1: depth, tie-broken on area flow.
    assign(aig, &cuts, &mut m, cost, Objective::Depth, None);
    // Every node must keep the arrival time the depth pass gave it.
    let required = m.arrival.clone();

    // Passes 2..: area recovery under the depth achieved.
    for pass in 0..area_passes {
        let objective = if pass == 0 {
            Objective::AreaFlow
        } else {
            Objective::ExactArea
        };
        reference(aig, &mut m);
        assign(aig, &cuts, &mut m, cost, objective, Some(&required));
    }
    reference(aig, &mut m);
    m.depth = depth_of(aig, &m);
    m
}

/// What an assignment pass minimises.
#[derive(Clone, Copy, PartialEq)]
enum Objective {
    /// Arrival time, then area flow.
    Depth,
    /// Area flow, subject to the required times.
    AreaFlow,
    /// Measured area, subject to the required times.
    ExactArea,
}

/// Chooses a cut for every node, in topological order.
fn assign(
    aig: &Aig,
    cuts: &PriorityCuts,
    m: &mut Mapping,
    cost: &dyn Cost,
    objective: Objective,
    required: Option<&[f64]>,
) {
    for id in 0..u32::try_from(aig.len()).expect("node count") {
        if !aig.is_and(id) {
            m.arrival[id as usize] = 0.0;
            m.area_flow[id as usize] = 0.0;
            continue;
        }
        let mut best: Option<(f64, f64, Cut)> = None;
        for cut in cuts.cuts(id) {
            // The trivial cut exists only so parents can use this node as
            // a leaf; implementing a node by itself is circular.
            if cut.is_trivial(id) || !cost.accepts(cut.leaves.len()) {
                continue;
            }
            let Some(area) = cost.area(&cut.function, cut.leaves.len()) else {
                continue;
            };
            let delay = cost.delay(&cut.function, cut.leaves.len());
            let arrival = cut
                .leaves
                .iter()
                .map(|&l| m.arrival[l as usize])
                .fold(0.0, f64::max)
                + delay;
            if let Some(req) = required
                && arrival > req[id as usize] + 1e-9
            {
                continue;
            }
            let flow = area
                + cut
                    .leaves
                    .iter()
                    .map(|&l| {
                        let r = f64::from(m.refs[l as usize].max(1));
                        m.area_flow[l as usize] / r
                    })
                    .sum::<f64>();
            let measured = match objective {
                Objective::ExactArea => exact_area(aig, m, cost, cut),
                _ => flow,
            };
            let key = match objective {
                Objective::Depth => (arrival, flow),
                Objective::AreaFlow => (flow, arrival),
                Objective::ExactArea => (measured, arrival),
            };
            let better = match &best {
                None => true,
                Some((ba, bb, bc)) => {
                    (key.0, key.1, cut.leaves.len()) < (*ba, *bb, bc.leaves.len())
                }
            };
            if better {
                best = Some((key.0, key.1, cut.clone()));
            }
        }
        // Every AND node can at worst be implemented by its own two
        // fanins, which is a single AND that any target provides.
        let cut = match best {
            Some((_, _, cut)) => cut,
            None => cuts::fanin_cut(aig, id),
        };
        let area = cost.area(&cut.function, cut.leaves.len()).unwrap_or(1.0);
        let delay = cost.delay(&cut.function, cut.leaves.len());
        m.arrival[id as usize] = cut
            .leaves
            .iter()
            .map(|&l| m.arrival[l as usize])
            .fold(0.0, f64::max)
            + delay;
        m.area_flow[id as usize] = area
            + cut
                .leaves
                .iter()
                .map(|&l| m.area_flow[l as usize] / f64::from(m.refs[l as usize].max(1)))
                .sum::<f64>();
        m.best[id as usize] = Some(cut);
    }
}

/// What implementing a node with `cut` really costs: the cut's own cell
/// plus, recursively, the cells of every leaf that nothing else uses and
/// that would therefore disappear with it.
///
/// This is a cheap stand-in for ABC's dereference-and-reference
/// measurement, which walks the current cover with reference counts; here
/// the counts from the last [`reference`] pass are read directly.
fn exact_area(aig: &Aig, m: &Mapping, cost: &dyn Cost, cut: &Cut) -> f64 {
    let mut area = cost.area(&cut.function, cut.leaves.len()).unwrap_or(1.0);
    for &leaf in &cut.leaves {
        if !aig.is_and(leaf) || m.refs[leaf as usize] > 1 {
            continue;
        }
        if let Some(inner) = &m.best[leaf as usize] {
            area += exact_area(aig, m, cost, inner);
        }
    }
    area
}

/// Recomputes how many chosen cuts use each node, from the outputs.
fn reference(aig: &Aig, m: &mut Mapping) {
    m.refs.iter_mut().for_each(|r| *r = 0);
    let mut stack: Vec<u32> = Vec::new();
    for &out in aig.outputs() {
        m.refs[out.index()] += 1;
        if m.refs[out.index()] == 1 {
            stack.push(out.node());
        }
    }
    while let Some(id) = stack.pop() {
        let Some(cut) = &m.best[id as usize] else {
            continue;
        };
        for &leaf in cut.leaves.clone().iter() {
            m.refs[leaf as usize] += 1;
            if m.refs[leaf as usize] == 1 {
                stack.push(leaf);
            }
        }
    }
}

/// The depth of the cover in cells.
fn depth_of(aig: &Aig, m: &Mapping) -> u32 {
    let mut level = vec![0u32; aig.len()];
    for id in 0..u32::try_from(aig.len()).expect("node count") {
        if let Some(cut) = &m.best[id as usize]
            && m.refs[id as usize] > 0
        {
            level[id as usize] = cut
                .leaves
                .iter()
                .map(|&l| level[l as usize])
                .max()
                .unwrap_or(0)
                + 1;
        }
    }
    aig.outputs()
        .iter()
        .map(|e| level[e.index()])
        .max()
        .unwrap_or(0)
}

/// Extracts the combinational logic of `module`, optimises it, maps it and
/// writes the result back.
///
/// Flip-flops, latches, memory ports, instances, black boxes and processes
/// stay untouched; the nets they drive become the mapped logic's inputs
/// and the nets they read its outputs. Internal nets that the mapping made
/// redundant are removed; module ports keep their nets.
pub fn map_module(module: &mut Module, options: &MapOptions<'_>) -> MapStats {
    let (mut aig, mapping) = super::aig::from_module(module);
    let before = aig.stats();
    super::aig::optimize(&mut aig, &options.aig_opt);
    let after = aig.stats();
    let mut stats = MapStats {
        before,
        after,
        ..MapStats::default()
    };
    match options.target {
        Target::Lut(k) => {
            let network = lut_map_with(&aig, k, &options.cuts, options.area_passes);
            stats.cells = network.len();
            stats.depth = network.depth;
            stats.area = network.len() as f64;
            network.to_module(&mapping, module);
        }
        Target::Gates(library) => {
            let network = gate_map_with(&aig, library, &options.cuts, options.area_passes);
            stats.cells = network.len();
            stats.depth = network.depth;
            stats.area = network.area;
            network.to_module(&mapping, module);
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::super::aig::Edge;
    use super::*;
    use crate::ir::Type;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::validate::validate_module;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// An AIG computing a few functions of four inputs.
    fn sample() -> Aig {
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..4).map(|_| g.add_input()).collect();
        let x = g.xor(ins[0], ins[1]);
        let y = g.xor(ins[2], ins[3]);
        let z = g.xor(x, y);
        let maj = {
            let a = g.and(ins[0], ins[1]);
            let b = g.and(ins[1], ins[2]);
            let c = g.and(ins[0], ins[2]);
            let t = g.or(a, b);
            g.or(t, c)
        };
        g.add_output(z);
        g.add_output(maj);
        g
    }

    #[test]
    fn lut_mapping_is_correct_and_bounded() {
        let aig = sample();
        for k in 2..=8u32 {
            let net = lut_map(&aig, k);
            assert!(!net.is_empty());
            assert!(
                net.luts.iter().all(|l| l.inputs.len() <= k as usize),
                "k = {k}"
            );
            for lut in &net.luts {
                assert_eq!(lut.init.width(), 1 << lut.inputs.len());
            }
            let hist = net.histogram();
            assert_eq!(hist.iter().sum::<usize>(), net.len());
            // Simulating the mapped network reproduces the AIG.
            for pat in 0..16u32 {
                let ins: Vec<bool> = (0..4).map(|i| (pat >> i) & 1 == 1).collect();
                assert_eq!(simulate_luts(&net, &ins), aig.eval(&ins), "k {k} pat {pat}");
            }
        }
        // The two outputs are a 4-input xor and a 3-input majority, so
        // one 4-LUT each, one level deep.
        assert_eq!(
            lut_map(&aig, 4).len(),
            2,
            "{:?}",
            lut_map(&aig, 4).histogram()
        );
        assert_eq!(lut_map(&aig, 4).depth, 1);
        // Two-input LUTs beat the AIG node count, because one of them
        // does an XOR that costs three AND nodes.
        let two = lut_map(&aig, 2);
        assert!(two.len() < aig.num_ands(), "{} luts", two.len());
        assert!(two.len() > lut_map(&aig, 4).len());
    }

    /// Evaluates a LUT network for one input assignment.
    fn simulate_luts(net: &LutNetwork, inputs: &[bool]) -> Vec<bool> {
        let mut values: Vec<bool> = Vec::with_capacity(net.luts.len());
        let value = |s: Signal, values: &[bool]| match s {
            Signal::Const(c) => c,
            Signal::Input(i) => inputs[i as usize],
            Signal::Node(n) => values[n as usize],
        };
        for lut in &net.luts {
            let mut pattern = 0usize;
            for (i, &s) in lut.inputs.iter().enumerate() {
                if value(s, &values) {
                    pattern |= 1 << i;
                }
            }
            values.push(lut.init.bit(u32::try_from(pattern).expect("pattern")) == Bit::One);
        }
        net.outputs.iter().map(|&s| value(s, &values)).collect()
    }

    /// Evaluates a gate network for one input assignment.
    fn simulate_gates(net: &GateNetwork, lib: &GateLibrary, inputs: &[bool]) -> Vec<bool> {
        let mut values: Vec<bool> = Vec::with_capacity(net.gates.len());
        let value = |s: Signal, values: &[bool]| match s {
            Signal::Const(c) => c,
            Signal::Input(i) => inputs[i as usize],
            Signal::Node(n) => values[n as usize],
        };
        for g in &net.gates {
            let gate = lib.gate(&g.gate).expect("gate in library");
            let mut pattern = 0usize;
            for (pin, &s) in g.fanins.iter().enumerate() {
                if value(s, &values) {
                    pattern |= 1 << pin;
                }
            }
            values.push(gate.function.bit(pattern));
        }
        net.outputs.iter().map(|&s| value(s, &values)).collect()
    }

    #[test]
    fn gate_mapping_is_correct() {
        let lib = GateLibrary::generic();
        let aig = sample();
        let net = gate_map(&aig, &lib);
        assert!(!net.is_empty());
        assert!(net.area > 0.0);
        assert!(net.depth > 0);
        let hist = net.histogram();
        assert_eq!(hist.iter().map(|(_, n)| n).sum::<usize>(), net.len());
        assert!(hist.windows(2).all(|w| w[0].0 <= w[1].0), "sorted");
        for pat in 0..16u32 {
            let ins: Vec<bool> = (0..4).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(
                simulate_gates(&net, &lib, &ins),
                aig.eval(&ins),
                "pattern {pat}"
            );
        }
        // XOR-heavy logic should find the XOR2 / XNOR2 cells rather than
        // spelling the xor out in NANDs.
        assert!(
            net.gates
                .iter()
                .any(|g| g.gate == "XOR2" || g.gate == "XNOR2"),
            "{:?}",
            net.histogram()
        );
    }

    #[test]
    fn maps_a_module_keeping_its_boundary() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let (an, cn) = (b.net(a), b.net(c));
        let sum = b.add(an, cn);
        b.assign(y, sum);
        let mut module = b.finish();
        let stats = map_module(&mut module, &MapOptions::lut(4));
        assert!(stats.cells > 0);
        assert_eq!(stats.before.inputs, 8);
        assert_eq!(stats.before.outputs, 4);
        assert!(stats.after.nodes <= stats.before.nodes);
        assert!(validate_module(&module).is_empty());
        assert_eq!(module.ports.len(), 3);
        assert!(
            module
                .cells
                .values()
                .all(|c| matches!(c.kind, CellKind::Lut { .. }))
        );
        // The ports keep their nets.
        assert!(module.net_by_name("a").is_some());
        assert!(module.net_by_name("y").is_some());
    }
}
