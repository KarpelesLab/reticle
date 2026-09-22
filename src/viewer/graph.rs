//! Turning a module into the directed graph the schematic lays out.
//!
//! A [`Graph`] is a list of [`Node`]s, each with input pins on its left and
//! output pins on its right, and a list of [`Edge`]s from an output pin to
//! an input pin. Everything the schematic draws is a node: a cell, a
//! sub-module instance, a continuous assignment, a process the module
//! still carries, a module port, and a small constant node for every
//! literal feeding a pin.
//!
//! The graph is built from the driver relation. Each net has at most one
//! driver, found in this order: cells, instances, processes, continuous
//! assignments, and only then module ports, so a bidirectional port does
//! not shadow the tri-state driving it from inside. An input pin whose net
//! has no driver keeps its label and simply has no wire, which is how a
//! dangling net shows up.
//!
//! Input expressions are not drawn as trees: an expression feeding a pin
//! contributes one edge per distinct net it reads, plus one constant node
//! per distinct literal, and the pin is labelled with the expression text.
//! That keeps a cell-form netlist (whose inputs are nets, slices and
//! concatenations) readable without drawing the operator graph synthesis
//! has already turned into cells. A module still in process form is drawn
//! at the same grain: one box per process, with the nets it reads on the
//! left and the nets it writes on the right.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::expr::operands;
use crate::ir::walk::walk_block;
use crate::ir::{
    Cell, CellId, CellKind, Design, ExprId, ExprKind, InstanceId, Module, ModuleId, ModuleRef,
    NetId, Polarity, PortDir, ProcessId, ProcessKind, Span, StmtKind,
};

/// A node's index in [`Graph::nodes`].
pub type NodeId = usize;

/// The longest label a pin or a title keeps; longer text is cut with an
/// ellipsis so one wide expression cannot stretch a whole column.
const MAX_LABEL: usize = 28;

/// What a node stands for in the module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A module port; `InOut` carries both an input and an output pin.
    Port {
        /// The port's index in `Module::ports`.
        index: usize,
        /// Its direction.
        dir: PortDir,
    },
    /// A cell of the cell form.
    Cell(CellId),
    /// A sub-module instantiation.
    Instance(InstanceId),
    /// A process the module still carries (a module that has not been
    /// synthesised, or a construct synthesis could not lower).
    Process(ProcessId),
    /// A continuous assignment, by index in `Module::assigns`.
    Assign(usize),
    /// A literal feeding one pin.
    Const,
}

/// How a node is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A plain box: combinational logic.
    Comb,
    /// A heavier box with a clock mark: state, which the eye should find
    /// first.
    Seq,
    /// A tag on the edge of the canvas.
    Port,
    /// A double-bordered box: an instance, a black box or a process.
    Instance,
    /// A small triangle: a constant.
    Const,
}

impl Shape {
    /// The CSS class the schematic gives a node of this shape.
    pub fn class(self) -> &'static str {
        match self {
            Shape::Comb => "node comb",
            Shape::Seq => "node seq",
            Shape::Port => "node port",
            Shape::Instance => "node inst",
            Shape::Const => "node konst",
        }
    }
}

/// One connection point of a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The port name on the cell or instance (`a`, `clk`, `q`); empty when
    /// the object has no port names, as for a process.
    pub name: String,
    /// What is connected, as text: a net name, a slice, an expression.
    pub label: String,
    /// The net the pin carries, when it carries exactly one.
    pub net: Option<NetId>,
    /// That net's name, for highlighting.
    pub net_name: Option<String>,
    /// Width in bits, when known.
    pub width: Option<u32>,
}

/// One box of the schematic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// What it stands for.
    pub kind: NodeKind,
    /// How it is drawn.
    pub shape: Shape,
    /// The headline: a cell kind keyword, `port in`, a module name.
    pub title: String,
    /// The object's own name; empty for a constant.
    pub name: String,
    /// Input pins, top to bottom.
    pub inputs: Vec<Pin>,
    /// Output pins, top to bottom.
    pub outputs: Vec<Pin>,
    /// Where the object came from.
    pub span: Span,
    /// Label / value pairs for the detail panel, in display order.
    pub details: Vec<(String, String)>,
}

impl Node {
    /// True when the node holds state, which the schematic draws
    /// differently.
    pub fn is_sequential(&self) -> bool {
        self.shape == Shape::Seq
    }

    /// The text a search box matches against: the name, the title and
    /// every net the node touches, lowercased.
    pub fn search_text(&self) -> String {
        let mut parts = vec![self.name.to_lowercase(), self.title.to_lowercase()];
        for pin in self.inputs.iter().chain(&self.outputs) {
            parts.push(pin.label.to_lowercase());
            if !pin.name.is_empty() {
                parts.push(pin.name.to_lowercase());
            }
        }
        parts.sort();
        parts.dedup();
        parts.retain(|p| !p.is_empty());
        parts.join(" ")
    }
}

/// A wire from one node's output pin to another's input pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    /// The driving node.
    pub from: NodeId,
    /// Index into the driving node's `outputs`.
    pub from_pin: usize,
    /// The driven node.
    pub to: NodeId,
    /// Index into the driven node's `inputs`.
    pub to_pin: usize,
    /// The net carried; `None` for the wire out of a constant node, which
    /// has no net of its own.
    pub net: Option<NetId>,
    /// The wire's label: the net name, or the literal.
    pub label: String,
    /// Width in bits, when known; more than one bit is drawn as a bus.
    pub width: Option<u32>,
}

/// A module as a directed graph of nodes and wires.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    /// Every node, in a deterministic order: ports in declaration order,
    /// then cells, instances, processes and assigns in arena order, then
    /// one constant node per literal, in the order their pins were
    /// visited.
    pub nodes: Vec<Node>,
    /// Every wire, in the order the consuming pins were visited.
    pub edges: Vec<Edge>,
}

/// What an input expression reads.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    /// A net, which becomes a wire when the net has a driver.
    Net(NetId),
    /// A literal, which becomes a constant node.
    Const(String),
}

impl Graph {
    /// Builds the graph of one module of `design`.
    ///
    /// The whole design is needed, not just the module, because an
    /// instance's port directions and the name it targets live in the
    /// module it refers to.
    pub fn of_module(design: &Design, id: ModuleId) -> Graph {
        let module = design.module(id);
        let mut nodes: Vec<Node> = Vec::new();
        // Input-pin sources, indexed the same way as `nodes`.
        let mut pin_sources: Vec<Vec<Vec<Source>>> = Vec::new();
        // Output pins offering to drive a net. Ports offer last so an
        // internal driver of a bidirectional port wins.
        let mut offers: Vec<(NetId, NodeId, usize)> = Vec::new();
        let mut port_offers: Vec<(NetId, NodeId, usize)> = Vec::new();

        let push = |nodes: &mut Vec<Node>,
                    pin_sources: &mut Vec<Vec<Vec<Source>>>,
                    offers: &mut Vec<(NetId, NodeId, usize)>,
                    node: Node,
                    sources: Vec<Vec<Source>>| {
            let id = nodes.len();
            for (pin, out) in node.outputs.iter().enumerate() {
                if let Some(net) = out.net {
                    offers.push((net, id, pin));
                }
            }
            nodes.push(node);
            pin_sources.push(sources);
        };

        for (index, port) in module.ports.iter().enumerate() {
            let (node, sources) = port_node(module, index, port.dir);
            let id = nodes.len();
            if !node.outputs.is_empty() {
                port_offers.push((port.net, id, 0));
            }
            nodes.push(node);
            pin_sources.push(sources);
        }
        for (id, cell) in module.cells.iter() {
            let (node, sources) = cell_node(module, id, cell);
            push(&mut nodes, &mut pin_sources, &mut offers, node, sources);
        }
        let internal = internally_driven(module);
        for (id, _) in module.instances.iter() {
            let (node, sources) = instance_node(design, module, id, &internal);
            push(&mut nodes, &mut pin_sources, &mut offers, node, sources);
        }
        for (id, _) in module.processes.iter() {
            let (node, sources) = process_node(module, id);
            push(&mut nodes, &mut pin_sources, &mut offers, node, sources);
        }
        for index in 0..module.assigns.len() {
            let (node, sources) = assign_node(module, index);
            push(&mut nodes, &mut pin_sources, &mut offers, node, sources);
        }

        let mut driver: BTreeMap<NetId, (NodeId, usize)> = BTreeMap::new();
        for (net, node, pin) in offers.into_iter().chain(port_offers) {
            driver.entry(net).or_insert((node, pin));
        }

        let mut graph = Graph {
            nodes,
            edges: Vec::new(),
        };
        for (to, pins) in pin_sources.into_iter().enumerate() {
            for (to_pin, sources) in pins.into_iter().enumerate() {
                for source in sources {
                    match source {
                        Source::Net(net) => {
                            let Some(&(from, from_pin)) = driver.get(&net) else {
                                continue;
                            };
                            let info = &module.nets[net];
                            graph.edges.push(Edge {
                                from,
                                from_pin,
                                to,
                                to_pin,
                                net: Some(net),
                                label: info.name.to_string(),
                                width: info.ty.width(),
                            });
                        }
                        Source::Const(text) => {
                            let from = graph.nodes.len();
                            let span = graph.nodes[to].span;
                            graph.nodes.push(const_node(&text, span));
                            graph.edges.push(Edge {
                                from,
                                from_pin: 0,
                                to,
                                to_pin,
                                net: None,
                                label: text,
                                width: None,
                            });
                        }
                    }
                }
            }
        }
        graph
    }

    /// Every edge entering `node`.
    pub fn incoming(&self, node: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.to == node)
    }

    /// Every edge leaving `node`.
    pub fn outgoing(&self, node: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.from == node)
    }
}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

fn port_node(module: &Module, index: usize, dir: PortDir) -> (Node, Vec<Vec<Source>>) {
    let port = &module.ports[index];
    let net = &module.nets[port.net];
    let pin = Pin {
        name: port.name.to_string(),
        label: net.name.to_string(),
        net: Some(port.net),
        net_name: Some(net.name.to_string()),
        width: net.ty.width(),
    };
    let (inputs, outputs, sources) = match dir {
        PortDir::In => (Vec::new(), vec![pin], Vec::new()),
        PortDir::Out => (vec![pin], Vec::new(), vec![vec![Source::Net(port.net)]]),
        PortDir::InOut => (
            vec![pin.clone()],
            vec![pin],
            vec![vec![Source::Net(port.net)]],
        ),
    };
    let mut details = vec![
        ("direction".to_owned(), dir.keyword().to_owned()),
        ("net".to_owned(), net.name.to_string()),
        ("type".to_owned(), net.ty.to_string()),
    ];
    details.extend(attr_details(&net.attrs));
    (
        Node {
            kind: NodeKind::Port { index, dir },
            shape: Shape::Port,
            title: format!("port {}", dir.keyword()),
            name: port.name.to_string(),
            inputs,
            outputs,
            span: port.span,
            details,
        },
        sources,
    )
}

fn cell_node(module: &Module, id: CellId, cell: &Cell) -> (Node, Vec<Vec<Source>>) {
    let mut sources = Vec::with_capacity(cell.inputs.len());
    let inputs = cell
        .inputs
        .iter()
        .map(|(name, expr)| {
            sources.push(expr_sources(module, *expr));
            Pin {
                name: name.to_string(),
                label: truncate(&expr_label(module, *expr)),
                net: module.exprs[*expr].as_net(),
                net_name: module.exprs[*expr]
                    .as_net()
                    .map(|n| module.nets[n].name.to_string()),
                width: module.exprs[*expr].ty.width(),
            }
        })
        .collect();
    let outputs = cell
        .outputs
        .iter()
        .map(|(name, net)| Pin {
            name: name.to_string(),
            label: module.nets[*net].name.to_string(),
            net: Some(*net),
            net_name: Some(module.nets[*net].name.to_string()),
            width: module.nets[*net].ty.width(),
        })
        .collect();

    let shape = match &cell.kind {
        CellKind::Dff { .. }
        | CellKind::Dlatch
        | CellKind::MemRdPort { .. }
        | CellKind::MemWrPort { .. } => Shape::Seq,
        CellKind::Blackbox(_) => Shape::Instance,
        _ => Shape::Comb,
    };
    let mut details = vec![("cell".to_owned(), cell.kind.keyword().to_owned())];
    match &cell.kind {
        CellKind::Dff {
            clk_pos,
            has_enable,
            reset,
        } => {
            details.push((
                "clock".to_owned(),
                if *clk_pos { "posedge" } else { "negedge" }.to_owned(),
            ));
            if *has_enable {
                details.push(("enable".to_owned(), "yes".to_owned()));
            }
            if let Some(r) = reset {
                details.push((
                    "reset".to_owned(),
                    format!(
                        "{} {}, value {}",
                        if r.asynchronous { "async" } else { "sync" },
                        if r.active_high {
                            "active-high"
                        } else {
                            "active-low"
                        },
                        const_text(&r.value)
                    ),
                ));
            }
        }
        CellKind::Lut { k, init } => {
            details.push(("inputs".to_owned(), k.to_string()));
            details.push(("init".to_owned(), const_text(init)));
        }
        CellKind::MemRdPort { mem, clocked } | CellKind::MemWrPort { mem, clocked } => {
            details.push(("memory".to_owned(), module.memories[*mem].name.to_string()));
            details.push((
                "timing".to_owned(),
                if *clocked { "clocked" } else { "asynchronous" }.to_owned(),
            ));
        }
        CellKind::Blackbox(name) => details.push(("module".to_owned(), name.to_string())),
        _ => {}
    }
    for (name, value) in cell.params.iter() {
        details.push((format!("param {name}"), value.to_string()));
    }
    details.extend(attr_details(&cell.attrs));

    (
        Node {
            kind: NodeKind::Cell(id),
            shape,
            title: cell_title(&cell.kind),
            name: cell.name.to_string(),
            inputs,
            outputs,
            span: cell.span,
            details,
        },
        sources,
    )
}

/// The headline a cell shows: its keyword, with the detail that makes two
/// cells of the same kind tell apart.
fn cell_title(kind: &CellKind) -> String {
    match kind {
        CellKind::Lut { k, .. } => format!("lut{k}"),
        CellKind::Blackbox(name) => name.to_string(),
        CellKind::Dff { clk_pos, .. } => {
            format!("dff {}", if *clk_pos { "pos" } else { "neg" })
        }
        other => other.keyword().to_owned(),
    }
}

fn instance_node(
    design: &Design,
    module: &Module,
    id: InstanceId,
    internal: &BTreeSet<NetId>,
) -> (Node, Vec<Vec<Source>>) {
    let inst = &module.instances[id];
    let target = inst.module.id().map(|m| design.module(m));

    let mut sources = Vec::new();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for (port, expr) in &inst.connections {
        let expr = *expr;
        let net = module.exprs[expr].as_net();
        let pin = Pin {
            name: port.to_string(),
            label: truncate(&expr_label(module, expr)),
            net,
            net_name: net.map(|n| module.nets[n].name.to_string()),
            width: module.exprs[expr].ty.width(),
        };
        // A resolved target tells each connection's direction. A black box
        // does not, so a connection that is a bare net nothing inside the
        // module drives is taken to be an output.
        let dir = match target.and_then(|m| m.port(port.as_str())) {
            Some(p) => p.dir,
            None => match net {
                Some(net) if !internal.contains(&net) => PortDir::Out,
                _ => PortDir::In,
            },
        };
        if dir == PortDir::Out {
            outputs.push(pin);
        } else {
            sources.push(expr_sources(module, expr));
            inputs.push(pin);
        }
    }

    let name = match (&inst.module, target) {
        (_, Some(m)) => m.name.to_string(),
        (ModuleRef::Unresolved(name), None) => name.to_string(),
        (ModuleRef::Resolved(_), None) => String::new(),
    };
    let mut details = vec![("module".to_owned(), name.clone())];
    if target.is_none() {
        details.push(("binding".to_owned(), "black box".to_owned()));
    }
    for (key, value) in inst.params.iter() {
        details.push((format!("param {key}"), value.to_string()));
    }
    details.extend(attr_details(&inst.attrs));

    (
        Node {
            kind: NodeKind::Instance(id),
            shape: Shape::Instance,
            title: truncate(&name),
            name: inst.name.to_string(),
            inputs,
            outputs,
            span: inst.span,
            details,
        },
        sources,
    )
}

fn process_node(module: &Module, id: ProcessId) -> (Node, Vec<Vec<Source>>) {
    let process = &module.processes[id];
    let mut written: BTreeSet<NetId> = BTreeSet::new();
    let mut read: Vec<NetId> = Vec::new();
    walk_block(&process.body, &mut |stmt| match &stmt.kind {
        StmtKind::Assign { target, .. } => written.extend(target.nets()),
        StmtKind::For { init, step, .. } => {
            for (lv, _) in init.iter().chain(step.iter()) {
                written.extend(lv.nets());
            }
        }
        _ => {}
    });
    walk_block(&process.body, &mut |stmt| {
        crate::ir::walk::stmt_exprs(stmt, &mut |expr| read.extend(expr_nets(module, expr)));
    });
    let mut trigger: Vec<NetId> = Vec::new();
    match &process.kind {
        ProcessKind::Sequential { clocks, resets } => {
            trigger.extend(clocks.iter().chain(resets).map(|e| e.net));
        }
        ProcessKind::Sensitive(nets) => trigger.extend(nets.iter().copied()),
        ProcessKind::Comb | ProcessKind::Initial | ProcessKind::Free => {}
    }

    let mut inputs = Vec::new();
    let mut sources = Vec::new();
    let mut seen: BTreeSet<NetId> = BTreeSet::new();
    for net in trigger.iter().copied().chain(read) {
        if written.contains(&net) || !seen.insert(net) {
            continue;
        }
        let info = &module.nets[net];
        inputs.push(Pin {
            name: if trigger.contains(&net) {
                "trigger"
            } else {
                ""
            }
            .to_owned(),
            label: info.name.to_string(),
            net: Some(net),
            net_name: Some(info.name.to_string()),
            width: info.ty.width(),
        });
        sources.push(vec![Source::Net(net)]);
    }
    let outputs = written
        .iter()
        .map(|net| Pin {
            name: String::new(),
            label: module.nets[*net].name.to_string(),
            net: Some(*net),
            net_name: Some(module.nets[*net].name.to_string()),
            width: module.nets[*net].ty.width(),
        })
        .collect();

    let mut details = vec![("process".to_owned(), process.kind.keyword().to_owned())];
    if let ProcessKind::Sequential { clocks, resets } = &process.kind {
        for edge in clocks {
            details.push((
                "clock".to_owned(),
                format!(
                    "{} {}",
                    polarity_word(edge.polarity),
                    module.nets[edge.net].name
                ),
            ));
        }
        for edge in resets {
            details.push((
                "async control".to_owned(),
                format!(
                    "{} {}",
                    polarity_word(edge.polarity),
                    module.nets[edge.net].name
                ),
            ));
        }
    }
    details.extend(attr_details(&process.attrs));

    (
        Node {
            kind: NodeKind::Process(id),
            shape: match process.kind {
                ProcessKind::Sequential { .. } => Shape::Seq,
                _ => Shape::Instance,
            },
            title: format!("process {}", process.kind.keyword()),
            name: process
                .name
                .as_ref()
                .map_or(String::new(), |n| n.to_string()),
            inputs,
            outputs,
            span: process.span,
            details,
        },
        sources,
    )
}

fn assign_node(module: &Module, index: usize) -> (Node, Vec<Vec<Source>>) {
    let assign = &module.assigns[index];
    let label = truncate(&expr_label(module, assign.value));
    let inputs = vec![Pin {
        name: String::new(),
        label: label.clone(),
        net: module.exprs[assign.value].as_net(),
        net_name: module.exprs[assign.value]
            .as_net()
            .map(|n| module.nets[n].name.to_string()),
        width: module.exprs[assign.value].ty.width(),
    }];
    let sources = vec![expr_sources(module, assign.value)];
    let outputs = assign
        .target
        .nets()
        .into_iter()
        .map(|net| Pin {
            name: String::new(),
            label: module.nets[net].name.to_string(),
            net: Some(net),
            net_name: Some(module.nets[net].name.to_string()),
            width: module.nets[net].ty.width(),
        })
        .collect();
    let mut details = vec![("assign".to_owned(), label)];
    details.extend(attr_details(&assign.attrs));
    (
        Node {
            kind: NodeKind::Assign(index),
            shape: Shape::Comb,
            title: "assign".to_owned(),
            name: String::new(),
            inputs,
            outputs,
            span: assign.span,
            details,
        },
        sources,
    )
}

/// The word naming an edge's polarity.
fn polarity_word(polarity: Polarity) -> &'static str {
    match polarity {
        Polarity::Pos => "posedge",
        Polarity::Neg => "negedge",
        Polarity::Any => "any edge",
    }
}

fn const_node(text: &str, span: Span) -> Node {
    Node {
        kind: NodeKind::Const,
        shape: Shape::Const,
        title: text.to_owned(),
        name: String::new(),
        inputs: Vec::new(),
        outputs: vec![Pin {
            name: String::new(),
            label: text.to_owned(),
            net: None,
            net_name: None,
            width: None,
        }],
        span,
        details: vec![("constant".to_owned(), text.to_owned())],
    }
}

/// Attributes rendered for the detail panel.
fn attr_details(attrs: &crate::ir::Attrs) -> Vec<(String, String)> {
    attrs
        .iter()
        .map(|(name, value)| (format!("attr {name}"), value.to_string()))
        .collect()
}

/// Every net driven from inside the module by something other than a port
/// or an instance.
fn internally_driven(module: &Module) -> BTreeSet<NetId> {
    let mut out = BTreeSet::new();
    for (_, cell) in module.cells.iter() {
        out.extend(cell.outputs.iter().map(|(_, net)| *net));
    }
    for assign in &module.assigns {
        out.extend(assign.target.nets());
    }
    for (_, process) in module.processes.iter() {
        walk_block(&process.body, &mut |stmt| {
            if let StmtKind::Assign { target, .. } = &stmt.kind {
                out.extend(target.nets());
            }
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// Every net an expression reads, operands included.
pub fn expr_nets(module: &Module, root: ExprId) -> Vec<NetId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(expr) = module.exprs.get(id) else {
            continue;
        };
        if let Some(net) = expr.as_net() {
            out.push(net);
        }
        stack.extend(operands(&expr.kind));
    }
    out
}

/// What an input expression contributes to the graph: each net it reads
/// once, and each distinct literal once, in source order.
fn expr_sources(module: &Module, root: ExprId) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    collect_sources(module, root, &mut out);
    let mut seen_nets = BTreeSet::new();
    let mut seen_consts = BTreeSet::new();
    out.retain(|s| match s {
        Source::Net(net) => seen_nets.insert(*net),
        Source::Const(text) => seen_consts.insert(text.clone()),
    });
    out
}

fn collect_sources(module: &Module, id: ExprId, out: &mut Vec<Source>) {
    let Some(expr) = module.exprs.get(id) else {
        return;
    };
    match &expr.kind {
        ExprKind::Const(c) => out.push(Source::Const(const_text(c))),
        ExprKind::String(s) => out.push(Source::Const(format!("{s:?}"))),
        ExprKind::Net(net) => out.push(Source::Net(*net)),
        kind => {
            for operand in operands(kind) {
                collect_sources(module, operand, out);
            }
        }
    }
}

/// A short rendering of an expression, close to the `.rtl` text format.
pub fn expr_label(module: &Module, id: ExprId) -> String {
    let Some(expr) = module.exprs.get(id) else {
        return "?".to_owned();
    };
    let sub = |e: &ExprId| expr_label(module, *e);
    match &expr.kind {
        ExprKind::Const(c) => const_text(c),
        ExprKind::String(s) => format!("{s:?}"),
        ExprKind::Net(net) => module.nets[*net].name.to_string(),
        ExprKind::Slice { base, hi, lo } if hi == lo => format!("{}[{hi}]", sub(base)),
        ExprKind::Slice { base, hi, lo } => format!("{}[{hi}:{lo}]", sub(base)),
        ExprKind::Index { base, index } => format!("{}[{}]", sub(base), sub(index)),
        ExprKind::IndexedSlice {
            base,
            offset,
            width,
            up,
        } => format!(
            "{}[{} {}: {width}]",
            sub(base),
            sub(offset),
            if *up { "+" } else { "-" }
        ),
        ExprKind::Concat(parts) => {
            let items: Vec<String> = parts.iter().map(sub).collect();
            format!("{{{}}}", items.join(", "))
        }
        ExprKind::Replicate { count, expr } => format!("{{{count}{{{}}}}}", sub(expr)),
        ExprKind::Unary { op, expr } => format!("{}({})", op.name(), sub(expr)),
        ExprKind::Binary { op, lhs, rhs } => {
            format!("{}({}, {})", op.name(), sub(lhs), sub(rhs))
        }
        ExprKind::Ternary { cond, then_, else_ } => {
            format!("{} ? {} : {}", sub(cond), sub(then_), sub(else_))
        }
        ExprKind::Resize {
            expr,
            width,
            signed,
        } => format!(
            "{} as {}{width}",
            sub(expr),
            if *signed { 's' } else { 'u' }
        ),
        ExprKind::MemRead { mem, addr } => {
            format!("{}[{}]", module.memories[*mem].name, sub(addr))
        }
        ExprKind::Call { name, args } => {
            let items: Vec<String> = args.iter().map(sub).collect();
            format!("{name}({})", items.join(", "))
        }
    }
}

/// A constant as the `.rtl` text format writes it: decimal when the value
/// is known and fits, binary otherwise.
///
/// The schematic shows the same spelling the IR text format does, so a
/// literal on a wire and the same literal in a `.rtl` file read alike.
pub fn const_text(value: &crate::ir::Const) -> String {
    let sign = if value.is_signed() { "s" } else { "" };
    if value.has_unknown() {
        format!("{}'{sign}b{}", value.width(), value.to_binary_string())
    } else if let Some(v) = value.to_u64() {
        format!("{}'{sign}d{v}", value.width())
    } else {
        value.to_verilog_literal()
    }
}

/// Cuts a label to [`MAX_LABEL`] characters, marking what was dropped.
fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_LABEL {
        return text.to_owned();
    }
    let kept: String = text.chars().take(MAX_LABEL - 1).collect();
    format!("{kept}\u{2026}")
}

// ---------------------------------------------------------------------------
// Depth
// ---------------------------------------------------------------------------

/// The longest chain of combinational cells between state, ports or memory
/// ports, the same estimate `synth::report` prints.
///
/// Registers, memory ports, instances and module ports start a chain at
/// zero, so this measures logic between state. It is an estimate: the
/// cells are generic, so one `add` counts as one level although it becomes
/// many gates. A combinational loop stops the walk rather than hanging.
pub fn combinational_depth(module: &Module) -> usize {
    let mut depth_of: Vec<usize> = vec![0; module.nets.len()];
    let mut driver: Vec<Option<CellId>> = vec![None; module.nets.len()];
    for (id, cell) in module.cells.iter() {
        if !cell.kind.is_combinational() {
            continue;
        }
        for (_, net) in &cell.outputs {
            driver[net.index()] = Some(id);
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Visiting,
        Done,
    }
    let mut mark = vec![Mark::New; module.nets.len()];
    let mut stack: Vec<(NetId, bool)> = Vec::new();
    for net in module.nets.ids() {
        if mark[net.index()] != Mark::New {
            continue;
        }
        stack.push((net, false));
        while let Some((net, returning)) = stack.pop() {
            if returning {
                let mut best = 0;
                if let Some(cell) = driver[net.index()] {
                    for (_, expr) in &module.cells[cell].inputs {
                        for input in expr_nets(module, *expr) {
                            best = best.max(depth_of[input.index()]);
                        }
                    }
                    best += 1;
                }
                depth_of[net.index()] = best;
                mark[net.index()] = Mark::Done;
                continue;
            }
            match mark[net.index()] {
                Mark::Done | Mark::Visiting => continue,
                Mark::New => {}
            }
            mark[net.index()] = Mark::Visiting;
            stack.push((net, true));
            if let Some(cell) = driver[net.index()] {
                for (_, expr) in &module.cells[cell].inputs {
                    for input in expr_nets(module, *expr) {
                        if mark[input.index()] == Mark::New {
                            stack.push((input, false));
                        }
                    }
                }
            }
        }
    }
    depth_of.into_iter().max().unwrap_or(0)
}
