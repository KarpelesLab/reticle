//! Vivado's constraints language, read directly.
//!
//! `reticle fpga --constraints board.xdc` reads an XDC file. It is Tcl, of
//! which Vivado takes a subset: [`read_xdc`] evaluates that subset against a
//! view of the design ([`XdcDesign`]) and produces an [`XdcReading`], what the
//! file says in Vivado's own terms, with every query already resolved to the
//! objects it names. [`XdcReading::constraints`] then says what of that the
//! flow can use, as a [`Constraints`]. The two steps are separate because they
//! have different oracles: the reading is what `tests/fpga_xdc.rs` holds to
//! what Vivado 2026.1 answered for the same file (`testdata/fpga/vivado/xdc/`),
//! and the translation is what Reticle does about it, on which Vivado has no
//! opinion.
//!
//! # What is said about what cannot be honoured
//!
//! Nothing is dropped in silence. There are three levels, and each diagnostic
//! names the command:
//!
//! - **[`XDC_REFUSED`], an error,** when going on could make a wrong
//!   bitstream: a pin or an electrical property that could not be read, a
//!   command that could hold one (`foreach`, `if`), a pad setting the flow
//!   does not make, a configuration option that changes what the part does.
//! - **[`XDC_NOT_HONOURED`], a warning,** when the design comes out the same
//!   logic and only its timing or placement may differ from what was asked.
//! - **[`XDC_NO_EFFECT`], a note,** when the file's statement is true of this
//!   flow already: `set_clock_groups`, since no path between two clocks is
//!   timed here at all, or `CFGBVS`, which changed no byte of Vivado's own
//!   bitstream when measured.
//!
//! # Whose names
//!
//! A query answers in Vivado's namespace, because a file is written against
//! that: the register `q` of `u_sub` is the cell `u_sub/q_reg`, and bit 4 of
//! `reg [7:4] t` is `t_reg[4]`. The design here has not been synthesised when
//! constraints are read, so [`XdcDesign`] works those names out from the
//! source, and only those: registers, instances and nets. A name Vivado's
//! synthesis invents (`a_IBUF[0]_inst`) matches nothing, and the constraint
//! naming it is reported rather than applied.
//!
//! The translation turns them into the names the flattened netlist has:
//! `u_sub.q$ff`, `t$ff$ff0`, and `a$io0` for a bit of a port's pad.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::constraints::{
    ClockDef, Constraints, InputDelay, IoAttrs, MaxDelay, MulticyclePath, Origin, PathSpec,
    PinAssignment, RegionAssignment, SiteRegion, matches_glob, written_nets,
};
use crate::diag::{Diagnostic, Diagnostics, Severity};
use crate::ir::walk::walk_block;
use crate::ir::{Design, Module, ModuleId, ModuleRef, PortDir, ProcessKind, StmtKind};
use crate::source::{SourceId, Span};

/// Diagnostic code for XDC that cannot be read, or whose loss could make a
/// wrong bitstream.
pub const XDC_REFUSED: &str = "F0112";
/// Diagnostic code for XDC that is read and not applied, which can change only
/// timing or placement.
pub const XDC_NOT_HONOURED: &str = "F0113";
/// Diagnostic code for XDC whose statement needs nothing done in this flow.
pub const XDC_NO_EFFECT: &str = "F0114";

/// The net attribute holding a declared range that is not `[width-1:0]`, as
/// `left:right`: the front ends set it, since the IR numbers every vector from
/// zero and Vivado names a bit by the index it was declared with.
pub const DECLARED_RANGE_ATTR: &str = "declared_range";

/// What kind of thing a query returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectClass {
    /// A bit of a top-level port.
    Port,
    /// A register or an instance.
    Cell,
    /// A pin of a cell.
    Pin,
    /// A net.
    Net,
    /// A clock.
    Clock,
    /// A placement block.
    Pblock,
    /// The design itself.
    Design,
    /// An IO bank.
    Bank,
    /// A site of the device.
    Site,
    /// A debug core or a port of one, which Vivado's synthesis inserts.
    Debug,
}

impl ObjectClass {
    /// Vivado's word for it, its `CLASS` property.
    pub fn word(self) -> &'static str {
        match self {
            ObjectClass::Port => "port",
            ObjectClass::Cell => "cell",
            ObjectClass::Pin => "pin",
            ObjectClass::Net => "net",
            ObjectClass::Clock => "clock",
            ObjectClass::Pblock => "pblock",
            ObjectClass::Design => "design",
            ObjectClass::Bank => "io_bank",
            ObjectClass::Site => "site",
            ObjectClass::Debug => "debug_core",
        }
    }
}

/// One object, under the name Vivado has for it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Object {
    /// What it is.
    pub class: ObjectClass,
    /// Its name.
    pub name: String,
}

impl Object {
    fn new(class: ObjectClass, name: impl Into<String>) -> Self {
        Object {
            class,
            name: name.into(),
        }
    }
}

/// A top-level port, as a query sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XdcPort {
    /// Its name.
    pub name: String,
    /// Its direction.
    pub dir: PortDir,
    /// Its width.
    pub width: u32,
    /// The index written first in its declaration, for a vector.
    pub left: i64,
    /// The index written second.
    pub right: i64,
    /// Whether it has indices at all.
    pub vector: bool,
}

impl XdcPort {
    /// The IR's zero-based, least-significant-first offset of declared index
    /// `index`, if it is one of this port's.
    pub fn offset(&self, index: i64) -> Option<u32> {
        offset_in(self.left, self.right, index)
    }

    /// Each bit's name, with its offset, lowest index first.
    fn bits(&self) -> Vec<(String, Option<u32>)> {
        if !self.vector {
            return vec![(self.name.clone(), None)];
        }
        let (low, high) = (self.left.min(self.right), self.left.max(self.right));
        (low..=high)
            .map(|index| (format!("{}[{index}]", self.name), self.offset(index)))
            .collect()
    }
}

/// The offset from the most significant end being `left`.
fn offset_in(left: i64, right: i64, index: i64) -> Option<u32> {
    let (low, high) = (left.min(right), left.max(right));
    if index < low || index > high {
        return None;
    }
    let distance = if left >= right {
        index - right
    } else {
        right - index
    };
    u32::try_from(distance).ok()
}

/// A flip-flop the source will become, under both of its names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XdcRegister {
    /// What Vivado calls it: `u_sub/q_reg`, `t_reg[4]`.
    pub vivado: String,
    /// What the netlist will call it: `u_sub.q$ff`, `t$ff$ff0`.
    pub netlist: String,
    /// The net clocking it, by its name at the top where it can be followed
    /// there.
    pub clock: Option<String>,
    /// Whether the register is one bit, so that its output is a whole net.
    pub scalar_net: Option<String>,
}

/// An instance in the source's hierarchy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XdcInstance {
    /// Its path, in Vivado's spelling: `u_a/u_b`.
    pub path: String,
    /// The name of what it instantiates.
    pub module: String,
    /// True when it is a module of the design with something inside it, not
    /// a leaf a constraint could begin or end a path at.
    pub hierarchical: bool,
    /// Its ports, with the net at each by its name at the top where it can be
    /// followed there. An unconnected pin has `None`.
    pub pins: Vec<(String, Option<String>)>,
}

/// What [`read_xdc`] needs of the design: the names a query can return.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XdcDesign {
    /// The top module's name.
    pub top: String,
    /// The top-level ports.
    pub ports: Vec<XdcPort>,
    /// The registers, sorted by Vivado's name.
    pub registers: Vec<XdcRegister>,
    /// The instances, sorted by path.
    pub instances: Vec<XdcInstance>,
    /// Every net, as `(Vivado's name, the netlist's name)`.
    pub nets: Vec<(String, String)>,
    /// Nets of the top that nothing writes.
    pub unwritten: Vec<String>,
}

impl XdcDesign {
    /// The view of `module` in `design`. This has to be taken before synthesis:
    /// it finds registers in the processes that synthesis replaces.
    pub fn of(design: &Design, module: ModuleId) -> XdcDesign {
        let mut out = XdcDesign::default();
        let Some(top) = design.modules.get(module) else {
            return out;
        };
        out.top = top.name.as_str().to_owned();
        for port in &top.ports {
            let Some(net) = top.nets.get(port.net) else {
                continue;
            };
            let width = net.ty.width().unwrap_or(1);
            let declared = net
                .attrs
                .get(DECLARED_RANGE_ATTR)
                .and_then(crate::ir::AttrValue::as_str)
                .and_then(parse_range);
            let (left, right) = declared.unwrap_or((i64::from(width) - 1, 0));
            out.ports.push(XdcPort {
                name: port.name.as_str().to_owned(),
                dir: port.dir,
                width,
                left,
                right,
                vector: declared.is_some() || width > 1,
            });
        }
        // What Vivado's synthesis calls the buffer's output of an input port,
        // measured for a scalar, is what a clock constraint often names.
        for port in &out.ports {
            if port.dir == PortDir::In && !port.vector {
                for suffix in ["_IBUF", "_IBUF_BUFG"] {
                    out.nets
                        .push((format!("{}{suffix}", port.name), port.name.clone()));
                }
            }
        }
        let written = written_nets(design, top);
        for (id, net) in top.nets.iter() {
            if !written.get(id.index()).copied().unwrap_or(true) {
                out.unwritten.push(net.name.as_str().to_owned());
            }
        }
        out.walk(design, top, "", "", &BTreeMap::new(), 0);
        out.registers.sort_by(|a, b| a.vivado.cmp(&b.vivado));
        out.instances.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    fn walk(
        &mut self,
        design: &Design,
        module: &Module,
        vivado: &str,
        netlist: &str,
        outer: &BTreeMap<String, String>,
        depth: usize,
    ) {
        // A net's name at the top, where a chain of plain connections reaches.
        let above = |name: &str| -> String {
            outer
                .get(name)
                .cloned()
                .unwrap_or_else(|| format!("{netlist}{name}"))
        };
        for (_, net) in module.nets.iter() {
            let name = net.name.as_str();
            self.nets
                .push((format!("{vivado}{name}"), format!("{netlist}{name}")));
        }
        let mut seen = Vec::new();
        for (_, process) in module.processes.iter() {
            let ProcessKind::Sequential { clocks, .. } = &process.kind else {
                continue;
            };
            let clock = clocks
                .first()
                .and_then(|edge| module.nets.get(edge.net))
                .map(|net| above(net.name.as_str()));
            walk_block(&process.body, &mut |stmt| {
                let StmtKind::Assign { target, .. } = &stmt.kind else {
                    return;
                };
                for id in target.nets() {
                    if seen.contains(&id) {
                        continue;
                    }
                    seen.push(id);
                    let Some(net) = module.nets.get(id) else {
                        continue;
                    };
                    let name = net.name.as_str();
                    let width = net.ty.width().unwrap_or(1);
                    let declared = net
                        .attrs
                        .get(DECLARED_RANGE_ATTR)
                        .and_then(crate::ir::AttrValue::as_str)
                        .and_then(parse_range);
                    if declared.is_none() && width == 1 {
                        self.registers.push(XdcRegister {
                            vivado: format!("{vivado}{name}_reg"),
                            netlist: format!("{netlist}{name}$ff"),
                            clock: clock.clone(),
                            scalar_net: Some(format!("{netlist}{name}")),
                        });
                        continue;
                    }
                    let (left, right) = declared.unwrap_or((i64::from(width) - 1, 0));
                    let (low, high) = (left.min(right), left.max(right));
                    for index in low..=high {
                        let Some(offset) = offset_in(left, right, index) else {
                            continue;
                        };
                        let cell = if width == 1 {
                            format!("{netlist}{name}$ff")
                        } else {
                            format!("{netlist}{name}$ff$ff{offset}")
                        };
                        self.registers.push(XdcRegister {
                            vivado: format!("{vivado}{name}_reg[{index}]"),
                            netlist: cell,
                            clock: clock.clone(),
                            scalar_net: None,
                        });
                    }
                }
            });
        }
        for (_, instance) in module.instances.iter() {
            let name = instance.name.as_str();
            let mut pins = Vec::new();
            let mut inner = BTreeMap::new();
            for (pin, expr) in &instance.connections {
                let net = module
                    .exprs
                    .get(*expr)
                    .and_then(|node| node.as_net())
                    .and_then(|id| module.nets.get(id))
                    .map(|net| above(net.name.as_str()));
                if let Some(net) = &net {
                    inner.insert(pin.as_str().to_owned(), net.clone());
                }
                pins.push((pin.as_str().to_owned(), net));
            }
            let child = match instance.module {
                ModuleRef::Resolved(id) => design.modules.get(id),
                ModuleRef::Unresolved(_) => None,
            };
            let module_name = child.map_or_else(
                || match &instance.module {
                    ModuleRef::Unresolved(n) => n.as_str().to_owned(),
                    ModuleRef::Resolved(_) => String::new(),
                },
                |m| m.name.as_str().to_owned(),
            );
            let hierarchical = child.is_some_and(|c| {
                !c.processes.is_empty() || !c.instances.is_empty() || !c.cells.is_empty()
            }) && !is_library_cell(&module_name);
            if let Some(child) = child {
                for port in &child.ports {
                    if !pins.iter().any(|(p, _)| p == port.name.as_str()) {
                        pins.push((port.name.as_str().to_owned(), None));
                    }
                }
            }
            self.instances.push(XdcInstance {
                path: format!("{vivado}{name}"),
                module: module_name,
                hierarchical,
                pins,
            });
            // A recursive instantiation is an elaboration error; the depth
            // limit only keeps a malformed design from running this forever.
            if hierarchical
                && depth < 64
                && let Some(child) = child
            {
                self.walk(
                    design,
                    child,
                    &format!("{vivado}{name}/"),
                    &format!("{netlist}{name}."),
                    &inner,
                    depth + 1,
                );
            }
        }
    }

    fn port_of_bit(&self, bit: &str) -> Option<(&XdcPort, Option<u32>)> {
        let (name, index) = match bit.rfind('[') {
            Some(open) if bit.ends_with(']') => (
                &bit[..open],
                bit[open + 1..bit.len() - 1].parse::<i64>().ok(),
            ),
            _ => (bit, None),
        };
        let port = self.ports.iter().find(|p| p.name == name)?;
        match index {
            Some(index) => port.offset(index).map(|offset| (port, Some(offset))),
            None => (!port.vector).then_some((port, None)),
        }
    }

    fn register(&self, name: &str) -> Option<&XdcRegister> {
        self.registers.iter().find(|r| r.vivado == name)
    }

    fn instance(&self, path: &str) -> Option<&XdcInstance> {
        self.instances.iter().find(|i| i.path == path)
    }

    fn is_cell(&self, name: &str) -> bool {
        self.register(name).is_some() || self.instance(name).is_some()
    }
}

/// A primitive of the 7-series library arrives as a module of the design, from
/// `xc7_cells.v`: to Vivado it is a leaf, whatever it is written as here.
fn is_library_cell(module: &str) -> bool {
    module
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
        && module
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn parse_range(text: &str) -> Option<(i64, i64)> {
    let (left, right) = text.split_once(':')?;
    Some((left.trim().parse().ok()?, right.trim().parse().ok()?))
}

/// What a file said of one bit of a port.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortRead {
    /// `PACKAGE_PIN`, or `LOC` with a pin's name.
    pub pin: Option<String>,
    /// `IOSTANDARD`.
    pub io_standard: Option<String>,
    /// `DRIVE`.
    pub drive: Option<u32>,
    /// `SLEW`, upper case.
    pub slew: Option<String>,
    /// `PULLUP` as last stated.
    pub pullup: Option<bool>,
    /// `PULLDOWN` as last stated.
    pub pulldown: Option<bool>,
}

/// One clock as Vivado would hold it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClockRead {
    /// Its name.
    pub name: String,
    /// Its period in ns.
    pub period: f64,
    /// Its rising and falling edge in ns.
    pub waveform: (f64, f64),
    /// From `create_generated_clock`.
    pub generated: bool,
    /// What it is on; nothing makes it a virtual clock.
    pub sources: Vec<Object>,
    /// Written with `-add`.
    pub add: bool,
    /// Where it was written.
    pub span: Span,
}

/// A path constraint, as Vivado kept it.
#[derive(Clone, Debug, PartialEq)]
pub struct TimingRead {
    /// The command.
    pub command: &'static str,
    /// Its flags, in the order Vivado writes them back.
    pub flags: Vec<String>,
    /// `set_clock_groups -name`.
    pub name: Option<String>,
    /// `-clock` of an IO delay.
    pub clock: Vec<Object>,
    /// `-from`.
    pub from: Vec<Object>,
    /// `-through`.
    pub through: Vec<Object>,
    /// `-to`.
    pub to: Vec<Object>,
    /// `-group`s.
    pub groups: Vec<Vec<Object>>,
    /// The number: cycles or nanoseconds.
    pub value: Option<f64>,
    /// The ports of an IO delay.
    pub objects: Vec<Object>,
    /// Where it was written.
    pub span: Span,
}

/// A placement block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PblockRead {
    /// Its name.
    pub name: String,
    /// Its ranges, as written.
    pub ranges: Vec<String>,
    /// The cells added to it, Vivado's names.
    pub cells: Vec<String>,
    /// Where it was created.
    pub span: Span,
}

/// A fixed location for a cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceRead {
    /// `LOC`.
    pub loc: Option<String>,
    /// `BEL`.
    pub bel: Option<String>,
}

/// What an XDC file says, in Vivado's terms, with each query resolved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XdcReading {
    /// What was said of each bit of each port, by the bit's name, and where
    /// it was first said.
    pub ports: BTreeMap<String, (PortRead, Span)>,
    /// The order bits were first mentioned in.
    pub port_order: Vec<String>,
    /// The clocks.
    pub clocks: Vec<ClockRead>,
    /// The path constraints Vivado would keep.
    pub timing: Vec<TimingRead>,
    /// The placement blocks.
    pub pblocks: Vec<PblockRead>,
    /// Fixed cell locations, by Vivado's name for the cell.
    pub places: BTreeMap<String, PlaceRead>,
    /// `set_property ... [current_design]`, as written.
    pub config: BTreeMap<String, String>,
    /// `KEEP_HIERARCHY` on a cell: its name and where it was said.
    pub keep_hierarchy: Vec<(String, Span)>,
}

impl XdcReading {
    /// The reading in the vocabulary of a `.answer` file, as
    /// `tools/vivado-parity/gen_xdc.py` has Vivado write it: the unstated
    /// options of a port show Vivado's defaults, and an output's drive and
    /// slew as well.
    pub fn render(&self, design: &XdcDesign) -> String {
        let mut lines = Vec::new();
        for port in &design.ports {
            let direction = match port.dir {
                PortDir::In => "IN",
                PortDir::Out => "OUT",
                PortDir::InOut => "INOUT",
            };
            for (bit, _) in port.bits() {
                let read = self.ports.get(&bit).map(|(r, _)| r.clone());
                let read = read.unwrap_or_default();
                let output = port.dir != PortDir::In;
                let pull = if read.pullup == Some(true) {
                    "PULLUP"
                } else if read.pulldown == Some(true) {
                    "PULLDOWN"
                } else {
                    ""
                };
                let drive = read.drive.map_or_else(
                    || if output { "12".into() } else { String::new() },
                    |d| d.to_string(),
                );
                let slew = read
                    .slew
                    .clone()
                    .unwrap_or_else(|| if output { "SLOW".into() } else { String::new() });
                lines.push(format!(
                    "port {bit} {direction} pin={} iostandard={} drive={drive} slew={slew} \
                     pulltype={pull}",
                    read.pin.as_deref().unwrap_or(""),
                    read.io_standard.as_deref().unwrap_or("LVCMOS18"),
                ));
            }
        }
        for clock in &self.clocks {
            let sources: Vec<&str> = clock.sources.iter().map(|o| o.name.as_str()).collect();
            lines.push(format!(
                "clock {} period={:.3} waveform={:.3},{:.3} generated={} virtual={} sources={}",
                clock.name,
                clock.period,
                clock.waveform.0,
                clock.waveform.1,
                u8::from(clock.generated),
                u8::from(clock.sources.is_empty()),
                sources.join(","),
            ));
        }
        for pblock in &self.pblocks {
            let mut cells = pblock.cells.clone();
            cells.sort();
            lines.push(format!(
                "pblock {} ranges={} cells={}",
                pblock.name,
                pblock.ranges.join(","),
                cells.join(","),
            ));
        }
        for (cell, place) in &self.places {
            lines.push(format!(
                "place {cell} loc={} bel={}",
                place.loc.as_deref().unwrap_or(""),
                place.bel.as_deref().unwrap_or(""),
            ));
        }
        for (key, value) in &self.config {
            lines.push(format!("config {key}={value}"));
        }
        for timing in &self.timing {
            lines.push(timing.render());
        }
        lines.sort();
        let mut out = lines.join("\n");
        out.push('\n');
        out
    }

    /// What of this reading the flow can use. Every part it cannot is said
    /// here; see the module documentation for at what level.
    pub fn constraints(&self, design: &XdcDesign, diags: &mut Diagnostics) -> Constraints {
        Translate {
            reading: self,
            design,
            diags,
            out: Constraints::new(),
            kept_clocks: Vec::new(),
        }
        .run()
    }
}

impl TimingRead {
    fn render(&self) -> String {
        let mut out = format!("timing {}", self.command);
        let list = |objects: &[Object]| {
            let mut names: Vec<String> = objects
                .iter()
                .map(|o| format!("{}:{}", o.class.word(), o.name))
                .collect();
            names.sort();
            names.join(",")
        };
        if let Some(name) = &self.name {
            let _ = write!(out, " -name {name}");
        }
        if !self.clock.is_empty() {
            let _ = write!(out, " -clock {}", list(&self.clock));
        }
        for flag in &self.flags {
            let _ = write!(out, " {flag}");
        }
        for group in &self.groups {
            let _ = write!(out, " -group {}", list(group));
        }
        for (option, objects) in [
            ("-from", &self.from),
            ("-through", &self.through),
            ("-to", &self.to),
        ] {
            if !objects.is_empty() {
                let _ = write!(out, " {option} {}", list(objects));
            }
        }
        if let Some(value) = self.value {
            if self.command == "set_multicycle_path" {
                let _ = write!(out, " {value}");
            } else {
                let _ = write!(out, " {value:.3}");
            }
        }
        if !self.objects.is_empty() {
            let _ = write!(out, " {}", list(&self.objects));
        }
        out
    }
}

/// Reads an XDC file.
///
/// Evaluates what Vivado's `read_xdc` evaluates, against `design`. What Vivado
/// refuses is reported at the command and kept out of the reading, whatever
/// the level of the report.
pub fn read_xdc(
    text: &str,
    file: SourceId,
    design: &XdcDesign,
    diags: &mut Diagnostics,
) -> XdcReading {
    let mut reader = Reader {
        bytes: text.as_bytes(),
        pos: 0,
        file,
        design,
        diags,
        vars: BTreeMap::new(),
        scope: String::new(),
        reading: XdcReading::default(),
        depth: 0,
    };
    reader.script(false);
    reader.reading
}

impl Constraints {
    /// Reads an XDC file against `module` of `design`: [`read_xdc`], then
    /// [`XdcReading::constraints`].
    pub fn parse_xdc(
        text: &str,
        file: SourceId,
        design: &Design,
        module: ModuleId,
        diags: &mut Diagnostics,
    ) -> Constraints {
        let view = XdcDesign::of(design, module);
        read_xdc(text, file, &view, diags).constraints(&view, diags)
    }
}

/// A Tcl value: a string, or what a query returned.
#[derive(Clone, Debug)]
enum Value {
    Text(String),
    Objects(Vec<Object>),
    /// A query this reader cannot answer, and why; what becomes of that
    /// depends on the command it was for.
    Unreadable(String),
    /// Something already reported, which the enclosing command must not run
    /// with.
    Failed,
}

impl Value {
    fn as_text(&self) -> String {
        match self {
            Value::Text(text) => text.clone(),
            Value::Objects(objects) => objects
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            Value::Unreadable(_) | Value::Failed => String::new(),
        }
    }

    fn empty() -> Value {
        Value::Text(String::new())
    }
}

/// The parsed arguments of one command.
#[derive(Default)]
struct Arguments {
    flags: Vec<&'static str>,
    options: Vec<(&'static str, Value)>,
    positional: Vec<Value>,
    unknown: Vec<String>,
}

impl Arguments {
    fn flag(&self, name: &str) -> bool {
        self.flags.contains(&name)
    }

    fn option(&self, name: &str) -> Option<&Value> {
        self.options
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v)
    }

    fn all(&self, name: &str) -> Vec<&Value> {
        self.options
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v)
            .collect()
    }
}

/// Splits the words after the command name. Vivado takes any unambiguous start
/// of an option's name (`-hier`), and `-1` is a number.
fn arguments(words: &[Value], with_value: &[&'static str], flags: &[&'static str]) -> Arguments {
    let mut out = Arguments::default();
    let mut iter = words.iter();
    while let Some(word) = iter.next() {
        let name = match word {
            Value::Text(text) => text
                .strip_prefix('-')
                .filter(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic())),
            _ => None,
        };
        let Some(name) = name else {
            out.positional.push(word.clone());
            continue;
        };
        if matches!(name, "quiet" | "verbose") {
            continue;
        }
        let exact = |list: &[&'static str]| list.iter().copied().find(|o| *o == name);
        let start = |list: &[&'static str]| {
            let mut found = list.iter().copied().filter(|o| o.starts_with(name));
            match (found.next(), found.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        };
        let valued = exact(with_value);
        let flagged = exact(flags);
        let (valued, flagged) = if valued.is_none() && flagged.is_none() {
            match (start(with_value), start(flags)) {
                (Some(v), None) => (Some(v), None),
                (None, Some(f)) => (None, Some(f)),
                _ => (None, None),
            }
        } else {
            (valued, flagged)
        };
        if let Some(option) = valued {
            match iter.next() {
                Some(value) => out.options.push((option, value.clone())),
                None => out.unknown.push(format!("-{name} (it needs a value)")),
            }
        } else if let Some(flag) = flagged {
            out.flags.push(flag);
        } else {
            out.unknown.push(format!("-{name}"));
        }
    }
    out
}

/// Splits a Tcl list.
fn split_list(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b'{' {
            let mut depth = 0usize;
            let start = i + 1;
            while i < bytes.len() {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            out.push(text[start..i.min(text.len())].to_owned());
            i += 1;
        } else if bytes[i] == b'"' {
            let start = i + 1;
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            out.push(text[start..i.min(text.len())].to_owned());
            i += 1;
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(text[start..i].to_owned());
        }
    }
    out
}

/// A pad option a flow could read, as opposed to `set_property`'s others.
const PORT_PROPERTIES: &[&str] = &[
    "PACKAGE_PIN",
    "LOC",
    "IOSTANDARD",
    "DRIVE",
    "SLEW",
    "PULLUP",
    "PULLDOWN",
    "PULLTYPE",
];

/// Cell, net and pblock properties that steer tools and leave the logic alone.
const STEERING_PROPERTIES: &[&str] = &[
    "ASYNC_REG",
    "BEL",
    "CLOCK_DEDICATED_ROUTE",
    "CLOCK_BUFFER_TYPE",
    "CONTAIN_ROUTING",
    "DIRECT_ENABLE",
    "DIRECT_RESET",
    "DONT_TOUCH",
    "EXCLUDE_PLACEMENT",
    "HLUTNM",
    "HU_SET",
    "IOB",
    "IS_SOFT",
    "KEEP",
    "LOCK_PINS",
    "LUTNM",
    "MARK_DEBUG",
    "MAX_FANOUT",
    "PARENT",
    "RAM_STYLE",
    "RLOC",
    "RLOC_ORIGIN",
    "ROM_STYLE",
    "RPM_GRID",
    "RPM",
    "SHREG_EXTRACT",
    "SNAPPING_MODE",
    "SRL_STYLE",
    "U_SET",
    "USE_DSP",
];

/// Commands that can only change what timing analysis believes or reports.
const TIMING_ONLY_COMMANDS: &[&str] = &[
    "config_timing_analysis",
    "config_timing_pessimism",
    "create_waiver",
    "group_path",
    "reset_path",
    "reset_timing",
    "set_bus_skew",
    "set_case_analysis",
    "set_clock_latency",
    "set_clock_sense",
    "set_clock_uncertainty",
    "set_data_check",
    "set_disable_timing",
    "set_external_delay",
    "set_input_jitter",
    "set_logic_dc",
    "set_logic_one",
    "set_logic_unconnected",
    "set_logic_zero",
    "set_max_time_borrow",
    "set_operating_conditions",
    "set_power_opt",
    "set_propagated_clock",
    "set_switching_activity",
    "set_system_jitter",
    "set_units",
];

/// Configuration options measured to change only how the part loads: Vivado's
/// bitstream differs in the configuration packets, and the design that comes
/// up is the same. See `docs/fpga-vivado-parity.md`.
const LOAD_ONLY_CONFIG: &[&str] = &[
    "CONFIG_MODE",
    "BITSTREAM.CONFIG.CONFIGRATE",
    "BITSTREAM.CONFIG.EXTMASTERCCLK_EN",
    "BITSTREAM.CONFIG.SPI_BUSWIDTH",
    "BITSTREAM.CONFIG.SPI_FALL_EDGE",
    "BITSTREAM.GENERAL.COMPRESS",
    "BITSTREAM.GENERAL.CRC",
];

/// Every `BITSTREAM.*` option Vivado 2026.1 lists for a 7-series design, for
/// telling one this flow does not honour from one that does not exist.
const BITSTREAM_OPTIONS: &[&str] = &[
    "BITSTREAM.CONFIG.BPI_1ST_READ_CYCLE",
    "BITSTREAM.CONFIG.BPI_PAGE_SIZE",
    "BITSTREAM.CONFIG.BPI_SYNC_MODE",
    "BITSTREAM.CONFIG.CCLKPIN",
    "BITSTREAM.CONFIG.CCLK_TRISTATE",
    "BITSTREAM.CONFIG.CONFIGFALLBACK",
    "BITSTREAM.CONFIG.CONFIGRATE",
    "BITSTREAM.CONFIG.DCIUPDATEMODE",
    "BITSTREAM.CONFIG.DONEPIN",
    "BITSTREAM.CONFIG.EXTMASTERCCLK_EN",
    "BITSTREAM.CONFIG.INITPIN",
    "BITSTREAM.CONFIG.INITSIGNALSERROR",
    "BITSTREAM.CONFIG.M0PIN",
    "BITSTREAM.CONFIG.M1PIN",
    "BITSTREAM.CONFIG.M2PIN",
    "BITSTREAM.CONFIG.NEXT_CONFIG_ADDR",
    "BITSTREAM.CONFIG.NEXT_CONFIG_REBOOT",
    "BITSTREAM.CONFIG.OVERTEMPPOWERDOWN",
    "BITSTREAM.CONFIG.PERSIST",
    "BITSTREAM.CONFIG.PROGPIN",
    "BITSTREAM.CONFIG.REVISIONSELECT",
    "BITSTREAM.CONFIG.REVISIONSELECT_TRISTATE",
    "BITSTREAM.CONFIG.SELECTMAPABORT",
    "BITSTREAM.CONFIG.SPI_32BIT_ADDR",
    "BITSTREAM.CONFIG.SPI_BUSWIDTH",
    "BITSTREAM.CONFIG.SPI_FALL_EDGE",
    "BITSTREAM.CONFIG.TCKPIN",
    "BITSTREAM.CONFIG.TDIPIN",
    "BITSTREAM.CONFIG.TDOPIN",
    "BITSTREAM.CONFIG.TIMER_CFG",
    "BITSTREAM.CONFIG.TIMER_USR",
    "BITSTREAM.CONFIG.TMSPIN",
    "BITSTREAM.CONFIG.UNUSEDPIN",
    "BITSTREAM.CONFIG.USERID",
    "BITSTREAM.CONFIG.USR_ACCESS",
    "BITSTREAM.ENCRYPTION.ENCRYPT",
    "BITSTREAM.ENCRYPTION.ENCRYPTKEYSELECT",
    "BITSTREAM.ENCRYPTION.HKEY",
    "BITSTREAM.ENCRYPTION.KEY0",
    "BITSTREAM.ENCRYPTION.KEYFILE",
    "BITSTREAM.ENCRYPTION.STARTCBC",
    "BITSTREAM.GENERAL.COMPRESS",
    "BITSTREAM.GENERAL.CRC",
    "BITSTREAM.GENERAL.DEBUGBITSTREAM",
    "BITSTREAM.GENERAL.DISABLE_JTAG",
    "BITSTREAM.GENERAL.JTAG_XADC",
    "BITSTREAM.GENERAL.PERFRAMECRC",
    "BITSTREAM.GENERAL.XADCENHANCEDLINEARITY",
    "BITSTREAM.GENERAL.XADCPOWERDOWN",
    "BITSTREAM.READBACK.ACTIVERECONFIG",
    "BITSTREAM.READBACK.ICAP_SELECT",
    "BITSTREAM.READBACK.SECURITY",
    "BITSTREAM.READBACK.XADCPARTIALRECONFIG",
    "BITSTREAM.STARTUP.DONEPIPE",
    "BITSTREAM.STARTUP.DONE_CYCLE",
    "BITSTREAM.STARTUP.GTS_CYCLE",
    "BITSTREAM.STARTUP.GWE_CYCLE",
    "BITSTREAM.STARTUP.LCK_CYCLE",
    "BITSTREAM.STARTUP.MATCH_CYCLE",
    "BITSTREAM.STARTUP.STARTUPCLK",
];

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    file: SourceId,
    design: &'a XdcDesign,
    diags: &'a mut Diagnostics,
    vars: BTreeMap<String, Value>,
    /// `current_instance`, with a trailing `/` when it is not the top.
    scope: String,
    reading: XdcReading,
    depth: usize,
}

impl Reader<'_> {
    fn span(&self, start: usize, end: usize) -> Span {
        // A backslash at the very end steps past it.
        let at = |offset: usize| u32::try_from(offset.min(self.bytes.len())).unwrap_or(u32::MAX);
        Span::new(self.file, at(start), at(end))
    }

    fn refuse(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(message)
                .with_code(XDC_REFUSED)
                .with_span(span),
        );
    }

    fn not_honoured(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::warning(message)
                .with_code(XDC_NOT_HONOURED)
                .with_span(span),
        );
    }

    fn no_effect(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::new(Severity::Note, message)
                .with_code(XDC_NO_EFFECT)
                .with_span(span),
        );
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Runs commands to the end of the text, or to the `]` that closes
    /// `nested`. The value is the last command's.
    fn script(&mut self, nested: bool) -> Value {
        let mut last = Value::empty();
        loop {
            while let Some(c) = self.peek() {
                if c.is_ascii_whitespace() || c == b';' {
                    self.pos += 1;
                } else if c == b'\\' && self.at_line_continuation() {
                    self.skip_continuation();
                } else {
                    break;
                }
            }
            let Some(c) = self.peek() else {
                if nested {
                    let span = self.span(self.bytes.len(), self.bytes.len());
                    self.refuse(span, "a `[` is never closed");
                    return Value::Failed;
                }
                return last;
            };
            if nested && c == b']' {
                self.pos += 1;
                return last;
            }
            if c == b'#' {
                self.skip_comment();
                continue;
            }
            last = self.command(nested);
        }
    }

    fn at_line_continuation(&self) -> bool {
        match self.bytes.get(self.pos + 1) {
            Some(b'\n') => true,
            Some(b'\r') => self.bytes.get(self.pos + 2) == Some(&b'\n'),
            _ => false,
        }
    }

    /// Past a backslash, its newline and the blanks after them.
    fn skip_continuation(&mut self) {
        self.pos += 1;
        if self.peek() == Some(b'\r') {
            self.pos += 1;
        }
        self.pos += 1;
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    /// A comment runs to a newline a backslash does not continue, as Vivado
    /// has it: a statement on the line after such a comment is comment too.
    fn skip_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == b'\\' {
                self.pos += 2;
                continue;
            }
            if c == b'\n' {
                return;
            }
            self.pos += 1;
        }
    }

    fn command(&mut self, nested: bool) -> Value {
        let start = self.pos;
        let mut words = Vec::new();
        loop {
            while let Some(c) = self.peek() {
                if c == b' ' || c == b'\t' || c == b'\r' {
                    self.pos += 1;
                } else if c == b'\\' && self.at_line_continuation() {
                    self.skip_continuation();
                } else {
                    break;
                }
            }
            match self.peek() {
                None | Some(b'\n' | b';') => break,
                Some(b']') if nested => break,
                _ => words.push(self.word(nested)),
            }
        }
        let span = self.span(start, self.pos);
        self.run(&words, span)
    }

    fn word(&mut self, nested: bool) -> Value {
        match self.peek() {
            Some(b'{') => self.braced(nested),
            Some(b'"') => {
                self.pos += 1;
                self.substituted(nested, true)
            }
            _ => self.substituted(nested, false),
        }
    }

    fn braced(&mut self, nested: bool) -> Value {
        let start = self.pos;
        let mut depth = 0usize;
        let mut text = Vec::new();
        while let Some(c) = self.peek() {
            match c {
                b'\\' if self.at_line_continuation() => {
                    self.skip_continuation();
                    text.push(b' ');
                    continue;
                }
                b'\\' => {
                    text.push(c);
                    if let Some(next) = self.bytes.get(self.pos + 1) {
                        text.push(*next);
                    }
                    self.pos += 2;
                    continue;
                }
                b'{' => {
                    depth += 1;
                    if depth > 1 {
                        text.push(c);
                    }
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos += 1;
                        let after = self.peek();
                        let ends = match after {
                            None | Some(b' ' | b'\t' | b'\r' | b'\n' | b';') => true,
                            Some(b']') => nested,
                            _ => false,
                        };
                        if !ends {
                            let span = self.span(start, self.pos);
                            self.refuse(span, "extra characters after a closing brace");
                            self.skip_word(nested);
                            return Value::Failed;
                        }
                        return Value::Text(String::from_utf8_lossy(&text).into_owned());
                    }
                    text.push(c);
                }
                _ => text.push(c),
            }
            self.pos += 1;
        }
        let span = self.span(start, self.pos);
        self.refuse(span, "a `{` is never closed");
        Value::Failed
    }

    fn skip_word(&mut self, nested: bool) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() || c == b';' || (nested && c == b']') {
                break;
            }
            self.pos += 1;
        }
    }

    /// A bare or a quoted word, with its substitutions. A word that is exactly
    /// one `[...]` or `$name` keeps its value, objects included.
    fn substituted(&mut self, nested: bool, quoted: bool) -> Value {
        let start = self.pos;
        let mut text = String::new();
        let mut single: Option<Value> = None;
        let mut pieces = 0usize;
        let mut poisoned: Option<Value> = None;
        let mut literal = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                if quoted {
                    let span = self.span(start, self.pos);
                    self.refuse(span, "a `\"` is never closed");
                    return Value::Failed;
                }
                break;
            };
            if quoted && c == b'"' {
                self.pos += 1;
                break;
            }
            if !quoted && (c.is_ascii_whitespace() || c == b';' || (nested && c == b']')) {
                break;
            }
            match c {
                b'[' => {
                    Self::flush(&mut literal, &mut text, &mut pieces);
                    // Vivado takes `a[0]` and `a[*]` in a bare word or quotes
                    // for the bit they spell, where Tcl would call a command
                    // named `0`. `a[1:0]` is taken the same way, and matches
                    // nothing.
                    if let Some(end) = self.index_end() {
                        let index = String::from_utf8_lossy(&self.bytes[self.pos..=end]);
                        text.push_str(&index);
                        pieces += 1;
                        self.pos = end + 1;
                        continue;
                    }
                    self.pos += 1;
                    self.depth += 1;
                    let value = if self.depth > 64 {
                        let span = self.span(start, self.pos);
                        self.refuse(span, "commands are nested too deeply");
                        Value::Failed
                    } else {
                        self.script(true)
                    };
                    self.depth -= 1;
                    pieces += 1;
                    match value {
                        Value::Failed | Value::Unreadable(_) => poisoned = Some(value),
                        Value::Text(ref t) => {
                            text.push_str(t);
                            single = Some(value);
                        }
                        Value::Objects(_) => {
                            text.push_str(&value.as_text());
                            single = Some(value);
                        }
                    }
                }
                b'$' => {
                    Self::flush(&mut literal, &mut text, &mut pieces);
                    let value = self.variable();
                    pieces += 1;
                    match value {
                        Value::Failed | Value::Unreadable(_) => poisoned = Some(value),
                        _ => {
                            text.push_str(&value.as_text());
                            single = Some(value);
                        }
                    }
                }
                b'\\' => {
                    if self.at_line_continuation() {
                        // In Tcl it separates words, and only quotes hold it in.
                        if !quoted {
                            break;
                        }
                        self.skip_continuation();
                        literal.push(b' ');
                        continue;
                    }
                    self.pos += 1;
                    let next = self.peek().unwrap_or(b'\\');
                    self.pos += 1;
                    literal.push(match next {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        other => other,
                    });
                }
                _ => {
                    literal.push(c);
                    self.pos += 1;
                }
            }
        }
        let had_literal = !literal.is_empty();
        Self::flush(&mut literal, &mut text, &mut pieces);
        if let Some(value) = poisoned {
            return value;
        }
        if pieces == 1
            && !had_literal
            && let Some(value) = single
        {
            return value;
        }
        Value::Text(text)
    }

    fn flush(literal: &mut Vec<u8>, text: &mut String, pieces: &mut usize) {
        if !literal.is_empty() {
            text.push_str(&String::from_utf8_lossy(literal));
            literal.clear();
            *pieces += 1;
        }
    }

    /// The offset of the `]` when the `[` here opens an index, not a command.
    fn index_end(&self) -> Option<usize> {
        let mut at = self.pos + 1;
        while let Some(c) = self.bytes.get(at) {
            match c {
                b']' => return (at > self.pos + 1).then_some(at),
                b'0'..=b'9' | b'*' | b'?' | b':' => at += 1,
                _ => return None,
            }
        }
        None
    }

    fn variable(&mut self) -> Value {
        let start = self.pos;
        self.pos += 1;
        let name = if self.peek() == Some(b'{') {
            self.pos += 1;
            let from = self.pos;
            while self.peek().is_some_and(|c| c != b'}') {
                self.pos += 1;
            }
            let name = String::from_utf8_lossy(&self.bytes[from..self.pos]).into_owned();
            self.pos += 1;
            name
        } else {
            let from = self.pos;
            while self
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b':')
            {
                self.pos += 1;
            }
            String::from_utf8_lossy(&self.bytes[from..self.pos]).into_owned()
        };
        if name.is_empty() {
            return Value::Text("$".to_owned());
        }
        match self.vars.get(&name) {
            Some(value) => value.clone(),
            None => {
                let span = self.span(start, self.pos);
                self.refuse(span, format!("can't read `{name}`: no such variable"));
                Value::Failed
            }
        }
    }

    /// One command, with its words already substituted.
    fn run(&mut self, words: &[Value], span: Span) -> Value {
        let Some(first) = words.first() else {
            return Value::empty();
        };
        if words.iter().any(|w| matches!(w, Value::Failed)) {
            return Value::Failed;
        }
        let Value::Text(name) = first else {
            self.refuse(span, "a command's name has to be a word");
            return Value::Failed;
        };
        let name = name.clone();
        let args = &words[1..];
        match name.as_str() {
            "set" => self.set_variable(args, span),
            "list" => list_value(args),
            "split" => Value::Text(
                split_list(&args.first().map_or(String::new(), Value::as_text)).join(" "),
            ),
            "join" => {
                let separator = args.get(1).map_or(" ".to_owned(), Value::as_text);
                Value::Text(
                    split_list(&args.first().map_or(String::new(), Value::as_text))
                        .join(&separator),
                )
            }
            "llength" => Value::Text(
                split_list(&args.first().map_or(String::new(), Value::as_text))
                    .len()
                    .to_string(),
            ),
            "expr" => self.expr(args, span),
            "current_design" => self.current_design(args, span),
            "current_instance" => self.current_instance(args, span),
            "get_ports" => self.get_ports(args),
            "all_inputs" => self.all_ports(PortDir::Out),
            "all_outputs" => self.all_ports(PortDir::In),
            "all_clocks" => Value::Objects(
                self.reading
                    .clocks
                    .iter()
                    .map(|c| Object::new(ObjectClass::Clock, c.name.clone()))
                    .collect(),
            ),
            "all_registers" => Value::Unreadable(
                "`all_registers` has no answer before synthesis has made the registers".to_owned(),
            ),
            "get_cells" => self.get_cells(args),
            "get_pins" => self.get_pins(args),
            "get_nets" => self.get_nets(args),
            "get_clocks" => self.get_clocks(args),
            "get_pblocks" => self.get_pblocks(args),
            "get_iobanks" => objects_of(args, ObjectClass::Bank),
            "get_sites" => objects_of(args, ObjectClass::Site),
            "get_debug_cores" | "get_debug_ports" => objects_of(args, ObjectClass::Debug),
            "get_property" => self.get_property(args),
            "filter" => self.filter(args),
            "set_property" => self.set_property(args, span),
            "create_clock" => self.create_clock(args, span),
            "create_generated_clock" => self.create_generated_clock(args, span),
            "set_false_path" => self.path_exception("set_false_path", args, span),
            "set_multicycle_path" => self.path_exception("set_multicycle_path", args, span),
            "set_max_delay" => self.path_exception("set_max_delay", args, span),
            "set_min_delay" => self.path_exception("set_min_delay", args, span),
            "set_input_delay" => self.io_delay("set_input_delay", args, span),
            "set_output_delay" => self.io_delay("set_output_delay", args, span),
            "set_clock_groups" => self.set_clock_groups(args, span),
            "create_pblock" => self.create_pblock(args, span),
            "add_cells_to_pblock" => self.add_cells_to_pblock(args, span),
            "resize_pblock" => self.resize_pblock(args, span),
            "create_debug_core"
            | "create_debug_port"
            | "connect_debug_port"
            | "implement_debug_core" => {
                self.not_honoured(
                    span,
                    format!(
                        "`{name}` is not honoured: Reticle inserts no debug core, and the design \
                         is the same without one"
                    ),
                );
                Value::empty()
            }
            "remove_cells_from_pblock" | "delete_pblocks" | "create_property" => {
                self.not_honoured(
                    span,
                    format!("`{name}` is not honoured: it can only change where cells go"),
                );
                Value::empty()
            }
            other if TIMING_ONLY_COMMANDS.contains(&other) => {
                self.not_honoured(
                    span,
                    format!(
                        "`{other}` is not honoured: Reticle's timing has no such setting, so \
                         its analysis takes no account of this"
                    ),
                );
                Value::empty()
            }
            other => {
                self.diags.push(
                    Diagnostic::error(format!(
                        "`{other}` is not a command of Vivado's XDC, and whatever it would have \
                         done here is not done"
                    ))
                    .with_code(XDC_REFUSED)
                    .with_span(span)
                    .with_note(
                        "`read_xdc` takes constraint commands, `set`, `list`, `expr`, `split`, \
                         `join`, `llength` and `get_property`; Vivado reports the rest, \
                         `foreach`, `if`, `proc`, `source`, `lindex` and `string` among them, \
                         as not supported in a constraint file and carries on without them",
                    ),
                );
                Value::Failed
            }
        }
    }

    fn set_variable(&mut self, args: &[Value], span: Span) -> Value {
        let Some(name) = args.first().map(Value::as_text) else {
            self.refuse(span, "`set` needs a variable's name");
            return Value::Failed;
        };
        match args.get(1) {
            Some(value) => {
                self.vars.insert(name, value.clone());
                value.clone()
            }
            None => match self.vars.get(&name) {
                Some(value) => value.clone(),
                None => {
                    self.refuse(span, format!("can't read `{name}`: no such variable"));
                    Value::Failed
                }
            },
        }
    }

    fn expr(&mut self, args: &[Value], span: Span) -> Value {
        let text = args
            .iter()
            .map(Value::as_text)
            .collect::<Vec<_>>()
            .join(" ");
        let vars: BTreeMap<String, String> = self
            .vars
            .iter()
            .map(|(k, v)| (k.clone(), v.as_text()))
            .collect();
        match Expr::new(&text, &vars).run() {
            Some(value) => Value::Text(value.text()),
            None => {
                self.refuse(span, format!("cannot evaluate `expr {text}` here"));
                Value::Failed
            }
        }
    }

    fn current_design(&mut self, args: &[Value], span: Span) -> Value {
        if let Some(name) = args
            .iter()
            .map(Value::as_text)
            .find(|a| !a.starts_with('-'))
        {
            // Vivado's own answer turns on what its project called the design,
            // and it carries on either way; nothing here turns on it.
            if name != self.design.top {
                self.not_honoured(
                    span,
                    format!(
                        "`current_design {name}` names a design this is not (the top is `{}`), \
                         and has no effect here",
                        self.design.top
                    ),
                );
            }
        }
        Value::Objects(vec![Object::new(
            ObjectClass::Design,
            self.design.top.clone(),
        )])
    }

    fn current_instance(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(args, &[], &[]);
        match parsed.positional.first() {
            None => self.scope.clear(),
            Some(value) => {
                let name = value.as_text();
                if name == "." {
                    self.scope.clear();
                } else if self
                    .design
                    .instance(&format!("{}{name}", self.scope))
                    .is_some()
                {
                    self.scope.push_str(&name);
                    self.scope.push('/');
                } else {
                    self.refuse(
                        span,
                        format!("`current_instance {name}`: there is no such instance"),
                    );
                    return Value::Failed;
                }
            }
        }
        Value::empty()
    }

    fn all_ports(&self, not: PortDir) -> Value {
        let mut out = Vec::new();
        for port in &self.design.ports {
            if port.dir != not {
                for (bit, _) in port.bits() {
                    out.push(Object::new(ObjectClass::Port, bit));
                }
            }
        }
        Value::Objects(out)
    }

    fn patterns(positional: &[Value]) -> Vec<String> {
        let mut out = Vec::new();
        for value in positional {
            match value {
                Value::Objects(objects) => out.extend(objects.iter().map(|o| o.name.clone())),
                other => out.extend(split_list(&other.as_text())),
            }
        }
        out
    }

    fn get_ports(&mut self, args: &[Value]) -> Value {
        let parsed = arguments(
            args,
            &["filter", "of_objects"],
            &[
                "regexp",
                "nocase",
                "scoped_to_current_instance",
                "prop_thru_buffers",
            ],
        );
        if parsed.flag("regexp") {
            return Value::Unreadable(
                "`get_ports -regexp`: Reticle has no regular expressions".into(),
            );
        }
        if let Some(bad) = parsed.unknown.first() {
            return Value::Unreadable(format!("`get_ports {bad}` is not understood here"));
        }
        let mut found: Vec<Object> = Vec::new();
        if let Some(of) = parsed.option("of_objects") {
            let Value::Objects(objects) = of else {
                return Value::Unreadable("`get_ports -of_objects` needs objects".into());
            };
            for object in objects {
                if object.class != ObjectClass::Net {
                    return Value::Unreadable(
                        "`get_ports -of_objects` is read only for nets".into(),
                    );
                }
                self.push_ports_matching(&object.name, &mut found);
            }
        } else {
            let mut patterns = Self::patterns(&parsed.positional);
            if patterns.is_empty() {
                patterns.push("*".to_owned());
            }
            for pattern in &patterns {
                self.push_ports_matching(pattern, &mut found);
            }
        }
        match parsed.option("filter") {
            Some(filter) => filter_objects(found, &filter.as_text(), self.design),
            None => Value::Objects(found),
        }
    }

    fn push_ports_matching(&self, pattern: &str, found: &mut Vec<Object>) {
        for port in &self.design.ports {
            // A bus's name stands for every bit of it, measured for the exact
            // name and for a glob.
            let whole = matches_glob(pattern, &port.name);
            for (bit, _) in port.bits() {
                if (whole || matches_glob(pattern, &bit)) && !found.iter().any(|o| o.name == bit) {
                    found.push(Object::new(ObjectClass::Port, bit));
                }
            }
        }
    }

    /// The cells (with `-hierarchical`, at any depth) whose names a pattern
    /// matches. Without it, `*` and `?` stop at `/`, as Vivado's do.
    fn cells_matching(&self, pattern: &str, hierarchical: bool) -> Vec<String> {
        let scope = self.scope.as_str();
        let names = self
            .design
            .registers
            .iter()
            .map(|r| r.vivado.as_str())
            .chain(self.design.instances.iter().map(|i| i.path.as_str()));
        let mut out = Vec::new();
        for name in names {
            let Some(relative) = name.strip_prefix(scope) else {
                continue;
            };
            if name_matches(pattern, relative, hierarchical) {
                out.push(name.to_owned());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn get_cells(&mut self, args: &[Value]) -> Value {
        let parsed = arguments(
            args,
            &["filter", "of_objects"],
            &[
                "hierarchical",
                "regexp",
                "nocase",
                "include_replicated_objects",
            ],
        );
        if parsed.flag("regexp") {
            return Value::Unreadable(
                "`get_cells -regexp`: Reticle has no regular expressions".into(),
            );
        }
        if let Some(bad) = parsed.unknown.first() {
            return Value::Unreadable(format!("`get_cells {bad}` is not understood here"));
        }
        if let Some(of) = parsed.option("of_objects") {
            let Value::Objects(objects) = of else {
                return Value::Unreadable("`get_cells -of_objects` needs objects".into());
            };
            let mut found = Vec::new();
            for object in objects {
                match object.class {
                    ObjectClass::Pin => {
                        if let Some((cell, _)) = object.name.rsplit_once('/') {
                            found.push(Object::new(ObjectClass::Cell, cell));
                        }
                    }
                    ObjectClass::Pblock => {
                        if let Some(p) = self.reading.pblocks.iter().find(|p| p.name == object.name)
                        {
                            found.extend(
                                p.cells
                                    .iter()
                                    .map(|c| Object::new(ObjectClass::Cell, c.clone())),
                            );
                        }
                    }
                    _ => {
                        return Value::Unreadable(
                            "`get_cells -of_objects` is read only for pins and pblocks".into(),
                        );
                    }
                }
            }
            return Value::Objects(found);
        }
        let hierarchical = parsed.flag("hierarchical");
        let mut patterns = Self::patterns(&parsed.positional);
        if patterns.is_empty() {
            patterns.push("*".to_owned());
        }
        let mut found: Vec<Object> = Vec::new();
        let filtering = parsed.option("filter").is_some();
        for pattern in &patterns {
            for name in self.cells_matching(pattern, hierarchical || filtering) {
                if !found.iter().any(|o| o.name == name) {
                    found.push(Object::new(ObjectClass::Cell, name));
                }
            }
        }
        match parsed.option("filter") {
            Some(filter) => filter_objects(found, &filter.as_text(), self.design),
            None => Value::Objects(found),
        }
    }

    fn get_pins(&mut self, args: &[Value]) -> Value {
        let parsed = arguments(
            args,
            &["filter", "of_objects"],
            &[
                "hierarchical",
                "regexp",
                "nocase",
                "leaf",
                "include_replicated_objects",
            ],
        );
        if parsed.flag("regexp") {
            return Value::Unreadable(
                "`get_pins -regexp`: Reticle has no regular expressions".into(),
            );
        }
        if let Some(bad) = parsed.unknown.first() {
            return Value::Unreadable(format!("`get_pins {bad}` is not understood here"));
        }
        if parsed.option("of_objects").is_some() {
            return Value::Unreadable("`get_pins -of_objects` is not read here".into());
        }
        let hierarchical = parsed.flag("hierarchical");
        let mut found: Vec<Object> = Vec::new();
        for pattern in Self::patterns(&parsed.positional) {
            let Some((cell_pattern, pin_pattern)) = pattern.rsplit_once('/') else {
                continue;
            };
            for cell in self.cells_matching(cell_pattern, hierarchical) {
                let pins: Vec<String> = if self.design.register(&cell).is_some() {
                    ["C", "CE", "D", "Q", "R", "S", "CLR", "PRE"]
                        .iter()
                        .map(|p| (*p).to_owned())
                        .collect()
                } else if let Some(instance) = self.design.instance(&cell) {
                    let mut names: Vec<String> =
                        instance.pins.iter().map(|(p, _)| p.clone()).collect();
                    // A library cell's unconnected pins are not listed; one
                    // named exactly is taken at its word.
                    if !pin_pattern.contains(['*', '?']) && !names.iter().any(|n| n == pin_pattern)
                    {
                        names.push(pin_pattern.to_owned());
                    }
                    names
                } else {
                    continue;
                };
                for pin in pins {
                    let name = format!("{cell}/{pin}");
                    if matches_glob(pin_pattern, &pin) && !found.iter().any(|o| o.name == name) {
                        found.push(Object::new(ObjectClass::Pin, name));
                    }
                }
            }
        }
        match parsed.option("filter") {
            Some(filter) => filter_objects(found, &filter.as_text(), self.design),
            None => Value::Objects(found),
        }
    }

    fn get_nets(&mut self, args: &[Value]) -> Value {
        let parsed = arguments(
            args,
            &["filter", "of_objects"],
            &[
                "hierarchical",
                "regexp",
                "nocase",
                "segments",
                "top_net_of_hierarchical_group",
                "boundary_type",
            ],
        );
        if parsed.flag("regexp") {
            return Value::Unreadable(
                "`get_nets -regexp`: Reticle has no regular expressions".into(),
            );
        }
        if parsed.option("of_objects").is_some() || parsed.option("filter").is_some() {
            return Value::Unreadable(
                "`get_nets` with `-of_objects` or `-filter` is not read here".into(),
            );
        }
        if let Some(bad) = parsed.unknown.first() {
            return Value::Unreadable(format!("`get_nets {bad}` is not understood here"));
        }
        let hierarchical = parsed.flag("hierarchical");
        let mut patterns = Self::patterns(&parsed.positional);
        if patterns.is_empty() {
            patterns.push("*".to_owned());
        }
        let mut found: Vec<Object> = Vec::new();
        for pattern in &patterns {
            for (net, _) in &self.design.nets {
                let Some(relative) = net.strip_prefix(self.scope.as_str()) else {
                    continue;
                };
                if name_matches(pattern, relative, hierarchical)
                    && !found.iter().any(|o| o.name == *net)
                {
                    found.push(Object::new(ObjectClass::Net, net.clone()));
                }
            }
        }
        Value::Objects(found)
    }

    fn get_clocks(&mut self, args: &[Value]) -> Value {
        let parsed = arguments(
            args,
            &["filter", "of_objects"],
            &[
                "regexp",
                "nocase",
                "include_generated_clocks",
                "match_style",
            ],
        );
        if parsed.flag("regexp") || parsed.option("filter").is_some() {
            return Value::Unreadable(
                "`get_clocks` with `-regexp` or `-filter` is not read here".into(),
            );
        }
        if let Some(bad) = parsed.unknown.first() {
            return Value::Unreadable(format!("`get_clocks {bad}` is not understood here"));
        }
        if let Some(of) = parsed.option("of_objects") {
            let Value::Objects(objects) = of else {
                return Value::Unreadable("`get_clocks -of_objects` needs objects".into());
            };
            let mut found = Vec::new();
            for object in objects {
                for clock in &self.reading.clocks {
                    if clock.sources.iter().any(|s| s.name == object.name) {
                        found.push(Object::new(ObjectClass::Clock, clock.name.clone()));
                    }
                }
            }
            return Value::Objects(found);
        }
        let mut patterns = Self::patterns(&parsed.positional);
        if patterns.is_empty() {
            patterns.push("*".to_owned());
        }
        let mut found: Vec<Object> = Vec::new();
        for pattern in &patterns {
            for clock in &self.reading.clocks {
                if matches_glob(pattern, &clock.name) && !found.iter().any(|o| o.name == clock.name)
                {
                    found.push(Object::new(ObjectClass::Clock, clock.name.clone()));
                }
            }
        }
        Value::Objects(found)
    }

    fn get_pblocks(&mut self, args: &[Value]) -> Value {
        let parsed = arguments(args, &[], &[]);
        let mut patterns = Self::patterns(&parsed.positional);
        if patterns.is_empty() {
            patterns.push("*".to_owned());
        }
        let mut found = Vec::new();
        for pattern in &patterns {
            for pblock in &self.reading.pblocks {
                if matches_glob(pattern, &pblock.name) {
                    found.push(Object::new(ObjectClass::Pblock, pblock.name.clone()));
                }
            }
        }
        Value::Objects(found)
    }

    fn get_property(&mut self, args: &[Value]) -> Value {
        let parsed = arguments(args, &[], &[]);
        let (Some(property), Some(Value::Objects(objects))) =
            (parsed.positional.first(), parsed.positional.get(1))
        else {
            return Value::Unreadable(
                "`get_property` is read only for a property of a port".into(),
            );
        };
        let property = property.as_text().to_ascii_uppercase();
        let Some(object) = objects.first().filter(|o| o.class == ObjectClass::Port) else {
            return Value::Unreadable(
                "`get_property` is read only for a property of a port".into(),
            );
        };
        let read = self.reading.ports.get(&object.name).map(|(r, _)| r);
        let answer = match property.as_str() {
            "PACKAGE_PIN" => read.and_then(|r| r.pin.clone()).unwrap_or_default(),
            "IOSTANDARD" => read
                .and_then(|r| r.io_standard.clone())
                // What Vivado says of a port nothing has set.
                .unwrap_or_else(|| "LVCMOS18".to_owned()),
            "NAME" => object.name.clone(),
            other => {
                return Value::Unreadable(format!("`get_property {other}` is not read here"));
            }
        };
        Value::Text(answer)
    }

    fn filter(&mut self, args: &[Value]) -> Value {
        match (args.first(), args.get(1)) {
            (Some(Value::Objects(objects)), Some(expr)) => {
                filter_objects(objects.clone(), &expr.as_text(), self.design)
            }
            (Some(unreadable @ Value::Unreadable(_)), _) => unreadable.clone(),
            _ => Value::Unreadable("`filter` needs objects and an expression".into()),
        }
    }

    /// `set_property [-dict {...}] [NAME VALUE] objects...`.
    fn set_property(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(args, &["dict"], &[]);
        if let Some(bad) = parsed.unknown.first() {
            self.refuse(span, format!("`set_property {bad}` is not understood"));
            return Value::Failed;
        }
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut rest = parsed.positional.as_slice();
        match parsed.option("dict") {
            Some(Value::Unreadable(why)) => {
                self.refuse(span, format!("`set_property -dict`: {why}"));
                return Value::Failed;
            }
            Some(dict) => {
                let items = split_list(&dict.as_text());
                if !items.len().is_multiple_of(2) {
                    self.refuse(span, "`set_property -dict` needs names and values in pairs");
                    return Value::Failed;
                }
                for pair in items.chunks(2) {
                    pairs.push((pair[0].clone(), pair[1].clone()));
                }
            }
            None => {
                let (Some(name), Some(value)) = (rest.first(), rest.get(1)) else {
                    self.refuse(span, "`set_property` needs a property, a value and objects");
                    return Value::Failed;
                };
                if let Value::Unreadable(why) = value {
                    self.refuse(
                        span,
                        format!(
                            "`set_property {}`: the value cannot be read, so it is not set: {why}",
                            name.as_text()
                        ),
                    );
                    return Value::Failed;
                }
                pairs.push((name.as_text(), value.as_text()));
                rest = &rest[2..];
            }
        }
        let names: Vec<String> = pairs.iter().map(|(n, _)| n.to_ascii_uppercase()).collect();
        // `LOC` of a slice can only be a cell's, and a cell's only moves it.
        let about_ports = pairs.iter().zip(&names).any(|((_, value), name)| {
            PORT_PROPERTIES.contains(&name.as_str())
                && !(name == "LOC" && site_family(value) == Some("SLICE"))
        });
        let mut objects: Vec<Object> = Vec::new();
        for value in rest {
            match value {
                Value::Objects(list) => objects.extend(list.iter().cloned()),
                Value::Unreadable(why) => {
                    if about_ports {
                        self.refuse(
                            span,
                            format!(
                                "`set_property {}`: the objects cannot be read, so it is not \
                                 set: {why}",
                                names.join(" ")
                            ),
                        );
                    } else {
                        self.not_honoured(
                            span,
                            format!("`set_property {}` is not honoured: {why}", names.join(" ")),
                        );
                    }
                    return Value::Failed;
                }
                Value::Failed => return Value::Failed,
                Value::Text(text) => {
                    // Vivado wants objects here, and refuses a name.
                    self.refuse(
                        span,
                        format!(
                            "`set_property {}` needs objects, and `{text}` is a word; \
                             `[get_ports {text}]` would be one",
                            names.join(" ")
                        ),
                    );
                    return Value::Failed;
                }
            }
        }
        if objects.is_empty() {
            let message = format!(
                "`set_property {}` applies to no object, so it is not set",
                names.join(" ")
            );
            if about_ports {
                self.refuse(span, message);
            } else {
                self.not_honoured(span, message);
            }
            return Value::Failed;
        }
        for ((_, value), name) in pairs.iter().zip(names.iter()) {
            match objects[0].class {
                ObjectClass::Port => self.port_property(name, value, &objects, span),
                ObjectClass::Design => self.design_property(name, value, span),
                ObjectClass::Cell => self.cell_property(name, value, &objects, span),
                ObjectClass::Net | ObjectClass::Pblock | ObjectClass::Clock => {
                    self.steering_property(name, value, span);
                }
                ObjectClass::Debug => self.not_honoured(
                    span,
                    format!(
                        "`{name}` is not honoured: Reticle inserts no debug core, and the design \
                         is the same without one"
                    ),
                ),
                ObjectClass::Pin | ObjectClass::Bank | ObjectClass::Site => {
                    self.refuse(
                        span,
                        format!(
                            "`set_property {name}` on a {}: Reticle has no such setting, and \
                             leaving it out could change what the part does",
                            objects[0].class.word()
                        ),
                    );
                }
            }
        }
        Value::empty()
    }

    fn port_property(&mut self, name: &str, value: &str, objects: &[Object], span: Span) {
        if !PORT_PROPERTIES.contains(&name) {
            if name == "IOB" || name == "MARK_DEBUG" || name == "DONT_TOUCH" || name == "KEEP" {
                self.not_honoured(
                    span,
                    format!("`{name}` on a port is not honoured: it can change only timing"),
                );
            } else {
                self.refuse(
                    span,
                    format!(
                        "`{name}` on a port is not something Reticle's pads have: leaving it \
                         out would configure the pad otherwise than asked"
                    ),
                );
            }
            return;
        }
        if matches!(name, "PACKAGE_PIN" | "LOC") && objects.len() > 1 && !value.is_empty() {
            self.refuse(
                span,
                format!(
                    "`{name} {value}` is applied to {} ports, and one pin is one port's",
                    objects.len()
                ),
            );
            return;
        }
        for object in objects {
            let Some((port, _)) = self.design.port_of_bit(&object.name) else {
                continue;
            };
            let output = port.dir != PortDir::In;
            if !output && matches!(name, "DRIVE" | "SLEW") {
                // Measured: Vivado warns and ignores it (12-4702).
                self.not_honoured(
                    span,
                    format!(
                        "`{name}` on `{}`, an input, is ignored, as Vivado ignores it",
                        object.name
                    ),
                );
                continue;
            }
            if !self.reading.ports.contains_key(&object.name) {
                self.reading.port_order.push(object.name.clone());
            }
            let parsed_bool = parse_bool(value);
            let drive = if name == "DRIVE" {
                match value.parse::<u32>() {
                    Ok(drive) => Some(drive),
                    Err(_) => {
                        self.refuse(span, format!("`DRIVE {value}` is not a number of mA"));
                        return;
                    }
                }
            } else {
                None
            };
            if matches!(name, "PULLUP" | "PULLDOWN") && parsed_bool.is_none() {
                self.refuse(span, format!("`{name} {value}` is neither true nor false"));
                return;
            }
            if name == "PULLTYPE"
                && !matches!(
                    value.to_ascii_uppercase().as_str(),
                    "" | "PULLUP" | "PULLDOWN" | "NONE"
                )
            {
                self.refuse(
                    span,
                    format!(
                        "`PULLTYPE {value}` is not a pull Reticle's pads have: leaving it out \
                         would configure the pad otherwise than asked"
                    ),
                );
                return;
            }
            let (read, _) = self
                .reading
                .ports
                .entry(object.name.clone())
                .or_insert_with(|| (PortRead::default(), span));
            match name {
                "PACKAGE_PIN" | "LOC" => {
                    read.pin = (!value.is_empty()).then(|| value.to_ascii_uppercase());
                }
                "IOSTANDARD" => read.io_standard = Some(value.to_ascii_uppercase()),
                "DRIVE" => read.drive = drive,
                "SLEW" => read.slew = Some(value.to_ascii_uppercase()),
                // One pull has one state, as Vivado holds it: asking for
                // one clears the other, and clearing one leaves the other.
                "PULLUP" => {
                    read.pullup = parsed_bool;
                    if parsed_bool == Some(true) && read.pulldown == Some(true) {
                        read.pulldown = Some(false);
                    }
                }
                "PULLDOWN" => {
                    read.pulldown = parsed_bool;
                    if parsed_bool == Some(true) && read.pullup == Some(true) {
                        read.pullup = Some(false);
                    }
                }
                "PULLTYPE" => match value.to_ascii_uppercase().as_str() {
                    "PULLUP" => {
                        read.pullup = Some(true);
                        read.pulldown = Some(false);
                    }
                    "PULLDOWN" => {
                        read.pulldown = Some(true);
                        read.pullup = Some(false);
                    }
                    _ => {
                        read.pullup = Some(false);
                        read.pulldown = Some(false);
                    }
                },
                _ => {}
            }
        }
    }

    fn design_property(&mut self, name: &str, value: &str, span: Span) {
        match name {
            "CFGBVS" => {
                if !matches!(value, "VCCO" | "GND") {
                    self.refuse(span, format!("`CFGBVS {value}` is neither VCCO nor GND"));
                    return;
                }
                self.no_effect(
                    span,
                    "`CFGBVS` has no effect here: Vivado checks it against the banks' voltage \
                     and writes the same bitstream, byte for byte",
                );
            }
            "CONFIG_VOLTAGE" => {
                if !matches!(value, "1.5" | "1.8" | "2.5" | "3.3") {
                    self.refuse(
                        span,
                        format!("`CONFIG_VOLTAGE {value}` is not 1.5, 1.8, 2.5 or 3.3"),
                    );
                    return;
                }
                self.no_effect(
                    span,
                    "`CONFIG_VOLTAGE` has no effect here: Vivado checks it against the banks' \
                     voltage and writes the same bitstream, byte for byte",
                );
            }
            _ if LOAD_ONLY_CONFIG.contains(&name) => {
                self.not_honoured(
                    span,
                    format!(
                        "`{name}` is not honoured: Reticle writes its own configuration \
                         packets, so the part loads as Reticle loads it, and the same design \
                         comes up"
                    ),
                );
            }
            _ if BITSTREAM_OPTIONS.contains(&name) => {
                self.refuse(
                    span,
                    format!(
                        "`{name}` is not honoured by Reticle's bitstream, and it changes what \
                         the part does, so going on without it could be wrong"
                    ),
                );
            }
            _ => {
                self.refuse(
                    span,
                    format!("`{name}` is not a property of a design that Vivado has either"),
                );
                return;
            }
        }
        self.reading
            .config
            .insert(name.to_owned(), value.to_owned());
    }

    fn cell_property(&mut self, name: &str, value: &str, objects: &[Object], span: Span) {
        match name {
            "LOC" | "BEL" => {
                let leaf: Vec<&Object> = objects
                    .iter()
                    .filter(|o| {
                        self.design
                            .instance(&o.name)
                            .is_none_or(|instance| !instance.hierarchical)
                    })
                    .collect();
                if leaf.len() != objects.len() {
                    self.not_honoured(
                        span,
                        format!("`{name}` on a hierarchical cell: Vivado refuses it too"),
                    );
                    return;
                }
                if name == "LOC" {
                    let Some(family) = site_family(value) else {
                        self.refuse(span, format!("`LOC {value}` is not a site's name"));
                        return;
                    };
                    if family != "SLICE" {
                        // A clock buffer's or a delay controller's site decides
                        // what it reaches, which placing it elsewhere can undo.
                        self.refuse(
                            span,
                            format!(
                                "`LOC {value}`: Reticle cannot fix a cell to a site, and on a \
                                 {family} that can change what the design does"
                            ),
                        );
                        return;
                    }
                }
                self.not_honoured(
                    span,
                    format!(
                        "`{name} {value}` is not honoured: Reticle's placer cannot fix a cell \
                         to a site, so `{}` goes where it places it",
                        leaf[0].name
                    ),
                );
                for object in leaf {
                    let place = self.reading.places.entry(object.name.clone()).or_default();
                    if name == "LOC" {
                        place.loc = Some(value.to_owned());
                    } else {
                        place.bel = Some(value.to_owned());
                    }
                }
            }
            "KEEP_HIERARCHY" => {
                if parse_bool(value) != Some(false) && !value.eq_ignore_ascii_case("soft") {
                    for object in objects {
                        self.reading
                            .keep_hierarchy
                            .push((object.name.clone(), span));
                    }
                }
            }
            _ => self.steering_property(name, value, span),
        }
    }

    fn steering_property(&mut self, name: &str, value: &str, span: Span) {
        if STEERING_PROPERTIES.contains(&name) {
            self.not_honoured(
                span,
                format!(
                    "`{name} {value}` is not honoured: it steers Vivado's tools, and the logic \
                     comes out the same without it"
                ),
            );
        } else {
            self.refuse(
                span,
                format!(
                    "`{name}` is not a property Reticle knows the effect of, and leaving it \
                     out could change what the part does"
                ),
            );
        }
    }

    /// What a word after `-from` and its like names: objects, or by the search
    /// Vivado calls implicit, measured to look for a clock first, then a cell,
    /// a port, and a pin.
    fn implicit(&mut self, value: &Value) -> Result<Vec<Object>, String> {
        match value {
            Value::Objects(objects) => Ok(objects.clone()),
            Value::Unreadable(why) => Err(why.clone()),
            Value::Failed => Err("it could not be read".to_owned()),
            Value::Text(text) => {
                let mut out = Vec::new();
                for name in split_list(text) {
                    let list = [Value::Text(name.clone())];
                    let attempts = [
                        self.get_clocks(&list),
                        self.get_cells(&list),
                        self.get_ports(&list),
                        self.get_pins(&list),
                    ];
                    for attempt in attempts {
                        if let Value::Objects(found) = attempt
                            && !found.is_empty()
                        {
                            out.extend(found);
                            break;
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    fn create_clock(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(args, &["period", "name", "waveform"], &["add"]);
        if let Some(bad) = parsed.unknown.first() {
            self.not_honoured(
                span,
                format!("`create_clock {bad}` is not understood, so this clock is not read"),
            );
            return Value::Failed;
        }
        let Some(period) = self.positive_period(parsed.option("period"), span) else {
            return Value::Failed;
        };
        let name = parsed.option("name").map(Value::as_text);
        let mut sources = Vec::new();
        for value in &parsed.positional {
            match value {
                Value::Objects(list) => sources.extend(list.iter().cloned()),
                Value::Unreadable(why) => {
                    self.not_honoured(span, format!("`create_clock` is not read: {why}"));
                    return Value::Failed;
                }
                Value::Failed => return Value::Failed,
                Value::Text(text) => match self.implicit(&Value::Text(text.clone())) {
                    Ok(found) => sources.extend(found),
                    Err(why) => {
                        self.not_honoured(span, format!("`create_clock` is not read: {why}"));
                        return Value::Failed;
                    }
                },
            }
        }
        if !parsed.positional.is_empty() && sources.is_empty() {
            // Vivado refuses it (12-4739): not a virtual clock, no clock.
            self.not_honoured(
                span,
                "`create_clock` names nothing that exists, so there is no such clock",
            );
            return Value::Failed;
        }
        let Some(name) = name.or_else(|| sources.first().map(|o| o.name.clone())) else {
            self.not_honoured(
                span,
                "`create_clock` with nothing to name it by makes no clock",
            );
            return Value::Failed;
        };
        let waveform = match parsed.option("waveform") {
            Some(value) => {
                let edges: Vec<Option<f64>> = split_list(&value.as_text())
                    .iter()
                    .map(|e| e.parse::<f64>().ok())
                    .collect();
                match edges.as_slice() {
                    [Some(rise), Some(fall)] => (*rise, *fall),
                    _ => {
                        self.not_honoured(
                            span,
                            format!(
                                "`create_clock -waveform {}` is not two edges, so this clock is \
                                 not read",
                                value.as_text()
                            ),
                        );
                        return Value::Failed;
                    }
                }
            }
            None => (0.0, period / 2.0),
        };
        self.define_clock(ClockRead {
            name: name.clone(),
            period,
            waveform,
            generated: false,
            sources,
            add: parsed.flag("add"),
            span,
        });
        Value::Objects(vec![Object::new(ObjectClass::Clock, name)])
    }

    fn positive_period(&mut self, value: Option<&Value>, span: Span) -> Option<f64> {
        let Some(value) = value else {
            self.not_honoured(
                span,
                "`create_clock` needs `-period`, so this clock is not read",
            );
            return None;
        };
        let text = value.as_text();
        match text.parse::<f64>() {
            Ok(period) if period.is_finite() && period > 0.0 => Some(period),
            _ => {
                self.not_honoured(
                    span,
                    format!("`create_clock -period {text}` is not a positive number of ns, so this clock is not read"),
                );
                None
            }
        }
    }

    /// A later clock of one name replaces it, and a later clock on a source
    /// without `-add` takes that source from every earlier one; each was
    /// measured (`clocks.answer`).
    fn define_clock(&mut self, clock: ClockRead) {
        // Vivado says so of each (`Constraints 18-619` and `18-1056`).
        let mut overridden: Vec<(String, Span)> = Vec::new();
        self.reading.clocks.retain(|c| {
            let same = c.name == clock.name;
            if same {
                overridden.push((c.name.clone(), c.span));
            }
            !same
        });
        if !clock.add {
            self.reading.clocks.retain_mut(|earlier| {
                // A virtual clock has no source to lose.
                if earlier.sources.is_empty() {
                    return true;
                }
                earlier
                    .sources
                    .retain(|s| !clock.sources.iter().any(|n| n.name == s.name));
                if earlier.sources.is_empty() {
                    overridden.push((earlier.name.clone(), earlier.span));
                    return false;
                }
                true
            });
        }
        for (name, span) in overridden {
            self.diags.push(
                Diagnostic::warning(format!(
                    "clock `{}` overrides clock `{name}`, which is gone, as it is in Vivado",
                    clock.name
                ))
                .with_code(XDC_NOT_HONOURED)
                .with_label(clock.span, "this clock stays")
                .with_secondary(span, "this one does not"),
            );
        }
        self.reading.clocks.push(clock);
    }

    fn create_generated_clock(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(
            args,
            &[
                "name",
                "source",
                "edges",
                "divide_by",
                "multiply_by",
                "edge_shift",
                "duty_cycle",
                "master_clock",
            ],
            &["combinational", "invert", "add"],
        );
        if let Some(bad) = parsed.unknown.first() {
            self.not_honoured(
                span,
                format!(
                    "`create_generated_clock {bad}` is not understood, so this clock is not read"
                ),
            );
            return Value::Failed;
        }
        let mut targets = Vec::new();
        for value in &parsed.positional {
            match self.implicit(value) {
                Ok(found) => targets.extend(found),
                Err(why) => {
                    self.not_honoured(span, format!("`create_generated_clock` is not read: {why}"));
                    return Value::Failed;
                }
            }
        }
        let Some(first) = targets.first() else {
            self.not_honoured(
                span,
                "`create_generated_clock` names nothing that exists, so there is no such clock",
            );
            return Value::Failed;
        };
        let name = parsed
            .option("name")
            .map_or_else(|| first.name.clone(), Value::as_text);
        let source = match parsed.option("source") {
            Some(value) => self.implicit(value).unwrap_or_default(),
            None => Vec::new(),
        };
        let Some(master) = self.master_clock(parsed.option("master_clock"), &source) else {
            self.not_honoured(
                span,
                format!(
                    "`create_generated_clock {name}`: no clock defined so far reaches its \
                     -source, so its period is not known and it is not read"
                ),
            );
            return Value::Failed;
        };
        let number = |value: Option<&Value>| value.and_then(|v| v.as_text().parse::<f64>().ok());
        let divide = number(parsed.option("divide_by")).unwrap_or(1.0);
        let multiply = number(parsed.option("multiply_by")).unwrap_or(1.0);
        let half = master / 2.0;
        let (period, mut waveform) = if let Some(edges) = parsed.option("edges") {
            let edges: Vec<f64> = split_list(&edges.as_text())
                .iter()
                .filter_map(|e| e.parse().ok())
                .collect();
            let [first, second, third] = edges.as_slice() else {
                self.not_honoured(
                    span,
                    format!("`create_generated_clock {name} -edges` is not three edges, so it is not read"),
                );
                return Value::Failed;
            };
            (
                (third - first) * half,
                ((first - 1.0) * half, (second - 1.0) * half),
            )
        } else {
            let period = master * divide / multiply;
            (period, (0.0, period / 2.0))
        };
        if parsed.flag("invert") {
            waveform = (waveform.1, waveform.0 + period);
        }
        self.define_clock(ClockRead {
            name: name.clone(),
            period,
            waveform,
            generated: true,
            sources: targets,
            add: true,
            span,
        });
        Value::Objects(vec![Object::new(ObjectClass::Clock, name)])
    }

    /// The period of the clock a generated clock divides.
    fn master_clock(&self, named: Option<&Value>, source: &[Object]) -> Option<f64> {
        if let Some(value) = named {
            let names = match value {
                Value::Objects(list) => list.iter().map(|o| o.name.clone()).collect(),
                other => split_list(&other.as_text()),
            };
            let name = names.first()?;
            return self
                .reading
                .clocks
                .iter()
                .find(|c| c.name == *name)
                .map(|c| c.period);
        }
        let object = source.first()?;
        // The net the source is on, at the top: a port is its own, a register's
        // clock pin and an instance's input are followed.
        let net = match object.class {
            ObjectClass::Port | ObjectClass::Net => object.name.clone(),
            ObjectClass::Pin => {
                let (cell, pin) = object.name.rsplit_once('/')?;
                if let Some(register) = self.design.register(cell) {
                    register.clock.clone()?
                } else {
                    let instance = self.design.instance(cell)?;
                    instance
                        .pins
                        .iter()
                        .find(|(name, _)| name == pin)
                        .and_then(|(_, net)| net.clone())?
                }
            }
            _ => return None,
        };
        self.reading
            .clocks
            .iter()
            .find(|c| c.sources.iter().any(|s| s.name == net))
            .map(|c| c.period)
    }

    /// A startpoint or an endpoint Vivado takes: not a hierarchical cell, not
    /// the output of a register as a start, not a port the wrong way round.
    fn valid_end(&self, object: &Object, from: bool) -> bool {
        match object.class {
            ObjectClass::Cell => self
                .design
                .instance(&object.name)
                .is_none_or(|instance| !instance.hierarchical),
            ObjectClass::Pin => {
                let Some((cell, pin)) = object.name.rsplit_once('/') else {
                    return false;
                };
                if self.design.register(cell).is_some() {
                    if from {
                        pin == "C"
                    } else {
                        pin != "Q" && pin != "C"
                    }
                } else {
                    self.design
                        .instance(cell)
                        .is_some_and(|instance| !instance.hierarchical)
                }
            }
            ObjectClass::Port => self
                .design
                .port_of_bit(&object.name)
                .is_some_and(|(port, _)| {
                    if from {
                        port.dir != PortDir::Out
                    } else {
                        port.dir != PortDir::In
                    }
                }),
            ObjectClass::Clock => true,
            _ => false,
        }
    }

    fn path_exception(&mut self, command: &'static str, args: &[Value], span: Span) -> Value {
        let parsed = arguments(
            args,
            &[
                "from",
                "to",
                "through",
                "rise_from",
                "rise_to",
                "fall_from",
                "fall_to",
                "rise_through",
                "fall_through",
            ],
            &[
                "setup",
                "hold",
                "start",
                "end",
                "datapath_only",
                "rise",
                "fall",
                "reset_path",
                "ignore_clock_latency",
            ],
        );
        if let Some(bad) = parsed.unknown.first() {
            self.not_honoured(
                span,
                format!("`{command} {bad}` is not understood, so this is not read"),
            );
            return Value::Failed;
        }
        let value = match command {
            "set_false_path" => None,
            _ => {
                let Some(number) = parsed.positional.first() else {
                    self.not_honoured(
                        span,
                        format!("`{command}` needs a number, so it is not read"),
                    );
                    return Value::Failed;
                };
                let text = number.as_text();
                let parsed_number = if command == "set_multicycle_path" {
                    text.parse::<u32>().ok().map(f64::from)
                } else {
                    text.parse::<f64>().ok()
                };
                let Some(parsed_number) = parsed_number else {
                    self.not_honoured(
                        span,
                        format!(
                            "`{command} {text}`: Vivado refuses this number, so it is not read"
                        ),
                    );
                    return Value::Failed;
                };
                Some(parsed_number)
            }
        };
        let edge_specific = [
            "rise_from",
            "rise_to",
            "fall_from",
            "fall_to",
            "rise_through",
            "fall_through",
        ];
        for option in edge_specific {
            if parsed.option(option).is_some() {
                self.not_honoured(
                    span,
                    format!(
                        "`{command} -{option}` is not honoured: Reticle's timing does not tell \
                         a rising path from a falling one, and this is dropped"
                    ),
                );
                return Value::Failed;
            }
        }
        let mut ends: [Vec<Object>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (slot, option) in ["from", "through", "to"].iter().enumerate() {
            for value in parsed.all(option) {
                match self.implicit(value) {
                    Ok(found) => {
                        if found.is_empty() {
                            self.not_honoured(
                                span,
                                format!(
                                    "`{command} -{option}` names nothing that exists here, so \
                                     it is not read",
                                ),
                            );
                            return Value::Failed;
                        }
                        ends[slot].extend(found);
                    }
                    Err(why) => {
                        self.not_honoured(
                            span,
                            format!("`{command} -{option}` is not read: {why}"),
                        );
                        return Value::Failed;
                    }
                }
            }
        }
        let [from, through, to] = ends;
        if from.is_empty() && through.is_empty() && to.is_empty() {
            self.not_honoured(
                span,
                format!("`{command}` needs `-from`, `-through` or `-to`, as Vivado says"),
            );
            return Value::Failed;
        }
        if parsed.flag("datapath_only") && from.is_empty() {
            self.not_honoured(
                span,
                format!(
                    "`{command} -datapath_only` needs `-from`, as Vivado says, so it is not read"
                ),
            );
            return Value::Failed;
        }
        let from: Vec<Object> = from
            .into_iter()
            .filter(|o| self.valid_end(o, true))
            .collect();
        let to: Vec<Object> = to
            .into_iter()
            .filter(|o| self.valid_end(o, false))
            .collect();
        let had_from = parsed.all("from").len();
        let had_to = parsed.all("to").len();
        if (had_from > 0 && from.is_empty()) || (had_to > 0 && to.is_empty()) {
            self.not_honoured(
                span,
                format!(
                    "`{command}`: nothing named is a valid start or end of a path, which Vivado \
                     refuses too, so it is not read"
                ),
            );
            return Value::Failed;
        }
        let mut flags = Vec::new();
        for flag in [
            "setup",
            "hold",
            "start",
            "end",
            "datapath_only",
            "rise",
            "fall",
        ] {
            if parsed.flag(flag) {
                flags.push(format!("-{flag}"));
            }
        }
        self.reading.timing.push(TimingRead {
            command,
            flags,
            name: None,
            clock: Vec::new(),
            from,
            through,
            to,
            groups: Vec::new(),
            value,
            objects: Vec::new(),
            span,
        });
        Value::empty()
    }

    fn io_delay(&mut self, command: &'static str, args: &[Value], span: Span) -> Value {
        let parsed = arguments(
            args,
            &["clock", "reference_pin"],
            &[
                "clock_fall",
                "max",
                "min",
                "add_delay",
                "rise",
                "fall",
                "network_latency_included",
                "source_latency_included",
            ],
        );
        if let Some(bad) = parsed.unknown.first() {
            self.not_honoured(
                span,
                format!("`{command} {bad}` is not understood, so this is not read"),
            );
            return Value::Failed;
        }
        let mut clock = Vec::new();
        if let Some(value) = parsed.option("clock") {
            let found = match value {
                Value::Objects(list) => list.clone(),
                other => match self.get_clocks(&[Value::Text(other.as_text())]) {
                    Value::Objects(list) => list,
                    _ => Vec::new(),
                },
            };
            if found.is_empty() {
                self.not_honoured(
                    span,
                    format!(
                        "`{command} -clock`: no clock of that name is known here, so it is not \
                         read{}",
                        if value.as_text().is_empty() {
                            String::new()
                        } else {
                            format!(" (`{}`)", value.as_text())
                        }
                    ),
                );
                return Value::Failed;
            }
            clock = found;
        }
        let Some(number) = parsed.positional.first() else {
            self.not_honoured(
                span,
                format!("`{command}` needs a delay, so it is not read"),
            );
            return Value::Failed;
        };
        let text = number.as_text();
        let Ok(delay) = text.parse::<f64>() else {
            self.not_honoured(
                span,
                format!("`{command} {text}` is not a delay, so it is not read"),
            );
            return Value::Failed;
        };
        let mut objects = Vec::new();
        for value in &parsed.positional[1..] {
            match self.implicit(value) {
                Ok(found) => objects.extend(found),
                Err(why) => {
                    self.not_honoured(span, format!("`{command}` is not read: {why}"));
                    return Value::Failed;
                }
            }
        }
        let wanted = command == "set_input_delay";
        let kept: Vec<Object> = objects
            .iter()
            .filter(|o| {
                o.class == ObjectClass::Port
                    && self.design.port_of_bit(&o.name).is_some_and(|(port, _)| {
                        if wanted {
                            port.dir != PortDir::Out
                        } else {
                            port.dir != PortDir::In
                        }
                    })
            })
            .cloned()
            .collect();
        if kept.is_empty() {
            self.not_honoured(
                span,
                format!(
                    "`{command}` names no {} port, which Vivado refuses too, so it is not read",
                    if wanted { "input" } else { "output" }
                ),
            );
            return Value::Failed;
        }
        let mut flags = Vec::new();
        for flag in [
            "clock_fall",
            "rise",
            "fall",
            "max",
            "min",
            "add_delay",
            "network_latency_included",
            "source_latency_included",
        ] {
            if parsed.flag(flag) {
                flags.push(format!("-{flag}"));
            }
        }
        if parsed.option("reference_pin").is_some() {
            flags.push("-reference_pin".to_owned());
        }
        self.reading.timing.push(TimingRead {
            command,
            flags,
            name: None,
            clock,
            from: Vec::new(),
            through: Vec::new(),
            to: Vec::new(),
            groups: Vec::new(),
            value: Some(delay),
            objects: kept,
            span,
        });
        Value::empty()
    }

    fn set_clock_groups(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(
            args,
            &["group", "name"],
            &[
                "asynchronous",
                "logically_exclusive",
                "physically_exclusive",
            ],
        );
        if let Some(bad) = parsed.unknown.first() {
            self.not_honoured(
                span,
                format!("`set_clock_groups {bad}` is not understood, so this is not read"),
            );
            return Value::Failed;
        }
        let kinds: Vec<String> = [
            "asynchronous",
            "logically_exclusive",
            "physically_exclusive",
        ]
        .iter()
        .filter(|k| parsed.flag(k))
        .map(|k| format!("-{k}"))
        .collect();
        if kinds.is_empty() {
            self.not_honoured(
                span,
                "`set_clock_groups` needs one of `-asynchronous`, `-logically_exclusive` and \
                 `-physically_exclusive`, as Vivado says, so it is not read",
            );
            return Value::Failed;
        }
        let mut groups = Vec::new();
        let mut missing = false;
        for value in parsed.all("group") {
            let found = match value {
                Value::Objects(list) => list.clone(),
                other => match self.get_clocks(&[Value::Text(other.as_text())]) {
                    Value::Objects(list) => list,
                    _ => Vec::new(),
                },
            };
            if found.is_empty() {
                missing = true;
            } else {
                groups.push(found);
            }
        }
        if groups.is_empty() || (missing && groups.len() < 2) {
            self.not_honoured(
                span,
                "`set_clock_groups`: a group names no clock known here and fewer than two \
                 remain, so it is not read",
            );
            return Value::Failed;
        }
        self.reading.timing.push(TimingRead {
            command: "set_clock_groups",
            flags: kinds,
            name: parsed.option("name").map(Value::as_text),
            clock: Vec::new(),
            from: Vec::new(),
            through: Vec::new(),
            to: Vec::new(),
            groups,
            value: None,
            objects: Vec::new(),
            span,
        });
        Value::empty()
    }

    fn create_pblock(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(args, &[], &[]);
        let Some(name) = parsed.positional.first().map(Value::as_text) else {
            self.not_honoured(span, "`create_pblock` needs a name");
            return Value::Failed;
        };
        if self.reading.pblocks.iter().any(|p| p.name == name) {
            self.not_honoured(
                span,
                format!(
                    "pblock `{name}` already exists, which Vivado refuses too; this is ignored"
                ),
            );
            return Value::Failed;
        }
        self.reading.pblocks.push(PblockRead {
            name: name.clone(),
            ranges: Vec::new(),
            cells: Vec::new(),
            span,
        });
        Value::Objects(vec![Object::new(ObjectClass::Pblock, name)])
    }

    fn pblock_index(&self, value: Option<&Value>) -> Option<usize> {
        let name = match value? {
            Value::Objects(list) => list.first()?.name.clone(),
            other => other.as_text(),
        };
        self.reading.pblocks.iter().position(|p| p.name == name)
    }

    fn add_cells_to_pblock(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(args, &[], &["top", "add_primitives", "clear_locs"]);
        let Some(index) = self.pblock_index(parsed.positional.first()) else {
            self.not_honoured(
                span,
                "`add_cells_to_pblock`: no such pblock, so it is not read",
            );
            return Value::Failed;
        };
        if parsed.flag("top") {
            self.not_honoured(
                span,
                "`add_cells_to_pblock -top` is not honoured: a whole design in one region is \
                 not something Reticle's regions say",
            );
            return Value::Failed;
        }
        let mut cells: Vec<String> = Vec::new();
        for value in &parsed.positional[1..] {
            match self.implicit(value) {
                Ok(found) => cells.extend(found.into_iter().map(|o| o.name)),
                Err(why) => {
                    self.not_honoured(span, format!("`add_cells_to_pblock` is not read: {why}"));
                    return Value::Failed;
                }
            }
        }
        if cells.is_empty() {
            self.not_honoured(
                span,
                "`add_cells_to_pblock` names no cell here, so nothing is added",
            );
            return Value::Failed;
        }
        self.reading.pblocks[index].cells.extend(cells);
        Value::empty()
    }

    fn resize_pblock(&mut self, args: &[Value], span: Span) -> Value {
        let parsed = arguments(args, &["add", "remove", "from", "to", "locs"], &["replace"]);
        let Some(index) = self.pblock_index(parsed.positional.first()) else {
            self.not_honoured(span, "`resize_pblock`: no such pblock, so it is not read");
            return Value::Failed;
        };
        if parsed.option("remove").is_some() {
            self.not_honoured(
                span,
                "`resize_pblock -remove` is not honoured: a Reticle region is one rectangle",
            );
        }
        for value in parsed.all("add") {
            self.reading.pblocks[index]
                .ranges
                .extend(split_list(&value.as_text()));
        }
        Value::empty()
    }
}

/// Whether a Vivado object name answers a pattern. Without `hierarchical`, the
/// pattern and the name must have as many `/`-separated levels, each answering;
/// with it, the pattern need only answer the end of the name, on a boundary.
fn name_matches(pattern: &str, name: &str, hierarchical: bool) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let levels: Vec<&str> = name.split('/').collect();
    if levels.len() < pattern.len() {
        return false;
    }
    if !hierarchical && levels.len() != pattern.len() {
        return false;
    }
    let tail = &levels[levels.len() - pattern.len()..];
    pattern
        .iter()
        .zip(tail)
        .all(|(glob, level)| matches_glob(glob, level))
}

fn list_value(args: &[Value]) -> Value {
    if args.iter().any(|a| matches!(a, Value::Objects(_))) {
        let mut out = Vec::new();
        for arg in args {
            match arg {
                Value::Objects(list) => out.extend(list.iter().cloned()),
                Value::Unreadable(_) => return arg.clone(),
                _ => {}
            }
        }
        return Value::Objects(out);
    }
    if let Some(bad) = args.iter().find(|a| matches!(a, Value::Unreadable(_))) {
        return bad.clone();
    }
    let words: Vec<String> = args
        .iter()
        .map(|a| {
            let text = a.as_text();
            if text.is_empty() || text.contains(char::is_whitespace) {
                format!("{{{text}}}")
            } else {
                text
            }
        })
        .collect();
    Value::Text(words.join(" "))
}

fn objects_of(args: &[Value], class: ObjectClass) -> Value {
    let parsed = arguments(args, &[], &[]);
    let mut out = Vec::new();
    for value in &parsed.positional {
        for word in split_list(&value.as_text()) {
            out.push(Object::new(class, word));
        }
    }
    Value::Objects(out)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// The part of a site's name before its coordinates: `SLICE_X0Y1` is `SLICE`.
fn site_family(site: &str) -> Option<&str> {
    let (family, coordinates) = site.rsplit_once('_')?;
    let rest = coordinates.strip_prefix('X')?;
    let (x, y) = rest.split_once('Y')?;
    (!family.is_empty()
        && x.bytes().all(|b| b.is_ascii_digit())
        && y.bytes().all(|b| b.is_ascii_digit())
        && !x.is_empty()
        && !y.is_empty())
    .then_some(family)
}

/// A `SLICE_X1Y2:SLICE_X3Y4` range, in either corner order.
fn slice_range(range: &str) -> Option<(u32, u32, u32, u32)> {
    let (a, b) = range.split_once(':').unwrap_or((range, range));
    let coordinates = |site: &str| -> Option<(u32, u32)> {
        let rest = site.strip_prefix("SLICE_X")?;
        let (x, y) = rest.split_once('Y')?;
        Some((x.parse().ok()?, y.parse().ok()?))
    };
    let (x0, y0) = coordinates(a)?;
    let (x1, y1) = coordinates(b)?;
    Some((x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)))
}

/// `-filter`: `&&` and `||` over comparisons of `NAME` and, of a port,
/// `DIRECTION`. Anything else is unreadable rather than guessed at.
fn filter_objects(objects: Vec<Object>, expression: &str, design: &XdcDesign) -> Value {
    let mut out = Vec::new();
    for object in objects {
        match filter_any(expression, &object, design) {
            Ok(true) => out.push(object),
            Ok(false) => {}
            Err(why) => return Value::Unreadable(why),
        }
    }
    Value::Objects(out)
}

fn filter_any(expression: &str, object: &Object, design: &XdcDesign) -> Result<bool, String> {
    for alternative in expression.split("||") {
        let mut all = true;
        for term in alternative.split("&&") {
            all &= filter_term(term.trim(), object, design)?;
        }
        if all {
            return Ok(true);
        }
    }
    Ok(false)
}

fn filter_term(term: &str, object: &Object, design: &XdcDesign) -> Result<bool, String> {
    let (key, operator, value) = ["=~", "!~", "==", "!="]
        .iter()
        .find_map(|op| {
            term.split_once(op)
                .map(|(k, v)| (k.trim(), *op, v.trim().trim_matches('"')))
        })
        .ok_or_else(|| format!("the filter `{term}` is not read here"))?;
    let actual = match key {
        "NAME" => object.name.clone(),
        "DIRECTION" => {
            let Some((port, _)) = design.port_of_bit(&object.name) else {
                return Err(format!("the filter `{term}` is read only for a port"));
            };
            match port.dir {
                PortDir::In => "IN".to_owned(),
                PortDir::Out => "OUT".to_owned(),
                PortDir::InOut => "INOUT".to_owned(),
            }
        }
        "REF_NAME" => {
            let Some(instance) = design.instance(&object.name) else {
                return Err(format!(
                    "the filter `{term}` is read only for an instance; synthesis decides what a \
                     register's cell is"
                ));
            };
            instance.module.clone()
        }
        _ => return Err(format!("the filter `{term}` is not read here")),
    };
    Ok(match operator {
        "==" => actual == value,
        "!=" => actual != value,
        "=~" => matches_glob(value, &actual),
        _ => !matches_glob(value, &actual),
    })
}

/// A value of `expr`.
#[derive(Clone, Debug, PartialEq)]
enum ExprValue {
    Int(i64),
    Float(f64),
    Text(String),
}

impl ExprValue {
    fn text(&self) -> String {
        match self {
            ExprValue::Int(v) => v.to_string(),
            ExprValue::Float(v) => {
                if v.fract() == 0.0 && v.abs() < 1e15 {
                    format!("{v:.1}")
                } else {
                    v.to_string()
                }
            }
            ExprValue::Text(t) => t.clone(),
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            ExprValue::Int(v) => i32::try_from(*v).ok().map(f64::from),
            ExprValue::Float(v) => Some(*v),
            ExprValue::Text(_) => None,
        }
    }
}

/// The arithmetic of `expr`: enough for a period computed from another, and a
/// `?:`. A thing it does not know ends the evaluation, and so the command.
struct Expr<'a> {
    bytes: &'a [u8],
    pos: usize,
    vars: &'a BTreeMap<String, String>,
}

impl<'a> Expr<'a> {
    fn new(text: &'a str, vars: &'a BTreeMap<String, String>) -> Self {
        Expr {
            bytes: text.as_bytes(),
            pos: 0,
            vars,
        }
    }

    fn run(mut self) -> Option<ExprValue> {
        let value = self.ternary()?;
        self.blanks();
        (self.pos == self.bytes.len()).then_some(value)
    }

    fn blanks(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.pos += 1;
        }
    }

    fn eat(&mut self, token: &str) -> bool {
        self.blanks();
        if self
            .bytes
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(token.as_bytes()))
        {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    fn ternary(&mut self) -> Option<ExprValue> {
        let condition = self.comparison()?;
        if !self.eat("?") {
            return Some(condition);
        }
        let yes = self.ternary()?;
        if !self.eat(":") {
            return None;
        }
        let no = self.ternary()?;
        let truth = condition.number()? != 0.0;
        Some(if truth { yes } else { no })
    }

    fn comparison(&mut self) -> Option<ExprValue> {
        let left = self.sum()?;
        for (token, test) in [
            ("==", (|a, b| a == b) as fn(f64, f64) -> bool),
            ("!=", |a, b| a != b),
            ("<=", |a, b| a <= b),
            (">=", |a, b| a >= b),
            ("<", |a, b| a < b),
            (">", |a, b| a > b),
        ] {
            if self.eat(token) {
                let right = self.sum()?;
                return Some(ExprValue::Int(i64::from(test(
                    left.number()?,
                    right.number()?,
                ))));
            }
        }
        Some(left)
    }

    fn sum(&mut self) -> Option<ExprValue> {
        let mut left = self.product()?;
        loop {
            if self.eat("+") {
                let right = self.product()?;
                left = arithmetic(&left, &right, b'+')?;
            } else if self.eat("-") {
                let right = self.product()?;
                left = arithmetic(&left, &right, b'-')?;
            } else {
                return Some(left);
            }
        }
    }

    fn product(&mut self) -> Option<ExprValue> {
        let mut left = self.unary()?;
        loop {
            if self.eat("*") {
                let right = self.unary()?;
                left = arithmetic(&left, &right, b'*')?;
            } else if self.eat("/") {
                let right = self.unary()?;
                left = arithmetic(&left, &right, b'/')?;
            } else {
                return Some(left);
            }
        }
    }

    fn unary(&mut self) -> Option<ExprValue> {
        if self.eat("-") {
            let value = self.unary()?;
            return arithmetic(&ExprValue::Int(0), &value, b'-');
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<ExprValue> {
        self.blanks();
        if self.eat("(") {
            let value = self.ternary()?;
            return self.eat(")").then_some(value);
        }
        match self.bytes.get(self.pos)? {
            b'"' => {
                self.pos += 1;
                let start = self.pos;
                while self.bytes.get(self.pos).is_some_and(|c| *c != b'"') {
                    self.pos += 1;
                }
                let text = String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned();
                self.pos += 1;
                Some(ExprValue::Text(text))
            }
            b'$' => {
                self.pos += 1;
                let start = self.pos;
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    self.pos += 1;
                }
                let name = String::from_utf8_lossy(&self.bytes[start..self.pos]);
                let value = self.vars.get(name.as_ref())?;
                Some(number_or_text(value))
            }
            c if c.is_ascii_digit() || *c == b'.' => {
                let start = self.pos;
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.pos += 1;
                }
                let text = String::from_utf8_lossy(&self.bytes[start..self.pos]);
                Some(number_or_text(&text))
            }
            _ => None,
        }
    }
}

fn number_or_text(text: &str) -> ExprValue {
    if let Ok(v) = text.parse::<i64>() {
        ExprValue::Int(v)
    } else if let Ok(v) = text.parse::<f64>() {
        ExprValue::Float(v)
    } else {
        ExprValue::Text(text.to_owned())
    }
}

fn arithmetic(left: &ExprValue, right: &ExprValue, op: u8) -> Option<ExprValue> {
    if let (ExprValue::Int(a), ExprValue::Int(b)) = (left, right) {
        return match op {
            b'+' => a.checked_add(*b),
            b'-' => a.checked_sub(*b),
            b'*' => a.checked_mul(*b),
            _ => a.checked_div_euclid(*b),
        }
        .map(ExprValue::Int);
    }
    let (a, b) = (left.number()?, right.number()?);
    Some(ExprValue::Float(match op {
        b'+' => a + b,
        b'-' => a - b,
        b'*' => a * b,
        _ => a / b,
    }))
}

/// What the flow can use of a reading.
struct Translate<'a> {
    reading: &'a XdcReading,
    design: &'a XdcDesign,
    diags: &'a mut Diagnostics,
    out: Constraints,
    /// Names of the clocks that became a [`ClockDef`].
    kept_clocks: Vec<String>,
}

impl Translate<'_> {
    fn warn(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::warning(message)
                .with_code(XDC_NOT_HONOURED)
                .with_span(span),
        );
    }

    fn note(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::new(Severity::Note, message)
                .with_code(XDC_NO_EFFECT)
                .with_span(span),
        );
    }

    fn run(mut self) -> Constraints {
        self.pins();
        self.clocks();
        let timing: Vec<&TimingRead> = self.reading.timing.iter().collect();
        for read in timing {
            match read.command {
                "set_false_path" | "set_multicycle_path" | "set_max_delay" => self.exception(read),
                "set_min_delay" => self.warn(
                    read.span,
                    "`set_min_delay` is not honoured: Reticle's placement has no hold analysis, \
                     so this is dropped",
                ),
                "set_input_delay" => self.input_delay(read),
                "set_output_delay" => self.warn(
                    read.span,
                    "`set_output_delay` is not honoured: Reticle's timing has no output delays, \
                     so this pad is timed from its buffer alone",
                ),
                _ => self.note(
                    read.span,
                    "`set_clock_groups` needs nothing done here: Reticle's placement times no \
                     path between two clocks at all, grouped or not",
                ),
            }
        }
        self.placement();
        self.out
    }

    fn pins(&mut self) {
        for bit in &self.reading.port_order {
            let Some((read, span)) = self.reading.ports.get(bit) else {
                continue;
            };
            let Some((port, offset)) = self.design.port_of_bit(bit) else {
                continue;
            };
            self.out.pins.push(PinAssignment {
                port: port.name.clone(),
                bit: offset,
                pin: read.pin.clone().unwrap_or_default(),
                io: IoAttrs {
                    io_standard: read.io_standard.clone(),
                    drive: read.drive,
                    slew: read.slew.as_ref().map(|s| s.to_ascii_lowercase()),
                    pullup: read.pullup,
                    pulldown: read.pulldown,
                    ddr: None,
                    delay: None,
                },
                span: *span,
                origin: Origin::File,
            });
        }
    }

    fn clocks(&mut self) {
        for clock in &self.reading.clocks {
            let name = clock.name.clone();
            if clock.sources.is_empty() {
                self.warn(
                    clock.span,
                    format!(
                        "clock `{name}` is virtual, which Reticle's timing has no way to hold: \
                         each delay against it is dropped, with a warning of its own"
                    ),
                );
                continue;
            }
            let default = (0.0, clock.period / 2.0);
            if (clock.waveform.0 - default.0).abs() > 1e-9
                || (clock.waveform.1 - default.1).abs() > 1e-9
            {
                self.warn(
                    clock.span,
                    format!(
                        "clock `{name}` has waveform {{{} {}}}, and Reticle's timing holds only \
                         a period: half-cycle paths are timed as if it were even",
                        clock.waveform.0, clock.waveform.1
                    ),
                );
            }
            if clock.sources.len() > 1 {
                self.warn(
                    clock.span,
                    format!(
                        "clock `{name}` is on {} objects, and Reticle's holds one net: it is \
                         kept on `{}` alone",
                        clock.sources.len(),
                        clock.sources[0].name
                    ),
                );
            }
            let source = &clock.sources[0];
            let Some(net) = self.clock_net(clock, source) else {
                continue;
            };
            if self.out.clocks.iter().any(|c| c.net == net) {
                self.warn(
                    clock.span,
                    format!(
                        "clock `{name}`: `{net}` already has a clock, and Reticle's timing holds \
                         one a net, so this `-add` is dropped"
                    ),
                );
                continue;
            }
            self.out.clocks.push(ClockDef {
                name: name.clone(),
                net,
                period_ns: clock.period,
                span: clock.span,
                origin: Origin::File,
            });
            self.kept_clocks.push(name);
        }
    }

    /// The net a clock can be held on, or why not.
    fn clock_net(&mut self, clock: &ClockRead, source: &Object) -> Option<String> {
        let span = clock.span;
        let name = &clock.name;
        match source.class {
            ObjectClass::Port => {
                let (port, offset) = self.design.port_of_bit(&source.name)?;
                if offset.is_some() {
                    self.warn(
                        span,
                        format!(
                            "clock `{name}` is on `{}`, one bit of a bus, and Reticle's clocks \
                             are on whole nets: it is dropped, and what it clocks is untimed",
                            source.name
                        ),
                    );
                    return None;
                }
                Some(port.name.clone())
            }
            ObjectClass::Net => {
                let (_, netlist) = self
                    .design
                    .nets
                    .iter()
                    .find(|(vivado, _)| *vivado == source.name)?;
                if self.design.unwritten.iter().any(|n| n == netlist) {
                    self.diags.push(
                        Diagnostic::error(format!(
                            "clock `{name}` is on `{netlist}`, which nothing drives. In a `.rcf` \
                             that asks for a PLL, which an XDC cannot mean, so it is refused"
                        ))
                        .with_code(XDC_REFUSED)
                        .with_span(span),
                    );
                    return None;
                }
                Some(netlist.clone())
            }
            ObjectClass::Pin => self.clock_on_pin(clock, source),
            _ => {
                self.warn(
                    span,
                    format!(
                        "clock `{name}` is on `{}`, which is not a net: it is dropped",
                        source.name
                    ),
                );
                None
            }
        }
    }

    fn clock_on_pin(&mut self, clock: &ClockRead, source: &Object) -> Option<String> {
        let span = clock.span;
        let name = &clock.name;
        let (cell, pin) = source.name.rsplit_once('/')?;
        if let Some(instance) = self.design.instance(cell) {
            let manager = instance.module.starts_with("MMCM") || instance.module.starts_with("PLL");
            if manager && pin.starts_with("CLK") && clock.generated {
                self.note(
                    span,
                    format!(
                        "generated clock `{name}` needs nothing done here: Reticle's timing \
                         works out `{}` from `{}`'s parameters, as Vivado would without being \
                         asked",
                        source.name, instance.module
                    ),
                );
                return None;
            }
        }
        if pin == "Q"
            && let Some(register) = self.design.register(cell)
        {
            if let Some(net) = &register.scalar_net {
                return Some(net.clone());
            }
            self.warn(
                span,
                format!(
                    "clock `{name}` is on `{}`, one bit of a register, and Reticle's clocks \
                     are on whole nets: it is dropped, and what it clocks is untimed",
                    source.name
                ),
            );
            return None;
        }
        self.warn(
            span,
            format!(
                "clock `{name}` is on the pin `{}`, which Reticle's timing cannot hold a clock \
                 on: it is dropped, and what it clocks is untimed",
                source.name
            ),
        );
        None
    }

    /// The netlist names a list of ports makes, a whole port by its name.
    fn port_names(&self, objects: &[&Object]) -> Vec<String> {
        let mut out = Vec::new();
        for port in &self.design.ports {
            let bits: Vec<u32> = objects
                .iter()
                .filter_map(|o| match self.design.port_of_bit(&o.name) {
                    Some((p, Some(offset))) if p.name == port.name => Some(offset),
                    _ => None,
                })
                .collect();
            let scalar = objects.iter().any(|o| o.name == port.name);
            if scalar || (port.vector && bits.len() == port.width as usize) {
                out.push(port.name.clone());
            } else {
                out.extend(bits.iter().map(|bit| format!("{}$io{bit}", port.name)));
            }
        }
        out
    }

    /// What the netlist calls one end's objects; those it cannot name are said.
    fn end_names(&mut self, command: &str, objects: &[Object], span: Span) -> Vec<String> {
        let ports: Vec<&Object> = objects
            .iter()
            .filter(|o| o.class == ObjectClass::Port)
            .collect();
        let mut out = self.port_names(&ports);
        for object in objects {
            match object.class {
                ObjectClass::Port => {}
                ObjectClass::Cell => {
                    if let Some(name) = cell_netlist_name(self.design, &object.name) {
                        out.push(name);
                    }
                }
                ObjectClass::Pin => {
                    let Some((cell, pin)) = object.name.rsplit_once('/') else {
                        continue;
                    };
                    // An exception here applies to a whole instance, so the
                    // clock pin and D stand for it, and a CE does not: that
                    // would take D's paths out with it.
                    if !matches!(pin, "C" | "D" | "CLK" | "I" | "I0" | "O") {
                        self.warn(
                            span,
                            format!(
                                "`{command}`'s `{}` is not honoured: Reticle applies an \
                                 exception to a whole cell, and this pin is not all of it",
                                object.name
                            ),
                        );
                        continue;
                    }
                    if let Some(name) = cell_netlist_name(self.design, cell) {
                        out.push(name);
                    }
                }
                _ => {
                    self.warn(
                        span,
                        format!(
                            "`{command}`'s `{}` is not honoured: it is not a thing a path \
                             begins or ends at here",
                            object.name
                        ),
                    );
                }
            }
        }
        out
    }

    fn exception(&mut self, read: &TimingRead) {
        let command = read.command;
        let span = read.span;
        if !read.through.is_empty() {
            let effect = if command == "set_max_delay" {
                "that delay is not asked of those paths"
            } else {
                "those paths are timed as ordinary paths"
            };
            self.warn(
                span,
                format!(
                    "`{command} -through` is not honoured: Reticle names a path by its two ends \
                     alone, so {effect}"
                ),
            );
            return;
        }
        let clocks = |list: &[Object]| list.iter().any(|o| o.class == ObjectClass::Clock);
        if clocks(&read.from) || clocks(&read.to) {
            self.clock_exception(read);
            return;
        }
        let hold = read.flags.iter().any(|f| f == "-hold");
        let setup = read.flags.iter().any(|f| f == "-setup");
        if hold && !setup && command == "set_false_path" {
            self.note(
                span,
                "`set_false_path -hold` needs nothing done here: Reticle's placement has no \
                 hold analysis",
            );
            return;
        }
        let from = self.end_names(command, &read.from, span);
        let to = self.end_names(command, &read.to, span);
        if (!read.from.is_empty() && from.is_empty()) || (!read.to.is_empty() && to.is_empty()) {
            return;
        }
        let from_list: Vec<Option<String>> = if from.is_empty() {
            vec![None]
        } else {
            from.into_iter().map(Some).collect()
        };
        let to_list: Vec<Option<String>> = if to.is_empty() {
            vec![None]
        } else {
            to.into_iter().map(Some).collect()
        };
        let both = !read.from.is_empty() && !read.to.is_empty();
        if command == "set_multicycle_path" {
            let cycles = read.value.unwrap_or(1.0);
            if cycles < 1.0 {
                self.warn(
                    span,
                    "`set_multicycle_path 0` is not honoured: Vivado takes it, and Reticle's \
                     paths span at least a cycle",
                );
                return;
            }
        }
        if command == "set_max_delay" && read.value.unwrap_or(0.0) <= 0.0 {
            self.warn(
                span,
                "`set_max_delay` of no time is not honoured: Reticle's asks a positive delay",
            );
            return;
        }
        if both {
            self.warn(
                span,
                format!(
                    "`{command}` with both `-from` and `-to` is kept and not applied by the \
                     timing-driven placement, which cannot tell the paths between two ends apart"
                ),
            );
        } else if command == "set_multicycle_path" && read.to.is_empty() && !hold {
            self.warn(
                span,
                "`set_multicycle_path -from` alone is kept and not applied by the timing-driven \
                 placement, which applies a multicycle path by where it ends",
            );
        } else if command == "set_multicycle_path" && hold {
            self.note(
                span,
                "`set_multicycle_path -hold` is kept and needs nothing done by the placement, \
                 which has no hold analysis",
            );
        }
        for f in &from_list {
            for t in &to_list {
                let path = PathSpec {
                    from: f.clone(),
                    to: t.clone(),
                    span,
                };
                match command {
                    "set_false_path" => self.out.false_paths.push(path),
                    "set_multicycle_path" => {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let cycles = read.value.unwrap_or(1.0) as u32;
                        self.out
                            .multicycle_paths
                            .push(MulticyclePath { cycles, hold, path });
                    }
                    _ => self.out.max_delays.push(MaxDelay {
                        delay_ns: read.value.unwrap_or(0.0),
                        path,
                    }),
                }
            }
        }
    }

    /// An exception naming clocks: between different clocks, there is nothing
    /// to remove, as nothing is timed across. Any other cannot be said here.
    fn clock_exception(&mut self, read: &TimingRead) {
        let command = read.command;
        let span = read.span;
        let from: Vec<&Object> = read
            .from
            .iter()
            .filter(|o| o.class == ObjectClass::Clock)
            .collect();
        let to: Vec<&Object> = read
            .to
            .iter()
            .filter(|o| o.class == ObjectClass::Clock)
            .collect();
        let across = !from.is_empty()
            && !to.is_empty()
            && from.len() == read.from.len()
            && to.len() == read.to.len()
            && !from.iter().any(|a| to.iter().any(|b| a.name == b.name));
        if across && command == "set_false_path" {
            self.note(
                span,
                "`set_false_path` between clocks needs nothing done here: Reticle's placement \
                 times no path between two clocks at all",
            );
        } else if across {
            self.warn(
                span,
                format!(
                    "`{command}` between clocks is not honoured: Reticle's placement times no \
                     path between two clocks at all, so nothing asks these paths for this"
                ),
            );
        } else {
            self.warn(
                span,
                format!(
                    "`{command}` on a clock's paths is not honoured: Reticle's exceptions name \
                     cells, not clocks, so those paths are timed as ordinary paths"
                ),
            );
        }
    }

    fn input_delay(&mut self, read: &TimingRead) {
        let span = read.span;
        let Some(clock) = read.clock.first() else {
            self.warn(
                span,
                "`set_input_delay` with no `-clock` is not honoured: Vivado works one out, and \
                 Reticle does not",
            );
            return;
        };
        for flag in [
            "-clock_fall",
            "-rise",
            "-fall",
            "-reference_pin",
            "-network_latency_included",
            "-source_latency_included",
        ] {
            if read.flags.iter().any(|f| f == flag) {
                self.warn(
                    span,
                    format!(
                        "`set_input_delay {flag}` is not honoured: Reticle's holds a delay after \
                         a rising edge of a clock, so this one is dropped"
                    ),
                );
                return;
            }
        }
        if read.flags.iter().any(|f| f == "-min") && !read.flags.iter().any(|f| f == "-max") {
            self.note(
                span,
                "`set_input_delay -min` needs nothing done here: Reticle's placement has no \
                 hold analysis",
            );
            return;
        }
        if !self.kept_clocks.contains(&clock.name) {
            self.warn(
                span,
                format!(
                    "`set_input_delay -clock {}` is not honoured: that clock has no net here, so \
                     this pad is timed in every domain from its buffer alone",
                    clock.name
                ),
            );
            return;
        }
        let ports: Vec<&Object> = read.objects.iter().collect();
        let names = self.port_names(&ports);
        let add = read.flags.iter().any(|f| f == "-add_delay");
        if add
            && self
                .out
                .input_delays
                .iter()
                .any(|d| d.clock != clock.name && d.ports.iter().any(|p| names.contains(p)))
        {
            self.warn(
                span,
                "`set_input_delay -add_delay` against a second clock is not honoured: Reticle's \
                 holds a pad in one domain, so this one is dropped",
            );
            return;
        }
        self.out.input_delays.push(InputDelay {
            clock: clock.name.clone(),
            delay_ns: read.value.unwrap_or(0.0),
            ports: names,
            span,
        });
    }

    fn placement(&mut self) {
        for (cell, span) in &self.reading.keep_hierarchy {
            if self.design.instance(cell).is_some() {
                self.out
                    .keep_hierarchy
                    .push((cell.replace('/', "."), *span, Origin::File));
            }
        }
        for pblock in &self.reading.pblocks {
            let span = pblock.span;
            let name = &pblock.name;
            let mut box_: Option<(u32, u32, u32, u32)> = None;
            let mut slices = 0usize;
            let mut others = Vec::new();
            for range in &pblock.ranges {
                match slice_range(range) {
                    Some(r) => {
                        slices += 1;
                        box_ = Some(match box_ {
                            Some(b) => (b.0.min(r.0), b.1.min(r.1), b.2.max(r.2), b.3.max(r.3)),
                            None => r,
                        });
                    }
                    None => others.push(range.as_str()),
                }
            }
            let Some((x0, y0, x1, y1)) = box_ else {
                self.warn(
                    span,
                    format!(
                        "pblock `{name}` has no `SLICE_` range{}, which is all a Reticle region \
                         is made from: it confines nothing",
                        if others.is_empty() {
                            String::new()
                        } else {
                            format!(" (it has {})", others.join(", "))
                        }
                    ),
                );
                continue;
            };
            if slices > 1 {
                self.warn(
                    span,
                    format!(
                        "pblock `{name}` has {slices} `SLICE_` ranges and a Reticle region is \
                         one rectangle: its cells may go anywhere in the box around them"
                    ),
                );
            }
            if !others.is_empty() {
                self.warn(
                    span,
                    format!(
                        "pblock `{name}`'s {} is not honoured: a Reticle region is a rectangle \
                         of tiles, from its `SLICE_` ranges",
                        others.join(", ")
                    ),
                );
            }
            self.out.site_regions.push(SiteRegion {
                name: name.clone(),
                x0,
                y0,
                x1,
                y1,
                span,
            });
            for cell in &pblock.cells {
                let pattern = match self.design.instance(cell) {
                    Some(instance) if instance.hierarchical => {
                        Some(format!("{}.*", cell.replace('/', ".")))
                    }
                    _ => cell_netlist_name(self.design, cell),
                };
                if let Some(pattern) = pattern {
                    self.out.region_assignments.push(RegionAssignment {
                        pattern,
                        region: name.clone(),
                        span,
                        origin: Origin::File,
                    });
                }
            }
        }
    }
}

/// What the netlist calls a cell of Vivado's naming.
fn cell_netlist_name(design: &XdcDesign, cell: &str) -> Option<String> {
    if let Some(register) = design.register(cell) {
        return Some(register.netlist.clone());
    }
    design.is_cell(cell).then(|| cell.replace('/', "."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn design() -> XdcDesign {
        XdcDesign {
            top: "t".to_owned(),
            ports: vec![
                XdcPort {
                    name: "clk".into(),
                    dir: PortDir::In,
                    width: 1,
                    left: 0,
                    right: 0,
                    vector: false,
                },
                XdcPort {
                    name: "a".into(),
                    dir: PortDir::In,
                    width: 4,
                    left: 3,
                    right: 0,
                    vector: true,
                },
                XdcPort {
                    name: "up".into(),
                    dir: PortDir::Out,
                    width: 2,
                    left: 0,
                    right: 1,
                    vector: true,
                },
            ],
            registers: vec![XdcRegister {
                vivado: "m_reg".into(),
                netlist: "m$ff".into(),
                clock: Some("clk".into()),
                scalar_net: Some("m".into()),
            }],
            instances: Vec::new(),
            nets: Vec::new(),
            unwritten: Vec::new(),
        }
    }

    fn read(text: &str) -> (XdcReading, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let file = map.add("t.xdc", text).unwrap();
        let mut diags = Diagnostics::new();
        let reading = read_xdc(text, file, &design(), &mut diags);
        (reading, diags, map)
    }

    /// **A declared index reaches the IR's bit by the declaration's direction.**
    /// `[0:3]` numbers from its most significant end, so its index 0 is the
    /// IR's bit 3. Would catch an offset taken as `index - low`, which on an
    /// ascending bus puts every pin on the mirrored bit: a wrong bitstream with
    /// no diagnostic. Would not catch a front end that records the range wrong.
    #[test]
    fn a_declared_index_reaches_the_bit_it_names() {
        assert_eq!(offset_in(3, 0, 1), Some(1));
        assert_eq!(offset_in(7, 4, 4), Some(0));
        assert_eq!(offset_in(7, 4, 7), Some(3));
        assert_eq!(offset_in(0, 3, 0), Some(3));
        assert_eq!(offset_in(0, 3, 3), Some(0));
        assert_eq!(offset_in(7, 4, 3), None);
    }

    /// **Comments end where Tcl says they do.** A `#` where a command would
    /// start is a comment, one after a command is an argument (Vivado refuses
    /// the line), and a comment's trailing backslash takes the next line with
    /// it. Each was measured (`tcl.answer`, `refused.answer`).
    #[test]
    fn comments_end_where_tcl_says() {
        let (reading, diags, map) = read(
            "# one \\\nset_property PACKAGE_PIN A1 [get_ports clk]\n\
             set_property PACKAGE_PIN B1 [get_ports {a[0]}];# two\n",
        );
        assert_eq!(diags.render(&map), "");
        assert!(!reading.ports.contains_key("clk"));
        assert_eq!(reading.ports["a[0]"].0.pin.as_deref(), Some("B1"));

        let (_, diags, _) = read("set_property PACKAGE_PIN B1 [get_ports {a[0]}] # no\n");
        assert!(diags.has_errors());
    }

    /// **A command that could hold a pin is an error, not a skipped line.**
    /// Vivado carries on past `foreach`; a pin inside it is then never set, and
    /// it says so only in a log. Would catch the command being read as a
    /// warning, or, worse, nothing.
    #[test]
    fn a_command_that_could_hold_a_pin_is_an_error() {
        for text in [
            "foreach p {clk} { set_property PACKAGE_PIN A1 [get_ports $p] }\n",
            "if {1} { set_property PACKAGE_PIN A1 [get_ports clk] }\n",
            "source other.xdc\n",
            "set_property PACKAGE_PIN [lindex {A1} 0] [get_ports clk]\n",
        ] {
            let (reading, diags, map) = read(text);
            assert!(diags.has_errors(), "{text}: {}", diags.render(&map));
            assert!(reading.ports.is_empty(), "{text}");
        }
    }

    /// **`expr` does what it was measured to.** Would catch a period computed
    /// wrongly and so a clock timed at the wrong rate.
    #[test]
    fn expr_computes() {
        let vars = BTreeMap::new();
        let run = |t: &str| Expr::new(t, &vars).run().map(|v| v.text());
        assert_eq!(run("3+1").as_deref(), Some("4"));
        assert_eq!(run("2.5*2").as_deref(), Some("5.0"));
        assert_eq!(run("1 ? \"R5\" : \"R6\"").as_deref(), Some("R5"));
        assert_eq!(run("0 ? \"R5\" : \"R6\"").as_deref(), Some("R6"));
        assert_eq!(run("(1+2)*3").as_deref(), Some("9"));
        assert_eq!(run("nonsense"), None);
    }

    /// **Levels of a hierarchical name are matched level by level.** `*` stops
    /// at a `/` without `-hierarchical`, as `get_cells *` was measured to, and
    /// with it the pattern answers the end of a name. Would catch a pblock's or
    /// a false path's glob reaching every cell below the level it names.
    #[test]
    fn a_glob_stops_at_a_level() {
        assert!(name_matches("*", "m_reg", false));
        assert!(!name_matches("*", "u/q_reg", false));
        assert!(name_matches("u/*", "u/q_reg", false));
        assert!(name_matches("q_reg", "u/q_reg", true));
        assert!(name_matches("*q*", "u/q_reg", true));
        assert!(!name_matches("r_reg", "u/q_reg", true));
    }
}
