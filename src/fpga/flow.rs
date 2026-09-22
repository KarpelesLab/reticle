//! The whole target-specific flow, and the files and command line a
//! place-and-route tool needs.
//!
//! Reticle does not run other programs. [`synthesize_for`] takes a design
//! all the way to a netlist of one device's primitives, and the export
//! functions return the exact bytes a tool expects and the argument list
//! it should be run with; the CLI, a build script or a person does the
//! running. That keeps the library sans-I/O and makes the hand-off
//! testable: a golden test compares the JSON and the constraints file
//! without a tool installed, and [`check_nextpnr_json`] says whether the
//! result is one the tool will accept.
//!
//! | Function | What it does |
//! |----------|--------------|
//! | [`synthesize_for`] | source-level IR to a netlist of `device`'s primitives |
//! | [`check_nextpnr_json`] | validates that netlist against the device database |
//! | [`place_and_route`] | that netlist onto a device's own fabric, and its bitstream |
//! | [`implement`] | both of the above in one call, source to bitstream |
//! | [`export_nextpnr`] | a Yosys-style JSON netlist and a `.pcf` / `.lpf`, for `nextpnr-ice40` / `nextpnr-ecp5` |
//! | [`export_vendor`] | structural Verilog and an `.xdc` / `.sdc`, for Vivado, Quartus or Diamond |
//!
//! There are therefore two ways out of a synthesised design: hand it to
//! nextpnr or a vendor tool, or take it the rest of the way here. The
//! second needs a routing architecture for the part ([`super::arch`]),
//! which exists for iCE40 and is *synthetic*: read
//! [`super::arch::synthetic`] before believing a bitstream it produced.
//!
//! # The order of the flow, and why it is that order
//!
//! [`synthesize_for`] runs five steps, and the order is the whole point:
//!
//! 1. **Generic synthesis** ([`synth::run`]): processes become cells,
//!    flip-flops, latches, memories and state machines are inferred, and
//!    the result is optimised. Everything later needs the cell form.
//! 2. **Primitive mapping** ([`map`](super::map)): block RAMs, DSP
//!    blocks, carry chains, IO buffers and global clock buffers.
//! 3. **LUT mapping** ([`techmap::map_module`]) with the device's LUT
//!    size, over whatever combinational logic is left.
//! 4. **Clean-up** ([`opt::Dce`]): the logic the mapper absorbed leaves
//!    dead cells, assignments and nets behind, and the netlist formats
//!    should not carry them.
//! 5. **Device cells** ([`map_cells`](super::techcells::map_cells)): the
//!    generic LUTs and flip-flops become the family's primitives with
//!    their parameters.
//!
//! Inference has to come before LUT mapping because it works on *shapes*:
//! a memory is recognised from its ports, a multiplier from a `mul` cell,
//! a carry chain from a wide `add`. Once step 3 has shredded an adder
//! into a pile of four-input truth tables there is no adder left to
//! recognise, and a block RAM inferred from LUTs is not a thing anyone
//! knows how to do. So primitives first, LUTs last, with the clean-up and
//! the rename to the family's own cell names after them.
//!
//! The steps are separately callable, so a caller who wants something
//! else — its own optimisation between two of them, or no IO buffers —
//! runs them itself; [`synthesize_for`] is the common path, not the only
//! one.
//!
//! # What the netlist contains
//!
//! The JSON is [`emit_json`] over the design, with the exported module
//! marked as the top. Every cell in it is a black box named exactly as
//! the family's database spells it (`SB_LUT4`, `SB_CARRY`, `SB_DFFESR`,
//! `SB_IO`, `SB_GB`, `SB_RAM40_4K`; `LUT4`, `CCU2C`, `TRELLIS_FF`,
//! `TRELLIS_IO`, `DCCA`, `DP16KD`), carrying the parameters that
//! primitive takes, which is what nextpnr looks for. A design that still
//! holds a generic cell (`$add`, `$dff`, `$mux`) is one the flow could
//! not finish; [`check_nextpnr_json`] finds those before the tool does
//! and says which cell and why.
//!
//! [`emit_json`]: crate::ir::emit::emit_json
//! [`synth::run`]: crate::synth::run
//! [`techmap::map_module`]: crate::synth::techmap::map_module
//! [`opt::Dce`]: crate::synth::opt::Dce

#[cfg(feature = "synth")]
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use std::sync::Arc;

use super::arch::{Arch, RoutingGraph};
use super::bitstream::Bitstream;
use super::constraints::Constraints;
use super::device::Device;
use super::place::{Netlist, PlaceOptions, Placement, PlacementReport};
use super::route::{RouteOptions, Routing, RoutingReport};
use super::techcells::CellMapReport;
#[cfg(feature = "synth")]
use crate::diag::Diagnostics;
use crate::ir::emit::{BitView, EmitError, SigBit, VerilogOptions, emit_json, emit_verilog_with};
use crate::ir::{Bit, CellKind, Design, ModuleId, PortDir};

/// Why an export could not be produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowError {
    /// The module id does not belong to the design.
    NoSuchModule,
    /// A construct the netlist format cannot express, from the emitter.
    Emit(EmitError),
    /// The family has no known place-and-route command line.
    UnknownFamily {
        /// The family as the device database spells it.
        family: String,
        /// The device that named it.
        device: String,
    },
    /// Generic synthesis reported errors, so nothing was mapped. The
    /// errors themselves are in the diagnostics the caller passed in.
    Synthesis,
    /// No routing architecture is known for the device, so the design
    /// cannot be placed and routed in Reticle. It can still be exported.
    NoArchitecture {
        /// The device that has none.
        device: String,
    },
    /// Placement failed.
    Placement(super::place::PlaceError),
    /// Routing failed.
    Routing(super::route::RouteError),
    /// The bitstream could not be built from the placed and routed
    /// design, which means the architecture contradicts itself.
    Bitstream(super::bitstream::BitstreamError),
}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowError::NoSuchModule => f.write_str("the module is not part of the design"),
            FlowError::Emit(err) => write!(f, "{err}"),
            FlowError::UnknownFamily { family, device } => write!(
                f,
                "no place-and-route flow is known for family `{family}` (device `{device}`)"
            ),
            FlowError::Synthesis => {
                f.write_str("the design could not be synthesised; see the reported errors")
            }
            FlowError::NoArchitecture { device } => write!(
                f,
                "no routing architecture is known for `{device}`, so Reticle cannot place \
                 and route it; export it to nextpnr or a vendor tool instead"
            ),
            FlowError::Placement(err) => write!(f, "placement failed: {err}"),
            FlowError::Routing(err) => write!(f, "routing failed: {err}"),
            FlowError::Bitstream(err) => write!(f, "the bitstream could not be built: {err}"),
        }
    }
}

impl Error for FlowError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            FlowError::Emit(err) => Some(err),
            FlowError::Placement(err) => Some(err),
            FlowError::Routing(err) => Some(err),
            FlowError::Bitstream(err) => Some(err),
            _ => None,
        }
    }
}

impl From<super::place::PlaceError> for FlowError {
    fn from(err: super::place::PlaceError) -> Self {
        FlowError::Placement(err)
    }
}

impl From<super::route::RouteError> for FlowError {
    fn from(err: super::route::RouteError) -> Self {
        FlowError::Routing(err)
    }
}

impl From<super::bitstream::BitstreamError> for FlowError {
    fn from(err: super::bitstream::BitstreamError) -> Self {
        FlowError::Bitstream(err)
    }
}

impl From<EmitError> for FlowError {
    fn from(err: EmitError) -> Self {
        FlowError::Emit(err)
    }
}

/// Everything nextpnr needs for one run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextpnrInputs {
    /// The netlist, in the Yosys JSON format nextpnr reads.
    pub json: String,
    /// The constraints, in the family's format: `.pcf` for iCE40, `.lpf`
    /// for ECP5.
    pub pcf_or_lpf: String,
    /// The file name the constraints should be written to, which is what
    /// [`NextpnrInputs::args`] refers to.
    pub constraints_name: String,
    /// The command line, program first. File names are the plain
    /// `<top>.json`, `<top>.pcf` / `<top>.lpf` and the family's output
    /// (`<top>.asc` for iCE40, `<top>.config` for ECP5); write the files
    /// under those names next to each other and run it.
    pub args: Vec<String>,
}

/// Everything a vendor tool needs for one run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VendorInputs {
    /// The netlist as structural Verilog.
    pub verilog: String,
    /// The constraints as XDC.
    pub xdc: String,
    /// The constraints as SDC, for a flow that wants timing separately.
    pub sdc: String,
}

/// Knobs for [`synthesize_for`].
///
/// The defaults are the ones a board design wants: full generic
/// synthesis, every primitive-mapping step on, LUTs of the device's own
/// size, and the device's cell names in the result.
#[cfg(feature = "synth")]
#[derive(Clone, Debug)]
pub struct FpgaOptions {
    /// How the technology-independent front half behaves.
    pub synth: crate::synth::SynthOptions,
    /// Which primitive-mapping steps run; see [`super::MapOptions`].
    pub map: super::primitives::MapOptions,
    /// Number of LUT inputs to map onto, or `None` for the device's own
    /// [`Device::lut_size`].
    pub lut_size: Option<u32>,
    /// Area recovery passes after the depth-oriented mapping pass.
    pub area_passes: u32,
    /// Rewrite the mapped LUTs and flip-flops into the device's
    /// primitives. Turning this off leaves a generic netlist, which is
    /// useful for comparing mappers but is not something nextpnr reads.
    pub device_cells: bool,
}

#[cfg(feature = "synth")]
impl Default for FpgaOptions {
    fn default() -> Self {
        FpgaOptions {
            synth: crate::synth::SynthOptions::default(),
            map: super::primitives::MapOptions::default(),
            lut_size: None,
            area_passes: 2,
            device_cells: true,
        }
    }
}

/// What [`synthesize_for`] did, from the primitives it inferred down to
/// the cell counts of the finished netlist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlowReport {
    /// The device the design was mapped for.
    pub device: String,
    /// What primitive mapping did (block RAM, DSP, carry, IO, clocks).
    pub primitives: super::primitives::MapReport,
    /// What the rewrite to device cells did.
    pub device_cells: CellMapReport,
    /// How many LUTs the technology mapper produced.
    pub luts: usize,
    /// The depth of the mapped logic, in LUTs.
    pub lut_depth: u32,
    /// Every cell type in the finished module with how many there are,
    /// sorted by type. A type starting with `$` is a generic cell the
    /// flow could not map, which is what [`check_nextpnr_json`] reports.
    pub netlist: Vec<(String, usize)>,
}

impl FlowReport {
    /// The number of cells of type `name` in the finished netlist.
    pub fn count(&self, name: &str) -> usize {
        self.netlist
            .iter()
            .find(|(cell, _)| cell == name)
            .map_or(0, |(_, n)| *n)
    }

    /// The total number of cells in the finished netlist.
    pub fn cells(&self) -> usize {
        self.netlist.iter().map(|(_, n)| *n).sum()
    }

    /// Renders the report as plain text: the primitive-mapping report,
    /// then the logic, then the netlist by cell type.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = self.primitives.to_text();
        let _ = writeln!(
            out,
            "logic:\n  {} LUTs, depth {}",
            self.luts, self.lut_depth
        );
        if !self.netlist.is_empty() {
            out.push_str("netlist:\n");
            for (name, count) in &self.netlist {
                let _ = writeln!(out, "  {count} x {name}");
            }
        }
        if !self.device_cells.inverted.is_empty() {
            out.push_str("inverted:\n");
            for (net, pin) in &self.device_cells.inverted {
                let _ = writeln!(out, "  {net} -> the {pin} polarity the device has");
            }
        }
        if !self.device_cells.declined.is_empty() {
            out.push_str("unmapped:\n");
            for (cell, why) in &self.device_cells.declined {
                let _ = writeln!(out, "  {cell} ({why})");
            }
        }
        out
    }
}

/// Takes `module` from the IR all the way to a netlist of `device`'s
/// primitives, in place.
///
/// The five steps and the reason for their order are in the module docs.
/// Problems the user can do something about are reported through
/// `diags`; the returned [`FlowReport`] says what was mapped and what the
/// netlist ended up holding. Run [`check_nextpnr_json`] afterwards to
/// find out whether a place-and-route tool will take the result.
///
/// Only `module` is mapped. A hierarchical design should be flattened
/// first ([`crate::ir::hier`]), since a place-and-route tool wants one
/// flat netlist.
///
/// # Errors
///
/// [`FlowError::NoSuchModule`] when the id does not belong to the design,
/// and [`FlowError::Synthesis`] when generic synthesis rejected the
/// design (an invalid IR, for instance), in which case the design is left
/// as synthesis left it and the errors are in `diags`.
#[cfg(feature = "synth")]
pub fn synthesize_for(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
    options: &FpgaOptions,
    diags: &mut Diagnostics,
) -> Result<FlowReport, FlowError> {
    use crate::synth::opt::Dce;
    use crate::synth::techmap::{MapOptions as TechMapOptions, map_module};
    use crate::synth::{Pass, run as synth_run};

    if design.modules.get(module).is_none() {
        return Err(FlowError::NoSuchModule);
    }
    let mut report = FlowReport {
        device: device.name.clone(),
        ..FlowReport::default()
    };

    // 1. Generic synthesis. Its own diagnostics decide whether the rest
    // of the flow is worth running at all, so they are collected apart
    // from whatever the caller already had.
    let mut synth_diags = Diagnostics::new();
    let _ = synth_run(design, &options.synth, &mut synth_diags);
    let refused = synth_diags.has_errors();
    diags.append(&mut synth_diags);
    if refused {
        return Err(FlowError::Synthesis);
    }

    // 2. What the device does in hard logic, while the shapes that say so
    // are still recognisable.
    report.primitives =
        super::primitives::map(design, module, device, constraints, &options.map, diags);

    // 3. Everything else onto LUTs.
    let k = options.lut_size.unwrap_or(device.lut_size).clamp(2, 8);
    let mut techmap = TechMapOptions::lut(k);
    techmap.area_passes = options.area_passes;
    let stats = map_module(&mut design.modules[module], &techmap);
    report.luts = stats.cells;
    report.lut_depth = stats.depth;

    // 4. Clean-up: the logic the mapper absorbed leaves dead cells and
    // nets behind.
    let mut clean = Diagnostics::new();
    let _ = Dce.run(&mut design.modules[module], &mut clean);
    diags.append(&mut clean);

    // 5. The family's own cell names and parameters.
    if options.device_cells {
        report.device_cells = super::techcells::map_cells(design, module, device, diags);
    }

    report.netlist = cell_types(design, module);
    Ok(report)
}

/// Every cell type in a module with how many there are, sorted by type.
#[cfg(feature = "synth")]
fn cell_types(design: &Design, module: ModuleId) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let Some(module) = design.modules.get(module) else {
        return Vec::new();
    };
    for (_, cell) in module.cells.iter() {
        let name = match &cell.kind {
            CellKind::Blackbox(name) => name.as_str().to_owned(),
            other => format!("${}", other.keyword()),
        };
        *counts.entry(name).or_default() += 1;
    }
    counts.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Placement, routing and the bitstream
// ---------------------------------------------------------------------------

/// Knobs for [`place_and_route`].
#[derive(Clone, Debug)]
pub struct PnrOptions {
    /// The routing architecture to use, or `None` for the built-in one
    /// serving the device.
    ///
    /// This is where a real, IceStorm-derived database is dropped in:
    /// parse it with [`Arch::parse`] and hand it over. Nothing else in
    /// the flow changes.
    ///
    /// [`Arch::parse`]: super::arch::Arch::parse
    pub arch: Option<Arch>,
    /// How placement behaves.
    pub place: PlaceOptions,
    /// How routing behaves.
    pub route: RouteOptions,
    /// Build the bitstream. Turning it off stops after routing, which is
    /// what a caller who only wants a placement report does.
    pub bitstream: bool,
}

impl Default for PnrOptions {
    fn default() -> Self {
        PnrOptions {
            arch: None,
            place: PlaceOptions::default(),
            route: RouteOptions::default(),
            bitstream: true,
        }
    }
}

impl PnrOptions {
    /// The defaults: the built-in architecture for the part, and a
    /// bitstream at the end.
    pub fn new() -> Self {
        PnrOptions::default()
    }
}

/// What [`place_and_route`] produced.
///
/// The architecture and its expanded graph come along because the
/// placement and the routing are indices into them: a site number means
/// nothing without [`PnrResult::graph`].
#[derive(Clone, Debug)]
pub struct PnrResult {
    /// The architecture that was used.
    pub arch: Arch,
    /// Its expanded graph, shared rather than copied.
    pub graph: Arc<RoutingGraph>,
    /// The netlist the placer and the router worked on.
    pub netlist: Netlist,
    /// Which site each instance sits on.
    pub placement: Placement,
    /// What placement did.
    pub placement_report: PlacementReport,
    /// Which pips carry which signal.
    pub routing: Routing,
    /// What routing did.
    pub routing_report: RoutingReport,
    /// The bitstream, when [`PnrOptions::bitstream`] asked for one.
    pub bitstream: Option<Bitstream>,
}

impl PnrResult {
    /// Checks that the routing really implements the netlist, by walking
    /// every sink back to its driver; see [`Routing::verify`].
    pub fn verify(&self) -> Vec<String> {
        self.routing
            .verify(&self.netlist, &self.graph, &self.placement)
    }

    /// The placement and routing reports, one after the other.
    pub fn to_text(&self) -> String {
        let mut out = self.placement_report.to_text();
        out.push_str(&self.routing_report.to_text());
        if let Some(bitstream) = &self.bitstream {
            out.push_str(&format!(
                "bitstream:\n  {} of {} bit(s) set over {} tile(s)\n",
                bitstream.ones(),
                bitstream.format.bits(),
                bitstream.format.tiles.len()
            ));
        }
        out
    }
}

/// Places, routes and (optionally) writes the bitstream for a netlist
/// that [`synthesize_for`] has already produced.
///
/// The three stages are [`place`](super::place::place),
/// [`route`](super::route::route) and
/// [`generate`](super::bitstream::generate), and each reports what it
/// did: [`PnrResult::placement_report`] gives the wirelength before and
/// after annealing, [`PnrResult::routing_report`] the overuse of every
/// rip-up iteration, and [`PnrResult::to_text`] prints both.
///
/// The design is not modified.
///
/// # Errors
///
/// [`FlowError::NoArchitecture`] when the part has no routing
/// architecture, and [`FlowError::Placement`], [`FlowError::Routing`] or
/// [`FlowError::Bitstream`] when a stage could not finish. Every one of
/// those says which cell, which signal or which wire, so a failure is
/// actionable rather than a shrug.
pub fn place_and_route(
    design: &Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
    options: &PnrOptions,
) -> Result<PnrResult, FlowError> {
    if design.modules.get(module).is_none() {
        return Err(FlowError::NoSuchModule);
    }
    let (arch, graph) = match &options.arch {
        Some(arch) => {
            let graph = Arc::new(arch.build_graph());
            (arch.clone(), graph)
        }
        None => {
            let arch = super::arch::architecture_for(&device.name).ok_or_else(|| {
                FlowError::NoArchitecture {
                    device: device.name.clone(),
                }
            })?;
            let graph = super::arch::builtin_graph_for(&device.name).ok_or_else(|| {
                FlowError::NoArchitecture {
                    device: device.name.clone(),
                }
            })?;
            (arch.clone(), graph)
        }
    };

    let netlist = Netlist::build(design, module, device, &graph)?;
    let (placement, placement_report) =
        super::place::place(&netlist, &arch, &graph, constraints, &options.place)?;
    let (routing, routing_report) =
        super::route::route(&netlist, &graph, &placement, &options.route)?;
    let bitstream = if options.bitstream {
        Some(super::bitstream::generate(
            design, module, &arch, &graph, &netlist, &placement, &routing,
        )?)
    } else {
        None
    };
    Ok(PnrResult {
        arch,
        graph,
        netlist,
        placement,
        placement_report,
        routing,
        routing_report,
        bitstream,
    })
}

/// A design taken all the way: the synthesis report and the
/// place-and-route result.
#[cfg(feature = "synth")]
#[derive(Clone, Debug)]
pub struct Implementation {
    /// What [`synthesize_for`] did.
    pub flow: FlowReport,
    /// What [`place_and_route`] did.
    pub pnr: PnrResult,
}

#[cfg(feature = "synth")]
impl Implementation {
    /// The bitstream, when one was asked for.
    pub fn bitstream(&self) -> Option<&Bitstream> {
        self.pnr.bitstream.as_ref()
    }

    /// Every report, in the order the stages ran.
    pub fn to_text(&self) -> String {
        let mut out = self.flow.to_text();
        out.push_str(&self.pnr.to_text());
        out
    }
}

/// Source-level IR to a bitstream, in one call.
///
/// [`synthesize_for`] followed by [`place_and_route`], with the design
/// left mapped for the device. This is the whole FPGA flow inside
/// Reticle; what comes out is only as trustworthy as the architecture it
/// used, and the one that ships is synthetic
/// ([`super::arch::synthetic`]).
///
/// # Errors
///
/// Everything either of the two stages can report.
#[cfg(feature = "synth")]
pub fn implement(
    design: &mut Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
    options: &FpgaOptions,
    pnr: &PnrOptions,
    diags: &mut Diagnostics,
) -> Result<Implementation, FlowError> {
    let flow = synthesize_for(design, module, device, constraints, options, diags)?;
    let pnr = place_and_route(design, module, device, constraints, pnr)?;
    Ok(Implementation { flow, pnr })
}

/// One thing wrong with an exported netlist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetlistProblem {
    /// The object the problem is about: a cell, net, port or pin name,
    /// or the module's name when it is about the whole netlist.
    pub object: String,
    /// What is wrong with it.
    pub message: String,
}

impl fmt::Display for NetlistProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}`: {}", self.object, self.message)
    }
}

/// Checks that the netlist [`export_nextpnr`] would produce is one the
/// tool will accept, against the device database.
///
/// This is the last gate before an external tool sees the design, and it
/// checks what that tool checks, on exactly the bit-level view the JSON
/// writer uses ([`BitView`]), so a clean result means the file itself is
/// clean and not merely the IR behind it:
///
/// - every cell is a *primitive the device declares*, so nothing generic
///   (`$lut`, `$dff`, `$add`) and nothing invented is left;
/// - every connected port is a port that primitive has;
/// - the netlist is flat (no instances) and in cell form (no processes);
/// - every net that something reads has exactly one driver;
/// - every constant is a `0` or a `1`, which is what the family's
///   constant convention can tie (see [`constant_convention`]); an `x`
///   or `z` reaches the tool as a bit with no driver at all;
/// - every `set_io` names a package pin the device has, that pin is an
///   IO pin, and the design has the port it constrains.
///
/// The result is a list of problems, empty when there is nothing wrong.
/// Its order is fixed: the checks run in the order above and each walks
/// the netlist in arena order, so two runs of the same design produce the
/// same list. It is a *check*, not a diagnostic pass: the objects it
/// names have already been reported with spans by the steps that could
/// not map them.
pub fn check_nextpnr_json(
    design: &Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
) -> Vec<NetlistProblem> {
    let mut problems = Vec::new();
    let mut push = |object: &str, message: String| {
        problems.push(NetlistProblem {
            object: object.to_owned(),
            message,
        });
    };
    let Some(m) = design.modules.get(module) else {
        push("", "the module is not part of the design".to_owned());
        return problems;
    };
    let name = m.name.as_str().to_owned();
    if constant_convention(&device.family).is_none() {
        push(
            &name,
            format!(
                "no nextpnr flow is known for family `{}`, so this check can only be structural",
                device.family
            ),
        );
    }
    if !m.processes.is_empty() {
        push(
            &name,
            format!(
                "{} process(es) are left: a netlist holds cells only, so run synthesis first",
                m.processes.len()
            ),
        );
    }
    for (_, inst) in m.instances.iter() {
        push(
            inst.name.as_str(),
            "an instance is left: nextpnr wants one flat netlist, so flatten the design".to_owned(),
        );
    }
    for (_, mem) in m.memories.iter() {
        push(
            mem.name.as_str(),
            "a memory is left: it did not become block RAM or logic".to_owned(),
        );
    }

    // Cell types and ports, against what the device declares.
    for (_, cell) in m.cells.iter() {
        let CellKind::Blackbox(primitive) = &cell.kind else {
            push(
                cell.name.as_str(),
                format!(
                    "`${}` is a generic cell, not a primitive of `{}`",
                    cell.kind.keyword(),
                    device.name
                ),
            );
            continue;
        };
        let Some(ports) = device.primitive_ports(primitive.as_str()) else {
            push(
                cell.name.as_str(),
                format!(
                    "`{primitive}` is not a primitive `{}` declares",
                    device.name
                ),
            );
            continue;
        };
        if ports.is_empty() {
            // Declared without a port map: the database cannot say what
            // the pins are, so there is nothing to check them against.
            continue;
        }
        let connected = cell
            .inputs
            .iter()
            .map(|(port, _)| port)
            .chain(cell.outputs.iter().map(|(port, _)| port));
        for port in connected {
            if !ports.iter().any(|p| p == port.as_str()) {
                push(
                    cell.name.as_str(),
                    format!("`{primitive}` has no port `{port}`"),
                );
            }
        }
    }

    // Drivers and constants, on the bits the JSON actually carries.
    let view = match BitView::new(m) {
        Ok(view) => view,
        Err(err) => {
            push(&name, format!("the netlist cannot be written: {err}"));
            return problems;
        }
    };
    let mut drivers = vec![0usize; view.slots()];
    let mut external = vec![false; view.slots()];
    let mark = |bits: &[SigBit], drivers: &mut Vec<usize>| {
        for bit in bits {
            if let SigBit::Slot(slot) = view.canonical(*bit) {
                drivers[slot] += 1;
            }
        }
    };
    for port in &m.ports {
        if matches!(port.dir, PortDir::In | PortDir::InOut)
            && let Ok(bits) = view.net_bits(port.net, port.span)
        {
            mark(&bits, &mut drivers);
            if port.dir == PortDir::InOut {
                for bit in &bits {
                    if let SigBit::Slot(slot) = view.canonical(*bit) {
                        external[slot] = true;
                    }
                }
            }
        }
    }
    for (_, cell) in m.cells.iter() {
        for (_, net) in &cell.outputs {
            if let Ok(bits) = view.net_bits(*net, cell.span) {
                mark(&bits, &mut drivers);
            }
        }
    }
    for (slot, count) in drivers.iter().enumerate() {
        if *count > 1 && !external[slot] {
            let (net, bit) = view.owner(slot);
            push(
                m.nets[net].name.as_str(),
                format!("bit {bit} has {count} drivers"),
            );
        }
    }
    // Everything that reads a bit: a cell input, or an output port.
    let mut reads: Vec<(String, Vec<SigBit>)> = Vec::new();
    for (_, cell) in m.cells.iter() {
        for (port, e) in &cell.inputs {
            if let Ok(bits) = view.expr_bits(*e) {
                reads.push((format!("{}.{}", cell.name, port), bits));
            }
        }
    }
    for port in &m.ports {
        if port.dir == PortDir::Out
            && let Ok(bits) = view.net_bits(port.net, port.span)
        {
            reads.push((port.name.as_str().to_owned(), bits));
        }
    }
    for (object, bits) in reads {
        for (index, bit) in bits.iter().enumerate() {
            match view.canonical(*bit) {
                SigBit::Const(Bit::Zero | Bit::One) => {}
                SigBit::Const(other) => push(
                    &object,
                    format!(
                        "bit {index} is the constant `{}`, which no driver ties: {}",
                        other.to_char(),
                        constant_convention(&device.family)
                            .unwrap_or("a constant has to be a 0 or a 1")
                    ),
                ),
                SigBit::Slot(slot) => {
                    if drivers[slot] == 0 {
                        let (net, netbit) = view.owner(slot);
                        push(
                            &object,
                            format!(
                                "bit {index} reads `{}` bit {netbit}, which nothing drives",
                                m.nets[net].name
                            ),
                        );
                    }
                }
            }
        }
    }

    // The pin constraints, against the package and the design.
    for pin in &constraints.pins {
        let signal = pin.signal();
        // Options stated as attributes without a pin (a standard, a DDR
        // clock) place nothing, so there is no package pin to check.
        let placed = (!pin.pin.is_empty()).then(|| device.pin(&pin.pin));
        match placed {
            Some(None) => push(
                &signal,
                format!("`{}` has no package pin `{}`", device.name, pin.pin),
            ),
            Some(Some(found)) if !found.kind.is_io() => push(
                &signal,
                format!(
                    "package pin `{}` is a {} pin, which a design cannot drive",
                    pin.pin, found.kind
                ),
            ),
            Some(Some(_)) | None => {}
        }
        if m.port(&pin.port).is_none() {
            push(
                &signal,
                format!("the netlist has no port `{}` to place", pin.port),
            );
        }
    }
    problems
}

/// How a family's nextpnr ties a constant bit of a netlist, as a phrase
/// for a diagnostic; `None` when no nextpnr flow is known for it.
///
/// Both supported families take the JSON strings `"0"` and `"1"` and
/// connect them to nets their packer drives, so a constant needs no cell
/// in the netlist — but it does have to be a *known* bit: an `x` or a `z`
/// arrives as a net with no driver, which the router then fails on.
pub fn constant_convention(family: &str) -> Option<&'static str> {
    match family {
        "ice40" => {
            Some("nextpnr-ice40 ties a `0` or `1` bit to the GND and VCC nets its packer creates")
        }
        "ecp5" => {
            Some("nextpnr-ecp5 ties a `0` or `1` bit to the GND and VCC nets its packer creates")
        }
        _ => None,
    }
}

/// Builds the files and the command line for nextpnr.
///
/// The design is not modified. Run [`map`](super::map) first so that the
/// IO, clock, memory and DSP primitives are in place.
pub fn export_nextpnr(
    design: &Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
) -> Result<NextpnrInputs, FlowError> {
    if design.modules.get(module).is_none() {
        return Err(FlowError::NoSuchModule);
    }
    // nextpnr picks the top module out of the JSON by its `top`
    // attribute, so say which one this export is about.
    let mut design = design.clone();
    design.top = Some(module);
    let json = emit_json(&design)?;
    let top = design.module(module).name.as_str().to_owned();
    match device.family.as_str() {
        "ice40" => Ok(NextpnrInputs {
            json,
            pcf_or_lpf: constraints.write_pcf(device),
            constraints_name: format!("{top}.pcf"),
            args: ice40_args(device, &top),
        }),
        "ecp5" => Ok(NextpnrInputs {
            json,
            pcf_or_lpf: constraints.write_lpf(device),
            constraints_name: format!("{top}.lpf"),
            args: ecp5_args(device, &top),
        }),
        other => Err(FlowError::UnknownFamily {
            family: other.to_owned(),
            device: device.name.clone(),
        }),
    }
}

/// The `nextpnr-ice40` command line for this part.
///
/// The device flag comes from the part name (`ice40-hx1k-tq144` gives
/// `--hx1k`), the package from the database.
fn ice40_args(device: &Device, top: &str) -> Vec<String> {
    let mut args = vec!["nextpnr-ice40".to_owned()];
    match ice40_part(&device.name) {
        Some(part) => args.push(format!("--{part}")),
        None => args.push("--hx1k".to_owned()),
    }
    if !device.package.is_empty() {
        args.push("--package".to_owned());
        args.push(device.package.clone());
    }
    args.extend([
        "--json".to_owned(),
        format!("{top}.json"),
        "--pcf".to_owned(),
        format!("{top}.pcf"),
        "--asc".to_owned(),
        format!("{top}.asc"),
    ]);
    args
}

/// The part name inside a device name: the first component that looks
/// like an iCE40 device flag.
fn ice40_part(name: &str) -> Option<&str> {
    const PARTS: [&str; 8] = [
        "lp384", "lp1k", "lp8k", "hx1k", "hx4k", "hx8k", "up5k", "u4k",
    ];
    name.split('-').find(|part| PARTS.contains(part))
}

/// The `nextpnr-ecp5` command line for this part.
fn ecp5_args(device: &Device, top: &str) -> Vec<String> {
    let mut args = vec!["nextpnr-ecp5".to_owned()];
    let size = ecp5_part(&device.name).unwrap_or("25k");
    args.push(format!("--{size}"));
    if !device.package.is_empty() {
        args.push("--package".to_owned());
        args.push(device.package.clone());
    }
    args.extend([
        "--json".to_owned(),
        format!("{top}.json"),
        "--lpf".to_owned(),
        format!("{top}.lpf"),
        "--textcfg".to_owned(),
        format!("{top}.config"),
    ]);
    args
}

/// The nextpnr size flag for an ECP5 part name (`ecp5-45f-...` gives
/// `45k`).
fn ecp5_part(name: &str) -> Option<&'static str> {
    const PARTS: [(&str, &str); 5] = [
        ("12f", "12k"),
        ("25f", "25k"),
        ("45f", "45k"),
        ("85f", "85k"),
        ("um5g", "85k"),
    ];
    name.split('-')
        .find_map(|part| PARTS.iter().find(|(key, _)| *key == part))
        .map(|(_, flag)| *flag)
}

/// Builds the files a vendor tool needs.
///
/// The Verilog is structural: black-box cells become instantiations of
/// the device primitives, which the vendor's library supplies. The
/// constraints are XDC (Vivado) and SDC (Quartus and other SDC
/// consumers); see [`Constraints::write_xdc`] and
/// [`Constraints::write_sdc`] for what each expresses.
///
/// [`Constraints::write_xdc`]: super::Constraints::write_xdc
/// [`Constraints::write_sdc`]: super::Constraints::write_sdc
pub fn export_vendor(
    design: &Design,
    module: ModuleId,
    device: &Device,
    constraints: &Constraints,
) -> Result<VendorInputs, FlowError> {
    if design.modules.get(module).is_none() {
        return Err(FlowError::NoSuchModule);
    }
    let mut design = design.clone();
    design.top = Some(module);
    let options = VerilogOptions {
        structural_only: false,
        ansi_ports: true,
        keep_attrs: true,
        blackboxes: false,
    };
    Ok(VendorInputs {
        verilog: emit_verilog_with(&design, &options)?,
        xdc: constraints.write_xdc(device),
        sdc: constraints.write_sdc(device),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::fpga::{MapOptions, map, target};
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Id, Name, Type};
    use crate::source::{SourceMap, Span};

    /// A one-flop design with a clock and an output.
    fn blinky() -> (Design, ModuleId, SourceMap) {
        let mut map = SourceMap::new();
        let file = map.add("blinky.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("blinky", span);
        let clk = b.input("clk", Type::bit());
        let led = b.output("led", Type::bit());
        let (clk_e, led_e) = (b.net(clk), b.net(led));
        let inverted = b.add_net("inverted", Type::bit());
        b.cell(
            "invert",
            CellKind::Not,
            vec![(Name::new("a"), led_e)],
            vec![(Name::new("y"), inverted)],
        );
        let next = b.net(inverted);
        b.cell(
            "toggle",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clk_e), (Name::new("d"), next)],
            vec![(Name::new("q"), led)],
        );
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top, map)
    }

    fn mapped(device: &str) -> (Design, ModuleId, Constraints, SourceMap) {
        let (mut design, top, mut map) = blinky();
        let rcf = "set_io clk 21\nset_io led 99\ncreate_clock -name sys -period 83.333 clk\n";
        let file = map.add("blinky.rcf", rcf).unwrap();
        let mut diags = Diagnostics::new();
        let constraints = Constraints::parse(rcf, file, &mut diags);
        let device = target(device).unwrap();
        map_design(&mut design, top, device, &constraints, &mut diags);
        (design, top, constraints, map)
    }

    fn map_design(
        design: &mut Design,
        top: ModuleId,
        device: &Device,
        constraints: &Constraints,
        diags: &mut Diagnostics,
    ) {
        map(
            design,
            top,
            device,
            constraints,
            &MapOptions {
                global_buffer_threshold: 1,
                ..MapOptions::default()
            },
            diags,
        );
    }

    #[test]
    fn exports_for_nextpnr_ice40() {
        let (design, top, constraints, _map) = mapped("ice40-hx1k-tq144");
        let device = target("ice40-hx1k-tq144").unwrap();
        let inputs = export_nextpnr(&design, top, device, &constraints).unwrap();
        assert!(inputs.json.contains("\"creator\": \"reticle\""));
        assert!(inputs.json.contains("\"SB_IO\""), "{}", inputs.json);
        assert!(inputs.json.contains("\"SB_GB\""));
        assert!(inputs.json.contains("\"top\": 1"));
        assert!(inputs.pcf_or_lpf.contains("set_io clk 21"));
        assert_eq!(inputs.constraints_name, "blinky.pcf");
        assert_eq!(
            inputs.args,
            vec![
                "nextpnr-ice40",
                "--hx1k",
                "--package",
                "tq144",
                "--json",
                "blinky.json",
                "--pcf",
                "blinky.pcf",
                "--asc",
                "blinky.asc",
            ]
        );
    }

    #[test]
    fn exports_for_nextpnr_ecp5() {
        let (design, top, constraints, _map) = mapped("ecp5-45f-CABGA381");
        let device = target("ecp5-45f-CABGA381").unwrap();
        let inputs = export_nextpnr(&design, top, device, &constraints).unwrap();
        assert!(inputs.json.contains("\"TRELLIS_IO\""), "{}", inputs.json);
        assert!(inputs.json.contains("\"DCCA\""));
        assert!(
            inputs
                .pcf_or_lpf
                .contains("LOCATE COMP \"clk\" SITE \"21\";")
        );
        assert_eq!(inputs.constraints_name, "blinky.lpf");
        assert_eq!(
            inputs.args,
            vec![
                "nextpnr-ecp5",
                "--45k",
                "--package",
                "CABGA381",
                "--json",
                "blinky.json",
                "--lpf",
                "blinky.lpf",
                "--textcfg",
                "blinky.config",
            ]
        );
    }

    #[test]
    fn exports_for_a_vendor_tool() {
        let (design, top, constraints, _map) = mapped("ice40-hx1k-tq144");
        let device = target("ice40-hx1k-tq144").unwrap();
        let inputs = export_vendor(&design, top, device, &constraints).unwrap();
        assert!(
            inputs.verilog.contains("module blinky"),
            "{}",
            inputs.verilog
        );
        assert!(inputs.verilog.contains("SB_IO"), "{}", inputs.verilog);
        assert!(inputs.xdc.contains("set_property PACKAGE_PIN 21"));
        assert!(inputs.sdc.contains("create_clock -name sys -period 83.333"));
        assert!(!inputs.sdc.contains("PACKAGE_PIN"));
    }

    #[test]
    fn reports_what_it_cannot_export() {
        let (design, top, constraints, _map) = mapped("generic");
        let err =
            export_nextpnr(&design, top, target("generic").unwrap(), &constraints).unwrap_err();
        assert_eq!(
            err.to_string(),
            "no place-and-route flow is known for family `generic` (device `generic`)"
        );
        assert!(err.source().is_none());

        let other = ModuleId::from_index(42);
        let err =
            export_nextpnr(&design, other, target("generic").unwrap(), &constraints).unwrap_err();
        assert_eq!(err, FlowError::NoSuchModule);
        assert_eq!(
            export_vendor(&design, other, target("generic").unwrap(), &constraints).unwrap_err(),
            FlowError::NoSuchModule
        );
        let _ = top;
    }

    #[test]
    fn a_process_form_module_cannot_be_a_netlist() {
        // JSON is for synthesised designs; a module with a process is
        // reported rather than silently half-emitted.
        let mut map = SourceMap::new();
        let file = map.add("t.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let q = b.output_reg("q", Type::bit());
        let one = b.const_bit(true);
        let mut p = b.process(None, crate::ir::ProcessKind::posedge(clk));
        p.nonblocking(q, one);
        b.end_process(p);
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        let err = export_nextpnr(
            &design,
            top,
            target("ice40-hx1k-tq144").unwrap(),
            &Constraints::new(),
        )
        .unwrap_err();
        assert!(matches!(err, FlowError::Emit(_)));
        assert!(err.source().is_some());
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn part_flags_come_from_the_device_name() {
        assert_eq!(ice40_part("ice40-hx8k-ct256"), Some("hx8k"));
        assert_eq!(ice40_part("ice40-lp1k-tq144"), Some("lp1k"));
        assert_eq!(ice40_part("mystery"), None);
        assert_eq!(ecp5_part("ecp5-85f-CABGA381"), Some("85k"));
        assert_eq!(ecp5_part("mystery"), None);
        // An unknown part still produces a runnable command line, with the
        // smallest device of the family.
        let mut device = target("ice40-hx1k-tq144").unwrap().clone();
        device.name = "mystery".to_owned();
        assert_eq!(ice40_args(&device, "t")[1], "--hx1k");
        let mut device = target("ecp5-25f-CABGA381").unwrap().clone();
        device.name = "mystery".to_owned();
        device.package.clear();
        let args = ecp5_args(&device, "t");
        assert_eq!(args[1], "--25k");
        assert_eq!(args[2], "--json");
    }

    /// The whole flow, from the IR to a netlist of one device's
    /// primitives, with nothing generic left in it.
    #[cfg(feature = "synth")]
    #[test]
    fn the_whole_flow_reaches_device_primitives() {
        let (mut design, top, sources) = blinky();
        let device = target("ice40-hx1k-tq144").unwrap();
        let mut diags = Diagnostics::new();
        let options = FpgaOptions {
            map: MapOptions {
                global_buffer_threshold: 1,
                ..MapOptions::default()
            },
            ..FpgaOptions::default()
        };
        let constraints = Constraints::new();
        let report =
            synthesize_for(&mut design, top, device, &constraints, &options, &mut diags).unwrap();
        assert_eq!(diags.render(&sources), "");
        assert_eq!(report.device, "ice40-hx1k-tq144");
        assert_eq!(report.count("SB_LUT4"), 1, "{:?}", report.netlist);
        assert_eq!(report.count("SB_DFF"), 1);
        assert_eq!(report.count("SB_GB"), 1);
        assert_eq!(report.count("SB_IO"), 2);
        assert_eq!(report.cells(), 5);
        assert!(
            report
                .netlist
                .iter()
                .all(|(name, _)| !name.starts_with('$'))
        );
        assert!(
            report.to_text().contains("1 x SB_LUT4"),
            "{}",
            report.to_text()
        );

        // What the netlist says is what the exported JSON says.
        assert!(check_nextpnr_json(&design, top, device, &constraints).is_empty());
        let inputs = export_nextpnr(&design, top, device, &constraints).unwrap();
        assert!(inputs.json.contains("\"SB_LUT4\""), "{}", inputs.json);
        assert!(inputs.json.contains("\"LUT_INIT\""));
        // Generated *names* may start with a `$`, as Yosys' do; no cell
        // *type* may, since a `$`-type is a generic cell.
        assert!(!inputs.json.contains("\"type\": \"$"), "{}", inputs.json);
    }

    /// The flow declines what it cannot do instead of half doing it.
    #[cfg(feature = "synth")]
    #[test]
    fn the_flow_reports_what_it_cannot_take() {
        let (mut design, _top, _sources) = blinky();
        let device = target("ice40-hx1k-tq144").unwrap();
        let mut diags = Diagnostics::new();
        let err = synthesize_for(
            &mut design,
            ModuleId::from_index(42),
            device,
            &Constraints::new(),
            &FpgaOptions::default(),
            &mut diags,
        )
        .unwrap_err();
        assert_eq!(err, FlowError::NoSuchModule);

        // An invalid design is synthesis' business to reject, and the
        // flow stops there rather than mapping nonsense.
        let mut sources = SourceMap::new();
        let file = sources.add("bad.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("bad", span);
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(8));
        let a_e = b.net(a);
        b.assign(y, a_e);
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        let err = synthesize_for(
            &mut design,
            top,
            device,
            &Constraints::new(),
            &FpgaOptions::default(),
            &mut diags,
        )
        .unwrap_err();
        assert_eq!(err, FlowError::Synthesis);
        assert!(err.to_string().contains("could not be synthesised"));
        assert!(diags.has_errors());
    }

    /// The netlist check finds everything nextpnr would trip over.
    #[cfg(feature = "synth")]
    #[test]
    fn the_check_finds_what_nextpnr_would_reject() {
        use crate::fpga::{IoAttrs, Origin, PinAssignment};

        let (mut design, top, _sources) = blinky();
        let device = target("ice40-hx1k-tq144").unwrap();
        let mut diags = Diagnostics::new();
        // Stopping before the rewrite to device cells leaves exactly the
        // netlist the flow used to produce: generic `$lut` and `$dff`.
        let options = FpgaOptions {
            device_cells: false,
            ..FpgaOptions::default()
        };
        let mut sources = SourceMap::new();
        let file = sources.add("c.rcf", "").unwrap();
        let mut constraints = Constraints::new();
        constraints.pins.push(PinAssignment {
            port: "led".to_owned(),
            bit: None,
            pin: "1234".to_owned(),
            io: IoAttrs::default(),
            span: Span::new(file, 0, 0),
            origin: Origin::File,
        });
        synthesize_for(&mut design, top, device, &constraints, &options, &mut diags).unwrap();
        let problems = check_nextpnr_json(&design, top, device, &constraints);
        let text: Vec<String> = problems.iter().map(NetlistProblem::to_string).collect();
        assert!(
            text.iter().any(|p| p.contains("`$lut` is a generic cell")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|p| p.contains("`$dff` is a generic cell")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|p| p.contains("has no package pin `1234`")),
            "{text:?}"
        );
    }

    /// A netlist with a cell the device never heard of, a port that
    /// primitive does not have and a constant no packer can tie is
    /// reported object by object.
    #[test]
    fn the_check_reads_the_device_database() {
        let mut sources = SourceMap::new();
        let file = sources.add("t.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("t", span);
        let a = b.input("a", Type::bit());
        let y = b.output("y", Type::bit());
        let a_e = b.net(a);
        let x = b.constant(crate::ir::Const::x(1));
        b.cell(
            "invented",
            CellKind::Blackbox(Name::new("SB_NONESUCH")),
            vec![(Name::new("A"), a_e)],
            vec![(Name::new("Y"), y)],
        );
        b.cell(
            "wrong_port",
            CellKind::Blackbox(Name::new("SB_GB")),
            vec![(Name::new("NOT_A_PORT"), x)],
            vec![],
        );
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        let device = target("ice40-hx1k-tq144").unwrap();
        let problems = check_nextpnr_json(&design, top, device, &Constraints::new());
        let text: Vec<String> = problems.iter().map(NetlistProblem::to_string).collect();
        assert!(
            text.iter()
                .any(|p| p.contains("`SB_NONESUCH` is not a primitive")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|p| p.contains("`SB_GB` has no port `NOT_A_PORT`")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|p| p.contains("the constant `x`")),
            "{text:?}"
        );
        // A family with no nextpnr says so rather than pretending.
        let generic = target("generic").unwrap();
        let problems = check_nextpnr_json(&design, top, generic, &Constraints::new());
        assert!(
            problems.iter().any(|p| p
                .message
                .contains("no nextpnr flow is known for family `generic`")),
            "{problems:?}"
        );
        assert!(constant_convention("ice40").is_some());
        assert!(constant_convention("ecp5").is_some());
        assert!(constant_convention("gowin").is_none());
        assert_eq!(
            NetlistProblem {
                object: "c".to_owned(),
                message: "why".to_owned(),
            }
            .to_string(),
            "`c`: why"
        );
    }

    /// An undriven or doubly driven net is one nextpnr cannot route, so
    /// the check reports it even though the IR is perfectly valid.
    #[test]
    fn the_check_finds_undriven_and_multiply_driven_nets() {
        let mut sources = SourceMap::new();
        let file = sources.add("t.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("t", span);
        let clk = b.input("clk", Type::bit());
        let y = b.output("y", Type::bit());
        let floating = b.add_net("floating", Type::bit());
        let floating_e = b.net(floating);
        let clk_e = b.net(clk);
        b.cell(
            "gb1",
            CellKind::Blackbox(Name::new("SB_GB")),
            vec![(Name::new("USER_SIGNAL_TO_GLOBAL_BUFFER"), floating_e)],
            vec![(Name::new("GLOBAL_BUFFER_OUTPUT"), y)],
        );
        b.cell(
            "gb2",
            CellKind::Blackbox(Name::new("SB_GB")),
            vec![(Name::new("USER_SIGNAL_TO_GLOBAL_BUFFER"), clk_e)],
            vec![(Name::new("GLOBAL_BUFFER_OUTPUT"), y)],
        );
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        let device = target("ice40-hx1k-tq144").unwrap();
        let problems = check_nextpnr_json(&design, top, device, &Constraints::new());
        let text: Vec<String> = problems.iter().map(NetlistProblem::to_string).collect();
        assert!(text.iter().any(|p| p.contains("has 2 drivers")), "{text:?}");
        assert!(
            text.iter()
                .any(|p| p.contains("reads `floating` bit 0, which nothing drives")),
            "{text:?}"
        );
    }
}
