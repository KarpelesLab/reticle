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
//! # Depth as Vivado's synthesis reaches it
//!
//! Options each off by default, and all on for the 7 series under
//! `reticle fpga`'s default `--synthesis vivado`, close what was measured
//! between this mapper's covers and Vivado 2026.1's (docs/fpga-vivado-parity.md,
//! "Synthesis against Vivado's", has the figures; `--synthesis legacy` is the
//! cover before, byte for byte):
//!
//! - [`CutOptions::depth_priority`]: half of each node's cut list goes to its
//!   shallowest cuts before the size quotas take the rest. The quotas alone
//!   keep large cuts from being crowded out, which is the area half of the
//!   priority, and say nothing about depth: a 32-input parity rebuilt as six
//!   tables of six under one came out three deep, the cut that makes it two
//!   never kept.
//! - [`MapOptions::xor_balance`]: the graph's XOR logic collapsed into
//!   parities of its leaves and rebuilt for `k`-input tables before mapping
//!   ([`super::aig::xor`]), which the AND balancing of the optimiser cannot do:
//!   `crc32` of the timing corpus went from seven tables deep to two. The
//!   mapping is then proved against the graph from before the rebuilding, so
//!   the proof covers both; the parities' proof is by linear algebra
//!   (`verify::merge_linear`), which a SAT solver under a conflict limit did
//!   not finish.
//! - [`MapOptions::late_hard_outputs`]: an input a hard block drives arrives a
//!   cell late, in the depth pass and in the XOR rebuilding.
//! - [`CutOptions::duplicate_complements`]: a node read in both polarities is
//!   two cells over its leaves rather than one and an inverter.
//!
//! And one for area, [`MapOptions::case_trees`]: a `pmux` that is a `case` is
//! blasted as a tree of multiplexers over what it compares, as Vivado's
//! synthesis builds one, rather than as each arm under its decoded select.
//! The cuts over a decoder share nothing between arms; over the tree, each
//! table is a subtree. `fsm` of the timing corpus, a 24-state machine, went
//! from 72 tables to 38 (Vivado's 33), `regfile_alu` from 503 to 469 and a
//! table shallower. The tree is proved against the `pmux` as blasted
//! without it ([`super::aig::BlastOptions::case_trees`]).
//!
//! Tried and taken out again: area recovery against required times
//! propagated back from the deepest output through the cover, instead of
//! each node held to its own arrival from the depth pass, the conservative
//! stand-in above. It saved a few tables in a thousand and cost a level
//! before a carry chain, which looks to it like any other output.
//!
//! # Wide multiplexers
//!
//! A 7-series slice has two more multiplexers behind its lookup tables, with
//! no bits of their own and no routing: `MUXF7` picks between two tables and
//! `MUXF8` between two `MUXF7`s. So a two-to-one choice between tables can
//! cost no table at all, which a cut cannot say: a cut is one cell over its
//! leaves, and this one's area is nothing. [`MapOptions::wide_muxes`] offers
//! it as one more way to implement a node, alongside its cuts and priced as
//! they are, so area flow takes it where it saves something and leaves it
//! where one six-input table covers the whole choice.
//!
//! What may be chosen depends on what the data inputs were: two tables make a
//! `MUXF7`, two `MUXF7`s a `MUXF8`, and anything else neither, so that is read
//! off each pass's choice for the fanins, which come first. Only the finished
//! cover knows whether a table's one dedicated wire is asked for twice
//! ([`Lut::wide`]'s rule), so that is settled afterwards, by making the
//! second reader an ordinary three-input table.
//!
//! **Such a cell is a [`Lut`]**: three inputs, `init` the multiplexer's truth
//! table, and a tag. Everything that reads a network, the proof of
//! [`verify`] first of all, reads a correct lookup table without knowing, so
//! the proof covers this mapping with no new case in it. The tag says only
//! which hardware `fpga::techcells` writes, and it rewrites a tagged cell
//! only when its `init` is that truth table.
//!
//! Vivado's own covers were the check on what to aim at, and the fixtures in
//! `testdata/fpga/vivado/muxf/` hold them: its 8-, 16- and 64-to-1 come to 2, 4
//! and 17 tables with 1, 2 and 8 `MUXF7` and 0, 1 and 4 `MUXF8`.
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
use super::aig::muxtree::{MuxShape, optimize_around_trees};
use super::aig::truth::TruthTable;
use super::aig::{Aig, AigOptions, AigStats, Edge};
use super::cells::{Gate, GateLibrary};
use crate::ir::attr::{AttrValue, Attrs};
use crate::ir::design::Module;
use crate::ir::types::Const;
use crate::ir::{CellKind, Name};
use crate::logic::Bit;

mod cuts;
pub mod verify;

pub use cuts::{Cut, CutOptions, PriorityCuts};
pub use verify::{
    MapEquivalence, MapVerifyOptions, check_gate_mapping, check_lut_mapping,
    check_lut_mapping_through,
};

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
    /// Offer a slice's `MUXF7` and `MUXF8` as well, for [`Target::Lut`]. The
    /// network is mapped both ways and this one kept only when it has fewer
    /// tables, so a design with nothing to gain maps exactly as it does
    /// without this. See the module docs.
    pub wide_muxes: bool,
    /// Balance the graph's XOR trees by level before mapping
    /// ([`super::aig::xor`]), which the AND balancing of the optimiser cannot
    /// see through. The mapping is then proved against the graph from before
    /// it. Off by default.
    pub xor_balance: bool,
    /// Take an input of the logic that a hard block drives — a carry chain,
    /// a multiplier, a memory, any cell the AIG does not absorb that is not
    /// a register — to arrive one cell late, in the depth pass and in the XOR
    /// rebuilding. Without it every input arrives at zero, and a carry
    /// chain's sum, which is late, was taken into a parity's first table as
    /// readily as a flip-flop's output: the Husky Plus analyser's FIFO read
    /// clock lost 0.95 ns to that. Off by default.
    pub late_hard_outputs: bool,
    /// Blast a `pmux` that is a `case` as a tree of multiplexers over what it
    /// compares ([`super::aig::BlastOptions::case_trees`]), the way Vivado's
    /// synthesis builds one. The tree is proved against the `pmux` as it was
    /// blasted without, beside the mapping's own proof, whenever the mapping
    /// is proved. Off by default.
    pub case_trees: bool,
}

impl Default for MapOptions<'_> {
    fn default() -> Self {
        MapOptions {
            target: Target::Lut(4),
            aig_opt: AigOptions::default(),
            area_passes: 2,
            cuts: CutOptions::default(),
            wide_muxes: false,
            xor_balance: false,
            late_hard_outputs: false,
            case_trees: false,
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
    /// Of a LUT mapping with [`MapOptions::wide_muxes`], the `MUXF7`s in it;
    /// they are not in [`MapStats::cells`].
    pub muxf7: usize,
    /// The `MUXF8`s, likewise.
    pub muxf8: usize,
    /// How many tables the network without them would have had over this
    /// one, or zero when it was the one kept.
    pub wide_saved: usize,
}

/// One mapped LUT.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    /// Its inputs, input 0 first.
    pub inputs: Vec<Signal>,
    /// The function, `2^inputs.len()` bits, bit `i` for pattern `i`.
    pub init: Const,
    /// When this is a slice's wide multiplexer rather than a table: then its
    /// inputs are `[I0, I1, S]` and `init` is [`WIDE_MUX_INIT`], so that it is
    /// still a correct lookup table to anything that reads it as one.
    ///
    /// The data inputs are the rule: each is a cell's own dedicated wire, so
    /// a `MUXF7`'s are two different tables and a `MUXF8`'s two different
    /// `MUXF7`s, and none of those is the data of a second multiplexer. Any
    /// of them may be read by logic elsewhere too, as Vivado's `f8_out` is.
    pub wide: Option<WideMux>,
}

/// Which of a slice's wide multiplexers a [`Lut`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WideMux {
    /// Between two lookup tables.
    F7,
    /// Between two [`WideMux::F7`]s.
    F8,
}

impl WideMux {
    /// What it adds to a path, in lookup table levels: read off Vivado 2026.1's
    /// routed reports of `testdata/fpga/vivado/muxf/`, where a `MUXF7` is 0.21 to
    /// 0.25 ns and a `MUXF8` 0.10 ns, against a table and the wire to it near
    /// 1 ns. It only has to be right about being much less than a level.
    fn delay(self) -> f64 {
        match self {
            WideMux::F7 => 0.25,
            WideMux::F8 => 0.1,
        }
    }

    /// The value of [`WIDE_MUX_ATTR`] for it.
    #[must_use]
    pub fn attr(self) -> &'static str {
        match self {
            WideMux::F7 => "F7",
            WideMux::F8 => "F8",
        }
    }
}

/// The truth table of `S ? I1 : I0` over inputs `[I0, I1, S]`.
pub const WIDE_MUX_INIT: u64 = 0xCA;

/// The attribute that marks a mapped three-input `lut` cell as a wide
/// multiplexer, with [`WideMux::attr`]'s value.
pub const WIDE_MUX_ATTR: &str = "wide_mux";

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
    /// Number of LUTs, with the wide multiplexers among them.
    pub fn len(&self) -> usize {
        self.luts.len()
    }

    /// How many of them are real lookup tables.
    pub fn tables(&self) -> usize {
        self.luts.iter().filter(|l| l.wide.is_none()).count()
    }

    /// How many of them are this wide multiplexer.
    pub fn wide(&self, level: WideMux) -> usize {
        self.luts.iter().filter(|l| l.wide == Some(level)).count()
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
                    attrs: {
                        let mut attrs = Attrs::new();
                        if let Some(level) = lut.wide {
                            attrs.set(WIDE_MUX_ATTR, AttrValue::String(level.attr().to_owned()));
                        }
                        attrs
                    },
                    prefix: if lut.wide.is_some() { "muxf" } else { "lut" },
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
    lut_map_arriving(aig, k, options, area_passes, false, None)
}

/// [`lut_map_with`] with a slice's `MUXF7` and `MUXF8` on offer: see the
/// module docs.
pub fn lut_map_wide(aig: &Aig, k: u32, options: &CutOptions, area_passes: u32) -> LutNetwork {
    lut_map_arriving(aig, k, options, area_passes, true, None)
}

/// [`lut_map_with`], or [`lut_map_wide`] when `wide`, with each node of
/// `aig` that is an input arriving at the depth `arrival` gives it, in cells
/// (indexed by node; `None` is zero for all).
///
/// # Panics
///
/// If `k` is outside `2..=8`.
pub fn lut_map_arriving(
    aig: &Aig,
    k: u32,
    options: &CutOptions,
    area_passes: u32,
    wide: bool,
    arrival: Option<&[u32]>,
) -> LutNetwork {
    assert!((2..=8).contains(&k), "LUT size {k} is outside 2..=8");
    let size = usize::try_from(k).expect("LUT size");
    let cost = LutCost;
    let mapping = run(aig, size, options, area_passes, &cost, wide, arrival);
    let mut emitter = LutEmitter { luts: Vec::new() };
    let outputs = mapping.emit(aig, &mut emitter);
    let mut luts = emitter.luts;
    if wide {
        settle_wide(&mut luts);
    }
    LutNetwork {
        luts,
        outputs,
        depth: mapping.depth,
    }
}

/// Makes an ordinary table of each wide multiplexer that breaks [`Lut::wide`]'s
/// rule: a second reader of one table's dedicated wire, or a `MUXF8` whose data
/// is no longer two `MUXF7`s. It is the same function either way, so nothing
/// else changes; it is one table more. The cells are in topological order, so
/// a `MUXF7` undone here is seen by the `MUXF8` above it.
fn settle_wide(luts: &mut [Lut]) {
    let mut claimed = vec![false; luts.len()];
    for index in 0..luts.len() {
        let Some(level) = luts[index].wide else {
            continue;
        };
        let data = match luts[index].inputs.as_slice() {
            [Signal::Node(a), Signal::Node(b), _] if a != b => Some((*a as usize, *b as usize)),
            _ => None,
        };
        let wanted = match level {
            WideMux::F7 => None,
            WideMux::F8 => Some(WideMux::F7),
        };
        match data {
            Some((a, b))
                if !claimed[a]
                    && !claimed[b]
                    && luts[a].wide == wanted
                    && luts[b].wide == wanted =>
            {
                claimed[a] = true;
                claimed[b] = true;
            }
            _ => luts[index].wide = None,
        }
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
            wide: None,
        }))
    }

    fn inverter(&mut self, input: Signal) -> u32 {
        // A one-input LUT computing `!a`: output 1 for pattern 0.
        self.push(Lut {
            inputs: vec![input],
            init: Const::from_u64(0b01, 2),
            wide: None,
        })
    }

    fn wide(&mut self, level: WideMux, i0: Signal, i1: Signal, s: Signal) -> u32 {
        self.push(Lut {
            inputs: vec![i0, i1, s],
            init: Const::from_u64(WIDE_MUX_INIT, 8),
            wide: Some(level),
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
    let mapping = run(aig, size, options, area_passes, &cost, false, None);
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
        let out = self.push(&gate, fanins);
        if m.output_invert {
            return Some(self.inverter(Signal::Node(out)));
        }
        Some(out)
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

    /// Creates a wide multiplexer: only a target that offers them is ever
    /// asked, which is [`LutEmitter`].
    fn wide(&mut self, level: WideMux, i0: Signal, i1: Signal, s: Signal) -> u32 {
        let _ = (level, i0, i1, s);
        unreachable!("this target offers no wide multiplexer")
    }
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
                let inv_delay = self.library.inverter().map_or(0.0, Gate::max_delay);
                // An inverted input sits in front of the gate and an
                // inverted output after it, so they add rather than
                // overlap.
                let front = if m.invert != 0 { inv_delay } else { 0.0 };
                let back = if m.output_invert { inv_delay } else { 0.0 };
                gate.max_delay() + front + back
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
    /// Depth of the mapped network, in cells. A wide multiplexer is not a
    /// level of it.
    depth: u32,
    /// Per node, the wide multiplexer that implements it, in which case its
    /// [`Mapping::best`] holds that multiplexer's three leaves and cost no
    /// table; empty unless they are on offer.
    wide: Vec<Option<(WideMux, MuxShape)>>,
    /// [`CutOptions::duplicate_complements`].
    duplicate_complements: bool,
}

/// The cut a wide multiplexer reads, the same three nodes [`Mapping::best`]
/// holds, with its function over them.
fn shape_cut(shape: MuxShape) -> Cut {
    let mut leaves = vec![shape.select, shape.when0.node(), shape.when1.node()];
    leaves.sort_unstable();
    let at = |node: u32| leaves.iter().position(|l| *l == node).unwrap_or(0);
    let (s, d0, d1) = (
        at(shape.select),
        at(shape.when0.node()),
        at(shape.when1.node()),
    );
    let mut word = 0u64;
    for pattern in 0..8u64 {
        let bit = |v: usize| (pattern >> v) & 1 == 1;
        let value = if bit(s) {
            bit(d1) != shape.when1.is_complement()
        } else {
            bit(d0) != shape.when0.is_complement()
        };
        if value {
            word |= 1 << pattern;
        }
    }
    Cut {
        leaves,
        function: TruthTable::from_u64(3, word),
    }
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

        // Which polarity of each node is actually used. Fanouts first,
        // because a wide multiplexer has no table to absorb its data's
        // polarity in and asks for what its own reader wants: its complement
        // is the multiplexer of its data's complements.
        let mut needs_plain = vec![false; aig.len()];
        let mut needs_comp = vec![false; aig.len()];
        for &e in aig.outputs() {
            if e.is_complement() {
                needs_comp[e.index()] = true;
            } else {
                needs_plain[e.index()] = true;
            }
        }
        for &id in order.iter().rev() {
            if let Some((_, shape)) = self.wide_at(id) {
                let flip = !needs_plain[id as usize];
                needs_plain[shape.select as usize] = true;
                for data in [shape.when0.xor(flip), shape.when1.xor(flip)] {
                    if data.is_complement() {
                        needs_comp[data.index()] = true;
                    } else {
                        needs_plain[data.index()] = true;
                    }
                }
                continue;
            }
            // A cut's leaves are always read in plain polarity.
            for &leaf in &self.best[id as usize].as_ref().expect("covered").leaves {
                needs_plain[leaf as usize] = true;
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
            if let Some((level, shape)) = self.wide_at(id) {
                let read = |edge: Edge| {
                    let slot = if edge.is_complement() {
                        &complemented
                    } else {
                        &plain
                    };
                    slot[edge.index()].expect("data emitted in the polarity asked for")
                };
                let flip = !needs_plain[id as usize];
                let index = emitter.wide(
                    level,
                    read(shape.when0.xor(flip)),
                    read(shape.when1.xor(flip)),
                    plain[shape.select as usize].expect("select emitted"),
                );
                if flip {
                    complemented[id as usize] = Some(Signal::Node(index));
                } else {
                    plain[id as usize] = Some(Signal::Node(index));
                    if needs_comp[id as usize] {
                        let inverse = emitter.inverter(Signal::Node(index));
                        complemented[id as usize] = Some(Signal::Node(inverse));
                    }
                }
                continue;
            }
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
            // Both: the complement a cell of its own over the same leaves,
            // where the target has one for it, is a level less than an
            // inverter after the plain one.
            if self.duplicate_complements
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

    /// The wide multiplexer that implements `id`, if one does.
    fn wide_at(&self, id: u32) -> Option<(WideMux, MuxShape)> {
        self.wide.get(id as usize).copied().flatten()
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
fn run(
    aig: &Aig,
    size: usize,
    options: &CutOptions,
    area_passes: u32,
    cost: &dyn Cost,
    wide: bool,
    arrival: Option<&[u32]>,
) -> Mapping {
    let n = aig.len();
    let inputs: Vec<f64> = (0..n)
        .map(|i| {
            arrival
                .and_then(|a| a.get(i))
                .map_or(0.0, |&d| f64::from(d))
        })
        .collect();
    let shapes: Vec<Option<MuxShape>> = if wide {
        (0..u32::try_from(n).expect("node count"))
            .map(|id| {
                if !aig.is_and(id) {
                    return None;
                }
                // Each data input must be some cell's output to be a
                // dedicated wire: what an AIG input reaches, a flip-flop's
                // say, would take a table as a route-through, and that is
                // no saving over the three-input table this replaces.
                MuxShape::of(aig, id)
                    .filter(|m| aig.is_and(m.when0.node()) && aig.is_and(m.when1.node()))
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut m = Mapping {
        best: vec![None; n],
        arrival: vec![0.0; n],
        area_flow: vec![0.0; n],
        refs: vec![0; n],
        depth: 0,
        wide: if wide { vec![None; n] } else { Vec::new() },
        duplicate_complements: options.duplicate_complements,
    };
    let cuts = PriorityCuts::compute_from(aig, size, options, arrival);

    // Pass 1: depth, tie-broken on area flow.
    assign(
        aig,
        &cuts,
        &mut m,
        cost,
        Objective::Depth,
        None,
        &shapes,
        &inputs,
    );
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
        assign(
            aig,
            &cuts,
            &mut m,
            cost,
            objective,
            Some(&required),
            &shapes,
            &inputs,
        );
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

/// Chooses a cut for every node, in topological order; an input arrives at
/// its figure in `inputs`.
#[allow(clippy::too_many_arguments)]
fn assign(
    aig: &Aig,
    cuts: &PriorityCuts,
    m: &mut Mapping,
    cost: &dyn Cost,
    objective: Objective,
    required: Option<&[f64]>,
    shapes: &[Option<MuxShape>],
    inputs: &[f64],
) {
    for id in 0..u32::try_from(aig.len()).expect("node count") {
        if !aig.is_and(id) {
            m.arrival[id as usize] = inputs.get(id as usize).copied().unwrap_or(0.0);
            m.area_flow[id as usize] = 0.0;
            continue;
        }
        let mut best: Option<(f64, f64, Cut)> = None;
        let mut wide_best: Option<(WideMux, MuxShape)> = None;
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
                Objective::ExactArea => exact_area(aig, m, cost, cut, false),
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
        // The wide multiplexer this node could be, given what its data
        // inputs were just made in this pass: it costs no table, so its area
        // flow is its leaves' alone.
        if let Some(shape) = shapes.get(id as usize).copied().flatten() {
            let level = match (
                m.wide[shape.when0.index()].map(|(l, _)| l),
                m.wide[shape.when1.index()].map(|(l, _)| l),
            ) {
                (None, None) => Some(WideMux::F7),
                (Some(WideMux::F7), Some(WideMux::F7)) => Some(WideMux::F8),
                _ => None,
            };
            if let Some(level) = level {
                let cut = shape_cut(shape);
                let arrival = cut
                    .leaves
                    .iter()
                    .map(|&l| m.arrival[l as usize])
                    .fold(0.0, f64::max)
                    + level.delay();
                let in_time = required.is_none_or(|req| arrival <= req[id as usize] + 1e-9);
                if in_time {
                    let flow: f64 = cut
                        .leaves
                        .iter()
                        .map(|&l| m.area_flow[l as usize] / f64::from(m.refs[l as usize].max(1)))
                        .sum();
                    let key = match objective {
                        Objective::Depth => (arrival, flow),
                        Objective::AreaFlow => (flow, arrival),
                        Objective::ExactArea => (exact_area(aig, m, cost, &cut, true), arrival),
                    };
                    let better = match &best {
                        None => true,
                        Some((ba, bb, bc)) => {
                            (key.0, key.1, cut.leaves.len()) < (*ba, *bb, bc.leaves.len())
                        }
                    };
                    if better {
                        best = Some((key.0, key.1, cut));
                        wide_best = Some((level, shape));
                    }
                }
            }
        }
        // Every AND node can at worst be implemented by its own two
        // fanins, which is a single AND that any target provides.
        let cut = match best {
            Some((_, _, cut)) => cut,
            None => cuts::fanin_cut(aig, id),
        };
        let (area, delay) = match wide_best {
            Some((level, _)) => (0.0, level.delay()),
            None => (
                cost.area(&cut.function, cut.leaves.len()).unwrap_or(1.0),
                cost.delay(&cut.function, cut.leaves.len()),
            ),
        };
        if let Some(slot) = m.wide.get_mut(id as usize) {
            *slot = wide_best;
        }
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
///
/// `wide` says the cut is a wide multiplexer, which is no cell of area.
fn exact_area(aig: &Aig, m: &Mapping, cost: &dyn Cost, cut: &Cut, wide: bool) -> f64 {
    let mut area = if wide {
        0.0
    } else {
        cost.area(&cut.function, cut.leaves.len()).unwrap_or(1.0)
    };
    for &leaf in &cut.leaves {
        if !aig.is_and(leaf) || m.refs[leaf as usize] > 1 {
            continue;
        }
        if let Some(inner) = &m.best[leaf as usize] {
            area += exact_area(aig, m, cost, inner, m.wide_at(leaf).is_some());
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
            let own = u32::from(m.wide_at(id).is_none());
            level[id as usize] = cut
                .leaves
                .iter()
                .map(|&l| level[l as usize])
                .max()
                .unwrap_or(0)
                + own;
        }
    }
    aig.outputs()
        .iter()
        .map(|e| level[e.index()])
        .max()
        .unwrap_or(0)
}

/// The network with wide multiplexers and the graph it came from, if it has
/// fewer tables than `plain`, which is what makes a design with nothing to gain
/// the same as if none were on offer. `None` for the graph is `optimized`.
fn wide_network(
    unoptimized: &Aig,
    optimized: &Aig,
    k: u32,
    options: &MapOptions<'_>,
    plain: &LutNetwork,
    late: Option<&[u32]>,
) -> Option<(LutNetwork, Option<Aig>)> {
    let mut trees = optimize_around_trees(unoptimized, &options.aig_opt);
    if options.xor_balance
        && let Some(trees) = trees.as_mut()
    {
        let arrival = late.map(|l| node_arrivals(trees, l));
        balance_xor_for(trees, k, &options.cuts, arrival.as_deref());
    }
    let from = trees.as_ref().unwrap_or(optimized);
    let arrival = late.map(|l| node_arrivals(from, l));
    let wide = lut_map_arriving(
        from,
        k,
        &options.cuts,
        options.area_passes,
        true,
        arrival.as_deref(),
    );
    (wide.tables() < plain.tables()).then_some((wide, trees))
}

/// [`super::aig::xor::balance_xor`] for `k`-input cells, each node's depth
/// estimated by the cuts `options` enumerates, its inputs arriving at
/// `arrival`.
fn balance_xor_for(aig: &mut Aig, k: u32, options: &CutOptions, arrival: Option<&[u32]>) {
    let k = usize::try_from(k).expect("LUT size");
    let cuts = PriorityCuts::compute_from(aig, k, options, arrival);
    let depths = cuts.depths().to_vec();
    super::aig::xor::balance_xor(aig, k, &depths);
}

/// Each node's arrival in `aig`: an input's from `late`, by its position
/// among the inputs, every other node's zero.
fn node_arrivals(aig: &Aig, late: &[u32]) -> Vec<u32> {
    let mut out = vec![0; aig.len()];
    for (pos, &id) in aig.inputs().iter().enumerate() {
        out[id as usize] = late.get(pos).copied().unwrap_or(0);
    }
    out
}

/// One cell late for each AIG input a hard block drives: a cell the AIG did
/// not absorb that is a black box, which by the time the 7 series maps its
/// logic is a carry chain, a multiplier or a memory. A register, a port and
/// anything else arrive at zero. By the AIG's input positions.
fn hard_block_arrivals(module: &Module, mapping: &super::aig::blast::Mapping) -> Vec<u32> {
    let mut hard: std::collections::HashSet<crate::ir::design::NetId> =
        std::collections::HashSet::new();
    for (_, cell) in module.cells.iter() {
        if matches!(cell.kind, CellKind::Blackbox(_)) {
            for (_, net) in &cell.outputs {
                hard.insert(*net);
            }
        }
    }
    mapping
        .inputs
        .iter()
        .map(|bit| u32::from(hard.contains(&bit.net)))
        .collect()
}

/// Extracts the combinational logic of `module`, optimises it, maps it and
/// writes the result back.
///
/// Flip-flops, latches, memory ports, instances, black boxes and processes
/// stay untouched; the nets they drive become the mapped logic's inputs
/// and the nets they read its outputs. Internal nets that the mapping made
/// redundant are removed; module ports keep their nets.
pub fn map_module(module: &mut Module, options: &MapOptions<'_>) -> MapStats {
    map_module_checked(module, options, None).0
}

/// [`map_module`] with a proof that the mapping preserved the logic.
///
/// The check is [`verify::check_lut_mapping`] or
/// [`verify::check_gate_mapping`] on the network against the AIG it was
/// mapped from, which is the one boundary `reticle synth --verify` does not
/// cover — see [`verify`] for why that mattered. `None` skips it and returns
/// `None`.
pub fn map_module_checked(
    module: &mut Module,
    options: &MapOptions<'_>,
    verify: Option<&MapVerifyOptions>,
) -> (MapStats, Option<MapEquivalence>) {
    let (mut aig, mapping, plain) = blast_for(module, options, verify.is_some());
    // The case trees as blasted, which `plain` is proved against.
    let blasted = plain.as_ref().map(|_| aig.clone());
    let late = options
        .late_hard_outputs
        .then(|| hard_block_arrivals(module, &mapping));
    let before = aig.stats();
    // What the wide network is mapped from has its multiplexer trees as they
    // were written, and is proved against this.
    let unoptimized = match options.target {
        Target::Lut(_) if options.wide_muxes => Some(aig.clone()),
        _ => None,
    };
    super::aig::optimize(&mut aig, &options.aig_opt);
    // The graph before XOR balancing, which the mapping is proved against.
    let unbalanced = if options.xor_balance {
        let before = verify.map(|_| aig.clone());
        let k = match options.target {
            Target::Lut(k) => k,
            Target::Gates(_) => 2,
        };
        let arrival = late.as_deref().map(|l| node_arrivals(&aig, l));
        balance_xor_for(&mut aig, k, &options.cuts, arrival.as_deref());
        before
    } else {
        None
    };
    let after = aig.stats();
    let mut stats = MapStats {
        before,
        after,
        ..MapStats::default()
    };
    let equivalence;
    match options.target {
        Target::Lut(k) => {
            let arrival = late.as_deref().map(|l| node_arrivals(&aig, l));
            let mut network = lut_map_arriving(
                &aig,
                k,
                &options.cuts,
                options.area_passes,
                false,
                arrival.as_deref(),
            );
            let mut kept_trees = None;
            if let Some(unoptimized) = &unoptimized
                && let Some((wide, trees)) =
                    wide_network(unoptimized, &aig, k, options, &network, late.as_deref())
            {
                stats.wide_saved = network.tables() - wide.tables();
                network = wide;
                kept_trees = Some(trees);
            }
            stats.cells = network.tables();
            stats.muxf7 = network.wide(WideMux::F7);
            stats.muxf8 = network.wide(WideMux::F8);
            stats.depth = network.depth;
            stats.area = network.len() as f64;
            equivalence = verify.map(|v| match (&unoptimized, &kept_trees) {
                // What took the trees out and put them back is proved as
                // well as the mapping, against the graph from before it.
                (Some(unoptimized), Some(Some(trees))) => {
                    check_lut_mapping_through(unoptimized, trees, &network, v)
                }
                _ => match &unbalanced {
                    Some(unbalanced) => check_lut_mapping_through(unbalanced, &aig, &network, v),
                    None => check_lut_mapping(&aig, &network, v),
                },
            });
            network.to_module(&mapping, module);
        }
        Target::Gates(library) => {
            let network = gate_map_with(&aig, library, &options.cuts, options.area_passes);
            stats.cells = network.len();
            stats.depth = network.depth;
            stats.area = network.area;
            equivalence = verify.map(|v| {
                let against = check_gate_mapping(&aig, &network, library, v);
                match &unbalanced {
                    // The balancing proved on its own, as `_through` does.
                    Some(unbalanced) if !against.wrong() => {
                        match verify::check_graphs(unbalanced, &aig, v) {
                            wrong @ MapEquivalence::Different { .. } => wrong,
                            other if against.proved() => other,
                            _ => against,
                        }
                    }
                    _ => against,
                }
            });
            network.to_module(&mapping, module);
        }
    }
    // The case trees, proved against the `pmux` as it is blasted without.
    let equivalence = match (equivalence, &plain, &blasted) {
        (Some(verdict), Some(plain), Some(blasted)) if !verdict.wrong() => {
            let v = verify.expect("a verdict means the mapping was proved");
            Some(match verify::check_graphs(plain, blasted, v) {
                wrong @ MapEquivalence::Different { .. } => wrong,
                MapEquivalence::Equivalent => verdict,
                undecided => {
                    if verdict.proved() {
                        undecided
                    } else {
                        verdict
                    }
                }
            })
        }
        (verdict, _, _) => verdict,
    };
    (stats, equivalence)
}

/// The graph [`map_module_checked`] maps, its mapping, and — when it was
/// blasted with [`MapOptions::case_trees`] and `proving` — the graph blasted
/// without, which it is proved against. Without case trees, or with nothing to
/// make one of, it is [`super::aig::from_module`]'s graph and nothing else.
fn blast_for(
    module: &Module,
    options: &MapOptions<'_>,
    proving: bool,
) -> (Aig, super::aig::Mapping, Option<Aig>) {
    let plain = super::aig::from_module(module);
    if !options.case_trees {
        return (plain.0, plain.1, None);
    }
    let blast = super::aig::BlastOptions { case_trees: true };
    let (trees, mapping) = super::aig::from_module_with(module, &blast);
    // The same inputs and outputs, or the proof would not be of the same
    // thing: the trees are only taken when that is so, which it always
    // should be.
    if mapping.inputs != plain.1.inputs || mapping.outputs != plain.1.outputs {
        return (plain.0, plain.1, None);
    }
    (trees, mapping, proving.then_some(plain.0))
}

#[cfg(test)]
mod case_tree_tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{Name, Type};
    use crate::source::{SourceMap, Span};

    /// `y = case (x) 0..5: arm<i>; default: dflt`, as process lowering
    /// leaves it: a `pmux` under `x == i`.
    fn case_module() -> Module {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        let mut b = ModuleBuilder::new("m", Span::new(id, 0, 0));
        let x = b.input("x", Type::bits(3));
        let dflt = b.input("dflt", Type::bits(2));
        let y = b.output("y", Type::bits(2));
        let xn = b.net(x);
        let mut arms = Vec::new();
        let mut selects = Vec::new();
        for i in 0..6u64 {
            let arm = b.input(format!("arm{i}"), Type::bits(2));
            arms.push(b.net(arm));
            let e = b.add_net(format!("e{i}"), Type::bit());
            let k = b.const_u64(3, i);
            b.cell(
                format!("eq{i}"),
                CellKind::Eq,
                vec![(Name::new("a"), xn), (Name::new("b"), k)],
                vec![(Name::new("y"), e)],
            );
            selects.push(b.net(e));
        }
        arms.reverse();
        selects.reverse();
        let arms = b.concat(arms);
        let selects = b.concat(selects);
        let d = b.net(dflt);
        b.cell(
            "pm",
            CellKind::Pmux,
            vec![
                (Name::new("a"), d),
                (Name::new("b"), arms),
                (Name::new("s"), selects),
            ],
            vec![(Name::new("y"), y)],
        );
        b.finish()
    }

    /// **A case tree is proved against the `pmux` as it was, whenever the
    /// mapping is proved**: [`blast_for`] hands back the graph without trees
    /// beside the one with, and they differ, so the proof
    /// [`map_module_checked`] adds is of something; and that proof and the
    /// mapping's together say equivalent. Would catch the second graph not
    /// made, or made from the trees themselves (so that a wrong tree would be
    /// proved against itself); the trees' being right is
    /// `aig::blast`'s tests' and every design's proof in
    /// `tests/fpga_xray_synthesis.rs`.
    #[test]
    fn a_case_tree_is_proved_against_the_pmux() {
        let m = case_module();
        let options = MapOptions {
            case_trees: true,
            ..MapOptions::lut(6)
        };
        let (trees, _, plain) = blast_for(&m, &options, true);
        let plain = plain.expect("the graph without trees, to prove against");
        assert_ne!(
            trees.len(),
            plain.len(),
            "the trees and the plain graph are one"
        );
        assert!(
            blast_for(&m, &options, false).2.is_none(),
            "not asked to prove"
        );
        let mut mapped = m.clone();
        let (_, verdict) =
            map_module_checked(&mut mapped, &options, Some(&MapVerifyOptions::default()));
        assert_eq!(verdict, Some(MapEquivalence::Equivalent));
    }
}

#[cfg(test)]
mod wide_tests {
    use super::*;

    /// A full multiplexer tree, `levels` selects deep, over fresh inputs.
    fn tree(levels: usize) -> Aig {
        let mut aig = Aig::new();
        let select: Vec<Edge> = (0..levels).map(|_| aig.add_input()).collect();
        let mut layer: Vec<Edge> = (0..1usize << levels).map(|_| aig.add_input()).collect();
        for s in &select {
            layer = layer
                .chunks(2)
                .map(|pair| aig.mux(*s, pair[1], pair[0]))
                .collect();
        }
        aig.add_output(layer[0]);
        aig
    }

    /// **The cover of a tree is Vivado's**: the tables and multiplexers its
    /// reports have for `mux8`, `mux16` and `mux64` in
    /// `testdata/fpga/vivado/muxf/vivado.txt`. And at both widths each is proved
    /// the tree it came from. Would catch a level of `MUXF7` missed, a `MUXF8`
    /// over anything but `MUXF7`s, or a polarity read wrong on the way into one;
    /// would not catch which pin of a slice any of it reaches, which is
    /// `tests/fpga_xray_muxf.rs`'s.
    #[test]
    fn a_tree_is_covered_as_vivado_covers_it() {
        for (levels, tables, f7, f8) in [(3, 2, 1, 0), (4, 4, 2, 1), (6, 17, 8, 4)] {
            let aig = tree(levels);
            let net = lut_map_wide(&aig, 6, &CutOptions::default(), 2);
            assert_eq!(
                (net.tables(), net.wide(WideMux::F7), net.wide(WideMux::F8)),
                (tables, f7, f8),
                "{levels} levels"
            );
            for k in [4, 6] {
                let net = lut_map_wide(&aig, k, &CutOptions::default(), 2);
                assert!(net.wide(WideMux::F7) > 0, "{levels} levels at LUT{k}");
                let verdict = check_lut_mapping(&aig, &net, &MapVerifyOptions::default());
                assert!(
                    verdict.proved(),
                    "{levels} levels at LUT{k}: {}",
                    verdict.render()
                );
            }
        }
    }

    /// The proof reads a wide multiplexer as the lookup table it also is, so
    /// a wrong one is a wrong table: this flips each bit of `init` in turn and
    /// each must be refused. Would catch the proof having learned to take a
    /// tag on trust.
    #[test]
    fn a_wrong_wide_multiplexer_is_refused_by_the_proof() {
        let aig = tree(3);
        let net = lut_map_wide(&aig, 6, &CutOptions::default(), 2);
        let at = net
            .luts
            .iter()
            .position(|l| l.wide == Some(WideMux::F7))
            .expect("a `MUXF7`");
        for bit in 0..8u32 {
            let mut wrong = net.clone();
            let flipped = if net.luts[at].init.bit(bit) == Bit::One {
                Bit::Zero
            } else {
                Bit::One
            };
            wrong.luts[at].init.set_bit(bit, flipped);
            let verdict = check_lut_mapping(&aig, &wrong, &MapVerifyOptions::default());
            assert!(verdict.wrong(), "bit {bit} flipped: {}", verdict.render());
        }
    }

    fn table(inputs: Vec<Signal>) -> Lut {
        Lut {
            init: Const::zero(1 << inputs.len()),
            inputs,
            wide: None,
        }
    }

    fn mux(level: WideMux, a: u32, b: u32) -> Lut {
        Lut {
            inputs: vec![Signal::Node(a), Signal::Node(b), Signal::Input(9)],
            init: Const::from_u64(WIDE_MUX_INIT, 8),
            wide: Some(level),
        }
    }

    /// **A table is one multiplexer's**, and a `MUXF8` wants two `MUXF7`s still
    /// standing. Would catch a table's one dedicated wire asked for twice, which
    /// the placer would refuse for the whole design, and a `MUXF8` left over a
    /// three-input table, which no slice has.
    #[test]
    fn a_second_reader_of_a_table_is_a_table_itself() {
        let input = |n| Signal::Input(n);
        let mut luts = vec![
            table(vec![input(0)]),
            table(vec![input(1)]),
            table(vec![input(2)]),
            table(vec![input(3)]),
            mux(WideMux::F7, 0, 1), // 4: keeps 0 and 1
            mux(WideMux::F7, 1, 2), // 5: 1 is 4's
            mux(WideMux::F7, 2, 3), // 6: keeps 2 and 3
            mux(WideMux::F8, 4, 5), // 7: 5 is no longer a `MUXF7`
            mux(WideMux::F8, 4, 6), // 8: 4 is free of 7, and both are `MUXF7`s
            mux(WideMux::F8, 4, 6), // 9: 8 has them
            mux(WideMux::F7, 4, 6), // 10: `MUXF7`s are no data for a `MUXF7`
        ];
        settle_wide(&mut luts);
        let tags: Vec<Option<WideMux>> = luts[4..].iter().map(|l| l.wide).collect();
        assert_eq!(
            tags,
            vec![
                Some(WideMux::F7),
                None,
                Some(WideMux::F7),
                None,
                Some(WideMux::F8),
                None,
                None,
            ]
        );
    }

    /// **What the mapper hands over keeps that rule by itself.** Two 8-to-1s that
    /// share a table: a `MUXF7` over it for one and the other's data are all
    /// on offer to the cover, and only one may have it. Would catch the
    /// rule's pass not being run on what the cover chose, which the test above
    /// cannot, since it calls that pass itself.
    #[test]
    fn a_table_two_trees_share_is_one_multiplexers() {
        let mut aig = Aig::new();
        let select: Vec<Edge> = (0..3).map(|_| aig.add_input()).collect();
        let a: Vec<Edge> = (0..8).map(|_| aig.add_input()).collect();
        let b: Vec<Edge> = (0..4).map(|_| aig.add_input()).collect();
        let build = |aig: &mut Aig, leaves: &[Edge]| {
            let mut layer = leaves.to_vec();
            for s in &select {
                layer = layer
                    .chunks(2)
                    .map(|pair| aig.mux(*s, pair[1], pair[0]))
                    .collect();
            }
            layer[0]
        };
        let one = build(&mut aig, &a);
        let two = [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]];
        let two = build(&mut aig, &two);
        aig.add_output(one);
        aig.add_output(two);
        let net = lut_map_wide(&aig, 6, &CutOptions::default(), 2);
        assert!(net.wide(WideMux::F7) > 0, "{net:?}");
        let mut seen = std::collections::HashSet::new();
        for lut in net.luts.iter().filter(|l| l.wide.is_some()) {
            for input in &lut.inputs[..2] {
                assert!(matches!(input, Signal::Node(_)), "{lut:?}");
                assert!(seen.insert(*input), "{input:?} read twice:\n{net:?}");
            }
        }
        let verdict = check_lut_mapping(&aig, &net, &MapVerifyOptions::default());
        assert!(verdict.proved(), "{}", verdict.render());
    }

    /// **The proof of a wide network sees the graph it came from as well.**
    /// Would catch a pass that takes trees out and puts one back wrong, whose
    /// network is a correct mapping of the wrong graph, and would not catch
    /// anything in the network that `check_lut_mapping` does not already.
    #[test]
    fn the_proof_through_a_graph_refuses_a_wrong_graph() {
        let aig = tree(4);
        let net = lut_map_wide(&aig, 6, &CutOptions::default(), 2);
        let options = MapVerifyOptions::default();
        assert!(check_lut_mapping_through(&aig, &aig, &net, &options).proved());
        // The same tree over its inputs in another order is a correct graph
        // of another function, and this network is a correct mapping of it.
        let mut other = Aig::new();
        let inputs: Vec<Edge> = (0..aig.inputs().len()).map(|_| other.add_input()).collect();
        let swapped: Vec<Edge> = inputs.iter().rev().copied().collect();
        let mut layer: Vec<Edge> = swapped[4..].to_vec();
        for s in &swapped[..4] {
            layer = layer
                .chunks(2)
                .map(|pair| other.mux(*s, pair[1], pair[0]))
                .collect();
        }
        other.add_output(layer[0]);
        let net = lut_map_wide(&other, 6, &CutOptions::default(), 2);
        let verdict = check_lut_mapping_through(&aig, &other, &net, &options);
        assert!(verdict.wrong(), "{}", verdict.render());
    }

    /// **A wide network with no fewer tables is not kept.** A tree whose every
    /// level reads one select looks like a tree and is a two-to-one choice, one
    /// table however it is mapped, so the two networks tie. Would catch the
    /// comparison gone, the wrong way round, or a tie going to the wide network.
    #[test]
    fn a_wide_network_with_no_fewer_tables_is_not_kept() {
        let mut aig = Aig::new();
        let s = aig.add_input();
        let mut layer: Vec<Edge> = (0..8).map(|_| aig.add_input()).collect();
        while layer.len() > 1 {
            layer = layer
                .chunks(2)
                .map(|pair| aig.mux(s, pair[1], pair[0]))
                .collect();
        }
        aig.add_output(layer[0]);
        let options = MapOptions::lut(6);
        let mut optimized = aig.clone();
        crate::synth::aig::optimize(&mut optimized, &options.aig_opt);
        let plain = lut_map_with(&optimized, 6, &options.cuts, options.area_passes);
        assert_eq!(plain.tables(), 1);
        assert!(wide_network(&aig, &optimized, 6, &options, &plain, None).is_none());
        // And a real one is, against a plain network that has more.
        let real = tree(4);
        let mut optimized = real.clone();
        crate::synth::aig::optimize(&mut optimized, &options.aig_opt);
        let plain = lut_map_with(&optimized, 6, &options.cuts, options.area_passes);
        let (wide, trees) =
            wide_network(&real, &optimized, 6, &options, &plain, None).expect("it saves tables");
        assert!(trees.is_some());
        assert_eq!(wide.tables(), 4);
    }

    /// **Keeping a design that saves nothing as it was**: a function a single
    /// table holds has no wide multiplexer in it, and `map_module`'s comparison
    /// keeps the network without. Together with `a_design_that_saves_nothing_is_not_moved`
    /// in `tests/fpga_xray_muxf.rs` this is what a board-measured build rests on.
    #[test]
    fn a_four_to_one_stays_one_table() {
        let net = lut_map_wide(&tree(2), 6, &CutOptions::default(), 2);
        assert_eq!((net.tables(), net.wide(WideMux::F7)), (1, 0));
    }
}

#[cfg(test)]
mod tests {
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

    /// The AIG this defect was reduced to, and what it is.
    ///
    /// `usb_ctrl_ep`'s class descriptor set is a 512-bit constant read by a
    /// variable-indexed part-select: a 64-entry ROM of eight six-input
    /// functions. Mapped onto LUT4, three of a CDC ACM descriptor set's
    /// sixty-seven bytes came out wrong on a real device, and only for that
    /// descriptor set — the shorter vendor one in the same constant was
    /// correct. This is the cone of one wrong bit, shrunk from 37 inputs and
    /// 171 AND nodes to three and twenty-four by fixing inputs to constants
    /// for as long as the fault survived. Three inputs is few enough to
    /// check every pattern of.
    ///
    /// It is not pretty and it is not meant to be: it is the smallest thing
    /// that still reproduces, and what makes it reproduce is the
    /// reconvergence. Nodes 5, 6 and 14 all read input 3; node 6 becomes a
    /// cut leaf and node 5 does not, so a leaf set that covers node 6 still
    /// leaves node 5 a path down to input 3.
    pub(super) fn descriptor_rom_cone() -> Aig {
        let mut g = Aig::new();
        let mut e: Vec<Edge> = vec![Edge::constant(false)];
        for _ in 0..3 {
            e.push(g.add_input());
        }
        // Each row is `(a, complement a, b, complement b)` for the next AND,
        // numbered from 4 in the order they are built.
        let spec: &[(usize, bool, usize, bool)] = &[
            (1, false, 3, true),   // 4
            (1, true, 3, false),   // 5
            (1, true, 3, true),    // 6
            (2, true, 6, false),   // 7
            (2, false, 6, true),   // 8
            (7, true, 8, true),    // 9
            (4, true, 5, true),    // 10
            (1, false, 6, false),  // 11
            (2, true, 10, false),  // 12
            (1, false, 3, false),  // 13
            (1, true, 6, false),   // 14
            (5, true, 14, true),   // 15
            (9, false, 15, true),  // 16
            (7, true, 16, true),   // 17
            (8, false, 17, true),  // 18
            (8, false, 18, false), // 19
            (9, true, 11, false),  // 20
            (4, true, 6, false),   // 21
            (4, true, 21, true),   // 22
            (9, false, 22, true),  // 23
            (8, true, 23, false),  // 24
            (20, true, 24, true),  // 25
            (8, true, 25, true),   // 26
            (19, true, 26, true),  // 27
        ];
        for &(a, ca, b, cb) in spec {
            let node = g.and(e[a].xor(ca), e[b].xor(cb));
            assert_eq!(
                node.node() as usize,
                e.len(),
                "structural hashing renumbered node {}",
                e.len()
            );
            e.push(node);
        }
        g.add_output(!e[27]);
        g
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
    /// A four-input AND that is an output in both polarities, as a carry
    /// chain's propagate is when it is the complement of a table something
    /// else reads: `y0 = a & b & c & d`, `y1 = !y0`, and a third output
    /// reading the AND as a leaf.
    fn read_both_ways() -> Aig {
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..6).map(|_| g.add_input()).collect();
        let all = g.and_n(&ins[..4]);
        let y2 = g.xor(all, ins[4]);
        g.add_output(all);
        g.add_output(!all);
        g.add_output(y2);
        g
    }

    /// **A node read in both polarities is two tables over its leaves, not a
    /// table and an inverter**, with [`CutOptions::duplicate_complements`]:
    /// the same count of tables (the inverter is one), and no table reads
    /// another's output only to invert it. Mapped onto four-input tables,
    /// where the AND is a table of its own. Would catch the option ignored
    /// (an inverter over the AND's table, a level more) and the duplicate
    /// given the plain function (every pattern of `y1` wrong).
    #[test]
    fn a_complement_read_beside_its_node_is_a_table_of_its_own() {
        let aig = read_both_ways();
        let inverters = |net: &LutNetwork| {
            net.luts
                .iter()
                .filter(|l| {
                    l.inputs.len() == 1
                        && matches!(l.inputs[0], Signal::Node(_))
                        && l.init == Const::from_u64(0b01, 2)
                })
                .count()
        };
        let plain = lut_map_with(&aig, 4, &CutOptions::default(), 2);
        assert_eq!(inverters(&plain), 1, "{:?}", plain.luts);
        let options = CutOptions {
            duplicate_complements: true,
            ..CutOptions::default()
        };
        let net = lut_map_with(&aig, 4, &options, 2);
        assert_eq!(inverters(&net), 0, "{:?}", net.luts);
        assert_eq!(net.tables(), plain.tables());
        for pat in 0..64u32 {
            let ins: Vec<bool> = (0..6).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(simulate_luts(&net, &ins), aig.eval(&ins), "pattern {pat:b}");
        }
    }

    pub(super) fn simulate_luts(net: &LutNetwork, inputs: &[bool]) -> Vec<bool> {
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

    /// The cone that miscompiled: mapped onto lookup tables of every width,
    /// the network must compute what the AIG computes, for all eight input
    /// patterns.
    ///
    /// [`super::cuts::tests::every_cut_computes_its_node`] holds the cause —
    /// a cut whose function was not its node's — and this holds the effect,
    /// which is what a user sees: before the fix, at every width from 3 up,
    /// the LUT network read `true` where the logic says `false`. That one bit
    /// was `bInterfaceNumber` of a CDC ACM data interface, and a Linux host
    /// refused the device over it.
    #[test]
    fn the_descriptor_rom_cone_maps_to_its_own_function() {
        let aig = descriptor_rom_cone();
        for k in 2..=8u32 {
            let net = lut_map(&aig, k);
            for pat in 0..8u32 {
                let ins: Vec<bool> = (0..3).map(|i| (pat >> i) & 1 == 1).collect();
                assert_eq!(
                    simulate_luts(&net, &ins),
                    aig.eval(&ins),
                    "LUT{k} cover of the descriptor ROM cone, inputs {ins:?}, \
                     {} luts: {:?}",
                    net.len(),
                    net.luts
                );
            }
        }
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

    /// A library of only an inverter and a two-input NAND is functionally
    /// complete, so it must cover any function. NAND2 with inverted inputs
    /// gives NAND or OR but never AND, so an AND node, the one operation
    /// an AIG is made of, needs the inversion on the output. Before output
    /// inversion was supported this panicked with "the cover only chose
    /// implementable cuts".
    #[test]
    fn a_nand_and_inverter_library_covers_everything() {
        use crate::synth::aig::truth::TruthTable;
        let v = |n: usize, i: usize| TruthTable::var(n, i);
        let mut b = GateLibrary::builder("nand_only");
        b.gate("INV", &["A"], v(1, 0).not(), 1.0, &[1.0]);
        b.gate(
            "NAND2",
            &["A", "B"],
            v(2, 0).and(&v(2, 1)).not(),
            1.0,
            &[1.0, 1.0],
        );
        let lib = b.finish();

        // The AND of two inputs has no direct match, only its complement.
        let and = v(2, 0).and(&v(2, 1));
        let (gate, m) = lib.match_function(&and).expect("AND through NAND and INV");
        assert_eq!(gate.name, "NAND2");
        assert!(m.output_invert, "{m:?}");

        // And the whole sample network, which the full library maps with
        // XOR cells, now maps onto NAND2 and INV alone, correctly.
        let aig = sample();
        let net = gate_map(&aig, &lib);
        for g in &net.gates {
            assert!(g.gate == "NAND2" || g.gate == "INV", "used {}", g.gate);
        }
        for pat in 0..16u32 {
            let ins: Vec<bool> = (0..4).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(
                simulate_gates(&net, &lib, &ins),
                aig.eval(&ins),
                "pattern {pat}"
            );
        }
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
