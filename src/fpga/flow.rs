//! Hand-off: the files and the command line a place-and-route tool needs.
//!
//! Reticle does not run other programs. These functions return the exact
//! bytes a tool expects and the argument list it should be run with; the
//! CLI, a build script or a person does the running. That keeps the
//! library sans-I/O and makes the hand-off testable: a golden test can
//! compare the JSON and the constraints file without a tool installed.
//!
//! | Function | Tool | Files |
//! |----------|------|-------|
//! | [`export_nextpnr`] | `nextpnr-ice40`, `nextpnr-ecp5` | a Yosys-style JSON netlist and a `.pcf` or `.lpf` |
//! | [`export_vendor`] | Vivado, Quartus, Diamond | structural Verilog and an `.xdc` |
//!
//! # What the netlist contains
//!
//! The JSON is [`emit_json`] over the design, with the exported module
//! marked as the top. After [`map`](super::map) the device primitives are
//! black-box cells named exactly as the family's database spells them
//! (`SB_IO`, `SB_GB`, `SB_RAM40_4K`, `TRELLIS_IO`, `DP16KD`), which is
//! what nextpnr looks for. Everything that has not been technology-mapped
//! yet is still a generic Yosys cell (`$add`, `$dff`, `$mux`); nextpnr
//! rejects those, so a complete flow needs the LUT mapper that lands with
//! the rest of phase 5. The export is therefore useful today for the
//! IO, clock, memory and DSP layers, and complete once mapping is.
//!
//! [`emit_json`]: crate::ir::emit::emit_json

use std::error::Error;
use std::fmt;

use super::constraints::Constraints;
use super::device::Device;
use crate::ir::emit::{EmitError, VerilogOptions, emit_json, emit_verilog_with};
use crate::ir::{Design, ModuleId};

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
        }
    }
}

impl Error for FlowError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            FlowError::Emit(err) => Some(err),
            _ => None,
        }
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
}
