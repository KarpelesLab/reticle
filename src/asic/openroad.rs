//! The hand-off to OpenROAD: the files it needs and the command line to
//! run it with.
//!
//! Reticle runs no other program. [`export_openroad`] returns the exact
//! bytes OpenROAD expects and the arguments it should be run with; the
//! CLI, a build script or a person does the running. That keeps the
//! library sans-I/O and makes the hand-off testable without OpenROAD
//! installed — a golden test compares the Tcl and the netlist byte for
//! byte, and [`check_physical`] says whether the netlist is one the tool
//! can place before it sees it.
//!
//! # What comes out
//!
//! | File | What it is |
//! |------|------------|
//! | `<top>.v` | the gate-level netlist, structural Verilog |
//! | `<top>.sdc` | the constraints ([`super::sdc`]) |
//! | `<top>.tcl` | the script below |
//! | `<top>.def` | the unplaced DEF, when a floorplan is given |
//!
//! The LEF and the Liberty are *not* produced: they are the PDK's, and
//! the script reads them from the paths
//! [`OpenRoadOptions::lef_files`] and [`OpenRoadOptions::liberty_files`]
//! name.
//!
//! # Which OpenROAD
//!
//! The script targets the **OpenROAD app** — the `openroad` binary of
//! the OpenROAD project — using the command set of the 2.0 series, which
//! is what OpenROAD-flow-scripts has driven since 2023 and what the
//! `openroad` package of a current distribution installs. Every command
//! it uses:
//!
//! | Stage | Commands |
//! |-------|----------|
//! | read | `read_lef`, `read_liberty`, `read_verilog`, `link_design`, `read_sdc` |
//! | floorplan | `initialize_floorplan`, `make_tracks`, `place_pins` |
//! | placement | `set_wire_rc`, `global_placement`, `estimate_parasitics`, `repair_design`, `detailed_placement`, `check_placement` |
//! | clock tree | `clock_tree_synthesis`, `set_propagated_clock`, `repair_clock_nets` |
//! | routing | `set_routing_layers`, `global_route`, `detailed_route` |
//! | finishing | `filler_placement`, `check_placement` |
//! | reports | `report_design_area`, `report_checks`, `report_worst_slack`, `report_tns`, `report_clock_skew` |
//! | write | `write_def`, `write_verilog` |
//!
//! They are in the order the flow needs them, which is the part that is
//! easy to get wrong: parasitics are estimated *after* placement and
//! again after global routing, because the numbers before placement are
//! wire-load guesses; `repair_design` runs after the first estimate so
//! it fixes real violations; the clock tree is built after placement and
//! before routing, and `set_propagated_clock` follows it so the timing
//! reports stop pretending the clock is ideal; `detailed_route` runs
//! last, after `filler_placement` would disturb nothing.
//!
//! A DEF is written only when [`OpenRoadOptions::floorplan`] is set,
//! since an unplaced DEF with no die area is of no use to anyone; when
//! one is given the script can skip `initialize_floorplan` and read the
//! DEF instead ([`OpenRoadOptions::read_def`]), which is how a
//! hand-drawn floorplan or a block with macros gets in.

use std::collections::BTreeSet;

use super::def::{self, DefOptions, Rect};
use super::flow::AsicError;
use super::fmt_num;
use super::lef::Lef;
use super::liberty::Library;
use super::sdc::AsicConstraints;
use crate::diag::Diagnostics;
use crate::ir::emit::{VerilogOptions, emit_verilog_with};
use crate::ir::{CellKind, Design, ModuleId};

/// A rectangle in microns, as a floorplan is written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    /// Left edge.
    pub x1: f64,
    /// Bottom edge.
    pub y1: f64,
    /// Right edge.
    pub x2: f64,
    /// Top edge.
    pub y2: f64,
}

impl Area {
    /// A rectangle from its corners.
    pub fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Area {
        Area { x1, y1, x2, y2 }
    }

    /// The `{x1 y1 x2 y2}` list an OpenROAD `-die_area` takes.
    pub fn tcl(&self) -> String {
        format!(
            "{} {} {} {}",
            microns(self.x1),
            microns(self.y1),
            microns(self.x2),
            microns(self.y2)
        )
    }

    /// The same rectangle in DEF database units.
    pub fn to_rect(self, units: i64) -> Rect {
        let scale = |v: f64| {
            #[allow(clippy::cast_possible_truncation)]
            {
                (v * units as f64).round() as i64
            }
        };
        Rect {
            x1: scale(self.x1),
            y1: scale(self.y1),
            x2: scale(self.x2),
            y2: scale(self.y2),
        }
    }
}

/// A micron coordinate as the script writes it: at most four decimals,
/// which is ten times finer than any manufacturing grid, with trailing
/// zeros trimmed.
///
/// Unlike [`crate::asic::fmt_num`] this rounds rather than falling back
/// to the exactly round-tripping form, because a die edge computed as
/// `30.0 - 2.76` is `27.240000000000002` and no floorplan wants to read
/// that.
fn microns(v: f64) -> String {
    let mut s = format!("{v:.4}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { "0".to_string() } else { s }
}

/// A floorplan: the die, the core the rows are built in, and the site
/// the rows are made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Floorplan {
    /// The die area.
    pub die: Area,
    /// The core area, inside the die, leaving room for the IO ring.
    pub core: Area,
}

impl Floorplan {
    /// A square die of `side` microns with a `margin` of core offset.
    pub fn square(side: f64, margin: f64) -> Floorplan {
        Floorplan {
            die: Area::new(0.0, 0.0, side, side),
            core: Area::new(margin, margin, side - margin, side - margin),
        }
    }
}

/// Knobs for [`export_openroad`].
#[derive(Clone, Debug)]
pub struct OpenRoadOptions {
    /// The file names the script reads the Liberty libraries from, in
    /// the order they are read.
    pub liberty_files: Vec<String>,
    /// The file names the script reads LEF from: the technology LEF
    /// first, then the cell LEFs.
    pub lef_files: Vec<String>,
    /// The floorplan. Without one the script falls back to
    /// [`OpenRoadOptions::utilization`].
    pub floorplan: Option<Floorplan>,
    /// Read the generated DEF instead of calling `initialize_floorplan`.
    /// Needs a floorplan, since that is what the DEF carries.
    pub read_def: bool,
    /// Core utilisation for `initialize_floorplan -utilization` when no
    /// floorplan is given.
    pub utilization: f64,
    /// The site rows are built on; the first `CORE` site of the LEF when
    /// `None`.
    pub site: Option<String>,
    /// Target density for `global_placement`.
    pub place_density: f64,
    /// Layers `place_pins` may use, as (horizontal, vertical).
    pub pin_layers: Option<(String, String)>,
    /// The routing layer range, as (lowest, highest).
    pub routing_layers: Option<(String, String)>,
    /// The layer whose RC `set_wire_rc` uses; the topmost routing layer
    /// of the LEF when `None`.
    pub wire_rc_layer: Option<String>,
    /// The buffers clock tree synthesis may use. Empty leaves the choice
    /// to OpenROAD.
    pub clock_buffers: Vec<String>,
    /// The filler cells placed at the end. Empty skips
    /// `filler_placement`.
    pub fillers: Vec<String>,
    /// DEF database units per micron.
    pub def_units: i64,
}

impl Default for OpenRoadOptions {
    fn default() -> Self {
        OpenRoadOptions::new()
    }
}

impl OpenRoadOptions {
    /// The defaults: no floorplan (so 60% utilisation), 60% placement
    /// density, 1000 DEF units per micron, and every layer and cell
    /// choice left to OpenROAD.
    pub fn new() -> OpenRoadOptions {
        OpenRoadOptions {
            liberty_files: Vec::new(),
            lef_files: Vec::new(),
            floorplan: None,
            read_def: false,
            utilization: 0.6,
            site: None,
            place_density: 0.6,
            pin_layers: None,
            routing_layers: None,
            wire_rc_layer: None,
            clock_buffers: Vec::new(),
            fillers: Vec::new(),
            def_units: 1000,
        }
    }

    /// The same options with a floorplan.
    pub fn with_floorplan(mut self, floorplan: Floorplan) -> OpenRoadOptions {
        self.floorplan = Some(floorplan);
        self
    }
}

/// Everything OpenROAD needs for one run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRoadInputs {
    /// The design name, which every generated file is named after.
    pub design: String,
    /// The gate-level netlist as structural Verilog.
    pub verilog: String,
    /// The constraints as SDC.
    pub sdc: String,
    /// The generated Tcl script.
    pub tcl: String,
    /// The unplaced DEF, when a floorplan was given.
    pub def: Option<String>,
    /// The files to write, as `(name, contents)`, in a fixed order. The
    /// names are the ones the Tcl and [`OpenRoadInputs::args`] refer to;
    /// write them next to each other and run the command.
    pub files: Vec<(String, String)>,
    /// The command line, program first.
    pub args: Vec<String>,
    /// What [`check_physical`] found, empty when the netlist is clean.
    pub problems: Vec<String>,
}

impl OpenRoadInputs {
    /// The contents of one generated file.
    pub fn file(&self, name: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, text)| text.as_str())
    }
}

/// Builds the files and the command line for one OpenROAD run.
///
/// # Errors
///
/// [`AsicError::NoSuchModule`] when the id does not belong to the
/// design, and [`AsicError::Emit`] when the netlist holds something
/// structural Verilog cannot express (a process, most likely, which
/// means the design was not synthesised).
pub fn export_openroad(
    design: &Design,
    module: ModuleId,
    library: &Library,
    lef: &Lef,
    constraints: &AsicConstraints,
    options: &OpenRoadOptions,
) -> Result<OpenRoadInputs, AsicError> {
    if design.modules.get(module).is_none() {
        return Err(AsicError::NoSuchModule);
    }
    let top = design.module(module).name.as_str().to_owned();

    let mut exported = design.clone();
    exported.top = Some(module);
    let verilog = emit_verilog_with(
        &exported,
        &VerilogOptions {
            structural_only: true,
            ansi_ports: true,
            // The `lib_cell` attribute is Reticle's own bookkeeping and
            // says nothing a Verilog reader wants; the instance already
            // names the cell.
            keep_attrs: false,
            blackboxes: false,
        },
    )?;
    let sdc = constraints.write_sdc(&top);

    let def = options.floorplan.as_ref().map(|plan| {
        let mut diags = Diagnostics::new();
        let def = def::from_netlist(
            design,
            module,
            &DefOptions {
                design: Some(top.clone()),
                units: options.def_units,
                die_area: Some(plan.die.to_rect(options.def_units)),
                ..DefOptions::default()
            },
            &mut diags,
        );
        def::write_def(&def)
    });

    let tcl = write_tcl(&top, lef, constraints, options, def.is_some());
    let mut files = vec![
        (format!("{top}.v"), verilog.clone()),
        (format!("{top}.sdc"), sdc.clone()),
        (format!("{top}.tcl"), tcl.clone()),
    ];
    if let Some(def) = &def {
        files.push((format!("{top}.def"), def.clone()));
    }
    Ok(OpenRoadInputs {
        design: top.clone(),
        verilog,
        sdc,
        tcl,
        def,
        files,
        args: vec![
            "openroad".to_owned(),
            "-no_init".to_owned(),
            "-exit".to_owned(),
            format!("{top}.tcl"),
        ],
        problems: check_physical(design, module, library, lef),
    })
}

/// Checks that the netlist is one OpenROAD can place and route.
///
/// This is the last gate before the tool sees the design, and it checks
/// what the tool checks:
///
/// - every cell is a black box naming a library cell, so nothing generic
///   (`$and`, `$dff`, `$lut`) is left;
/// - every such cell has a `cell` in the Liberty, which the timing needs;
/// - and a `MACRO` in the LEF, which the placement needs;
/// - every connected pin is a pin that macro has.
///
/// The result is a list of problems in a fixed order: the checks run in
/// the order above, each walking the netlist in cell order.
pub fn check_physical(
    design: &Design,
    module: ModuleId,
    library: &Library,
    lef: &Lef,
) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(m) = design.modules.get(module) else {
        return vec!["the module is not part of the design".to_owned()];
    };
    if !m.processes.is_empty() {
        problems.push(format!(
            "`{}` still holds {} process(es); it was not synthesised",
            m.name,
            m.processes.len()
        ));
    }
    let mut generic = BTreeSet::new();
    let mut missing_liberty = BTreeSet::new();
    let mut missing_macro = BTreeSet::new();
    let mut bad_pins = Vec::new();
    for (_, cell) in m.cells.iter() {
        let CellKind::Blackbox(name) = &cell.kind else {
            generic.insert(format!("${}", cell.kind.keyword()));
            continue;
        };
        let name = name.as_str();
        if library.cell(name).is_none() {
            missing_liberty.insert(name.to_owned());
        }
        match lef.macro_(name) {
            None => {
                missing_macro.insert(name.to_owned());
            }
            Some(mac) => {
                for (port, _) in &cell.inputs {
                    if !mac.pins.iter().any(|p| p.name == port.as_str()) {
                        bad_pins.push(format!("`{}`: `{name}` has no pin `{port}`", cell.name));
                    }
                }
                for (port, _) in &cell.outputs {
                    if !mac.pins.iter().any(|p| p.name == port.as_str()) {
                        bad_pins.push(format!("`{}`: `{name}` has no pin `{port}`", cell.name));
                    }
                }
            }
        }
    }
    for kind in generic {
        problems.push(format!(
            "the netlist holds generic `{kind}` cells, which OpenROAD cannot place"
        ));
    }
    for name in missing_liberty {
        problems.push(format!("`{name}` is not a cell of the Liberty library"));
    }
    for name in missing_macro {
        problems.push(format!("`{name}` has no `MACRO` in the LEF"));
    }
    problems.extend(bad_pins);
    problems
}

/// The site rows are built on: the one asked for, else the first `CORE`
/// site of the LEF.
fn site_of<'a>(lef: &'a Lef, options: &'a OpenRoadOptions) -> Option<&'a str> {
    if let Some(site) = &options.site {
        return Some(site.as_str());
    }
    lef.sites
        .iter()
        .find(|s| s.class.as_deref() == Some("CORE"))
        .map(|s| s.name.as_str())
}

/// The layers pins are placed on: the lowest horizontal routing layer
/// and the lowest vertical one, which is what a pin placer needs — a
/// pair of layers it can reach in both directions.
fn pin_layers(lef: &Lef) -> Option<(String, String)> {
    use super::lef::Direction;
    let lowest = |want: Direction| {
        lef.routing_layers()
            .find(|l| l.direction == Some(want))
            .map(|l| l.name.clone())
    };
    Some((lowest(Direction::Horizontal)?, lowest(Direction::Vertical)?))
}

/// The routing layers to use: the ones asked for, else the bottom and
/// top routing layers of the LEF.
fn routing_layers(lef: &Lef, options: &OpenRoadOptions) -> Option<(String, String)> {
    if let Some(layers) = &options.routing_layers {
        return Some(layers.clone());
    }
    let mut routing = lef.routing_layers();
    let first = routing.next()?.name.clone();
    let last = lef
        .routing_layers()
        .last()
        .map_or_else(|| first.clone(), |l| l.name.clone());
    Some((first, last))
}

/// Writes the OpenROAD script; see the module docs for the command set
/// and the order.
fn write_tcl(
    top: &str,
    lef: &Lef,
    constraints: &AsicConstraints,
    options: &OpenRoadOptions,
    have_def: bool,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# OpenROAD script for `{top}`, generated by Reticle.\n\
         #\n\
         # Targets the OpenROAD app (2.0 series command set). Run it from the\n\
         # directory holding the generated files:\n\
         #\n\
         #   openroad -no_init -exit {top}.tcl\n\
         #\n\
         # The LEF and Liberty paths below are the ones the caller named; they\n\
         # come from the PDK, not from Reticle."
    );
    out.push('\n');
    if options.lef_files.is_empty() {
        out.push_str("# no LEF was named; add `read_lef <tech.lef>` here\n");
    }
    for file in &options.lef_files {
        let _ = writeln!(out, "read_lef {file}");
    }
    if options.liberty_files.is_empty() {
        out.push_str("# no Liberty was named; add `read_liberty <cells.lib>` here\n");
    }
    for file in &options.liberty_files {
        let _ = writeln!(out, "read_liberty {file}");
    }
    let _ = writeln!(out, "read_verilog {top}.v");
    let _ = writeln!(out, "link_design {top}");
    let _ = writeln!(out, "read_sdc {top}.sdc");

    out.push_str("\n# --- floorplan ---\n");
    let site = site_of(lef, options);
    match (&options.floorplan, options.read_def && have_def) {
        (Some(_), true) => {
            let _ = writeln!(out, "read_def -floorplan_initialize {top}.def");
        }
        (Some(plan), false) => {
            let _ = write!(
                out,
                "initialize_floorplan -die_area {{{}}} -core_area {{{}}}",
                plan.die.tcl(),
                plan.core.tcl()
            );
            if let Some(site) = site {
                let _ = write!(out, " -site {site}");
            }
            out.push('\n');
        }
        (None, _) => {
            let _ = write!(
                out,
                "initialize_floorplan -utilization {} -aspect_ratio 1.0",
                fmt_num(options.utilization * 100.0)
            );
            if let Some(site) = site {
                let _ = write!(out, " -site {site}");
            }
            out.push('\n');
        }
    }
    out.push_str("make_tracks\n");
    match options.pin_layers.clone().or_else(|| pin_layers(lef)) {
        Some((hor, ver)) => {
            let _ = writeln!(out, "place_pins -hor_layers {hor} -ver_layers {ver}");
        }
        None => out.push_str(
            "# the LEF declares no horizontal and vertical routing pair; add \
             `place_pins -hor_layers ... -ver_layers ...`\n",
        ),
    }

    out.push_str("\n# --- placement ---\n");
    let wire_layer = options
        .wire_rc_layer
        .clone()
        .or_else(|| routing_layers(lef, options).map(|(_, high)| high));
    if let Some(layer) = &wire_layer {
        let _ = writeln!(out, "set_wire_rc -layer {layer}");
    }
    let _ = writeln!(
        out,
        "global_placement -density {}",
        fmt_num(options.place_density)
    );
    out.push_str(
        "estimate_parasitics -placement\n\
         repair_design\n\
         detailed_placement\n\
         check_placement -verbose\n",
    );

    out.push_str("\n# --- clock tree ---\n");
    if constraints.clocks.is_empty() {
        out.push_str("# no `create_clock` in the SDC, so there is no clock tree to build\n");
    } else {
        if options.clock_buffers.is_empty() {
            out.push_str("clock_tree_synthesis -sink_clustering_enable\n");
        } else {
            let _ = writeln!(
                out,
                "clock_tree_synthesis -buf_list {{{}}} -root_buf {} -sink_clustering_enable",
                options.clock_buffers.join(" "),
                options.clock_buffers[0]
            );
        }
        out.push_str(
            "set_propagated_clock [all_clocks]\n\
             estimate_parasitics -placement\n\
             repair_clock_nets\n\
             detailed_placement\n",
        );
    }

    out.push_str("\n# --- routing ---\n");
    if let Some((low, high)) = routing_layers(lef, options) {
        let _ = writeln!(out, "set_routing_layers -signal {low}-{high}");
    }
    out.push_str("global_route -congestion_iterations 30\n");
    out.push_str("estimate_parasitics -global_routing\n");
    let _ = writeln!(out, "detailed_route -output_drc {top}.drc.rpt -verbose 0");

    out.push_str("\n# --- finishing ---\n");
    if options.fillers.is_empty() {
        out.push_str("# no filler cells were named, so no `filler_placement`\n");
    } else {
        let _ = writeln!(out, "filler_placement {{{}}}", options.fillers.join(" "));
    }
    out.push_str("check_placement\n");

    out.push_str("\n# --- reports ---\n");
    out.push_str(
        "report_design_area\n\
         report_checks -path_delay max -format full_clock_expanded\n\
         report_checks -path_delay min\n\
         report_worst_slack -max\n\
         report_tns\n",
    );
    if !constraints.clocks.is_empty() {
        out.push_str("report_clock_skew\n");
    }

    out.push_str("\n# --- results ---\n");
    let _ = writeln!(out, "write_def {top}.routed.def");
    let _ = writeln!(out, "write_verilog {top}.routed.v");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asic::lef::parse_lef;
    use crate::source::SourceMap;

    const LEF: &str = "\
VERSION 5.8 ;
UNITS
  DATABASE MICRONS 1000 ;
END UNITS
LAYER li1
  TYPE ROUTING ;
  DIRECTION VERTICAL ;
  PITCH 0.46 ;
END li1
LAYER met1
  TYPE ROUTING ;
  DIRECTION HORIZONTAL ;
  PITCH 0.34 ;
END met1
SITE core
  CLASS CORE ;
  SIZE 0.46 BY 2.72 ;
END core
MACRO INV_X1
  CLASS CORE ;
  SIZE 1.38 BY 2.72 ;
  SITE core ;
  PIN A
    DIRECTION INPUT ;
  END A
  PIN Y
    DIRECTION OUTPUT ;
  END Y
END INV_X1
END LIBRARY
";

    const LIB: &str = r#"
library (tiny) {
  cell (INV_X1) {
    area : 1.0;
    pin (A) { direction : input; }
    pin (Y) { direction : output; function : "!A"; }
  }
}
"#;

    const RTL: &str = "\
top top

module top
  net %a u1 wire
  net %y u1 wire
  port a in %a
  port y out %y
  attr lib_cell = \"INV_X1\"
  cell u_inv blackbox INV_X1 (A=%a) -> (Y=%y)
end
";

    fn parse_design(text: &str) -> (Design, ModuleId) {
        let mut sources = SourceMap::new();
        let file = sources.add("t.rtl", text.to_string()).unwrap();
        let design =
            Design::parse_text(text, file).unwrap_or_else(|d| panic!("{}", d.render(&sources)));
        let top = design.top.expect("a top module");
        (design, top)
    }

    fn physical() -> (Library, Lef) {
        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let lef_file = sources.add("t.lef", LEF.to_string()).unwrap();
        let lef = parse_lef(LEF, lef_file, &mut diags);
        let lib_file = sources.add("t.lib", LIB.to_string()).unwrap();
        let library = Library::parse(LIB, lib_file, &mut diags).unwrap();
        assert!(!diags.has_errors(), "{}", diags.render(&sources));
        (library, lef)
    }

    fn options() -> OpenRoadOptions {
        OpenRoadOptions {
            liberty_files: vec!["cells.lib".to_owned()],
            lef_files: vec!["tech.lef".to_owned(), "cells.lef".to_owned()],
            clock_buffers: vec!["BUF_X2".to_owned()],
            fillers: vec!["FILL_X1".to_owned()],
            ..OpenRoadOptions::new()
        }
    }

    #[test]
    fn the_script_reads_everything_and_runs_the_stages_in_order() {
        let (design, top) = parse_design(RTL);
        let (library, lef) = physical();
        let constraints =
            AsicConstraints::new().with_clock(super::super::sdc::Clock::new("sys", "a", 10.0));
        let inputs =
            export_openroad(&design, top, &library, &lef, &constraints, &options()).unwrap();
        assert_eq!(inputs.design, "top");
        assert_eq!(inputs.args, ["openroad", "-no_init", "-exit", "top.tcl"]);
        assert!(inputs.def.is_none());
        assert_eq!(inputs.files.len(), 3);
        assert!(inputs.file("top.v").unwrap().contains("INV_X1 u_inv"));
        assert!(inputs.file("top.sdc").unwrap().contains("create_clock"));
        assert!(inputs.file("nope").is_none());
        assert!(inputs.problems.is_empty(), "{:?}", inputs.problems);

        let tcl = &inputs.tcl;
        let at = |needle: &str| {
            tcl.find(needle)
                .unwrap_or_else(|| panic!("`{needle}` is missing from\n{tcl}"))
        };
        // The order of the stages is the point of the script.
        let order = [
            "read_lef tech.lef",
            "read_liberty cells.lib",
            "read_verilog top.v",
            "link_design top",
            "read_sdc top.sdc",
            "initialize_floorplan -utilization 60",
            "make_tracks",
            "place_pins",
            "set_wire_rc -layer met1",
            "global_placement -density 0.6",
            "estimate_parasitics -placement",
            "repair_design",
            "detailed_placement",
            "clock_tree_synthesis -buf_list {BUF_X2} -root_buf BUF_X2",
            "set_propagated_clock [all_clocks]",
            "repair_clock_nets",
            "set_routing_layers -signal li1-met1",
            "global_route",
            "estimate_parasitics -global_routing",
            "detailed_route -output_drc top.drc.rpt",
            "filler_placement {FILL_X1}",
            "report_design_area",
            "report_checks -path_delay max",
            "write_def top.routed.def",
            "write_verilog top.routed.v",
        ];
        let mut last = 0;
        for needle in order {
            let found = at(needle);
            assert!(found >= last, "`{needle}` is out of order in\n{tcl}");
            last = found;
        }
    }

    #[test]
    fn a_floorplan_adds_a_def_and_an_explicit_die_area() {
        let (design, top) = parse_design(RTL);
        let (library, lef) = physical();
        let options = OpenRoadOptions {
            floorplan: Some(Floorplan::square(20.0, 2.0)),
            ..options()
        };
        let inputs = export_openroad(
            &design,
            top,
            &library,
            &lef,
            &AsicConstraints::new(),
            &options,
        )
        .unwrap();
        assert!(
            inputs.tcl.contains(
                "initialize_floorplan -die_area {0 0 20 20} -core_area {2 2 18 18} -site core"
            ),
            "{}",
            inputs.tcl
        );
        let def = inputs.def.as_ref().expect("a DEF");
        assert!(def.contains("DIEAREA ( 0 0 ) ( 20000 20000 ) ;"), "{def}");
        assert!(def.contains("COMPONENTS 1 ;"), "{def}");
        assert_eq!(inputs.files.len(), 4);
        assert!(inputs.file("top.def").is_some());
        // No clock, so no clock tree and no skew report.
        assert!(!inputs.tcl.contains("clock_tree_synthesis"));
        assert!(!inputs.tcl.contains("report_clock_skew"));

        // And the DEF can be read back as the floorplan instead.
        let options = OpenRoadOptions {
            read_def: true,
            ..options
        };
        let inputs = export_openroad(
            &design,
            top,
            &library,
            &lef,
            &AsicConstraints::new(),
            &options,
        )
        .unwrap();
        assert!(
            inputs
                .tcl
                .contains("read_def -floorplan_initialize top.def"),
            "{}",
            inputs.tcl
        );
    }

    #[test]
    fn a_netlist_the_tool_cannot_place_is_reported() {
        const BAD: &str = "\
top top

module top
  net %a u1 wire
  net %y u1 wire
  net %t u1 wire
  port a in %a
  port y out %y
  cell u_and and (a=%a, b=%a) -> (y=%t)
  cell u_odd blackbox NOPE_X1 (A=%t) -> (Z=%y)
  cell u_bad blackbox INV_X1 (NOPIN=%t) -> (Y=%y)
end
";
        let (design, top) = parse_design(BAD);
        let (library, lef) = physical();
        let problems = check_physical(&design, top, &library, &lef);
        assert_eq!(
            problems,
            [
                "the netlist holds generic `$and` cells, which OpenROAD cannot place",
                "`NOPE_X1` is not a cell of the Liberty library",
                "`NOPE_X1` has no `MACRO` in the LEF",
                "`u_bad`: `INV_X1` has no pin `NOPIN`",
            ]
        );
        // The export still produces files; the problems ride along.
        let inputs = export_openroad(
            &design,
            top,
            &library,
            &lef,
            &AsicConstraints::new(),
            &OpenRoadOptions::new(),
        )
        .unwrap();
        assert_eq!(inputs.problems.len(), 4);
        assert!(inputs.tcl.contains("# no LEF was named"), "{}", inputs.tcl);
        assert!(inputs.tcl.contains("# no Liberty was named"));
        assert!(inputs.tcl.contains("# no filler cells were named"));
    }

    #[test]
    fn a_module_of_another_design_is_an_error() {
        let (design, _) = parse_design(RTL);
        let (other, top) = parse_design(RTL);
        let _ = other;
        let mut design = design;
        design.modules.retain(|id, _| id != top);
        let (library, lef) = physical();
        assert_eq!(
            export_openroad(
                &design,
                top,
                &library,
                &lef,
                &AsicConstraints::new(),
                &OpenRoadOptions::new()
            )
            .unwrap_err(),
            AsicError::NoSuchModule
        );
        assert_eq!(check_physical(&design, top, &library, &lef).len(), 1);
    }

    #[test]
    fn an_area_converts_to_database_units() {
        let area = Area::new(0.0, 0.0, 1.5, 2.25);
        assert_eq!(area.tcl(), "0 0 1.5 2.25");
        assert_eq!(
            area.to_rect(1000),
            Rect {
                x1: 0,
                y1: 0,
                x2: 1500,
                y2: 2250
            }
        );
        assert!(OpenRoadOptions::new().floorplan.is_none());
        assert!(
            OpenRoadOptions::new()
                .with_floorplan(Floorplan::square(10.0, 1.0))
                .floorplan
                .is_some()
        );
    }
}
