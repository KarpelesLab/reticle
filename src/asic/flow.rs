//! The ASIC flow: from RTL to a netlist of standard cells, with area
//! and timing from one call.
//!
//! [`synthesize_asic`] is the ASIC counterpart of
//! [`crate::fpga::synthesize_for`]. It takes a design in the IR, a
//! Liberty library, and rewrites one module into cells of that library:
//!
//! 1. **Generic synthesis** ([`crate::synth::run`]): processes become
//!    cells, flip-flops and memories are inferred, the logic is
//!    optimised. Everything after this works on the cell form.
//! 2. **Flip-flop legalisation** ([`legalize_flops`]): a clock enable or
//!    a synchronous reset the library has no pin for becomes a
//!    multiplexer on `d`. This happens *before* mapping so those
//!    multiplexers are mapped into gates with the rest of the logic —
//!    doing it afterwards would leave unmapped cells in the netlist.
//! 3. **Standard-cell mapping** ([`crate::synth::techmap::map_module`])
//!    over the [`StdCells`] built from the library.
//! 4. **Clean-up** ([`crate::synth::opt::Dce`]): the logic the mapper
//!    absorbed leaves dead cells and nets behind.
//! 5. **Flip-flop mapping** ([`map_flops`]): each inferred `dff` becomes
//!    one library cell per bit, with an inverter on a reset or enable
//!    whose polarity the library does not have.
//! 6. **Drive strength** ([`resize_drivers`], optional): a cell driving
//!    more capacitance than its `max_capacitance` allows is swapped for
//!    a larger variant of the same cell, when the library ships one.
//! 7. **Timing** (with the `timing` feature): static timing analysis
//!    over the mapped netlist with [`crate::timing::delay::LibertyModel`],
//!    so the same call that reports area reports the critical path.
//!
//! Only the named module is mapped; a hierarchical design is flattened
//! first ([`crate::ir::hier`]), as place and route wants one flat
//! netlist.
//!
//! # What the netlist holds
//!
//! Every mapped cell is a [`CellKind::Blackbox`] named exactly as the
//! library spells it, carrying a `lib_cell` attribute with the same
//! name, and its ports are the library cell's pins. Nothing is emitted
//! as an IR primitive even when a cell is exactly one (an `AND2` is not
//! turned into `$and`), because a place-and-route tool wants an instance
//! of a library cell and nothing else, and because `lib_cell` is what
//! [`crate::asic::def::from_netlist`] keys on.
//!
//! That makes the netlist opaque to anything that wants to *evaluate* it
//! — the simulator, the bit-blaster, the equivalence checker.
//! [`logic_model`] is the way back: it rewrites a mapped module into one
//! where every standard cell is the generic IR cell computing the same
//! function (a [`CellKind::Lut`] for a gate, a [`CellKind::Dff`] for a
//! flip-flop), so the mapped design can be simulated and proved
//! equivalent to the design it came from.
//!
//! # What is not done here
//!
//! - **No buffer insertion.** [`resize_drivers`] only swaps a cell for a
//!   stronger variant of itself. A net that no drive strength can carry
//!   is reported ([`AsicReport::overloaded`]) and left for the physical
//!   flow, which has to do it anyway once it knows the wire load —
//!   OpenROAD's `repair_design` is in the generated script for exactly
//!   this, and the Tcl runs it after placement where the numbers are
//!   real. Inserting buffers here, against a wire load of zero, would be
//!   guessing.
//! - **No clock tree.** The clock reaches every flip-flop directly;
//!   `clock_tree_synthesis` builds the tree.
//! - **No scan insertion, no power intent, no multi-corner analysis.**

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::liberty::Library;
use super::library::{
    FlopPinUse, FlopRequest, LibraryOptions, ResetRequest, StdCells, skip_summary,
};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{
    Assign, Attrs, Bit, Cell, CellId, CellKind, Const, Design, Expr, ExprId, ExprKind, Lvalue,
    Module, ModuleId, Name, Net, NetId, NetKind, Reset, Span, Type, infer_type,
};
use crate::synth::cells::Gate;

/// Diagnostic code for a flip-flop the library cannot implement.
pub const NO_FLOP_CELL: &str = "A0301";
/// Diagnostic code for a net no drive strength in the library can carry.
pub const OVERLOADED_NET: &str = "A0302";
/// Diagnostic code for a cell the mapper left generic.
pub const UNMAPPED_CELL: &str = "A0303";

/// Why the ASIC flow could not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsicError {
    /// The module id does not belong to the design.
    NoSuchModule,
    /// Generic synthesis reported errors, so nothing was mapped; the
    /// errors are in the diagnostics the caller passed in.
    Synthesis,
    /// The library has nothing to map onto; the text says what is
    /// missing.
    UnusableLibrary(String),
    /// A construct the netlist format cannot express.
    Emit(crate::ir::emit::EmitError),
}

impl fmt::Display for AsicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AsicError::NoSuchModule => f.write_str("the module is not part of the design"),
            AsicError::Synthesis => {
                f.write_str("the design could not be synthesised; see the reported errors")
            }
            AsicError::UnusableLibrary(why) => {
                write!(f, "the library cannot be mapped onto: {why}")
            }
            AsicError::Emit(err) => write!(f, "{err}"),
        }
    }
}

impl Error for AsicError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            AsicError::Emit(err) => Some(err),
            _ => None,
        }
    }
}

impl From<crate::ir::emit::EmitError> for AsicError {
    fn from(err: crate::ir::emit::EmitError) -> Self {
        AsicError::Emit(err)
    }
}

/// Knobs for [`synthesize_asic`].
#[derive(Clone, Debug)]
pub struct AsicOptions {
    /// How the technology-independent front half behaves.
    pub synth: crate::synth::SynthOptions,
    /// How the Liberty library is read.
    pub library: LibraryOptions,
    /// Area recovery passes after the depth-oriented mapping pass.
    pub area_passes: u32,
    /// Run the drive-strength pass.
    pub resize: bool,
    /// How many times the drive-strength pass is repeated: upsizing a
    /// cell loads its own driver, so one pass does not settle.
    pub resize_passes: u32,
    /// Capacitance added to every net on top of the pins it drives, for
    /// the drive-strength pass. Zero by default: there is no placement
    /// yet, so any wire load here is a guess.
    pub wire_load: f64,
    /// Run static timing analysis (needs the `timing` feature).
    pub timing: bool,
    /// The timing constraints, used by the analysis and written out as
    /// SDC by [`super::openroad::export_openroad`].
    pub constraints: super::sdc::AsicConstraints,
}

impl Default for AsicOptions {
    fn default() -> Self {
        AsicOptions::new()
    }
}

impl AsicOptions {
    /// The defaults: full synthesis, two area recovery passes, the
    /// drive-strength pass on, timing on, no constraints.
    pub fn new() -> AsicOptions {
        AsicOptions {
            synth: crate::synth::SynthOptions::default(),
            library: LibraryOptions::default(),
            area_passes: 2,
            resize: true,
            resize_passes: 2,
            wire_load: 0.0,
            timing: true,
            constraints: super::sdc::AsicConstraints::new(),
        }
    }
}

/// What the static timing analysis said, in a form that does not depend
/// on the `timing` feature being compiled in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AsicTiming {
    /// The delay model's name.
    pub model: String,
    /// The worst setup slack, if anything was checked.
    pub worst_setup: Option<f64>,
    /// The worst hold slack, if anything was checked.
    pub worst_hold: Option<f64>,
    /// The worst path: `start -> end` and which check it failed or
    /// passed most narrowly.
    pub critical_path: Option<String>,
    /// The delay along the critical path.
    pub critical_delay: Option<f64>,
    /// How many end points fail a check.
    pub violations: usize,
    /// The analyser's own summary, for reports.
    pub summary: String,
}

/// What [`synthesize_asic`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AsicReport {
    /// The library mapped against.
    pub library: String,
    /// The module mapped.
    pub module: String,
    /// How many library cells the mapper was offered.
    pub gates_offered: usize,
    /// How many library cells were skipped, by reason.
    pub gates_skipped: Vec<(String, usize)>,
    /// Combinational cells the mapper produced.
    pub logic_cells: usize,
    /// Depth of the mapped logic, in cells.
    pub logic_depth: u32,
    /// Flip-flop cells produced.
    pub flop_cells: usize,
    /// Inverters inserted because the library lacked a polarity.
    pub inverters_inserted: usize,
    /// Clock enables moved into a multiplexer on `d`.
    pub lowered_enables: usize,
    /// Resets moved into a multiplexer on `d`.
    pub lowered_resets: usize,
    /// Cells swapped for a stronger drive variant.
    pub resized: usize,
    /// Nets no drive strength in the library can carry, sorted.
    pub overloaded: Vec<String>,
    /// Every cell type in the finished netlist with how many there are,
    /// sorted by type. A name starting with `$` is a generic cell the
    /// flow could not map.
    pub cells: Vec<(String, usize)>,
    /// Total area, in the library's area unit.
    pub area: f64,
    /// Cells left generic, as `(cell, why)`.
    pub unmapped: Vec<(String, String)>,
    /// The timing analysis, when it ran.
    pub timing: Option<AsicTiming>,
    /// Anything worth saying about the run, in a fixed order.
    pub notes: Vec<String>,
}

impl AsicReport {
    /// How many cells of type `name` the netlist holds.
    pub fn count(&self, name: &str) -> usize {
        self.cells
            .iter()
            .find(|(cell, _)| cell == name)
            .map_or(0, |(_, n)| *n)
    }

    /// The total number of cells.
    pub fn total_cells(&self) -> usize {
        self.cells.iter().map(|(_, n)| *n).sum()
    }

    /// True when every cell in the netlist is a library cell.
    pub fn is_fully_mapped(&self) -> bool {
        self.unmapped.is_empty() && !self.cells.iter().any(|(name, _)| name.starts_with('$'))
    }

    /// Renders the report as plain text.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "module `{}` mapped onto `{}`",
            self.module, self.library
        );
        let _ = writeln!(
            out,
            "library: {} cells offered to the mapper, {} skipped",
            self.gates_offered,
            self.gates_skipped.iter().map(|(_, n)| n).sum::<usize>()
        );
        for (reason, count) in &self.gates_skipped {
            let _ = writeln!(out, "  {count} x {reason}");
        }
        let _ = writeln!(
            out,
            "logic: {} cells, depth {}",
            self.logic_cells, self.logic_depth
        );
        let _ = writeln!(out, "sequential: {} flip-flop cells", self.flop_cells);
        if self.lowered_enables > 0 || self.lowered_resets > 0 || self.inverters_inserted > 0 {
            let _ = writeln!(
                out,
                "  {} enable(s) and {} reset(s) moved into the data path, {} inverter(s) inserted",
                self.lowered_enables, self.lowered_resets, self.inverters_inserted
            );
        }
        if self.resized > 0 || !self.overloaded.is_empty() {
            let _ = writeln!(
                out,
                "drive: {} cell(s) upsized, {} overloaded net(s)",
                self.resized,
                self.overloaded.len()
            );
            for net in &self.overloaded {
                let _ = writeln!(out, "  {net}");
            }
        }
        let _ = writeln!(out, "netlist: {} cells", self.total_cells());
        for (name, count) in &self.cells {
            let _ = writeln!(out, "  {count} x {name}");
        }
        let _ = writeln!(out, "area: {}", super::fmt_num(self.area));
        for (cell, why) in &self.unmapped {
            let _ = writeln!(out, "unmapped: {cell} ({why})");
        }
        if let Some(timing) = &self.timing {
            let _ = writeln!(out, "timing ({} delays):", timing.model);
            let slack = |v: Option<f64>| v.map_or("-".to_string(), |s| format!("{s:.3}"));
            let _ = writeln!(
                out,
                "  worst setup slack {}, worst hold slack {}, {} violation(s)",
                slack(timing.worst_setup),
                slack(timing.worst_hold),
                timing.violations
            );
            if let Some(path) = &timing.critical_path {
                let _ = writeln!(
                    out,
                    "  worst path: {path}, {} of delay",
                    slack(timing.critical_delay)
                );
            }
        }
        for note in &self.notes {
            let _ = writeln!(out, "note: {note}");
        }
        out
    }
}

/// Takes `module` from the IR to a netlist of `library`'s cells, in
/// place.
///
/// The steps and the reason for their order are in the module docs.
/// Problems the user can act on are reported through `diags`; the
/// returned [`AsicReport`] says what was mapped, what it costs and how
/// fast it is.
///
/// # Errors
///
/// [`AsicError::NoSuchModule`] when the id does not belong to the
/// design, [`AsicError::Synthesis`] when generic synthesis rejected it,
/// and [`AsicError::UnusableLibrary`] when the library offers no gate
/// the mapper can use or has no inverter (the mapper needs one to
/// complement a cut).
pub fn synthesize_asic(
    design: &mut Design,
    module: ModuleId,
    library: &Library,
    options: &AsicOptions,
    diags: &mut Diagnostics,
) -> Result<AsicReport, AsicError> {
    use crate::synth::Pass;
    use crate::synth::opt::Dce;
    use crate::synth::techmap::{MapOptions, map_module};

    if design.modules.get(module).is_none() {
        return Err(AsicError::NoSuchModule);
    }
    let cells = StdCells::from_library(library, &options.library);
    if cells.gates().is_empty() {
        return Err(AsicError::UnusableLibrary(format!(
            "`{}` has no combinational cell the mapper can use",
            library.name
        )));
    }
    if cells.gates().inverter().is_none() {
        return Err(AsicError::UnusableLibrary(format!(
            "`{}` has no inverter",
            library.name
        )));
    }

    let mut report = AsicReport {
        library: library.name.clone(),
        module: design.module(module).name.as_str().to_owned(),
        gates_offered: cells.gates().len(),
        gates_skipped: skip_summary(cells.skipped()),
        ..AsicReport::default()
    };
    if cells.flops().is_empty() {
        report
            .notes
            .push(format!("`{}` has no flip-flop cell", library.name));
    }

    // 1. Generic synthesis, with its own diagnostics deciding whether
    // the rest of the flow is worth running.
    let mut synth_diags = Diagnostics::new();
    let _ = crate::synth::run(design, &options.synth, &mut synth_diags);
    let refused = synth_diags.has_errors();
    diags.append(&mut synth_diags);
    if refused {
        return Err(AsicError::Synthesis);
    }

    // 2. What the library has no pin for goes into the data path, while
    // there is still a mapper to map it.
    legalize_flops(&mut design.modules[module], &cells, &mut report, diags);

    // 3. Everything combinational onto library cells.
    let mut map = MapOptions::gates(cells.gates());
    map.area_passes = options.area_passes;
    let stats = map_module(&mut design.modules[module], &map);
    report.logic_cells = stats.cells;
    report.logic_depth = stats.depth;
    restore_output_pins(&mut design.modules[module], &cells);

    // 4. Clean-up: the mapper absorbed logic and left the old cells.
    let mut clean = Diagnostics::new();
    let _ = Dce.run(&mut design.modules[module], &mut clean);
    diags.append(&mut clean);

    // 5. The flip-flops.
    map_flops(&mut design.modules[module], &cells, &mut report, diags);

    // 6. Drive strengths.
    if options.resize {
        resize_drivers(
            &mut design.modules[module],
            &cells,
            options,
            &mut report,
            diags,
        );
    } else {
        report
            .notes
            .push("the drive-strength pass was not run".to_owned());
    }

    report.cells = cell_types(design.module(module));
    report.area = netlist_area(design.module(module), &cells);
    for (name, _) in &report.cells {
        if name.starts_with('$') {
            diags.push(
                Diagnostic::warning(format!(
                    "the netlist still holds generic `{name}` cells, which no \
                     place-and-route tool will read"
                ))
                .with_code(UNMAPPED_CELL)
                .with_span(design.module(module).span),
            );
        }
    }

    report.timing = run_timing(design, module, library, options, &mut report.notes);
    Ok(report)
}

/// Runs the static timing analysis over the mapped netlist.
#[cfg(feature = "timing")]
fn run_timing(
    design: &Design,
    module: ModuleId,
    library: &Library,
    options: &AsicOptions,
    notes: &mut Vec<String>,
) -> Option<AsicTiming> {
    use crate::timing::delay::LibertyModel;
    use crate::timing::sta::{Check, TimingOptions, analyze_with};

    if !options.timing {
        notes.push("static timing analysis was not run".to_owned());
        return None;
    }
    let spec = options.constraints.to_timing_spec();
    if spec.clocks.is_empty() {
        notes.push(
            "no `create_clock`, so the analysis used a default period for every clock pin"
                .to_owned(),
        );
    }
    let mut timing_options = TimingOptions::default();
    options.constraints.apply_to_options(&mut timing_options);
    let model = LibertyModel::new(library);
    let report = analyze_with(design.module(module), &timing_options, &model, &spec);
    // "The critical path" means the longest data path, which is the
    // worst *setup* path; a hold path with less slack is a different
    // problem and is counted among the violations instead.
    let worst = report
        .paths
        .iter()
        .find(|p| p.check == Check::Setup)
        .or_else(|| report.paths.first());
    Some(AsicTiming {
        model: report.model.clone(),
        worst_setup: report.worst_setup(),
        worst_hold: report.worst_hold(),
        critical_path: worst.map(|p| {
            format!(
                "{} -> {} ({} check)",
                p.start_pin,
                p.end_pin,
                p.check.as_str()
            )
        }),
        critical_delay: worst.map(|p| p.arrival - p.launch_edge),
        violations: report.endpoints.iter().filter(|e| e.slack < 0.0).count(),
        summary: report.render_summary(),
    })
}

/// Without the `timing` feature there is no analyser to run.
#[cfg(not(feature = "timing"))]
fn run_timing(
    _design: &Design,
    _module: ModuleId,
    _library: &Library,
    options: &AsicOptions,
    notes: &mut Vec<String>,
) -> Option<AsicTiming> {
    if options.timing {
        notes.push("static timing analysis needs the `timing` feature".to_owned());
    }
    None
}

/// Gives every mapped cell its library output pin name back.
///
/// The technology mapper names the output port of every cell it emits
/// `y`, since it has no opinion about a library's spelling. A
/// place-and-route tool has one: the port has to be the pin the LEF
/// macro and the Liberty cell declare, which is `Y` here, `Z` in some
/// libraries and `ZN` in others.
fn restore_output_pins(module: &mut Module, cells: &StdCells) {
    let ids: Vec<CellId> = module.cells.iter().map(|(id, _)| id).collect();
    for id in ids {
        let CellKind::Blackbox(name) = &module.cells[id].kind else {
            continue;
        };
        let Some(gate) = cells.gates().gate(name.as_str()) else {
            continue;
        };
        let output = gate.output.clone();
        for (port, _) in &mut module.cells[id].outputs {
            if port.as_str() != output {
                *port = Name::new(output.clone());
            }
        }
    }
}

/// Every cell type in a module with how many there are, sorted by type.
fn cell_types(module: &Module) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, cell) in module.cells.iter() {
        let name = match &cell.kind {
            CellKind::Blackbox(name) => name.as_str().to_owned(),
            other => format!("${}", other.keyword()),
        };
        *counts.entry(name).or_default() += 1;
    }
    counts.into_iter().collect()
}

/// The area of every cell of the netlist the library knows.
fn netlist_area(module: &Module, cells: &StdCells) -> f64 {
    let mut area = 0.0;
    for (_, cell) in module.cells.iter() {
        let CellKind::Blackbox(name) = &cell.kind else {
            continue;
        };
        if let Some(gate) = cells.gates().gate(name.as_str()) {
            area += gate.area;
        } else if let Some(flop) = cells.flops().iter().find(|f| f.name == name.as_str()) {
            area += flop.area;
        }
    }
    area
}

// --- flip-flop legalisation -------------------------------------------------

/// Moves a clock enable or a reset the library has no pin for into a
/// multiplexer on `d`.
///
/// Run before technology mapping: the multiplexers this leaves behind are
/// ordinary combinational logic and are meant to be mapped with
/// everything else. The rewrite keeps the IR's priority — a reset wins
/// over an enable — by wrapping the enable multiplexer in the reset one.
pub fn legalize_flops(
    module: &mut Module,
    cells: &StdCells,
    report: &mut AsicReport,
    diags: &mut Diagnostics,
) {
    let ids: Vec<CellId> = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
        .map(|(id, _)| id)
        .collect();
    for id in ids {
        legalize_flop(module, cells, id, report, diags);
    }
}

fn legalize_flop(
    module: &mut Module,
    cells: &StdCells,
    id: CellId,
    report: &mut AsicReport,
    diags: &mut Diagnostics,
) {
    let cell = &module.cells[id];
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset,
    } = cell.kind.clone()
    else {
        return;
    };
    let name = cell.name.as_str().to_owned();
    let span = cell.span;
    let (Some(d), Some(q)) = (cell.input("d"), cell.output("q")) else {
        return;
    };
    let en = cell.input("en");
    let rst = cell.input("rst");
    let width = net_width(module, q);

    // The library is asked about every reset value a bit of this
    // register wants, since a bit reset to one needs a `preset` cell and
    // a bit reset to zero a `clear` one.
    let mut lower_enable = false;
    let mut lower_reset = false;
    let mut problem = None;
    for request in requests_of(&cell.kind, width) {
        let plan = cells.plan_flop(&request);
        lower_enable |= plan.lower_enable;
        lower_reset |= plan.lower_reset;
        if plan.cell.is_none() && problem.is_none() {
            problem = plan.problem;
        }
    }
    if let Some(problem) = problem {
        // Nothing to lower would help; `map_flops` reports it once the
        // mapping is done, so the message names the final netlist.
        let _ = problem;
        return;
    }
    if !lower_enable && !lower_reset {
        return;
    }

    let mut next = d;
    if lower_enable && has_enable {
        let Some(en) = en else { return };
        // `en ? d : q`, with the multiplexer's `s = 1` arm taking `d`.
        let q_read = net_expr(module, q, span);
        let out = add_net(module, &format!("{name}$en"), net_type(module, q), span);
        add_cell(
            module,
            &format!("{name}$enmux"),
            CellKind::Mux,
            vec![
                (Name::new("a"), q_read),
                (Name::new("b"), next),
                (Name::new("s"), en),
            ],
            vec![(Name::new("y"), out)],
            span,
        );
        next = net_expr(module, out, span);
        report.lowered_enables += 1;
    }
    let mut new_reset = reset.clone();
    if lower_reset {
        let (Some(rst), Some(r)) = (rst, reset.as_ref()) else {
            return;
        };
        let value = const_expr(module, r.value.clone(), span);
        let out = add_net(module, &format!("{name}$rst"), net_type(module, q), span);
        // The multiplexer's `s = 1` arm is `b`, so an active-low reset
        // swaps the arms instead of costing an inverter.
        let (a, b) = if r.active_high {
            (next, value)
        } else {
            (value, next)
        };
        add_cell(
            module,
            &format!("{name}$rstmux"),
            CellKind::Mux,
            vec![
                (Name::new("a"), a),
                (Name::new("b"), b),
                (Name::new("s"), rst),
            ],
            vec![(Name::new("y"), out)],
            span,
        );
        next = net_expr(module, out, span);
        new_reset = None;
        report.lowered_resets += 1;
        diags.push(
            Diagnostic::note(format!(
                "flip-flop `{name}`: the {} reset became a multiplexer on `d`",
                if r.asynchronous {
                    "asynchronous"
                } else {
                    "synchronous"
                }
            ))
            .with_span(span),
        );
    }

    let cell = &mut module.cells[id];
    cell.kind = CellKind::Dff {
        clk_pos,
        has_enable: has_enable && !lower_enable,
        reset: new_reset,
    };
    cell.inputs.retain(|(port, _)| {
        !(lower_enable && port.as_str() == "en") && !(lower_reset && port.as_str() == "rst")
    });
    for (port, value) in &mut cell.inputs {
        if port.as_str() == "d" {
            *value = next;
        }
    }
}

/// One request per distinct reset value among the bits of a register.
fn requests_of(kind: &CellKind, width: u32) -> Vec<FlopRequest> {
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset,
    } = kind
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut push = |reset: Option<ResetRequest>| {
        let request = FlopRequest {
            clk_pos: *clk_pos,
            enable: *has_enable,
            reset,
        };
        if !out.contains(&request) {
            out.push(request);
        }
    };
    match reset {
        None => push(None),
        Some(r) => {
            for bit in 0..width.max(1) {
                push(Some(ResetRequest {
                    asynchronous: r.asynchronous,
                    active_high: r.active_high,
                    // An `x` in the reset value is a don't-care; taking
                    // it as a zero keeps the flip-flop the simpler of the
                    // two.
                    sets: r.value.bit(bit) == Bit::One,
                }));
            }
        }
    }
    out
}

// --- flip-flop mapping ------------------------------------------------------

/// Rewrites every inferred `dff` into library cells, one per bit.
///
/// A flip-flop the library has no variant for is left generic and
/// reported (`A0301`), the way the FPGA side does: a netlist that
/// simulates differently from the design is worse than one a tool
/// refuses.
pub fn map_flops(
    module: &mut Module,
    cells: &StdCells,
    report: &mut AsicReport,
    diags: &mut Diagnostics,
) {
    let ids: Vec<CellId> = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
        .map(|(id, _)| id)
        .collect();
    let mut inverters: BTreeMap<(NetId, bool), NetId> = BTreeMap::new();
    let mut replaced = Vec::new();
    for id in ids {
        if map_flop(module, cells, id, &mut inverters, report, diags) {
            replaced.push(id);
        }
    }
    if !replaced.is_empty() {
        module.cells.retain(|id, _| !replaced.contains(&id));
    }
}

/// Maps one flip-flop; true when the original cell is to be removed
/// because per-bit cells replaced it.
fn map_flop(
    module: &mut Module,
    cells: &StdCells,
    id: CellId,
    inverters: &mut BTreeMap<(NetId, bool), NetId>,
    report: &mut AsicReport,
    diags: &mut Diagnostics,
) -> bool {
    let cell = &module.cells[id];
    let kind = cell.kind.clone();
    let name = cell.name.as_str().to_owned();
    let span = cell.span;
    let (Some(clk), Some(d), Some(q)) = (cell.input("clk"), cell.input("d"), cell.output("q"))
    else {
        report
            .unmapped
            .push((name, "the flip-flop is not wired".to_owned()));
        return false;
    };
    let en = cell.input("en");
    let rst = cell.input("rst");
    let width = net_width(module, q);

    // Every bit must map before anything is rewritten, so a register
    // with one impossible bit is reported whole.
    let mut plans = Vec::with_capacity(usize::try_from(width).unwrap_or(0));
    for bit in 0..width.max(1) {
        let request = request_of(&kind, bit);
        let plan = cells.plan_flop(&request);
        match plan.cell {
            Some(matched) if !plan.lower_enable && !plan.lower_reset => plans.push(matched),
            _ => {
                let why = plan.problem.unwrap_or_else(|| {
                    format!(
                        "`{}` needs {} moved into the data path first",
                        name,
                        if plan.lower_enable {
                            "its clock enable"
                        } else {
                            "its reset"
                        }
                    )
                });
                diags.push(
                    Diagnostic::error(why.clone())
                        .with_code(NO_FLOP_CELL)
                        .with_span(span)
                        .with_note(format!(
                            "flip-flop `{name}`{} stays a generic cell, which a \
                             place-and-route tool will reject",
                            if width > 1 {
                                format!(" (bit {bit})")
                            } else {
                                String::new()
                            }
                        )),
                );
                report.unmapped.push((name, why));
                return false;
            }
        }
    }

    let mut bits = Vec::with_capacity(usize::try_from(width).unwrap_or(0));
    for bit in 0..width.max(1) {
        let matched = plans[usize::try_from(bit).unwrap_or(0)].clone();
        let mut inputs = vec![(Name::new(matched.clock_pin.clone()), clk)];
        let data = if width == 1 {
            d
        } else {
            slice_expr(module, d, bit, bit, span)
        };
        inputs.push((Name::new(matched.data_pin.clone()), data));
        for (use_, signal) in [
            (&matched.enable, en),
            (&matched.clear, rst),
            (&matched.preset, rst),
        ] {
            let Some(use_) = use_ else { continue };
            let value = match use_ {
                FlopPinUse::Direct(_) => match signal {
                    Some(e) => e,
                    None => continue,
                },
                FlopPinUse::Inverted(_) => {
                    let Some(e) = signal else { continue };
                    let net = inverted(module, cells, e, inverters, report, span);
                    net_expr(module, net, span)
                }
                FlopPinUse::Tied(_, level) => const_expr(module, Const::from_bool(*level), span),
            };
            inputs.push((Name::new(use_.pin().to_owned()), value));
        }

        let out = if width == 1 {
            q
        } else {
            add_net(module, &format!("{name}$q{bit}"), Type::bit(), span)
        };
        let outputs = vec![(Name::new(matched.q_pin.clone()), out)];
        let kind = CellKind::Blackbox(Name::new(matched.cell.clone()));
        if width == 1 {
            let cell = &mut module.cells[id];
            cell.kind = kind;
            cell.inputs = inputs;
            cell.outputs = outputs;
            cell.attrs.set("lib_cell", matched.cell.as_str());
            report.flop_cells += 1;
            return false;
        }
        let new = add_cell(
            module,
            &format!("{name}$ff{bit}"),
            kind,
            inputs,
            outputs,
            span,
        );
        module.cells[new]
            .attrs
            .set("lib_cell", matched.cell.as_str());
        report.flop_cells += 1;
        bits.push(net_expr(module, out, span));
    }
    bits.reverse();
    let value = expr(module, ExprKind::Concat(bits), span);
    add_assign(module, q, value, span);
    true
}

/// The request one bit of a flip-flop makes.
fn request_of(kind: &CellKind, bit: u32) -> FlopRequest {
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset,
    } = kind
    else {
        return FlopRequest::new();
    };
    FlopRequest {
        clk_pos: *clk_pos,
        enable: *has_enable,
        reset: reset.as_ref().map(|r| ResetRequest {
            asynchronous: r.asynchronous,
            active_high: r.active_high,
            sets: r.value.bit(bit) == Bit::One,
        }),
    }
}

/// The net carrying the complement of `signal`, inserting an inverter
/// cell the first time it is asked for.
fn inverted(
    module: &mut Module,
    cells: &StdCells,
    signal: ExprId,
    cache: &mut BTreeMap<(NetId, bool), NetId>,
    report: &mut AsicReport,
    span: Span,
) -> NetId {
    let key = match &module.expr(signal).kind {
        ExprKind::Net(net) => Some((*net, true)),
        _ => None,
    };
    if let Some(key) = key
        && let Some(net) = cache.get(&key)
    {
        return *net;
    }
    let gate = cells
        .gates()
        .inverter()
        .expect("the library was checked for an inverter");
    let base = match key {
        Some((net, _)) => format!("{}$n", module.nets[net].name),
        None => "inv$n".to_owned(),
    };
    let out = add_net(module, &base, Type::bit(), span);
    let new = add_cell(
        module,
        &format!("{base}$inv"),
        CellKind::Blackbox(Name::new(gate.name.clone())),
        vec![(Name::new(gate.pins[0].clone()), signal)],
        vec![(Name::new(gate.output.clone()), out)],
        span,
    );
    module.cells[new].attrs.set("lib_cell", gate.name.as_str());
    report.inverters_inserted += 1;
    if let Some(key) = key {
        cache.insert(key, out);
    }
    out
}

// --- drive strength ---------------------------------------------------------

/// Swaps a cell driving more capacitance than it is rated for with a
/// stronger variant of itself.
///
/// The load of a net is the sum of the `capacitance` of every library
/// pin reading it, plus [`AsicOptions::wire_load`]. A cell whose output
/// pin has a `max_capacitance` below that load is replaced by the
/// smallest variant of the same cell whose rating covers it; a net that
/// no variant covers is reported (`A0302`) and left alone, because
/// fixing it means inserting buffers, which needs a placement to be
/// worth anything.
///
/// The pass is repeated ([`AsicOptions::resize_passes`]): making a cell
/// stronger makes its own input pins bigger, which loads whatever drives
/// it.
pub fn resize_drivers(
    module: &mut Module,
    cells: &StdCells,
    options: &AsicOptions,
    report: &mut AsicReport,
    diags: &mut Diagnostics,
) {
    let mut overloaded: BTreeMap<String, String> = BTreeMap::new();
    for _ in 0..options.resize_passes.max(1) {
        let loads = net_loads(module, cells, options.wire_load);
        let mut changed = false;
        overloaded.clear();
        let ids: Vec<CellId> = module.cells.iter().map(|(id, _)| id).collect();
        for id in ids {
            let cell = &module.cells[id];
            let CellKind::Blackbox(name) = &cell.kind else {
                continue;
            };
            let name = name.as_str().to_owned();
            let Some(gate) = cells.gates().gate(&name).cloned() else {
                continue;
            };
            let Some(out) = cell.output(&gate.output) else {
                continue;
            };
            let load = loads.get(&out).copied().unwrap_or(0.0);
            let rating = cells
                .electrical(&name)
                .and_then(|e| e.max_capacitance)
                .unwrap_or(f64::INFINITY);
            if load <= rating {
                continue;
            }
            let variants = cells.drive_variants(&name);
            let bigger = variants.iter().find(|candidate| {
                cells
                    .electrical(candidate)
                    .and_then(|e| e.max_capacitance)
                    .is_some_and(|max| max >= load)
            });
            match bigger {
                Some(bigger) if *bigger != name => {
                    let bigger = (*bigger).to_owned();
                    retarget(module, id, &gate, &bigger, cells);
                    report.resized += 1;
                    changed = true;
                }
                _ => {
                    overloaded.insert(
                        module.nets[out].name.as_str().to_owned(),
                        format!(
                            "`{}` drives {} of capacitance, and the strongest \
                             `{name}` is rated for {}",
                            module.nets[out].name,
                            super::fmt_num(load),
                            super::fmt_num(rating)
                        ),
                    );
                }
            }
        }
        if !changed {
            break;
        }
    }
    for (net, why) in overloaded {
        diags.push(
            Diagnostic::warning(why)
                .with_code(OVERLOADED_NET)
                .with_span(module.span)
                .with_note(
                    "place and route has to buffer it; OpenROAD's `repair_design` \
                     does so after placement",
                ),
        );
        report.overloaded.push(net);
    }
    report.overloaded.sort();
    report.overloaded.dedup();
}

/// Points a cell at a different variant of the same library cell,
/// renaming its pins if the variants spell them differently.
fn retarget(module: &mut Module, id: CellId, from: &Gate, to: &str, cells: &StdCells) {
    let Some(target) = cells.gates().gate(to).cloned() else {
        return;
    };
    let cell = &mut module.cells[id];
    cell.kind = CellKind::Blackbox(Name::new(target.name.clone()));
    cell.attrs.set("lib_cell", target.name.as_str());
    for (port, _) in &mut cell.inputs {
        if let Some(at) = from.pins.iter().position(|p| p == port.as_str())
            && let Some(new) = target.pins.get(at)
        {
            *port = Name::new(new.clone());
        }
    }
    for (port, _) in &mut cell.outputs {
        if port.as_str() == from.output {
            *port = Name::new(target.output.clone());
        }
    }
}

/// The capacitance every net drives: the sum over the library pins that
/// read it, plus a flat wire load.
fn net_loads(module: &Module, cells: &StdCells, wire_load: f64) -> BTreeMap<NetId, f64> {
    let mut loads: BTreeMap<NetId, f64> = BTreeMap::new();
    for (_, cell) in module.cells.iter() {
        let CellKind::Blackbox(name) = &cell.kind else {
            continue;
        };
        let electrical = cells.electrical(name.as_str());
        for (port, value) in &cell.inputs {
            let cap = electrical
                .and_then(|e| e.capacitance(port.as_str()))
                .unwrap_or(0.0);
            if cap == 0.0 {
                continue;
            }
            for net in nets_of(module, *value) {
                *loads.entry(net).or_default() += cap;
            }
        }
    }
    for load in loads.values_mut() {
        *load += wire_load;
    }
    loads
}

/// Every net an expression reads.
fn nets_of(module: &Module, id: ExprId) -> Vec<NetId> {
    let mut out = Vec::new();
    collect_nets(module, id, &mut out);
    out
}

fn collect_nets(module: &Module, id: ExprId, out: &mut Vec<NetId>) {
    let Some(expr) = module.exprs.get(id) else {
        return;
    };
    match &expr.kind {
        ExprKind::Net(net) => {
            if !out.contains(net) {
                out.push(*net);
            }
        }
        other => {
            crate::ir::expr::operands(other)
                .into_iter()
                .for_each(|child| collect_nets(module, child, out));
        }
    }
}

// --- the model of a mapped netlist ------------------------------------------

/// Rewrites a mapped module into one whose cells are generic IR cells
/// computing the same functions.
///
/// A mapped netlist is a pile of black boxes: correct, but opaque to the
/// simulator, the bit-blaster and the equivalence checker, all of which
/// need to know what a cell *does*. This turns each one back into the IR
/// cell that computes its function — a [`CellKind::Lut`] holding the
/// library cell's truth table for a gate, a [`CellKind::Dff`] for a
/// flip-flop — so that
///
/// ```text
/// check_equivalent(design, before_mapping, logic_model(after_mapping))
/// ```
///
/// is a proof that mapping did not change the design's behaviour. That
/// check is what the test suite runs over every mapped design.
///
/// The result is a new module named `name`; the input is not modified.
/// Cells the library does not describe are left as they are, with a
/// warning, since nothing better can be said about them.
pub fn logic_model(
    module: &Module,
    cells: &StdCells,
    name: &str,
    diags: &mut Diagnostics,
) -> Module {
    let mut out = module.clone();
    out.name = Name::new(name.to_owned());
    let ids: Vec<CellId> = out.cells.iter().map(|(id, _)| id).collect();
    for id in ids {
        let cell = &out.cells[id];
        let CellKind::Blackbox(lib) = &cell.kind else {
            continue;
        };
        let lib = lib.as_str().to_owned();
        if let Some(gate) = cells.gates().gate(&lib).cloned() {
            model_gate(&mut out, id, &gate, diags);
        } else if let Some(flop) = cells.flops().iter().find(|f| f.name == lib).cloned() {
            model_flop(&mut out, id, &flop, diags);
        } else {
            diags.push(
                Diagnostic::warning(format!(
                    "`{}` is not a cell of `{}`, so its behaviour is unknown",
                    lib,
                    cells.name()
                ))
                .with_span(cell.span),
            );
        }
    }
    out
}

/// One combinational cell as the lookup table of its function.
fn model_gate(module: &mut Module, id: CellId, gate: &Gate, diags: &mut Diagnostics) {
    let cell = &module.cells[id];
    let span = cell.span;
    let Some(out) = cell.output(&gate.output) else {
        diags.push(
            Diagnostic::warning(format!("cell `{}` drives no output", cell.name)).with_span(span),
        );
        return;
    };
    let mut pins = Vec::with_capacity(gate.pins.len());
    for pin in &gate.pins {
        match cell.input(pin) {
            Some(e) => pins.push(e),
            None => {
                diags.push(
                    Diagnostic::warning(format!(
                        "cell `{}` leaves pin `{pin}` unconnected",
                        cell.name
                    ))
                    .with_span(span),
                );
                return;
            }
        }
    }
    // `Concat` is most significant first and a LUT's input 0 is the
    // least significant bit of `a`, so the pins go in reverse.
    let mut parts = pins;
    parts.reverse();
    let a = expr(module, ExprKind::Concat(parts), span);
    let k = u32::try_from(gate.pins.len()).unwrap_or(1);
    let cell = &mut module.cells[id];
    cell.kind = CellKind::Lut {
        k,
        init: truth_to_const(&gate.function, gate.pins.len()),
    };
    cell.inputs = vec![(Name::new("a"), a)];
    cell.outputs = vec![(Name::new("y"), out)];
}

/// One sequential cell as the flip-flop it implements.
fn model_flop(
    module: &mut Module,
    id: CellId,
    flop: &super::library::FlopCell,
    diags: &mut Diagnostics,
) {
    let cell = &module.cells[id];
    let span = cell.span;
    let name = cell.name.as_str().to_owned();
    let (Some(clk), Some(d), Some(q)) = (
        cell.input(&flop.clock_pin),
        cell.input(&flop.data_pin),
        cell.output(&flop.q_pin),
    ) else {
        diags.push(
            Diagnostic::warning(format!("flip-flop `{name}` is not fully wired")).with_span(span),
        );
        return;
    };
    let width = net_width(module, q);
    let driven = |module: &Module, pin: &Option<super::library::FlopControl>| -> Option<ExprId> {
        let control = pin.as_ref()?;
        let e = module.cells[id].input(&control.pin)?;
        // A pin tied to a constant is one the flip-flop does not use.
        match &module.expr(e).kind {
            ExprKind::Const(_) => None,
            _ => Some(e),
        }
    };
    let enable = driven(module, &flop.enable);
    let clear = driven(module, &flop.clear);
    let preset = driven(module, &flop.preset);
    if clear.is_some() && preset.is_some() {
        diags.push(
            Diagnostic::warning(format!(
                "flip-flop `{name}` has both a clear and a preset driven; \
                 the model keeps the clear"
            ))
            .with_span(span),
        );
    }
    let reset = match (clear, preset) {
        (Some(_), _) => flop.clear.as_ref().map(|c| Reset {
            asynchronous: true,
            active_high: c.active_high,
            value: Const::zero(width),
        }),
        (None, Some(_)) => flop.preset.as_ref().map(|c| Reset {
            asynchronous: true,
            active_high: c.active_high,
            value: Const::ones(width),
        }),
        (None, None) => None,
    };
    // A `dff`'s `en` is active high; a cell with an active-low enable
    // needs the signal complemented in the model.
    let enable = match (enable, flop.enable.as_ref()) {
        (Some(e), Some(control)) if !control.active_high => {
            let net = add_net(module, &format!("{name}$en"), Type::bit(), span);
            add_cell(
                module,
                &format!("{name}$eninv"),
                CellKind::Not,
                vec![(Name::new("a"), e)],
                vec![(Name::new("y"), net)],
                span,
            );
            Some(net_expr(module, net, span))
        }
        (other, _) => other,
    };

    let mut inputs = vec![(Name::new("clk"), clk), (Name::new("d"), d)];
    if let Some(en) = enable {
        inputs.push((Name::new("en"), en));
    }
    if let Some(rst) = clear.or(preset) {
        inputs.push((Name::new("rst"), rst));
    }
    let cell = &mut module.cells[id];
    cell.kind = CellKind::Dff {
        clk_pos: flop.clk_pos,
        has_enable: enable.is_some(),
        reset,
    };
    cell.inputs = inputs;
    cell.outputs = vec![(Name::new("q"), q)];
}

/// A truth table as a LUT's `init` constant.
fn truth_to_const(function: &crate::synth::aig::truth::TruthTable, inputs: usize) -> Const {
    let width = 1u32 << inputs;
    let mut init = Const::zero(width);
    for pattern in 0..(1usize << inputs) {
        if function.bit(pattern) {
            init.set_bit(u32::try_from(pattern).expect("pattern"), Bit::One);
        }
    }
    init
}

// --- small IR helpers -------------------------------------------------------

fn net_width(module: &Module, net: NetId) -> u32 {
    module.nets.get(net).and_then(|n| n.ty.width()).unwrap_or(1)
}

fn net_type(module: &Module, net: NetId) -> Type {
    module
        .nets
        .get(net)
        .map_or_else(Type::bit, |n| n.ty.clone())
}

fn expr(module: &mut Module, kind: ExprKind, span: Span) -> ExprId {
    let ty = infer_type(module, &kind).unwrap_or_else(|_| Type::bit());
    module.add_expr(Expr::new(kind, ty, span))
}

fn net_expr(module: &mut Module, net: NetId, span: Span) -> ExprId {
    expr(module, ExprKind::Net(net), span)
}

fn const_expr(module: &mut Module, value: Const, span: Span) -> ExprId {
    expr(module, ExprKind::Const(value), span)
}

/// `base[hi:lo]`, or `base` itself when the slice covers everything.
fn slice_expr(module: &mut Module, base: ExprId, hi: u32, lo: u32, span: Span) -> ExprId {
    let width = module.exprs.get(base).and_then(|e| e.ty.width());
    if lo == 0 && width == Some(hi + 1) {
        return base;
    }
    expr(module, ExprKind::Slice { base, hi, lo }, span)
}

/// A name no net and no cell of the module uses yet.
fn unique_name(module: &Module, base: &str) -> Name {
    if module.net_by_name(base).is_none() && module.cell_by_name(base).is_none() {
        return Name::new(base);
    }
    for suffix in 1u32.. {
        let candidate = format!("{base}${suffix}");
        if module.net_by_name(&candidate).is_none() && module.cell_by_name(&candidate).is_none() {
            return Name::new(candidate);
        }
    }
    unreachable!("a free name exists")
}

fn add_net(module: &mut Module, base: &str, ty: Type, span: Span) -> NetId {
    let name = unique_name(module, base);
    module.nets.push(Net {
        name,
        ty,
        kind: NetKind::Wire,
        attrs: Attrs::new(),
        span,
    })
}

fn add_cell(
    module: &mut Module,
    base: &str,
    kind: CellKind,
    inputs: Vec<(Name, ExprId)>,
    outputs: Vec<(Name, NetId)>,
    span: Span,
) -> CellId {
    let name = unique_name(module, base);
    module.cells.push(Cell {
        name,
        kind,
        inputs,
        outputs,
        params: Attrs::new(),
        attrs: Attrs::new(),
        span,
    })
}

fn add_assign(module: &mut Module, target: NetId, value: ExprId, span: Span) {
    module.assigns.push(Assign {
        target: Lvalue::Net(target),
        value,
        delay: None,
        attrs: Attrs::new(),
        span,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::validate::validate;
    use crate::source::SourceMap;

    /// Enough of a library to map onto: an inverter, a NAND2, a NOR2
    /// and one flip-flop with an active-low asynchronous reset.
    const LIB: &str = r#"
library (tiny) {
  time_unit : "1ns";
  capacitive_load_unit (1.0, pf);
  default_max_transition : 1.0;
  cell (INV) {
    area : 1.0;
    pin (A) { direction : input; capacitance : 0.002; }
    pin (Y) { direction : output; function : "!A"; max_capacitance : 0.05; }
  }
  cell (NAND2) {
    area : 2.0;
    pin (A) { direction : input; capacitance : 0.002; }
    pin (B) { direction : input; capacitance : 0.002; }
    pin (ZN) { direction : output; function : "!(A B)"; max_capacitance : 0.05; }
  }
  cell (NOR2) {
    area : 2.0;
    pin (A) { direction : input; capacitance : 0.002; }
    pin (B) { direction : input; capacitance : 0.002; }
    pin (ZN) { direction : output; function : "!(A + B)"; max_capacitance : 0.05; }
  }
  cell (DFFR) {
    area : 10.0;
    ff (IQ, IQ_N) { next_state : "D"; clocked_on : "CLK"; clear : "!RN"; }
    pin (CLK) { direction : input; clock : true; capacitance : 0.003; }
    pin (D) { direction : input; capacitance : 0.002; }
    pin (RN) { direction : input; capacitance : 0.002; }
    pin (Q) { direction : output; function : "IQ"; max_capacitance : 0.05; }
  }
}
"#;

    /// A one-bit design with an active-high asynchronous reset, which
    /// the library has no pin for.
    const RTL: &str = "\
top t

module t
  net %clk u1 wire
  net %rst u1 wire
  net %a u1 wire
  net %b u1 wire
  net %q u1 wire
  net %d u1 wire
  port clk in %clk
  port rst in %rst
  port a in %a
  port b in %b
  port q out %q
  cell u_x xor (a=%a, b=%b) -> (y=%d)
  cell u_ff dff pos arst pos 1'd0 (clk=%clk, d=%d, rst=%rst) -> (q=%q)
end
";

    fn library() -> Library {
        let mut sources = SourceMap::new();
        let file = sources.add("t.lib", LIB.to_string()).unwrap();
        let mut diags = Diagnostics::new();
        let library = Library::parse(LIB, file, &mut diags).expect("a library");
        assert!(!diags.has_errors(), "{}", diags.render(&sources));
        library
    }

    fn design_of(text: &str) -> (Design, ModuleId) {
        let mut sources = SourceMap::new();
        let file = sources.add("t.rtl", text.to_string()).unwrap();
        let design =
            Design::parse_text(text, file).unwrap_or_else(|d| panic!("{}", d.render(&sources)));
        let top = design.top.expect("a top module");
        (design, top)
    }

    fn design() -> (Design, ModuleId) {
        design_of(RTL)
    }

    #[test]
    fn the_flow_maps_a_design_onto_a_library() {
        let (mut design, top) = design();
        let library = library();
        let mut diags = Diagnostics::new();
        let report = synthesize_asic(&mut design, top, &library, &AsicOptions::new(), &mut diags)
            .expect("the flow runs");
        assert!(!diags.has_errors(), "{}", report.to_text());
        assert!(!validate(&design).has_errors());
        assert!(report.is_fully_mapped(), "{}", report.to_text());
        // The active-high reset had to be inverted, since the library
        // only has an active-low pin.
        assert_eq!(report.inverters_inserted, 1);
        assert_eq!(report.count("DFFR"), 1);
        assert_eq!(report.flop_cells, 1);
        assert!(report.area > 10.0);
        assert!(report.to_text().contains("inverter(s) inserted"));
        // The output pin keeps the library's own name.
        let module = design.module(top);
        let ff = module
            .cell_by_name("u_ff")
            .map(|id| &module.cells[id])
            .expect("the flip-flop");
        assert!(ff.output("Q").is_some(), "{:?}", ff.outputs);
        assert_eq!(
            ff.attrs.get("lib_cell").and_then(|v| v.as_str()),
            Some("DFFR")
        );
        for (_, cell) in module.cells.iter() {
            assert!(
                matches!(cell.kind, CellKind::Blackbox(_)),
                "`{}` is still generic",
                cell.name
            );
        }
    }

    #[test]
    fn the_model_of_a_mapped_netlist_is_generic_cells() {
        let (mut design, top) = design();
        let library = library();
        let mut diags = Diagnostics::new();
        let _ = synthesize_asic(&mut design, top, &library, &AsicOptions::new(), &mut diags)
            .expect("the flow runs");
        let cells = StdCells::from_library(&library, &LibraryOptions::default());
        let mut diags = Diagnostics::new();
        let model = logic_model(design.module(top), &cells, "t$model", &mut diags);
        assert!(!diags.has_errors());
        assert_eq!(model.name.as_str(), "t$model");
        let mut luts = 0;
        let mut flops = 0;
        for (_, cell) in model.cells.iter() {
            match &cell.kind {
                CellKind::Lut { .. } => luts += 1,
                CellKind::Dff { reset, .. } => {
                    // The library's pin is active low, and so is the
                    // model's reset; the inverter in front of it is a
                    // cell of its own.
                    let reset = reset.as_ref().expect("a reset");
                    assert!(reset.asynchronous);
                    assert!(!reset.active_high);
                    flops += 1;
                }
                other => panic!("`{}` is a {other:?}", cell.name),
            }
        }
        assert_eq!(flops, 1);
        assert!(luts >= 2, "{luts} LUTs");
        let mut combined = design.clone();
        combined.add_module(model);
        assert!(!validate(&combined).has_errors());
    }

    #[test]
    fn a_library_with_nothing_to_map_onto_is_refused() {
        let (mut design, top) = design();
        let mut sources = SourceMap::new();
        let text = "library (empty) { cell (FILL) { area : 1.0; } }";
        let file = sources.add("e.lib", text.to_string()).unwrap();
        let mut diags = Diagnostics::new();
        let empty = Library::parse(text, file, &mut diags).expect("a library");
        assert_eq!(
            synthesize_asic(&mut design, top, &empty, &AsicOptions::new(), &mut diags).unwrap_err(),
            AsicError::UnusableLibrary(
                "`empty` has no combinational cell the mapper can use".to_owned()
            )
        );

        // A library with gates but no inverter is refused too: the
        // mapper needs one to complement a cut.
        let text = r#"
library (no_inverter) {
  cell (AND2) {
    area : 2.0;
    pin (A) { direction : input; }
    pin (B) { direction : input; }
    pin (Y) { direction : output; function : "A B"; }
  }
}
"#;
        let file = sources.add("n.lib", text.to_string()).unwrap();
        let none = Library::parse(text, file, &mut diags).expect("a library");
        assert!(matches!(
            synthesize_asic(&mut design, top, &none, &AsicOptions::new(), &mut diags),
            Err(AsicError::UnusableLibrary(_))
        ));
        for error in [
            AsicError::NoSuchModule,
            AsicError::Synthesis,
            AsicError::UnusableLibrary("why".to_owned()),
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn a_module_of_another_design_is_refused() {
        let (mut other, other_top) =
            design_of("top u\n\nmodule u\n  net %a u1 wire\n  port a in %a\nend\n");
        other.modules.retain(|id, _| id != other_top);
        let (mut design, top) = design();
        let library = library();
        let mut diags = Diagnostics::new();
        assert_eq!(
            synthesize_asic(&mut other, top, &library, &AsicOptions::new(), &mut diags)
                .unwrap_err(),
            AsicError::NoSuchModule
        );
        // Turning the optional passes off is not an error, it is a note.
        let report = synthesize_asic(
            &mut design,
            top,
            &library,
            &AsicOptions {
                resize: false,
                timing: false,
                ..AsicOptions::default()
            },
            &mut diags,
        )
        .expect("the flow runs");
        assert!(report.timing.is_none());
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("drive-strength pass was not run")),
            "{:?}",
            report.notes
        );
    }

    /// A flip-flop the library has no cell for is left generic and
    /// reported, rather than approximated.
    #[test]
    fn a_flip_flop_the_library_cannot_build_is_reported() {
        // The library has only a positive-edge flip-flop.
        let (mut design, top) = design_of(
            "top t\n\nmodule t\n  net %clk u1 wire\n  net %d u1 wire\n  net %q u1 wire\n  \
             port clk in %clk\n  port d in %d\n  port q out %q\n  \
             cell u_ff dff neg (clk=%clk, d=%d) -> (q=%q)\nend\n",
        );
        let library = library();
        let mut diags = Diagnostics::new();
        let report = synthesize_asic(&mut design, top, &library, &AsicOptions::new(), &mut diags)
            .expect("the flow runs");
        assert!(diags.has_errors());
        assert_eq!(diags.iter().next().and_then(|d| d.code), Some(NO_FLOP_CELL));
        assert!(!report.is_fully_mapped());
        assert_eq!(report.unmapped.len(), 1);
        assert!(report.to_text().contains("unmapped: u_ff"));
    }
}
