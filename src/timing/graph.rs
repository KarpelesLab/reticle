//! The timing graph: pins are nodes, delay arcs are edges.
//!
//! [`TimingGraph::build`] turns a module in cell form into a directed
//! graph whose nodes are *pins* — a cell's input pins, a cell's output
//! pins and the module's ports — and whose edges are of exactly two
//! kinds:
//!
//! - a **cell arc** from an input pin to an output pin of the same cell,
//!   carrying the delay through that cell;
//! - a **net arc** from the pin driving a net to each pin loading it,
//!   carrying the interconnect delay.
//!
//! Sequential cells break the graph. A flip-flop has a `clk` to `q` arc
//! (its clock-to-output delay) but no `d` to `q` arc, so no path crosses
//! it: `q` is a **start point** and `d` is an **end point**, checked for
//! setup and hold against the flop's own `clk` pin. The same holds for a
//! clocked memory port, and for a black box, whose outputs start paths
//! and whose inputs end them because nothing is known about its insides.
//!
//! Everything else follows from that. A path runs from a start point,
//! through alternating cell and net arcs, to an end point; the analysis
//! in [`super::sta`] walks the graph forwards for arrival times and
//! backwards for required times.
//!
//! # Flat modules
//!
//! The graph is built over **one module**. A hierarchical design is
//! flattened first: [`flatten_for_timing`] does it on a copy, so the
//! caller's design is untouched. A module that still holds instances is
//! analysed anyway — the instance's pins become black-box boundaries,
//! with each connection classified as a driver or a load by looking at
//! whether anything else already drives the nets it names — and the
//! graph records a note saying the result is only as good as that guess.
//!
//! # Combinational loops
//!
//! Arrival times are propagated in topological order, so a cycle has no
//! answer. The builder finds every pin that a topological sort cannot
//! place and reports the cycles in [`TimingGraph::loops`]; those pins get
//! no arrival time and the analysis carries on around them instead of
//! looping forever.
//!
//! # Not modelled
//!
//! Latches are treated as transparent (`en` to `q` and `d` to `q` arcs)
//! with a setup check on `d` against `en`; there is no time borrowing.
//! Asynchronous set and reset pins get no recovery / removal checks.
//! Tri-state enable arcs are ordinary delay arcs, with no separate
//! enable / disable timing.

use std::fmt;

use crate::diag::Diagnostics;
use crate::ir::{
    Cell, CellId, CellKind, Design, ExprId, ExprKind, FlattenOptions, InstanceId, Module, ModuleId,
    NetId, PortDir,
};
use crate::source::Span;

use super::delay::{ArcResult, Sense};

/// Identifies a pin inside a [`TimingGraph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PinId(u32);

impl PinId {
    /// The index into [`TimingGraph::pins`].
    pub fn index(self) -> usize {
        usize::try_from(self.0).expect("a pin index fits in a usize")
    }

    /// The raw number, for rendering.
    pub fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for PinId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// Which object a pin belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinSite {
    /// A module port, by index into [`Module::ports`].
    Port(usize),
    /// An input pin of a cell, by index into [`Cell::inputs`].
    CellInput(CellId, usize),
    /// An output pin of a cell, by index into [`Cell::outputs`].
    CellOutput(CellId, usize),
    /// The value side of a continuous assignment, by index into
    /// [`Module::assigns`].
    AssignInput(usize),
    /// The target side of a continuous assignment.
    AssignOutput(usize),
    /// A connection of an instance that loads its nets.
    InstanceInput(InstanceId, usize),
    /// A connection of an instance that drives its nets.
    InstanceOutput(InstanceId, usize),
}

/// Whether a pin loads its nets or drives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinDirection {
    /// The pin reads the nets it names.
    Input,
    /// The pin drives the nets it names.
    Output,
}

/// What a pin is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinRole {
    /// An ordinary data pin.
    Data,
    /// The clock pin of a sequential cell; the analysis seeds it from
    /// the clock network rather than from data propagation.
    Clock,
}

/// One node of the timing graph.
#[derive(Clone, Debug)]
pub struct Pin {
    /// Where the pin lives.
    pub site: PinSite,
    /// The name used in reports, unique within the graph.
    pub name: String,
    /// The port name on its cell, or the module port name.
    pub port: String,
    /// The cell, instance or port the pin belongs to, by name.
    pub owner: String,
    /// The cell type name: a primitive keyword, a black box's name,
    /// `assign`, `port` or the instantiated module's name.
    pub cell_type: String,
    /// Which nets the pin reads or drives.
    pub nets: Vec<NetId>,
    /// Whether the pin loads or drives those nets.
    pub direction: PinDirection,
    /// Data or clock.
    pub role: PinRole,
    /// Where the object came from in the source.
    pub span: Span,
}

/// Which of the two kinds of edge an arc is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArcKind {
    /// From an input pin to an output pin of one cell.
    Cell,
    /// From a driver to one of the loads on its net.
    Net,
}

impl ArcKind {
    /// A word for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            ArcKind::Cell => "cell",
            ArcKind::Net => "net",
        }
    }
}

/// One edge of the timing graph.
#[derive(Clone, Debug)]
pub struct Arc {
    /// Where the arc starts.
    pub from: PinId,
    /// Where the arc ends.
    pub to: PinId,
    /// Cell arc or net arc.
    pub kind: ArcKind,
    /// The net an arc of kind [`ArcKind::Net`] crosses.
    pub net: Option<NetId>,
    /// The delay, filled in by [`TimingGraph::annotate`]; zero until
    /// then.
    pub delay: ArcResult,
}

/// A combinational loop, reported rather than walked into.
#[derive(Clone, Debug)]
pub struct CombLoop {
    /// The pins on the cycle, starting at the lowest-numbered one and
    /// following the arcs round; the first pin is not repeated at the
    /// end.
    pub pins: Vec<PinId>,
    /// The names of those pins, for reports.
    pub names: Vec<String>,
    /// Where the first cell on the cycle came from.
    pub span: Span,
}

/// What kind of point a start or end point is, which decides how the
/// analysis constrains it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    /// A module port.
    Port,
    /// The data pin or output of a sequential cell.
    Sequential,
    /// A pin of something the analysis knows nothing about.
    Blackbox,
}

/// An end point: a pin where a path stops and a check is made.
#[derive(Clone, Debug)]
pub struct EndPoint {
    /// The pin the path ends at.
    pub pin: PinId,
    /// What sort of end point it is.
    pub kind: PointKind,
    /// The clock pin the check is against, for a sequential end point.
    pub clock_pin: Option<PinId>,
    /// True when the capturing cell uses the rising clock edge.
    pub clock_rising: bool,
}

/// A start point: a pin where a path begins.
#[derive(Clone, Debug)]
pub struct StartPoint {
    /// The pin the path begins at.
    pub pin: PinId,
    /// What sort of start point it is.
    pub kind: PointKind,
    /// The clock pin that launches it, for a sequential start point.
    pub clock_pin: Option<PinId>,
    /// True when the launching cell uses the rising clock edge.
    pub clock_rising: bool,
}

/// Pins, arcs, and everything derived from them.
#[derive(Clone, Debug)]
pub struct TimingGraph {
    /// Every pin, in construction order: ports, then cells, then
    /// assignments, then instances.
    pub pins: Vec<Pin>,
    /// Every arc, cell arcs before net arcs for each pin.
    pub arcs: Vec<Arc>,
    /// Arcs ending at each pin, by index into [`TimingGraph::arcs`].
    fanin: Vec<Vec<usize>>,
    /// Arcs leaving each pin.
    fanout: Vec<Vec<usize>>,
    /// Where paths begin.
    pub start_points: Vec<StartPoint>,
    /// Where paths end and checks are made.
    pub end_points: Vec<EndPoint>,
    /// The clock pins of sequential cells.
    pub clock_pins: Vec<PinId>,
    /// Every pin a topological sort could place, in that order. Pins on
    /// a combinational loop are missing.
    pub order: Vec<PinId>,
    /// The combinational loops found, sorted by their first pin.
    pub loops: Vec<CombLoop>,
    /// Anything worth saying about how the graph was built, in the order
    /// it was noticed.
    pub notes: Vec<String>,
}

impl TimingGraph {
    /// Builds the graph of a module in cell form.
    ///
    /// Black-box cells become boundaries, since nothing here knows what
    /// is inside them; [`TimingGraph::build_with`] asks a delay model
    /// instead.
    pub fn build(module: &Module) -> TimingGraph {
        Builder::new(module, None).build()
    }

    /// Builds the graph, asking `model` for the topology of every cell
    /// type the IR's primitive set does not describe.
    ///
    /// This is what lets a standard-cell netlist — black boxes named
    /// after library cells — time end to end: the model says which of
    /// the cell's pins reach which, which pin is the clock, and which
    /// pins carry a setup and hold check.
    pub fn build_with(module: &Module, model: &dyn super::delay::DelayModel) -> TimingGraph {
        Builder::new(module, Some(model)).build()
    }

    /// The arcs ending at `pin`.
    pub fn fanin(&self, pin: PinId) -> impl Iterator<Item = &Arc> {
        self.fanin[pin.index()].iter().map(|&a| &self.arcs[a])
    }

    /// The arcs leaving `pin`.
    pub fn fanout(&self, pin: PinId) -> impl Iterator<Item = &Arc> {
        self.fanout[pin.index()].iter().map(|&a| &self.arcs[a])
    }

    /// Indices into [`TimingGraph::arcs`] of the arcs ending at `pin`.
    pub fn fanin_indices(&self, pin: PinId) -> &[usize] {
        &self.fanin[pin.index()]
    }

    /// Indices into [`TimingGraph::arcs`] of the arcs leaving `pin`.
    pub fn fanout_indices(&self, pin: PinId) -> &[usize] {
        &self.fanout[pin.index()]
    }

    /// The pin with the given id.
    pub fn pin(&self, pin: PinId) -> &Pin {
        &self.pins[pin.index()]
    }

    /// The pin's report name.
    pub fn name(&self, pin: PinId) -> &str {
        &self.pins[pin.index()].name
    }

    /// The pin with the given report name.
    pub fn pin_by_name(&self, name: &str) -> Option<PinId> {
        self.pins
            .iter()
            .position(|p| p.name == name)
            .map(|i| PinId(u32::try_from(i).expect("pin count fits in a u32")))
    }

    /// The pin driving `net`, if the module has exactly one driver for
    /// it.
    pub fn driver_of(&self, net: NetId) -> Option<PinId> {
        self.pins
            .iter()
            .position(|p| p.direction == PinDirection::Output && p.nets.contains(&net))
            .map(|i| PinId(u32::try_from(i).expect("pin count fits in a u32")))
    }

    /// True when `pin` is a start point.
    pub fn is_start(&self, pin: PinId) -> bool {
        self.start_points.iter().any(|s| s.pin == pin)
    }

    /// True when `pin` is an end point.
    pub fn is_end(&self, pin: PinId) -> bool {
        self.end_points.iter().any(|e| e.pin == pin)
    }

    /// Fills in [`Arc::delay`] for every arc by asking `model`, walking
    /// the graph in topological order so that each arc is asked with the
    /// input slew the earlier stages produced.
    ///
    /// `input_slew` is the transition time assumed at every start point,
    /// and `output_load` the capacitance assumed on every output port.
    /// Arcs on a combinational loop are asked with a zero input slew,
    /// since no order can be established for them.
    ///
    /// Returns the transition time left at each pin, in pin order,
    /// which the setup and hold checks need.
    pub fn annotate(
        &mut self,
        module: &Module,
        model: &dyn super::delay::DelayModel,
        input_slew: super::delay::Transition,
        output_load: f64,
        wire_load: f64,
    ) -> Vec<super::delay::Transition> {
        use super::delay::{ArcQuery, NetQuery, PinQuery, Transition};

        // The capacitance each driver sees: the pins on its net plus the
        // flat per-net wire load. Output ports add the caller's load.
        let mut drivers: Vec<Option<PinId>> = vec![None; module.nets.len()];
        for (i, pin) in self.pins.iter().enumerate() {
            if pin.direction != PinDirection::Output {
                continue;
            }
            let id = PinId(u32::try_from(i).expect("pin count fits in a u32"));
            for net in &pin.nets {
                if let Some(slot @ None) = drivers.get_mut(net.index()) {
                    *slot = Some(id);
                }
            }
        }
        let mut load = vec![0.0f64; self.pins.len()];
        for pin in &self.pins {
            if pin.direction != PinDirection::Input {
                continue;
            }
            let cap = model.pin_capacitance(&PinQuery {
                module,
                cell: self.cell_of(module, pin),
                cell_type: &pin.cell_type,
                port: &pin.port,
            });
            for net in &pin.nets {
                if let Some(Some(driver)) = drivers.get(net.index()).copied() {
                    load[driver.index()] += cap;
                }
            }
        }
        for (i, pin) in self.pins.iter().enumerate() {
            match pin.site {
                PinSite::Port(p) if module.ports[p].dir == PortDir::Out => {
                    load[i] += output_load;
                }
                _ => {}
            }
            if pin.direction == PinDirection::Output {
                load[i] += wire_load;
            }
        }

        let mut slew = vec![Transition::ZERO; self.pins.len()];
        for start in &self.start_points {
            slew[start.pin.index()] = input_slew;
        }
        for pin in &self.clock_pins {
            slew[pin.index()] = input_slew;
        }
        let order = self.order.clone();
        for pin_id in order {
            let outgoing = self.fanout[pin_id.index()].clone();
            for arc_index in outgoing {
                let (from, to, kind, net) = {
                    let arc = &self.arcs[arc_index];
                    (arc.from, arc.to, arc.kind, arc.net)
                };
                let result = match kind {
                    ArcKind::Cell => {
                        let (cell_type, from_port, to_port, sense) = {
                            let src = &self.pins[from.index()];
                            let dst = &self.pins[to.index()];
                            (
                                dst.cell_type.clone(),
                                src.port.clone(),
                                dst.port.clone(),
                                self.arcs[arc_index].delay.sense,
                            )
                        };
                        let cell = self.cell_of(module, &self.pins[to.index()]);
                        model.cell_arc(&ArcQuery {
                            module,
                            cell,
                            cell_type: &cell_type,
                            from_port: &from_port,
                            to_port: &to_port,
                            input_slew: slew[from.index()],
                            load: load[to.index()],
                            fanout: self.fanout[to.index()].len(),
                            sense,
                        })
                    }
                    ArcKind::Net => {
                        let driver_type = self.pins[from.index()].cell_type.clone();
                        model.net_arc(&NetQuery {
                            module,
                            net: net.unwrap_or(NetId(0)),
                            driver_type: &driver_type,
                            fanout: self.fanout[from.index()].len(),
                            load: load[from.index()],
                            input_slew: slew[from.index()],
                        })
                    }
                };
                slew[to.index()] = slew[to.index()].max_with(result.slew);
                self.arcs[arc_index].delay = result;
            }
        }
        self.notes.retain(|n| !n.starts_with("annotated with "));
        self.notes.push(format!("annotated with {}", model.name()));
        slew
    }

    /// The cell a pin belongs to, when it belongs to one.
    fn cell_of<'a>(&self, module: &'a Module, pin: &Pin) -> Option<&'a Cell> {
        match pin.site {
            PinSite::CellInput(id, _) | PinSite::CellOutput(id, _) => module.cells.get(id),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Construction

/// The cell arcs of a primitive: from an input port to an output port,
/// with the sense of the arc.
///
/// A sequential cell contributes only its clock-to-output arc, which is
/// what makes `d` an end point and `q` a start point.
pub fn cell_arcs(kind: &CellKind) -> Vec<(&'static str, &'static str, Sense)> {
    use Sense::{Negative, NonUnate, Positive};
    match kind {
        CellKind::Not => vec![("a", "y", Negative)],
        CellKind::Buf => vec![("a", "y", Positive)],
        CellKind::And | CellKind::Or => vec![("a", "y", Positive), ("b", "y", Positive)],
        CellKind::Xor => vec![("a", "y", NonUnate), ("b", "y", NonUnate)],
        CellKind::Add
        | CellKind::Sub
        | CellKind::Mul
        | CellKind::Div
        | CellKind::Mod
        | CellKind::Shl
        | CellKind::Shr
        | CellKind::Sshr
        | CellKind::Eq
        | CellKind::Ne
        | CellKind::Lt
        | CellKind::Le
        | CellKind::Gt
        | CellKind::Ge => vec![("a", "y", NonUnate), ("b", "y", NonUnate)],
        CellKind::ReduceAnd | CellKind::ReduceOr => vec![("a", "y", Positive)],
        CellKind::ReduceXor => vec![("a", "y", NonUnate)],
        CellKind::Mux | CellKind::Pmux => vec![
            ("a", "y", Positive),
            ("b", "y", Positive),
            ("s", "y", NonUnate),
        ],
        CellKind::Lut { .. } => vec![("a", "y", NonUnate)],
        CellKind::Tristate => vec![("a", "y", Positive), ("en", "y", NonUnate)],
        CellKind::Dff { .. } => vec![("clk", "q", Positive)],
        // A latch is transparent while enabled, so both its data and its
        // enable reach `q`; time borrowing is not modelled.
        CellKind::Dlatch => vec![("en", "q", NonUnate), ("d", "q", Positive)],
        CellKind::MemRdPort { clocked, .. } => {
            if *clocked {
                vec![("clk", "data", Positive)]
            } else {
                vec![("addr", "data", NonUnate)]
            }
        }
        CellKind::MemWrPort { .. } | CellKind::Blackbox(_) => Vec::new(),
    }
}

/// The data pins of a sequential primitive that carry a setup and hold
/// check, with the pin the check is against.
pub fn sequential_checks(kind: &CellKind) -> (Vec<&'static str>, Option<&'static str>) {
    match kind {
        CellKind::Dff {
            has_enable, reset, ..
        } => {
            let mut pins = vec!["d"];
            if *has_enable {
                pins.push("en");
            }
            // An asynchronous reset needs recovery / removal checks,
            // which are not modelled; a synchronous one is an ordinary
            // data pin.
            if reset.as_ref().is_some_and(|r| !r.asynchronous) {
                pins.push("rst");
            }
            (pins, Some("clk"))
        }
        CellKind::Dlatch => (vec!["d"], Some("en")),
        CellKind::MemRdPort { clocked: true, .. } => (vec!["addr", "en"], Some("clk")),
        CellKind::MemWrPort { clocked: true, .. } => (vec!["addr", "data", "en"], Some("clk")),
        _ => (Vec::new(), None),
    }
}

/// The name a delay model sees for a cell.
pub fn cell_type_name(kind: &CellKind) -> String {
    match kind {
        CellKind::Blackbox(name) => name.as_str().to_owned(),
        other => other.keyword().to_owned(),
    }
}

struct Builder<'a> {
    module: &'a Module,
    model: Option<&'a dyn super::delay::DelayModel>,
    pins: Vec<Pin>,
    arcs: Vec<Arc>,
    notes: Vec<String>,
    /// Stamp per expression node, so a shared node is visited once per
    /// query without clearing a whole vector each time.
    stamps: Vec<u32>,
    stamp: u32,
}

impl<'a> Builder<'a> {
    fn new(module: &'a Module, model: Option<&'a dyn super::delay::DelayModel>) -> Builder<'a> {
        Builder {
            module,
            model,
            pins: Vec::new(),
            arcs: Vec::new(),
            notes: Vec::new(),
            stamps: vec![0; module.exprs.len()],
            stamp: 0,
        }
    }

    /// The nets an expression reads, in first-use order without
    /// duplicates.
    fn expr_nets(&mut self, root: ExprId) -> Vec<NetId> {
        self.stamp += 1;
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(expr) = self.module.exprs.get(id) else {
                continue;
            };
            if self.stamps[id.index()] == self.stamp {
                continue;
            }
            self.stamps[id.index()] = self.stamp;
            if let ExprKind::Net(net) = expr.kind
                && !out.contains(&net)
            {
                out.push(net);
            }
            for operand in crate::ir::expr::operands(&expr.kind).into_iter().rev() {
                stack.push(operand);
            }
        }
        out
    }

    /// The arcs and checks of a cell: straight from the primitive
    /// table, or from the delay model when the cell is a black box.
    fn topology_of(&self, cell: &Cell) -> super::delay::CellTopology {
        use super::delay::CellTopology;
        if let CellKind::Blackbox(name) = &cell.kind {
            let inputs: Vec<String> = cell
                .inputs
                .iter()
                .map(|(p, _)| p.as_str().to_owned())
                .collect();
            let outputs: Vec<String> = cell
                .outputs
                .iter()
                .map(|(p, _)| p.as_str().to_owned())
                .collect();
            return self
                .model
                .and_then(|m| m.cell_topology(name.as_str(), &inputs, &outputs))
                .unwrap_or_default();
        }
        let (checked, clock) = sequential_checks(&cell.kind);
        CellTopology {
            arcs: cell_arcs(&cell.kind)
                .into_iter()
                .map(|(a, b, sense)| (a.to_owned(), b.to_owned(), sense))
                .collect(),
            checked_pins: checked.into_iter().map(str::to_owned).collect(),
            clock_pin: clock.map(str::to_owned),
        }
    }

    fn push_pin(&mut self, mut pin: Pin) -> PinId {
        // Report names must be unique; a collision can only come from
        // two assignments to slices of one net, and numbering them in
        // construction order keeps the output stable.
        if self.pins.iter().any(|p| p.name == pin.name) {
            let base = pin.name.clone();
            let mut n = 2;
            while self.pins.iter().any(|p| p.name == pin.name) {
                pin.name = format!("{base}#{n}");
                n += 1;
            }
        }
        let id = PinId(u32::try_from(self.pins.len()).expect("pin count fits in a u32"));
        self.pins.push(pin);
        id
    }

    fn build(mut self) -> TimingGraph {
        let module = self.module;

        // --- ports -------------------------------------------------
        let mut port_pins = Vec::new();
        for (i, port) in module.ports.iter().enumerate() {
            let direction = match port.dir {
                PortDir::In | PortDir::InOut => PinDirection::Output,
                PortDir::Out => PinDirection::Input,
            };
            if port.dir == PortDir::InOut {
                self.notes.push(format!(
                    "port `{}` is bidirectional and is timed as an input only",
                    port.name
                ));
            }
            let id = self.push_pin(Pin {
                site: PinSite::Port(i),
                name: port.name.as_str().to_owned(),
                port: port.name.as_str().to_owned(),
                owner: port.name.as_str().to_owned(),
                cell_type: "port".to_owned(),
                nets: vec![port.net],
                direction,
                role: PinRole::Data,
                span: port.span,
            });
            port_pins.push(id);
        }

        // --- cells -------------------------------------------------
        let mut cell_inputs: Vec<Vec<PinId>> = Vec::new();
        let mut cell_outputs: Vec<Vec<PinId>> = Vec::new();
        let topologies: Vec<super::delay::CellTopology> = module
            .cells
            .values()
            .map(|cell| self.topology_of(cell))
            .collect();
        for (index, (id, cell)) in module.cells.iter().enumerate() {
            let type_name = cell_type_name(&cell.kind);
            let clock_port = topologies[index].clock_pin.as_deref();
            let mut ins = Vec::new();
            for (i, (port, expr)) in cell.inputs.iter().enumerate() {
                let nets = self.expr_nets(*expr);
                let role = if clock_port == Some(port.as_str()) {
                    PinRole::Clock
                } else {
                    PinRole::Data
                };
                ins.push(self.push_pin(Pin {
                    site: PinSite::CellInput(id, i),
                    name: format!("{}/{}", cell.name, port),
                    port: port.as_str().to_owned(),
                    owner: cell.name.as_str().to_owned(),
                    cell_type: type_name.clone(),
                    nets,
                    direction: PinDirection::Input,
                    role,
                    span: cell.span,
                }));
            }
            let mut outs = Vec::new();
            for (i, (port, net)) in cell.outputs.iter().enumerate() {
                outs.push(self.push_pin(Pin {
                    site: PinSite::CellOutput(id, i),
                    name: format!("{}/{}", cell.name, port),
                    port: port.as_str().to_owned(),
                    owner: cell.name.as_str().to_owned(),
                    cell_type: type_name.clone(),
                    nets: vec![*net],
                    direction: PinDirection::Output,
                    role: PinRole::Data,
                    span: cell.span,
                }));
            }
            cell_inputs.push(ins);
            cell_outputs.push(outs);
        }

        // --- continuous assignments --------------------------------
        let mut assign_pins = Vec::new();
        for (i, assign) in module.assigns.iter().enumerate() {
            let targets = assign.target.nets();
            let label = targets
                .first()
                .and_then(|n| module.nets.get(*n))
                .map_or_else(|| format!("assign{i}"), |n| n.name.as_str().to_owned());
            let mut nets = self.expr_nets(assign.value);
            crate::ir::walk::lvalue_exprs(&assign.target, &mut |e| {
                // An indexed target reads its index, which is a timing
                // path into the assignment like any other.
                for net in Builder::new(module, None).expr_nets(e) {
                    if !nets.contains(&net) {
                        nets.push(net);
                    }
                }
            });
            let input = self.push_pin(Pin {
                site: PinSite::AssignInput(i),
                name: format!("{label}.in"),
                port: "in".to_owned(),
                owner: label.clone(),
                cell_type: "assign".to_owned(),
                nets,
                direction: PinDirection::Input,
                role: PinRole::Data,
                span: assign.span,
            });
            let output = self.push_pin(Pin {
                site: PinSite::AssignOutput(i),
                name: label.clone(),
                port: "out".to_owned(),
                owner: label,
                cell_type: "assign".to_owned(),
                nets: targets,
                direction: PinDirection::Output,
                role: PinRole::Data,
                span: assign.span,
            });
            assign_pins.push((input, output));
        }

        // --- instances ---------------------------------------------
        //
        // A module that still has instances is not flat. Classify each
        // connection by whether anything else already drives the nets it
        // names: if not, the instance is the driver.
        let mut driven: Vec<bool> = vec![false; module.nets.len()];
        for pin in &self.pins {
            if pin.direction == PinDirection::Output {
                for net in &pin.nets {
                    if let Some(slot) = driven.get_mut(net.index()) {
                        *slot = true;
                    }
                }
            }
        }
        let mut instance_pins: Vec<Vec<PinId>> = Vec::new();
        if !module.instances.is_empty() {
            self.notes.push(format!(
                "{} instance(s) are not flattened; their connections are timed as black-box \
                 boundaries",
                module.instances.len()
            ));
        }
        for (id, inst) in module.instances.iter() {
            let type_name = match &inst.module {
                crate::ir::ModuleRef::Resolved(m) => format!("m{}", m.raw()),
                crate::ir::ModuleRef::Unresolved(name) => name.as_str().to_owned(),
            };
            let mut pins = Vec::new();
            for (i, (port, expr)) in inst.connections.iter().enumerate() {
                let nets = self.expr_nets(*expr);
                let drives = !nets.is_empty()
                    && nets
                        .iter()
                        .all(|n| !driven.get(n.index()).copied().unwrap_or(true));
                let (site, direction) = if drives {
                    (PinSite::InstanceOutput(id, i), PinDirection::Output)
                } else {
                    (PinSite::InstanceInput(id, i), PinDirection::Input)
                };
                if drives {
                    for net in &nets {
                        if let Some(slot) = driven.get_mut(net.index()) {
                            *slot = true;
                        }
                    }
                }
                pins.push(self.push_pin(Pin {
                    site,
                    name: format!("{}/{}", inst.name, port),
                    port: port.as_str().to_owned(),
                    owner: inst.name.as_str().to_owned(),
                    cell_type: type_name.clone(),
                    nets,
                    direction,
                    role: PinRole::Data,
                    span: inst.span,
                }));
            }
            instance_pins.push(pins);
        }

        // --- cell arcs ---------------------------------------------
        for (index, (_, cell)) in module.cells.iter().enumerate() {
            for (from_port, to_port, sense) in &topologies[index].arcs {
                let Some(from) = cell
                    .inputs
                    .iter()
                    .position(|(n, _)| n.as_str() == from_port)
                    .map(|i| cell_inputs[index][i])
                else {
                    continue;
                };
                let Some(to) = cell
                    .outputs
                    .iter()
                    .position(|(n, _)| n.as_str() == to_port)
                    .map(|i| cell_outputs[index][i])
                else {
                    continue;
                };
                let sense = *sense;
                self.arcs.push(Arc {
                    from,
                    to,
                    kind: ArcKind::Cell,
                    net: None,
                    delay: ArcResult {
                        sense,
                        ..ArcResult::ZERO
                    },
                });
            }
        }
        for (input, output) in &assign_pins {
            self.arcs.push(Arc {
                from: *input,
                to: *output,
                kind: ArcKind::Cell,
                net: None,
                delay: ArcResult::ZERO,
            });
        }

        // --- net arcs ----------------------------------------------
        let mut drivers: Vec<Option<PinId>> = vec![None; module.nets.len()];
        for (i, pin) in self.pins.iter().enumerate() {
            if pin.direction != PinDirection::Output {
                continue;
            }
            let id = PinId(u32::try_from(i).expect("pin count fits in a u32"));
            for net in &pin.nets {
                match drivers.get_mut(net.index()) {
                    Some(slot @ None) => *slot = Some(id),
                    Some(Some(_)) => {
                        let name = module
                            .nets
                            .get(*net)
                            .map_or_else(|| net.to_string(), |n| n.name.to_string());
                        let note = format!(
                            "net `{name}` has more than one driver; only the first is timed"
                        );
                        if !self.notes.contains(&note) {
                            self.notes.push(note);
                        }
                    }
                    None => {}
                }
            }
        }
        let loads: Vec<(PinId, Vec<NetId>)> = self
            .pins
            .iter()
            .enumerate()
            .filter(|(_, p)| p.direction == PinDirection::Input)
            .map(|(i, p)| {
                (
                    PinId(u32::try_from(i).expect("pin count fits in a u32")),
                    p.nets.clone(),
                )
            })
            .collect();
        for (load, nets) in loads {
            for net in nets {
                if let Some(Some(driver)) = drivers.get(net.index()).copied()
                    && driver != load
                {
                    self.arcs.push(Arc {
                        from: driver,
                        to: load,
                        kind: ArcKind::Net,
                        net: Some(net),
                        delay: ArcResult::ZERO,
                    });
                }
            }
        }

        // --- start, end and clock points ---------------------------
        let mut start_points = Vec::new();
        let mut end_points = Vec::new();
        let mut clock_pins = Vec::new();
        for (i, port) in module.ports.iter().enumerate() {
            match port.dir {
                PortDir::In | PortDir::InOut => start_points.push(StartPoint {
                    pin: port_pins[i],
                    kind: PointKind::Port,
                    clock_pin: None,
                    clock_rising: true,
                }),
                PortDir::Out => end_points.push(EndPoint {
                    pin: port_pins[i],
                    kind: PointKind::Port,
                    clock_pin: None,
                    clock_rising: true,
                }),
            }
        }
        for (index, (_, cell)) in module.cells.iter().enumerate() {
            let rising = match &cell.kind {
                CellKind::Dff { clk_pos, .. } => *clk_pos,
                _ => true,
            };
            let topology = &topologies[index];
            let clock_pin = topology.clock_pin.as_deref().and_then(|p| {
                cell.inputs
                    .iter()
                    .position(|(n, _)| n.as_str() == p)
                    .map(|i| cell_inputs[index][i])
            });
            if let Some(pin) = clock_pin {
                clock_pins.push(pin);
            }
            for port in &topology.checked_pins {
                if let Some(pin) = cell
                    .inputs
                    .iter()
                    .position(|(n, _)| n.as_str() == port.as_str())
                    .map(|i| cell_inputs[index][i])
                {
                    end_points.push(EndPoint {
                        pin,
                        kind: PointKind::Sequential,
                        clock_pin,
                        clock_rising: rising,
                    });
                }
            }
            if topology.is_sequential() {
                for pin in &cell_outputs[index] {
                    start_points.push(StartPoint {
                        pin: *pin,
                        kind: PointKind::Sequential,
                        clock_pin,
                        clock_rising: rising,
                    });
                }
            } else if matches!(cell.kind, CellKind::Blackbox(_)) && topology.arcs.is_empty() {
                for pin in &cell_outputs[index] {
                    start_points.push(StartPoint {
                        pin: *pin,
                        kind: PointKind::Blackbox,
                        clock_pin: None,
                        clock_rising: true,
                    });
                }
                for pin in &cell_inputs[index] {
                    end_points.push(EndPoint {
                        pin: *pin,
                        kind: PointKind::Blackbox,
                        clock_pin: None,
                        clock_rising: true,
                    });
                }
            }
        }
        for pins in &instance_pins {
            for pin in pins {
                match self.pins[pin.index()].direction {
                    PinDirection::Output => start_points.push(StartPoint {
                        pin: *pin,
                        kind: PointKind::Blackbox,
                        clock_pin: None,
                        clock_rising: true,
                    }),
                    PinDirection::Input => end_points.push(EndPoint {
                        pin: *pin,
                        kind: PointKind::Blackbox,
                        clock_pin: None,
                        clock_rising: true,
                    }),
                }
            }
        }

        let mut graph = TimingGraph {
            fanin: vec![Vec::new(); self.pins.len()],
            fanout: vec![Vec::new(); self.pins.len()],
            pins: self.pins,
            arcs: self.arcs,
            start_points,
            end_points,
            clock_pins,
            order: Vec::new(),
            loops: Vec::new(),
            notes: self.notes,
        };
        for (i, arc) in graph.arcs.iter().enumerate() {
            graph.fanout[arc.from.index()].push(i);
            graph.fanin[arc.to.index()].push(i);
        }
        let (order, loops) = topological_order(&graph);
        graph.order = order;
        graph.loops = loops;
        graph
    }
}

/// Kahn's algorithm over the whole graph. Pins it cannot place are on a
/// combinational loop; one representative cycle is extracted per group.
fn topological_order(graph: &TimingGraph) -> (Vec<PinId>, Vec<CombLoop>) {
    let n = graph.pins.len();
    let mut indegree: Vec<usize> = (0..n).map(|i| graph.fanin[i].len()).collect();
    let mut ready: Vec<PinId> = (0..n)
        .filter(|i| indegree[*i] == 0)
        .map(|i| PinId(u32::try_from(i).expect("pin count fits in a u32")))
        .collect();
    // Lowest id first, so the order is the same on every run.
    ready.sort_unstable();
    let mut order = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    let mut head = 0;
    while head < ready.len() {
        let pin = ready[head];
        head += 1;
        placed[pin.index()] = true;
        order.push(pin);
        for &arc in &graph.fanout[pin.index()] {
            let to = graph.arcs[arc].to.index();
            indegree[to] -= 1;
            if indegree[to] == 0 {
                ready.push(PinId(u32::try_from(to).expect("pin count fits in a u32")));
            }
        }
    }

    // What is left is one or more cycles. A depth-first search over
    // the unplaced pins finds a back edge, and the stack between the
    // target of that edge and the top of the stack is the cycle.
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        White,
        Gray,
        Black,
    }
    let mut loops = Vec::new();
    let mut reported = vec![false; n];
    let mut mark = vec![Mark::White; n];
    for root in 0..n {
        if placed[root] || mark[root] != Mark::White {
            continue;
        }
        mark[root] = Mark::Gray;
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&(node, next)) = stack.last() {
            if next >= graph.fanout[node].len() {
                mark[node] = Mark::Black;
                stack.pop();
                continue;
            }
            stack.last_mut().expect("the stack is not empty").1 += 1;
            let to = graph.arcs[graph.fanout[node][next]].to.index();
            if placed[to] {
                continue;
            }
            match mark[to] {
                Mark::White => {
                    mark[to] = Mark::Gray;
                    stack.push((to, 0));
                }
                Mark::Gray => {
                    let at = stack
                        .iter()
                        .position(|(nd, _)| *nd == to)
                        .expect("a grey pin is on the stack");
                    let cycle: Vec<usize> = stack[at..].iter().map(|(nd, _)| *nd).collect();
                    if cycle.iter().any(|c| reported[*c]) {
                        continue;
                    }
                    // Start the cycle at its lowest-numbered pin so the
                    // same loop always renders the same way.
                    let rotate = cycle
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, p)| **p)
                        .map_or(0, |(i, _)| i);
                    let pins: Vec<PinId> = cycle[rotate..]
                        .iter()
                        .chain(cycle[..rotate].iter())
                        .map(|i| PinId(u32::try_from(*i).expect("pin count fits in a u32")))
                        .collect();
                    for pin in &pins {
                        reported[pin.index()] = true;
                    }
                    let names = pins.iter().map(|p| graph.name(*p).to_owned()).collect();
                    let span = graph.pin(pins[0]).span;
                    loops.push(CombLoop { pins, names, span });
                }
                Mark::Black => {}
            }
        }
    }
    loops.sort_by_key(|l| l.pins[0]);
    (order, loops)
}

// ---------------------------------------------------------------------------
// Flattening

/// Flattens `top` on a copy of `design` and returns the resulting
/// module, so timing can be run over a hierarchical design without
/// disturbing the caller's.
///
/// Instances that survive flattening — black boxes, unresolved targets
/// and anything marked `keep_hierarchy` — stay in the module, and
/// [`TimingGraph::build`] treats them as black-box boundaries.
pub fn flatten_for_timing(design: &Design, top: ModuleId) -> Result<Module, Diagnostics> {
    let mut copy = design.clone();
    let options = FlattenOptions {
        // Timing does not care about the designer's wish to keep a block
        // whole; it needs every arc.
        keep_hierarchy_attr: false,
        ..FlattenOptions::default()
    };
    copy.flatten(top, &options)?;
    Ok(copy.module(top).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::types::Type;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("timing-test", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// `clk`, `d` -> ff -> `and` with `b` -> ff2 -> `q`.
    fn two_flops() -> Module {
        let mut b = ModuleBuilder::new("two", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bit());
        let other = b.input("b", Type::bit());
        let q = b.output("q", Type::bit());
        let mid = b.add_net("mid", Type::bit());
        let gated = b.add_net("gated", Type::bit());
        let (ce, de) = (b.net(clk), b.net(d));
        b.cell(
            "ff1",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![("clk".into(), ce), ("d".into(), de)],
            vec![("q".into(), mid)],
        );
        let (me, be) = (b.net(mid), b.net(other));
        b.cell2("g", CellKind::And, me, be, gated);
        let ge = b.net(gated);
        b.cell(
            "ff2",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![("clk".into(), ce), ("d".into(), ge)],
            vec![("q".into(), q)],
        );
        b.finish()
    }

    #[test]
    fn builds_pins_and_arcs() {
        let m = two_flops();
        let g = TimingGraph::build(&m);
        // Ports: clk d b q. Cells: ff1 (clk d q), g (a b y), ff2 (clk d q).
        assert_eq!(g.pins.len(), 4 + 3 + 3 + 3);
        assert!(g.pin_by_name("ff1/q").is_some());
        assert!(g.pin_by_name("nope").is_none());
        // Cell arcs: ff1 clk->q, g a->y, g b->y, ff2 clk->q.
        let cell_arcs = g.arcs.iter().filter(|a| a.kind == ArcKind::Cell).count();
        assert_eq!(cell_arcs, 4);
        // No d->q arc anywhere: the flops break the graph.
        let d_to_q = g
            .arcs
            .iter()
            .any(|a| g.name(a.from).ends_with("/d") && g.name(a.to).ends_with("/q"));
        assert!(!d_to_q);
        // clk fans out to both flops' clock pins.
        let clk = g.pin_by_name("clk").unwrap();
        assert_eq!(g.fanout(clk).count(), 2);
        assert_eq!(g.fanin_indices(clk).len(), 0);
        assert!(g.loops.is_empty());
        assert_eq!(g.order.len(), g.pins.len());
        // Start points: clk, d, b, ff1/q, ff2/q.
        let mut starts: Vec<&str> = g.start_points.iter().map(|s| g.name(s.pin)).collect();
        starts.sort_unstable();
        assert_eq!(starts, ["b", "clk", "d", "ff1/q", "ff2/q"]);
        let mut ends: Vec<&str> = g.end_points.iter().map(|e| g.name(e.pin)).collect();
        ends.sort_unstable();
        assert_eq!(ends, ["ff1/d", "ff2/d", "q"]);
        assert_eq!(g.clock_pins.len(), 2);
        assert!(g.is_start(g.pin_by_name("ff1/q").unwrap()));
        assert!(g.is_end(g.pin_by_name("ff2/d").unwrap()));
        assert_eq!(
            g.driver_of(m.net_by_name("mid").unwrap()),
            g.pin_by_name("ff1/q")
        );
        assert_eq!(ArcKind::Net.as_str(), "net");
        // The end point knows which clock pin it is checked against.
        let end = g
            .end_points
            .iter()
            .find(|e| g.name(e.pin) == "ff2/d")
            .unwrap();
        assert_eq!(end.clock_pin, g.pin_by_name("ff2/clk"));
        assert_eq!(end.kind, PointKind::Sequential);
    }

    #[test]
    fn annotates_with_the_unit_model() {
        let m = two_flops();
        let mut g = TimingGraph::build(&m);
        let model = super::super::delay::UnitModel::new();
        let slews = g.annotate(&m, &model, super::super::delay::Transition::ZERO, 0.0, 0.0);
        assert_eq!(slews.len(), g.pins.len());
        for arc in &g.arcs {
            let expected = if arc.kind == ArcKind::Cell { 1.0 } else { 0.0 };
            assert!((arc.delay.max_delay.rise - expected).abs() < 1e-9);
        }
        assert!(g.notes.iter().any(|n| n == "annotated with unit"));
    }

    #[test]
    fn reports_a_combinational_loop_instead_of_hanging() {
        let mut b = ModuleBuilder::new("loopy", span());
        let a = b.input("a", Type::bit());
        let y = b.output("y", Type::bit());
        let t = b.add_net("t", Type::bit());
        let ae = b.net(a);
        let te = b.net(t);
        // t = a & y ; y = not t  -> a cycle through the two cells.
        let ye = b.net(y);
        b.cell2("g1", CellKind::And, ae, ye, t);
        b.cell(
            "g2",
            CellKind::Not,
            vec![("a".into(), te)],
            vec![("y".into(), y)],
        );
        let m = b.finish();
        let g = TimingGraph::build(&m);
        assert_eq!(g.loops.len(), 1);
        assert!(g.loops[0].names.iter().any(|n| n == "g1/y"));
        assert!(g.loops[0].names.iter().any(|n| n == "g2/y"));
        // The pins on the loop are the only ones missing from the order.
        assert!(g.order.len() < g.pins.len());
    }

    #[test]
    fn assignments_and_multiple_drivers() {
        let mut b = ModuleBuilder::new("asg", span());
        let a = b.input("a", Type::bits(8));
        let y = b.output("y", Type::bit());
        let ae = b.net(a);
        let s = b.slice(ae, 7, 7);
        b.assign(y, s);
        let m = b.finish();
        let g = TimingGraph::build(&m);
        assert!(g.pin_by_name("y.in").is_some());
        // The assignment drives the port net, so the port pin is the
        // load and the assignment's own output pin is the driver; the
        // two would collide on the name `y`, and the second one is
        // renumbered.
        assert!(g.pin_by_name("y").is_some());
        assert!(g.pin_by_name("y#2").is_some());
        assert!(g.notes.is_empty());
        // One net arc, from the assignment to the port.
        assert_eq!(g.arcs.iter().filter(|a| a.kind == ArcKind::Net).count(), 2);
    }

    #[test]
    fn flattens_a_hierarchical_design() {
        let mut design = Design::new();
        let leaf = {
            let mut b = ModuleBuilder::new("leaf", span());
            let a = b.input("a", Type::bit());
            let y = b.output("y", Type::bit());
            let ae = b.net(a);
            b.cell(
                "inv",
                CellKind::Not,
                vec![("a".into(), ae)],
                vec![("y".into(), y)],
            );
            design.add_module(b.finish())
        };
        let top = {
            let mut b = ModuleBuilder::new("top", span());
            let a = b.input("a", Type::bit());
            let y = b.output("y", Type::bit());
            let (ae, ye) = (b.net(a), b.net(y));
            b.instance(
                "u",
                crate::ir::ModuleRef::Resolved(leaf),
                vec![("a".into(), ae), ("y".into(), ye)],
            );
            design.add_module(b.finish())
        };
        design.top = Some(top);
        let flat = flatten_for_timing(&design, top).expect("flattens");
        assert_eq!(flat.instances.len(), 0);
        assert_eq!(flat.cells.len(), 1);
        let g = TimingGraph::build(&flat);
        assert!(g.notes.is_empty());
        // The unflattened module notes the instance instead.
        let g = TimingGraph::build(design.module(top));
        assert!(g.notes.iter().any(|n| n.contains("not flattened")));
    }

    #[test]
    fn arcs_and_checks_of_every_primitive() {
        assert_eq!(cell_arcs(&CellKind::Not)[0].2, Sense::Negative);
        assert_eq!(cell_arcs(&CellKind::Xor)[0].2, Sense::NonUnate);
        assert_eq!(cell_arcs(&CellKind::Mux).len(), 3);
        assert!(cell_arcs(&CellKind::Blackbox(crate::ir::Name::new("X"))).is_empty());
        assert!(
            cell_arcs(&CellKind::MemWrPort {
                mem: crate::ir::MemoryId(0),
                clocked: true
            })
            .is_empty()
        );
        assert_eq!(
            cell_arcs(&CellKind::MemRdPort {
                mem: crate::ir::MemoryId(0),
                clocked: false
            })[0]
                .0,
            "addr"
        );
        let dff = CellKind::Dff {
            clk_pos: false,
            has_enable: true,
            reset: Some(crate::ir::Reset {
                asynchronous: true,
                active_high: true,
                value: crate::ir::Const::zero(1),
            }),
        };
        // An asynchronous reset is not an ordinary data pin.
        assert_eq!(sequential_checks(&dff).0, ["d", "en"]);
        let dff = CellKind::Dff {
            clk_pos: true,
            has_enable: false,
            reset: Some(crate::ir::Reset {
                asynchronous: false,
                active_high: true,
                value: crate::ir::Const::zero(1),
            }),
        };
        assert_eq!(sequential_checks(&dff).0, ["d", "rst"]);
        assert_eq!(sequential_checks(&CellKind::Dlatch).1, Some("en"));
        assert_eq!(sequential_checks(&CellKind::And).1, None);
        assert_eq!(cell_type_name(&CellKind::And), "and");
        assert_eq!(
            cell_type_name(&CellKind::Blackbox(crate::ir::Name::new("SB_LUT4"))),
            "SB_LUT4"
        );
    }
}
