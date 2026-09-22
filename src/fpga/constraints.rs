//! Placement, IO and timing constraints as a checked language.
//!
//! Constraints say where a design touches the outside world (which package
//! pin a port uses, at which IO standard), where parts of it may be placed
//! (regions, relative placement, kept hierarchy) and what its clocks are
//! (period, domains, paths that do not need to meet them). Reticle treats
//! them as a first-class input: they are parsed with spans, merged from two
//! sources with a defined precedence, and *checked against the device
//! database* before anything downstream uses them.
//!
//! # Two front doors
//!
//! 1. A native `.rcf` file, parsed by [`Constraints::parse`]:
//!
//!    ```text
//!    # pins
//!    set_io -io_standard LVCMOS33 -drive 8 -pullup yes clk 21
//!    set_io led[0] 99
//!    set_io -ddr clk -delay 12 dq[0] A3
//!
//!    # placement
//!    region core 0 0 7 7
//!    assign u_cpu/* region core
//!    rloc adder_bit* 0 1 group adder
//!    keep_hierarchy u_cpu
//!
//!    # timing
//!    create_clock -name sys -period 83.3 clk
//!    clock_domain sys clk
//!    set_false_path -from rst -to *
//!    set_multicycle_path 2 -setup -from mul_a -to mul_q
//!    ```
//!
//! 2. Attributes on the IR, read by [`Constraints::merge_attrs`]: `(* PIN
//!    = "A3" *)` (also `LOC`), `io_standard`, `drive`, `slew`, `pullup`,
//!    `ddr` (the clock of a double-data-rate register at the pin) and
//!    `io_delay` on a port's net, `clock_period` or `clock_mhz` on any
//!    net, and `keep_hierarchy`, `rloc` and `region` on an
//!    instance, a cell or a module. Attribute names are matched
//!    case-insensitively, so `(* PIN *)` and `(* pin *)` are the same
//!    constraint.
//!
//! **Precedence: the file wins.** A constraints file is written against a
//! specific board and is the later, more specific statement; an attribute
//! in the HDL is the design's own default. When both name the same port,
//! the file's pin, standard, drive, slew and pull-up are used and a note
//! says so. Attributes that the file says nothing about are kept, key by
//! key, so an HDL `io_standard` survives a file that only assigns a pin.
//!
//! # Checking
//!
//! [`Constraints::check`] verifies everything that can be verified without
//! placing anything: every port named exists and is wide enough, every pin
//! exists on the device and is usable for IO, no pin is used twice, IO
//! standards are known and agree on a bank supply voltage within each
//! bank, drive strengths and slew rates are offered by the standard,
//! regions lie inside the tile grid, instance patterns match something,
//! and clock nets exist and actually clock something. Every problem is a
//! [`Diagnostic`] with a span into the `.rcf` file or into the HDL the
//! attribute came from.
//!
//! # Vendor output
//!
//! The writers render the subset each vendor format can express, so an
//! externally placed flow works today: [`Constraints::write_pcf`] for
//! nextpnr-ice40, [`Constraints::write_lpf`] for nextpnr-ecp5,
//! [`Constraints::write_xdc`] for Vivado and [`Constraints::write_sdc`]
//! for Quartus and other SDC consumers. Each one documents what it drops,
//! and says so in a comment in its own output rather than silently losing
//! a constraint.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::device::{Device, PinName};
use super::text::{Line, Token, tokenize};
use crate::diag::{Diagnostic, Diagnostics, Severity};
use crate::ir::expr::operands;
use crate::ir::{
    AttrValue, Attrs, CellKind, Design, ExprId, Module, ModuleId, PortDir, ProcessKind, StmtKind,
};
use crate::source::{SourceId, Span};

/// Diagnostic code for a malformed line in a `.rcf` file.
pub const RCF_SYNTAX: &str = "F0110";
/// Diagnostic code for a `.rcf` construct that is well formed but unknown.
pub const RCF_UNKNOWN: &str = "F0111";
/// Diagnostic code for a constraint naming a port the design lacks.
pub const NO_SUCH_PORT: &str = "F0201";
/// Diagnostic code for a constraint naming a pin the device lacks.
pub const NO_SUCH_PIN: &str = "F0202";
/// Diagnostic code for a pin that cannot carry a design signal.
pub const PIN_NOT_IO: &str = "F0203";
/// Diagnostic code for a pin or a port constrained twice.
pub const DUPLICATE_ASSIGNMENT: &str = "F0204";
/// Diagnostic code for an IO standard the device does not offer.
pub const NO_SUCH_IO_STANDARD: &str = "F0205";
/// Diagnostic code for IO standards that cannot share a bank.
pub const BANK_CONFLICT: &str = "F0206";
/// Diagnostic code for a drive strength or slew rate the standard lacks.
pub const BAD_IO_OPTION: &str = "F0207";
/// Diagnostic code for a region outside the device grid.
pub const BAD_REGION: &str = "F0209";
/// Diagnostic code for a reference to an undeclared region.
pub const NO_SUCH_REGION: &str = "F0210";
/// Diagnostic code for a pattern that matches nothing in the design.
pub const NO_MATCH: &str = "F0211";
/// Diagnostic code for a clock constraint on a net that is not a clock.
pub const BAD_CLOCK: &str = "F0212";

/// Where a constraint came from, which decides precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A constraints file.
    File,
    /// An attribute on an IR object.
    Attribute,
}

impl Origin {
    /// A word for diagnostics.
    pub fn describe(self) -> &'static str {
        match self {
            Origin::File => "the constraints file",
            Origin::Attribute => "a source attribute",
        }
    }
}

/// The electrical options of one IO.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IoAttrs {
    /// The IO standard's name (`LVCMOS33`).
    pub io_standard: Option<String>,
    /// Drive strength in milliamperes.
    pub drive: Option<u32>,
    /// Slew rate (`slow`, `fast`), lower-cased on the way in.
    pub slew: Option<String>,
    /// `Some(true)` enables the pull-up, `Some(false)` disables it
    /// explicitly, `None` leaves it to the tool's default.
    pub pullup: Option<bool>,
    /// The clock of a double-data-rate register at the pin, when the port
    /// is one. A DDR port carries twice as many bits as it has pins: bit
    /// `i` and bit `i + N` of an `2N`-bit port share pin `i`, the low
    /// half on the rising edge of this clock and the high half on the
    /// falling one.
    pub ddr: Option<String>,
    /// A delay between the pin and the fabric, in the device's own delay
    /// steps, on a family that has a programmable one.
    pub delay: Option<u32>,
}

impl IoAttrs {
    /// True when nothing is set.
    pub fn is_empty(&self) -> bool {
        *self == IoAttrs::default()
    }

    /// Fills in every option `self` does not set from `other`.
    ///
    /// This is the key-by-key half of the precedence rule: the receiver is
    /// the winning source, `other` the losing one.
    pub fn fill_from(&mut self, other: &IoAttrs) {
        if self.io_standard.is_none() {
            self.io_standard = other.io_standard.clone();
        }
        if self.drive.is_none() {
            self.drive = other.drive;
        }
        if self.slew.is_none() {
            self.slew = other.slew.clone();
        }
        if self.pullup.is_none() {
            self.pullup = other.pullup;
        }
        if self.ddr.is_none() {
            self.ddr = other.ddr.clone();
        }
        if self.delay.is_none() {
            self.delay = other.delay;
        }
    }
}

/// One port (or one bit of one port) tied to one package pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinAssignment {
    /// The top-level port name.
    pub port: String,
    /// The bit of the port, for a vector constrained bit by bit.
    pub bit: Option<u32>,
    /// The package pin.
    pub pin: PinName,
    /// The electrical options.
    pub io: IoAttrs,
    /// Where the constraint was written.
    pub span: Span,
    /// Which front door it came through.
    pub origin: Origin,
}

impl PinAssignment {
    /// The name a vendor file uses for the constrained signal: `d` for a
    /// whole port, `d[3]` for one bit.
    pub fn signal(&self) -> String {
        match self.bit {
            Some(bit) => format!("{}[{bit}]", self.port),
            None => self.port.clone(),
        }
    }
}

/// A rectangle of the device the tools must place something inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// The region name, referred to by a [`RegionAssignment`].
    pub name: String,
    /// Left edge, inclusive.
    pub x0: u32,
    /// Bottom edge, inclusive.
    pub y0: u32,
    /// Right edge, inclusive.
    pub x1: u32,
    /// Top edge, inclusive.
    pub y1: u32,
    /// Where the constraint was written.
    pub span: Span,
}

/// Instances matching a pattern, confined to a [`Region`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionAssignment {
    /// A glob over instance and cell names (`u_cpu/*`).
    pub pattern: String,
    /// The region's name.
    pub region: String,
    /// Where the constraint was written.
    pub span: Span,
    /// Which front door it came through.
    pub origin: Origin,
}

/// A relative placement macro: instances placed at a fixed offset from
/// each other, so a datapath keeps its shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rloc {
    /// A glob over instance and cell names.
    pub pattern: String,
    /// Column offset from the group's anchor.
    pub dx: i32,
    /// Row offset from the group's anchor.
    pub dy: i32,
    /// The macro this offset belongs to; instances in one group are placed
    /// together.
    pub group: Option<String>,
    /// Where the constraint was written.
    pub span: Span,
    /// Which front door it came through.
    pub origin: Origin,
}

/// One clock: a name, the net that carries it and its period.
#[derive(Clone, Debug, PartialEq)]
pub struct ClockDef {
    /// The clock's name, used by the path constraints.
    pub name: String,
    /// The net or port carrying the clock.
    pub net: String,
    /// The period in nanoseconds.
    pub period_ns: f64,
    /// Where the constraint was written.
    pub span: Span,
    /// Which front door it came through.
    pub origin: Origin,
}

impl ClockDef {
    /// The frequency in MHz implied by the period.
    pub fn frequency_mhz(&self) -> f64 {
        if self.period_ns > 0.0 {
            1000.0 / self.period_ns
        } else {
            0.0
        }
    }
}

/// A set of paths, named by where they start and end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathSpec {
    /// A glob over start points, or `None` for "anywhere".
    pub from: Option<String>,
    /// A glob over end points, or `None` for "anywhere".
    pub to: Option<String>,
    /// Where the constraint was written.
    pub span: Span,
}

/// Paths allowed to take more than one clock period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MulticyclePath {
    /// How many periods the path may take.
    pub cycles: u32,
    /// True when the constraint applies to the hold check rather than the
    /// setup check.
    pub hold: bool,
    /// Which paths.
    pub path: PathSpec,
}

/// A named group of nets that belong to one clock domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockDomain {
    /// The domain's name.
    pub name: String,
    /// The nets in it.
    pub nets: Vec<String>,
    /// Where the constraint was written.
    pub span: Span,
}

/// Everything constraining one design on one device.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Constraints {
    /// Pin assignments, in the order they were stated.
    pub pins: Vec<PinAssignment>,
    /// Declared placement regions.
    pub regions: Vec<Region>,
    /// Instances confined to a region.
    pub region_assignments: Vec<RegionAssignment>,
    /// Relative placement macros.
    pub rlocs: Vec<Rloc>,
    /// Globs over instances whose hierarchy must survive optimisation.
    pub keep_hierarchy: Vec<(String, Span, Origin)>,
    /// Clock definitions.
    pub clocks: Vec<ClockDef>,
    /// Paths excluded from timing analysis.
    pub false_paths: Vec<PathSpec>,
    /// Paths given more than one period.
    pub multicycle_paths: Vec<MulticyclePath>,
    /// Declared clock domains.
    pub clock_domains: Vec<ClockDomain>,
}

impl Constraints {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when nothing is constrained.
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
            && self.regions.is_empty()
            && self.region_assignments.is_empty()
            && self.rlocs.is_empty()
            && self.keep_hierarchy.is_empty()
            && self.clocks.is_empty()
            && self.false_paths.is_empty()
            && self.multicycle_paths.is_empty()
            && self.clock_domains.is_empty()
    }

    /// The assignment constraining `port`, or one of its bits.
    pub fn pin_of(&self, port: &str, bit: Option<u32>) -> Option<&PinAssignment> {
        self.pins
            .iter()
            .find(|p| p.port == port && p.bit == bit)
            .or_else(|| self.pins.iter().find(|p| p.port == port && p.bit.is_none()))
    }

    /// The clock carried by `net`, if one is declared.
    pub fn clock_of_net(&self, net: &str) -> Option<&ClockDef> {
        self.clocks.iter().find(|c| c.net == net)
    }

    /// True when `name` matches a `keep_hierarchy` pattern.
    pub fn keeps_hierarchy(&self, name: &str) -> bool {
        self.keep_hierarchy
            .iter()
            .any(|(pattern, _, _)| matches_glob(pattern, name))
    }

    /// Parses a `.rcf` file.
    ///
    /// Malformed lines are reported through `diags` and skipped, so one
    /// bad constraint does not lose the file.
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Constraints {
        let lines = tokenize(text, file);
        let mut out = Constraints::new();
        for line in &lines {
            let mut parser = LineParser {
                line,
                diags,
                failed: false,
            };
            parser.directive(&mut out);
        }
        out
    }

    /// Adds every constraint of `other` that `self` does not already
    /// state, following the precedence rule: `self` wins.
    ///
    /// A pin assignment of `other` for a port `self` already constrains is
    /// dropped, but its IO options fill in the ones `self` leaves open.
    /// Everything else is appended when an equal constraint is not already
    /// present.
    pub fn merge(&mut self, other: Constraints, diags: &mut Diagnostics) {
        for pin in other.pins {
            match self
                .pins
                .iter_mut()
                .find(|p| p.port == pin.port && p.bit == pin.bit)
            {
                Some(existing) => {
                    if existing.pin != pin.pin {
                        diags.push(
                            Diagnostic::new(
                                Severity::Note,
                                format!(
                                    "port `{}` is assigned pin {} by {}, overriding pin {} from {}",
                                    existing.signal(),
                                    existing.pin,
                                    existing.origin.describe(),
                                    pin.pin,
                                    pin.origin.describe()
                                ),
                            )
                            .with_label(existing.span, "this assignment is used")
                            .with_secondary(pin.span, "this one is ignored"),
                        );
                    }
                    existing.io.fill_from(&pin.io);
                }
                None => self.pins.push(pin),
            }
        }
        for region in other.regions {
            if !self.regions.iter().any(|r| r.name == region.name) {
                self.regions.push(region);
            }
        }
        for assignment in other.region_assignments {
            if !self
                .region_assignments
                .iter()
                .any(|a| a.pattern == assignment.pattern)
            {
                self.region_assignments.push(assignment);
            }
        }
        for rloc in other.rlocs {
            if !self.rlocs.iter().any(|r| r.pattern == rloc.pattern) {
                self.rlocs.push(rloc);
            }
        }
        for keep in other.keep_hierarchy {
            if !self.keep_hierarchy.iter().any(|(p, _, _)| *p == keep.0) {
                self.keep_hierarchy.push(keep);
            }
        }
        for clock in other.clocks {
            if !self.clocks.iter().any(|c| c.net == clock.net) {
                self.clocks.push(clock);
            }
        }
        self.false_paths.extend(other.false_paths);
        self.multicycle_paths.extend(other.multicycle_paths);
        for domain in other.clock_domains {
            if !self.clock_domains.iter().any(|d| d.name == domain.name) {
                self.clock_domains.push(domain);
            }
        }
    }

    /// Reads the constraints written as attributes in `module` and merges
    /// them in behind what is already here.
    ///
    /// See the module documentation for the attribute names and for the
    /// precedence rule.
    pub fn merge_attrs(&mut self, design: &Design, module: ModuleId, diags: &mut Diagnostics) {
        let from_attrs = Constraints::from_attrs(design, module);
        self.merge(from_attrs, diags);
    }

    /// The constraints written as attributes in `module`, on their own.
    pub fn from_attrs(design: &Design, module: ModuleId) -> Constraints {
        let mut out = Constraints::new();
        let Some(module) = design.modules.get(module) else {
            return out;
        };
        for port in &module.ports {
            let Some(net) = module.nets.get(port.net) else {
                continue;
            };
            let attrs = &net.attrs;
            let io = io_attrs_of(attrs);
            if let Some(pin) = attr_str(attrs, &["pin", "loc", "package_pin"]) {
                out.pins.push(PinAssignment {
                    port: port.name.as_str().to_owned(),
                    bit: None,
                    pin,
                    io,
                    span: net.span,
                    origin: Origin::Attribute,
                });
            } else if !io.is_empty() {
                // Electrical options without a pin still travel to the IO
                // buffer, so keep them as an assignment to no pin.
                out.pins.push(PinAssignment {
                    port: port.name.as_str().to_owned(),
                    bit: None,
                    pin: String::new(),
                    io,
                    span: net.span,
                    origin: Origin::Attribute,
                });
            }
            if let Some(period) = attr_f64(attrs, &["clock_period", "period"]).or_else(|| {
                attr_f64(attrs, &["clock_mhz"])
                    .filter(|mhz| *mhz > 0.0)
                    .map(|mhz| 1000.0 / mhz)
            }) {
                out.clocks.push(ClockDef {
                    name: port.name.as_str().to_owned(),
                    net: net.name.as_str().to_owned(),
                    period_ns: period,
                    span: net.span,
                    origin: Origin::Attribute,
                });
            }
        }
        // An internal net may carry a clock too, which is how a design
        // asks for a generated one in its source:
        // `(* clock_mhz = 48 *) wire sys;` with nothing driving `sys` is
        // a request for a PLL, and `fpga::primitives` answers it.
        for (id, net) in module.nets.iter() {
            if module.ports.iter().any(|p| p.net == id) {
                continue;
            }
            let attrs = &net.attrs;
            let Some(period) = attr_f64(attrs, &["clock_period"]).or_else(|| {
                attr_f64(attrs, &["clock_mhz"])
                    .filter(|mhz| *mhz > 0.0)
                    .map(|mhz| 1000.0 / mhz)
            }) else {
                continue;
            };
            out.clocks.push(ClockDef {
                name: net.name.as_str().to_owned(),
                net: net.name.as_str().to_owned(),
                period_ns: period,
                span: net.span,
                origin: Origin::Attribute,
            });
        }
        let objects = module
            .instances
            .iter()
            .map(|(_, i)| (i.name.as_str(), &i.attrs, i.span))
            .chain(
                module
                    .cells
                    .iter()
                    .map(|(_, c)| (c.name.as_str(), &c.attrs, c.span)),
            );
        for (name, attrs, span) in objects {
            if attrs.is_set("keep_hierarchy") || attrs.is_set("KEEP_HIERARCHY") {
                out.keep_hierarchy
                    .push((name.to_owned(), span, Origin::Attribute));
            }
            if let Some(value) = attr_str(attrs, &["rloc"])
                && let Some((dx, dy)) = parse_rloc(&value)
            {
                out.rlocs.push(Rloc {
                    pattern: name.to_owned(),
                    dx,
                    dy,
                    group: attr_str(attrs, &["rloc_group", "u_set", "hu_set"]),
                    span,
                    origin: Origin::Attribute,
                });
            }
            if let Some(region) = attr_str(attrs, &["region", "pblock"]) {
                out.region_assignments.push(RegionAssignment {
                    pattern: name.to_owned(),
                    region,
                    span,
                    origin: Origin::Attribute,
                });
            }
        }
        if module.attrs.is_set("keep_hierarchy") {
            out.keep_hierarchy.push((
                module.name.as_str().to_owned(),
                module.span,
                Origin::Attribute,
            ));
        }
        out
    }

    /// Checks every constraint against the design's top module and the
    /// device; see the module documentation for the list.
    pub fn check(&self, design: &Design, device: &Device, diags: &mut Diagnostics) {
        match design.top {
            Some(top) => self.check_module(design, top, device, diags),
            None => diags.push(Diagnostic::error(
                "the design has no top module, so constraints cannot be checked",
            )),
        }
    }

    /// Checks every constraint against `module` and the device.
    pub fn check_module(
        &self,
        design: &Design,
        module: ModuleId,
        device: &Device,
        diags: &mut Diagnostics,
    ) {
        let Some(module) = design.modules.get(module) else {
            diags.push(Diagnostic::error("the module to check does not exist"));
            return;
        };
        self.check_pins(module, device, diags);
        self.check_placement(module, device, diags);
        self.check_timing(module, diags);
    }

    fn check_pins(&self, module: &Module, device: &Device, diags: &mut Diagnostics) {
        // Which pin and which port bit has been claimed, and by whom.
        let mut pin_used: Vec<(&str, Span)> = Vec::new();
        let mut port_used: Vec<(String, Span)> = Vec::new();
        // Bank name to the (vccio, standard, span) that fixed it.
        let mut bank_vccio: BTreeMap<String, (String, String, Span)> = BTreeMap::new();

        for assignment in &self.pins {
            let signal = assignment.signal();
            match module.port(&assignment.port) {
                Some(port) => {
                    let width = module.nets.get(port.net).and_then(|n| n.ty.width());
                    if let (Some(bit), Some(width)) = (assignment.bit, width)
                        && bit >= width
                    {
                        diags.push(
                            Diagnostic::error(format!(
                                "port `{}` has {width} bits, so there is no bit {bit}",
                                assignment.port
                            ))
                            .with_code(NO_SUCH_PORT)
                            .with_span(assignment.span),
                        );
                    }
                }
                None => diags.push(
                    Diagnostic::error(format!(
                        "no port named `{}` in module `{}`",
                        assignment.port, module.name
                    ))
                    .with_code(NO_SUCH_PORT)
                    .with_span(assignment.span),
                ),
            }

            if let Some((_, first)) = port_used.iter().find(|(name, _)| *name == signal) {
                diags.push(
                    Diagnostic::error(format!("port `{signal}` is assigned a pin twice"))
                        .with_code(DUPLICATE_ASSIGNMENT)
                        .with_label(assignment.span, "assigned again here")
                        .with_secondary(*first, "first assigned here"),
                );
            } else {
                port_used.push((signal.clone(), assignment.span));
            }

            let pin = if assignment.pin.is_empty() {
                None
            } else {
                match device.pin(&assignment.pin) {
                    Some(pin) => {
                        if !pin.kind.is_io() {
                            diags.push(
                                Diagnostic::error(format!(
                                    "pin {} of `{}` is a {} pin and cannot carry `{signal}`",
                                    pin.name, device.name, pin.kind
                                ))
                                .with_code(PIN_NOT_IO)
                                .with_span(assignment.span),
                            );
                        }
                        if let Some((_, first)) =
                            pin_used.iter().find(|(name, _)| *name == pin.name)
                        {
                            diags.push(
                                Diagnostic::error(format!(
                                    "pin {} is assigned to two signals",
                                    pin.name
                                ))
                                .with_code(DUPLICATE_ASSIGNMENT)
                                .with_label(assignment.span, "assigned again here")
                                .with_secondary(*first, "first assigned here"),
                            );
                        } else {
                            pin_used.push((pin.name.as_str(), assignment.span));
                        }
                        Some(pin)
                    }
                    None => {
                        let mut diag = Diagnostic::error(format!(
                            "`{}` has no pin named {}",
                            device.name, assignment.pin
                        ))
                        .with_code(NO_SUCH_PIN)
                        .with_span(assignment.span);
                        if device.pins.is_empty() {
                            diag = diag.with_note(format!(
                                "the database records no package pins for `{}`",
                                device.name
                            ));
                        }
                        diags.push(diag);
                        None
                    }
                }
            };

            self.check_io_attrs(assignment, pin, device, &mut bank_vccio, diags);
        }
    }

    fn check_io_attrs(
        &self,
        assignment: &PinAssignment,
        pin: Option<&super::device::Pin>,
        device: &Device,
        bank_vccio: &mut BTreeMap<String, (String, String, Span)>,
        diags: &mut Diagnostics,
    ) {
        let Some(name) = &assignment.io.io_standard else {
            return;
        };
        let Some(standard) = device.io_standard(name) else {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` does not offer the IO standard `{name}`",
                    device.name
                ))
                .with_code(NO_SUCH_IO_STANDARD)
                .with_span(assignment.span)
                .with_note(format!(
                    "known standards: {}",
                    device
                        .io_standards
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
            return;
        };
        if let Some(drive) = assignment.io.drive
            && !standard.drive_strengths.is_empty()
            && !standard.drive_strengths.contains(&drive)
        {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` does not offer a drive strength of {drive} mA",
                    standard.name
                ))
                .with_code(BAD_IO_OPTION)
                .with_span(assignment.span)
                .with_note(format!(
                    "available: {}",
                    standard
                        .drive_strengths
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
        if let Some(slew) = &assignment.io.slew
            && !standard.slew_rates.is_empty()
            && !standard.slew_rates.iter().any(|s| s == slew)
        {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` does not offer the slew rate `{slew}`",
                    standard.name
                ))
                .with_code(BAD_IO_OPTION)
                .with_span(assignment.span)
                .with_note(format!("available: {}", standard.slew_rates.join(", "))),
            );
        }
        // Bank rules: the standard's supply must be one the bank offers,
        // and one bank cannot run at two voltages at once.
        let (Some(pin), Some(vccio)) = (pin, standard.vccio.as_ref()) else {
            return;
        };
        let Some(bank_name) = pin.bank.as_deref() else {
            return;
        };
        let Some(bank) = device.io_bank(bank_name) else {
            return;
        };
        if !bank.vccio_options.is_empty() && !bank.vccio_options.contains(vccio) {
            diags.push(
                Diagnostic::error(format!(
                    "bank {bank_name} cannot supply {vccio} V, which `{}` needs",
                    standard.name
                ))
                .with_code(BANK_CONFLICT)
                .with_span(assignment.span)
                .with_note(format!(
                    "bank {bank_name} offers {}",
                    bank.vccio_options.join(", ")
                )),
            );
            return;
        }
        match bank_vccio.get(bank_name) {
            Some((first_vccio, first_std, first_span)) if first_vccio != vccio => {
                diags.push(
                    Diagnostic::error(format!(
                        "bank {bank_name} would need {vccio} V for `{}` and {first_vccio} V for `{first_std}`",
                        standard.name
                    ))
                    .with_code(BANK_CONFLICT)
                    .with_label(assignment.span, "this IO standard")
                    .with_secondary(*first_span, "conflicts with this one")
                    .with_note("every pin of a bank shares one supply voltage"),
                );
            }
            Some(_) => {}
            None => {
                bank_vccio.insert(
                    bank.name.clone(),
                    (vccio.clone(), standard.name.clone(), assignment.span),
                );
            }
        }
    }

    fn check_placement(&self, module: &Module, device: &Device, diags: &mut Diagnostics) {
        let names = placeable_names(module);
        for region in &self.regions {
            if region.x0 > region.x1 || region.y0 > region.y1 {
                diags.push(
                    Diagnostic::error(format!(
                        "region `{}` is empty: ({}, {}) is not below-left of ({}, {})",
                        region.name, region.x0, region.y0, region.x1, region.y1
                    ))
                    .with_code(BAD_REGION)
                    .with_span(region.span),
                );
                continue;
            }
            match device.tile_grid {
                Some(grid) => {
                    if !grid.contains(region.x1, region.y1) {
                        diags.push(
                            Diagnostic::error(format!(
                                "region `{}` reaches ({}, {}), outside the {}x{} grid of `{}`",
                                region.name,
                                region.x1,
                                region.y1,
                                grid.width,
                                grid.height,
                                device.name
                            ))
                            .with_code(BAD_REGION)
                            .with_span(region.span),
                        );
                    }
                }
                None => diags.push(
                    Diagnostic::warning(format!(
                        "`{}` records no tile grid, so region `{}` cannot be checked",
                        device.name, region.name
                    ))
                    .with_code(BAD_REGION)
                    .with_span(region.span),
                ),
            }
        }
        let mut seen: Vec<&str> = Vec::new();
        for region in &self.regions {
            if seen.contains(&region.name.as_str()) {
                diags.push(
                    Diagnostic::error(format!("region `{}` is declared twice", region.name))
                        .with_code(DUPLICATE_ASSIGNMENT)
                        .with_span(region.span),
                );
            }
            seen.push(&region.name);
        }
        for assignment in &self.region_assignments {
            if !self.regions.iter().any(|r| r.name == assignment.region) {
                diags.push(
                    Diagnostic::error(format!("no region named `{}`", assignment.region))
                        .with_code(NO_SUCH_REGION)
                        .with_span(assignment.span),
                );
            }
            check_pattern(
                &assignment.pattern,
                &names,
                "instance",
                assignment.span,
                diags,
            );
        }
        for rloc in &self.rlocs {
            check_pattern(&rloc.pattern, &names, "instance", rloc.span, diags);
        }
        for (pattern, span, _) in &self.keep_hierarchy {
            let mut all = names.clone();
            all.push(module.name.as_str().to_owned());
            check_pattern(pattern, &all, "instance", *span, diags);
        }
    }

    fn check_timing(&self, module: &Module, diags: &mut Diagnostics) {
        let clock_nets = clock_driven_nets(module);
        // A clock constrained on a net nothing drives is one the design
        // asks a PLL for, and the PLL is fed from a clock constrained on
        // an input port — which then clocks no flip-flop directly and
        // must not be called useless for it.
        let driven = driven_nets(module);
        let feeds_a_pll = self.clocks.iter().any(|clock| {
            module
                .net_by_name(&clock.net)
                .is_some_and(|net| !driven.get(net.index()).copied().unwrap_or(true))
        });
        let mut seen: Vec<&str> = Vec::new();
        for clock in &self.clocks {
            if seen.contains(&clock.name.as_str()) {
                diags.push(
                    Diagnostic::error(format!("clock `{}` is defined twice", clock.name))
                        .with_code(DUPLICATE_ASSIGNMENT)
                        .with_span(clock.span),
                );
            }
            seen.push(&clock.name);
            if clock.period_ns <= 0.0 {
                diags.push(
                    Diagnostic::error(format!(
                        "clock `{}` has a period of {} ns",
                        clock.name, clock.period_ns
                    ))
                    .with_code(BAD_CLOCK)
                    .with_span(clock.span),
                );
            }
            if module.net_by_name(&clock.net).is_none() {
                diags.push(
                    Diagnostic::error(format!(
                        "no net named `{}` in module `{}`",
                        clock.net, module.name
                    ))
                    .with_code(BAD_CLOCK)
                    .with_span(clock.span),
                );
            } else if !clock_nets.iter().any(|n| n == &clock.net)
                && !self
                    .pins
                    .iter()
                    .any(|p| p.io.ddr.as_deref() == Some(clock.net.as_str()))
                && !(feeds_a_pll
                    && module
                        .port(&clock.net)
                        .is_some_and(|p| p.dir == PortDir::In))
            {
                diags.push(
                    Diagnostic::warning(format!(
                        "net `{}` does not clock anything in `{}`",
                        clock.net, module.name
                    ))
                    .with_code(BAD_CLOCK)
                    .with_span(clock.span)
                    .with_note(
                        "a clock constraint on a net that reaches no flip-flop clock pin has no effect",
                    ),
                );
            }
        }
        let names = timing_point_names(module);
        for path in &self.false_paths {
            check_path(path, &names, diags);
        }
        for path in &self.multicycle_paths {
            if path.cycles == 0 {
                diags.push(
                    Diagnostic::error("a multicycle path must span at least one cycle")
                        .with_code(BAD_CLOCK)
                        .with_span(path.path.span),
                );
            }
            check_path(&path.path, &names, diags);
        }
        for domain in &self.clock_domains {
            for net in &domain.nets {
                if module.net_by_name(net).is_none() {
                    diags.push(
                        Diagnostic::error(format!(
                            "no net named `{net}` in module `{}`",
                            module.name
                        ))
                        .with_code(BAD_CLOCK)
                        .with_span(domain.span),
                    );
                }
            }
        }
    }

    /// Renders the constraints as a nextpnr-ice40 `.pcf` file.
    ///
    /// PCF expresses pin assignment (`set_io`), the pull-up (`-pullup`)
    /// and a clock frequency (`set_frequency`). IO standards, drive
    /// strengths, slew rates, placement regions, relative placement,
    /// kept hierarchy and path exceptions have no PCF syntax; they are
    /// listed in a comment at the end instead of being dropped silently.
    pub fn write_pcf(&self, device: &Device) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# nextpnr PCF written by reticle for {} ({})",
            device.name, device.family
        );
        for pin in &self.pins {
            if pin.pin.is_empty() {
                continue;
            }
            let pullup = match pin.io.pullup {
                Some(true) => " -pullup yes",
                Some(false) => " -pullup no",
                None => "",
            };
            let _ = writeln!(out, "set_io{pullup} {} {}", pin.signal(), pin.pin);
        }
        for clock in &self.clocks {
            let _ = writeln!(
                out,
                "set_frequency {} {:.3}",
                clock.net,
                clock.frequency_mhz()
            );
        }
        self.write_dropped(&mut out, "#", &["io standards", "drive", "slew"]);
        out
    }

    /// Renders the constraints as a nextpnr-ecp5 `.lpf` file.
    ///
    /// LPF expresses pin assignment (`LOCATE COMP`), the IO standard,
    /// drive strength, slew rate and pull mode (`IOBUF PORT`) and a clock
    /// frequency (`FREQUENCY PORT`). Placement regions, relative
    /// placement, kept hierarchy and path exceptions are listed in a
    /// comment instead: Reticle does not emit `REGION`/`UGROUP` or
    /// `BLOCK PATH` statements it cannot verify.
    pub fn write_lpf(&self, device: &Device) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# nextpnr LPF written by reticle for {} ({})",
            device.name, device.family
        );
        for pin in &self.pins {
            if !pin.pin.is_empty() {
                let _ = writeln!(
                    out,
                    "LOCATE COMP \"{}\" SITE \"{}\";",
                    pin.signal(),
                    pin.pin
                );
            }
            let mut options = String::new();
            if let Some(standard) = &pin.io.io_standard {
                let _ = write!(options, " IO_TYPE={}", standard.to_ascii_uppercase());
            }
            if let Some(drive) = pin.io.drive {
                let _ = write!(options, " DRIVE={drive}");
            }
            if let Some(slew) = &pin.io.slew {
                let _ = write!(options, " SLEWRATE={}", slew.to_ascii_uppercase());
            }
            match pin.io.pullup {
                Some(true) => options.push_str(" PULLMODE=UP"),
                Some(false) => options.push_str(" PULLMODE=NONE"),
                None => {}
            }
            if !options.is_empty() {
                let _ = writeln!(out, "IOBUF PORT \"{}\"{options};", pin.signal());
            }
        }
        for clock in &self.clocks {
            let is_port = self.pins.iter().any(|p| p.port == clock.net);
            let object = if is_port { "PORT" } else { "NET" };
            let _ = writeln!(
                out,
                "FREQUENCY {object} \"{}\" {:.3} MHZ;",
                clock.net,
                clock.frequency_mhz()
            );
        }
        self.write_dropped(&mut out, "#", &[]);
        out
    }

    /// Renders the constraints as a Vivado `.xdc` file.
    ///
    /// XDC expresses everything Reticle models: pin assignment, IO
    /// standard, drive, slew and pull-up, pblocks for regions, `RLOC`
    /// properties, `KEEP_HIERARCHY`, clocks and path exceptions. The
    /// pblock rectangles are written as `SLICE_X..Y..` ranges, which is
    /// the usual Xilinx site naming; a device whose sites are named
    /// otherwise needs the ranges rewritten by hand.
    pub fn write_xdc(&self, device: &Device) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# Vivado XDC written by reticle for {} ({})",
            device.name, device.family
        );
        for pin in &self.pins {
            let ports = format!("[get_ports {{{}}}]", pin.signal());
            if !pin.pin.is_empty() {
                let _ = writeln!(out, "set_property PACKAGE_PIN {} {ports}", pin.pin);
            }
            if let Some(standard) = &pin.io.io_standard {
                let _ = writeln!(
                    out,
                    "set_property IOSTANDARD {} {ports}",
                    standard.to_ascii_uppercase()
                );
            }
            if let Some(drive) = pin.io.drive {
                let _ = writeln!(out, "set_property DRIVE {drive} {ports}");
            }
            if let Some(slew) = &pin.io.slew {
                let _ = writeln!(
                    out,
                    "set_property SLEW {} {ports}",
                    slew.to_ascii_uppercase()
                );
            }
            if let Some(pullup) = pin.io.pullup {
                let value = if pullup { "TRUE" } else { "FALSE" };
                let _ = writeln!(out, "set_property PULLUP {value} {ports}");
            }
        }
        for region in &self.regions {
            let _ = writeln!(out, "create_pblock {}", region.name);
            let _ = writeln!(
                out,
                "resize_pblock [get_pblocks {}] -add {{SLICE_X{}Y{}:SLICE_X{}Y{}}}",
                region.name, region.x0, region.y0, region.x1, region.y1
            );
        }
        for assignment in &self.region_assignments {
            let _ = writeln!(
                out,
                "add_cells_to_pblock [get_pblocks {}] [get_cells {{{}}}]",
                assignment.region, assignment.pattern
            );
        }
        for rloc in &self.rlocs {
            let _ = writeln!(
                out,
                "set_property RLOC X{}Y{} [get_cells {{{}}}]",
                rloc.dx, rloc.dy, rloc.pattern
            );
            if let Some(group) = &rloc.group {
                let _ = writeln!(
                    out,
                    "set_property U_SET {group} [get_cells {{{}}}]",
                    rloc.pattern
                );
            }
        }
        for (pattern, _, _) in &self.keep_hierarchy {
            let _ = writeln!(
                out,
                "set_property KEEP_HIERARCHY TRUE [get_cells {{{pattern}}}]"
            );
        }
        out.push_str(&self.timing_sdc());
        out
    }

    /// Renders the timing constraints as an `.sdc` file for Quartus and
    /// other SDC consumers.
    ///
    /// SDC is a timing language: clocks, false paths and multicycle paths
    /// are expressed, and pin assignment, IO standards and placement are
    /// not, because they belong in a vendor-specific file (a Quartus
    /// `.qsf`, a Vivado `.xdc`). They are listed in a comment.
    pub fn write_sdc(&self, device: &Device) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# SDC timing constraints written by reticle for {} ({})",
            device.name, device.family
        );
        out.push_str(&self.timing_sdc());
        self.write_dropped(
            &mut out,
            "#",
            &["pin assignments", "io standards", "placement"],
        );
        out
    }

    /// The SDC subset shared by [`Constraints::write_xdc`] and
    /// [`Constraints::write_sdc`].
    fn timing_sdc(&self) -> String {
        let mut out = String::new();
        for clock in &self.clocks {
            let _ = writeln!(
                out,
                "create_clock -name {} -period {:.3} [get_ports {{{}}}]",
                clock.name, clock.period_ns, clock.net
            );
        }
        for path in &self.false_paths {
            let _ = writeln!(out, "set_false_path{}", self.path_options(path));
        }
        for path in &self.multicycle_paths {
            let when = if path.hold { " -hold" } else { " -setup" };
            let _ = writeln!(
                out,
                "set_multicycle_path {}{when}{}",
                path.cycles,
                self.path_options(&path.path)
            );
        }
        out
    }

    /// The `-from` and `-to` of a path exception.
    ///
    /// A pattern that names a constrained port is written as `get_ports`,
    /// anything else as `get_cells`, which is the best guess available
    /// without the design in hand.
    fn path_options(&self, path: &PathSpec) -> String {
        let mut out = String::new();
        for (option, pattern) in [("from", &path.from), ("to", &path.to)] {
            let Some(pattern) = pattern else {
                continue;
            };
            let kind = if self.pins.iter().any(|p| p.port == *pattern) {
                "get_ports"
            } else {
                "get_cells"
            };
            let _ = write!(out, " -{option} [{kind} {{{pattern}}}]");
        }
        out
    }

    /// Appends a comment listing the constraints this format cannot say.
    fn write_dropped(&self, out: &mut String, comment: &str, extra: &[&str]) {
        let mut dropped: Vec<String> = Vec::new();
        if !self.regions.is_empty() {
            dropped.push(format!("{} placement region(s)", self.regions.len()));
        }
        if !self.rlocs.is_empty() {
            dropped.push(format!("{} relative placement(s)", self.rlocs.len()));
        }
        if !self.keep_hierarchy.is_empty() {
            dropped.push(format!(
                "{} keep_hierarchy constraint(s)",
                self.keep_hierarchy.len()
            ));
        }
        let paths = self.false_paths.len() + self.multicycle_paths.len();
        if paths > 0 {
            dropped.push(format!("{paths} path exception(s)"));
        }
        for item in extra {
            if !self.pins.is_empty() {
                dropped.push((*item).to_owned());
            }
        }
        if dropped.is_empty() {
            return;
        }
        let _ = writeln!(
            out,
            "{comment} not expressible in this format: {}",
            dropped.join(", ")
        );
    }
}

/// The names a placement constraint may match: instances and cells.
fn placeable_names(module: &Module) -> Vec<String> {
    module
        .instances
        .iter()
        .map(|(_, i)| i.name.as_str().to_owned())
        .chain(module.cells.iter().map(|(_, c)| c.name.as_str().to_owned()))
        .collect()
}

/// The names a timing exception may match: instances, cells, nets and
/// ports.
fn timing_point_names(module: &Module) -> Vec<String> {
    let mut names = placeable_names(module);
    names.extend(module.nets.iter().map(|(_, n)| n.name.as_str().to_owned()));
    names.extend(module.ports.iter().map(|p| p.name.as_str().to_owned()));
    names
}

/// The nets that reach a flip-flop clock pin, by name.
/// For every net of `module`, whether something may drive it: an input
/// or inout port, a continuous assignment, a cell output, an assignment
/// in a process, or an instance connection.
///
/// It errs towards "driven", since [`super::primitives`] uses the answer
/// to decide whether a PLL may be wired onto a net: an instance
/// connection counts whatever its direction, since which connections are
/// outputs is the other module's business.
pub(crate) fn driven_nets(module: &Module) -> Vec<bool> {
    let mut driven = vec![false; module.nets.len()];
    let mark = |net: crate::ir::NetId, driven: &mut Vec<bool>| {
        if let Some(slot) = driven.get_mut(net.index()) {
            *slot = true;
        }
    };
    for port in &module.ports {
        if port.dir != PortDir::Out {
            mark(port.net, &mut driven);
        }
    }
    for assign in &module.assigns {
        for net in assign.target.nets() {
            mark(net, &mut driven);
        }
    }
    for (_, cell) in module.cells.iter() {
        for (_, net) in &cell.outputs {
            mark(*net, &mut driven);
        }
    }
    let mut written = Vec::new();
    module.for_each_stmt(|stmt| {
        if let StmtKind::Assign { target, .. } = &stmt.kind {
            written.extend(target.nets());
        }
    });
    for net in written {
        mark(net, &mut driven);
    }
    let mut touched = Vec::new();
    for (_, instance) in module.instances.iter() {
        for (_, expr) in &instance.connections {
            collect_net_ids(module, *expr, &mut touched);
        }
    }
    for net in touched {
        mark(net, &mut driven);
    }
    driven
}

/// Every net `id` reads, into `out`.
fn collect_net_ids(module: &Module, id: ExprId, out: &mut Vec<crate::ir::NetId>) {
    let mut stack = vec![id];
    while let Some(id) = stack.pop() {
        let Some(node) = module.exprs.get(id) else {
            continue;
        };
        if let Some(net) = node.as_net() {
            out.push(net);
            continue;
        }
        stack.extend(operands(&node.kind));
    }
}

fn clock_driven_nets(module: &Module) -> Vec<String> {
    let mut nets = Vec::new();
    for (_, cell) in module.cells.iter() {
        let port = match &cell.kind {
            CellKind::Dff { .. } => "clk",
            CellKind::MemRdPort { clocked: true, .. }
            | CellKind::MemWrPort { clocked: true, .. } => "clk",
            _ => continue,
        };
        if let Some(expr) = cell.input(port) {
            collect_net_names(module, expr, &mut nets);
        }
    }
    // A module that has not been through synthesis still carries its
    // clocks on the processes, so look there too.
    for (_, process) in module.processes.iter() {
        if let ProcessKind::Sequential { clocks, .. } = &process.kind {
            for edge in clocks {
                if let Some(net) = module.nets.get(edge.net) {
                    nets.push(net.name.as_str().to_owned());
                }
            }
        }
    }
    nets
}

/// Adds the name of every net `expr` reads to `out`.
fn collect_net_names(module: &Module, expr: ExprId, out: &mut Vec<String>) {
    let Some(node) = module.exprs.get(expr) else {
        return;
    };
    if let Some(net) = node.as_net()
        && let Some(net) = module.nets.get(net)
    {
        out.push(net.name.as_str().to_owned());
        return;
    }
    for operand in operands(&node.kind) {
        collect_net_names(module, operand, out);
    }
}

fn check_pattern(pattern: &str, names: &[String], what: &str, span: Span, diags: &mut Diagnostics) {
    if names.iter().any(|name| matches_glob(pattern, name)) {
        return;
    }
    diags.push(
        Diagnostic::warning(format!("no {what} matches `{pattern}`"))
            .with_code(NO_MATCH)
            .with_span(span),
    );
}

fn check_path(path: &PathSpec, names: &[String], diags: &mut Diagnostics) {
    for end in [&path.from, &path.to].into_iter().flatten() {
        check_pattern(end, names, "object", path.span, diags);
    }
}

/// Matches a glob with `*` (any run) and `?` (one character).
///
/// Anything else, `/` included, is matched literally, so `u_cpu/*` means
/// "everything under `u_cpu`" once hierarchical names exist.
pub fn matches_glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Iterative backtracking: linear in the common case, and no
    // recursion depth to worry about on long names.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Splits `d[3]` into the port name and the bit index.
fn split_port_bit(text: &str) -> (String, Option<u32>) {
    if let Some(open) = text.rfind('[')
        && text.ends_with(']')
        && let Ok(bit) = text[open + 1..text.len() - 1].parse::<u32>()
    {
        return (text[..open].to_owned(), Some(bit));
    }
    (text.to_owned(), None)
}

/// The first attribute present under any of `keys`, case-insensitively.
fn attr_lookup<'a>(attrs: &'a Attrs, keys: &[&str]) -> Option<&'a AttrValue> {
    attrs.iter().find_map(|(name, value)| {
        keys.iter()
            .any(|key| name.as_str().eq_ignore_ascii_case(key))
            .then_some(value)
    })
}

fn attr_str(attrs: &Attrs, keys: &[&str]) -> Option<String> {
    attr_lookup(attrs, keys).map(|v| match v {
        AttrValue::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn attr_u32(attrs: &Attrs, keys: &[&str]) -> Option<u32> {
    let value = attr_lookup(attrs, keys)?;
    match value {
        AttrValue::String(s) => s.parse().ok(),
        other => other.as_int().and_then(|v| u32::try_from(v).ok()),
    }
}

fn attr_f64(attrs: &Attrs, keys: &[&str]) -> Option<f64> {
    let value = attr_lookup(attrs, keys)?;
    match value {
        AttrValue::String(s) => s.parse().ok(),
        #[allow(clippy::cast_precision_loss)]
        other => other.as_int().map(|v| v as f64),
    }
}

fn attr_bool(attrs: &Attrs, keys: &[&str]) -> Option<bool> {
    let value = attr_lookup(attrs, keys)?;
    match value {
        AttrValue::String(s) => match s.to_ascii_lowercase().as_str() {
            "yes" | "true" | "on" | "up" | "1" => Some(true),
            "no" | "false" | "off" | "none" | "0" => Some(false),
            _ => None,
        },
        other => Some(other.is_truthy()),
    }
}

/// The IO options written as attributes on a port's net.
fn io_attrs_of(attrs: &Attrs) -> IoAttrs {
    IoAttrs {
        io_standard: attr_str(attrs, &["io_standard", "iostandard"]),
        drive: attr_u32(attrs, &["drive"]),
        slew: attr_str(attrs, &["slew", "slewrate"]).map(|s| s.to_ascii_lowercase()),
        pullup: attr_bool(attrs, &["pullup", "pullmode"]),
        ddr: attr_str(attrs, &["ddr", "ddr_clock"]),
        delay: attr_u32(attrs, &["io_delay", "delay_value"]),
    }
}

/// Parses an `rloc` attribute: `X1Y2` or `1,2`.
fn parse_rloc(text: &str) -> Option<(i32, i32)> {
    if let Some((x, y)) = text.split_once(',') {
        return Some((x.trim().parse().ok()?, y.trim().parse().ok()?));
    }
    let rest = text.strip_prefix('X').or_else(|| text.strip_prefix('x'))?;
    let (x, y) = rest.split_once(['Y', 'y'])?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// Parses one `.rcf` line.
struct LineParser<'a> {
    line: &'a Line,
    diags: &'a mut Diagnostics,
    failed: bool,
}

impl<'a> LineParser<'a> {
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.failed = true;
        self.diags.push(
            Diagnostic::error(message)
                .with_code(RCF_SYNTAX)
                .with_span(span),
        );
    }

    fn unknown(&mut self, span: Span, message: impl Into<String>) {
        self.failed = true;
        self.diags.push(
            Diagnostic::error(message)
                .with_code(RCF_UNKNOWN)
                .with_span(span),
        );
    }

    /// The options (`-key value`) and the positional words of the line.
    fn split(&mut self, known: &[&str], flags: &[&str]) -> (Vec<(String, Token)>, Vec<Token>) {
        let mut options = Vec::new();
        let mut positional = Vec::new();
        let mut index = 1;
        while let Some(token) = self.line.get(index) {
            index += 1;
            // `-2` is a negative number, not an option: an option's name
            // starts with a letter.
            let name = token
                .as_str()
                .strip_prefix('-')
                .filter(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic()));
            let Some(name) = name else {
                positional.push(token.clone());
                continue;
            };
            if flags.contains(&name) {
                options.push((name.to_owned(), token.clone()));
                continue;
            }
            if !known.contains(&name) {
                let span = token.span;
                self.unknown(span, format!("unknown option `-{name}`"));
                continue;
            }
            match self.line.get(index) {
                Some(value) => {
                    index += 1;
                    options.push((name.to_owned(), value.clone()));
                }
                None => {
                    let span = token.span;
                    self.error(span, format!("`-{name}` needs a value"));
                }
            }
        }
        (options, positional)
    }

    fn number(&mut self, token: &Token) -> Option<u32> {
        match token.as_str().parse::<u32>() {
            Ok(v) => Some(v),
            Err(_) => {
                let span = token.span;
                let text = token.as_str().to_owned();
                self.error(span, format!("expected a number, found `{text}`"));
                None
            }
        }
    }

    fn signed(&mut self, token: &Token) -> Option<i32> {
        match token.as_str().parse::<i32>() {
            Ok(v) => Some(v),
            Err(_) => {
                let span = token.span;
                let text = token.as_str().to_owned();
                self.error(span, format!("expected an offset, found `{text}`"));
                None
            }
        }
    }

    fn float(&mut self, token: &Token) -> Option<f64> {
        match token.as_str().parse::<f64>() {
            Ok(v) if v.is_finite() => Some(v),
            _ => {
                let span = token.span;
                let text = token.as_str().to_owned();
                self.error(span, format!("expected a number, found `{text}`"));
                None
            }
        }
    }

    fn expect_count(&mut self, words: &[Token], want: usize, what: &str) -> bool {
        if words.len() == want {
            return true;
        }
        let span = self.line.span;
        self.error(span, format!("expected {what}"));
        false
    }

    fn directive(&mut self, out: &mut Constraints) {
        match self.line.keyword() {
            "set_io" => self.set_io(out),
            "region" => self.region(out),
            "assign" => self.assign(out),
            "rloc" => self.rloc(out),
            "keep_hierarchy" => self.keep_hierarchy(out),
            "create_clock" => self.create_clock(out),
            "set_false_path" => {
                let (options, words) = self.split(&["from", "to"], &[]);
                if self.expect_count(&words, 0, "no arguments besides `-from` and `-to`") {
                    let span = self.line.span;
                    out.false_paths.push(path_spec(&options, span));
                }
            }
            "set_multicycle_path" => self.set_multicycle_path(out),
            "clock_domain" => self.clock_domain(out),
            other => {
                let span = self.line.tokens[0].span;
                self.unknown(span, format!("unknown constraint `{other}`"));
            }
        }
    }

    fn set_io(&mut self, out: &mut Constraints) {
        let (options, words) = self.split(
            &["io_standard", "drive", "slew", "pullup", "ddr", "delay"],
            &["nowarn"],
        );
        if !self.expect_count(&words, 2, "a port and a pin: `set_io <port> <pin>`") {
            return;
        }
        let mut io = IoAttrs::default();
        for (name, value) in &options {
            match name.as_str() {
                "io_standard" => io.io_standard = Some(value.as_str().to_owned()),
                "drive" => io.drive = self.number(value),
                "slew" => io.slew = Some(value.as_str().to_ascii_lowercase()),
                "ddr" => io.ddr = Some(value.as_str().to_owned()),
                "delay" => io.delay = self.number(value),
                "pullup" => {
                    io.pullup = match value.as_str() {
                        "yes" | "true" | "1" => Some(true),
                        "no" | "false" | "0" => Some(false),
                        other => {
                            let span = value.span;
                            self.error(span, format!("expected `yes` or `no`, found `{other}`"));
                            None
                        }
                    }
                }
                _ => {}
            }
        }
        let (port, bit) = split_port_bit(words[0].as_str());
        out.pins.push(PinAssignment {
            port,
            bit,
            pin: words[1].as_str().to_owned(),
            io,
            span: self.line.span,
            origin: Origin::File,
        });
    }

    fn region(&mut self, out: &mut Constraints) {
        let (_, words) = self.split(&[], &[]);
        if !self.expect_count(&words, 5, "`region <name> <x0> <y0> <x1> <y1>`") {
            return;
        }
        let name = words[0].as_str().to_owned();
        let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
            self.number(&words[1]),
            self.number(&words[2]),
            self.number(&words[3]),
            self.number(&words[4]),
        ) else {
            return;
        };
        out.regions.push(Region {
            name,
            x0,
            y0,
            x1,
            y1,
            span: self.line.span,
        });
    }

    fn assign(&mut self, out: &mut Constraints) {
        let (_, words) = self.split(&[], &[]);
        if !self.expect_count(&words, 3, "`assign <pattern> region <name>`") {
            return;
        }
        if !words[1].is("region") {
            let span = words[1].span;
            self.error(span, "expected `region`");
            return;
        }
        out.region_assignments.push(RegionAssignment {
            pattern: words[0].as_str().to_owned(),
            region: words[2].as_str().to_owned(),
            span: self.line.span,
            origin: Origin::File,
        });
    }

    fn rloc(&mut self, out: &mut Constraints) {
        let (_, words) = self.split(&[], &[]);
        if words.len() != 3 && words.len() != 5 {
            let span = self.line.span;
            self.error(span, "expected `rloc <pattern> <dx> <dy> [group <name>]`");
            return;
        }
        let (Some(dx), Some(dy)) = (self.signed(&words[1]), self.signed(&words[2])) else {
            return;
        };
        let mut group = None;
        if words.len() == 5 {
            if !words[3].is("group") {
                let span = words[3].span;
                self.error(span, "expected `group`");
                return;
            }
            group = Some(words[4].as_str().to_owned());
        }
        out.rlocs.push(Rloc {
            pattern: words[0].as_str().to_owned(),
            dx,
            dy,
            group,
            span: self.line.span,
            origin: Origin::File,
        });
    }

    fn keep_hierarchy(&mut self, out: &mut Constraints) {
        let (_, words) = self.split(&[], &[]);
        if !self.expect_count(&words, 1, "`keep_hierarchy <pattern>`") {
            return;
        }
        out.keep_hierarchy
            .push((words[0].as_str().to_owned(), self.line.span, Origin::File));
    }

    fn create_clock(&mut self, out: &mut Constraints) {
        let (options, words) = self.split(&["name", "period", "waveform"], &[]);
        if !self.expect_count(&words, 1, "`create_clock -period <ns> <net>`") {
            return;
        }
        let net = words[0].as_str().to_owned();
        let mut name = None;
        let mut period = None;
        for (option, value) in &options {
            match option.as_str() {
                "name" => name = Some(value.as_str().to_owned()),
                "period" => period = self.float(value),
                _ => {}
            }
        }
        let Some(period_ns) = period else {
            if !self.failed {
                let span = self.line.span;
                self.error(span, "`create_clock` needs `-period <ns>`");
            }
            return;
        };
        out.clocks.push(ClockDef {
            name: name.unwrap_or_else(|| net.clone()),
            net,
            period_ns,
            span: self.line.span,
            origin: Origin::File,
        });
    }

    fn set_multicycle_path(&mut self, out: &mut Constraints) {
        let (options, words) = self.split(&["from", "to"], &["setup", "hold"]);
        if !self.expect_count(&words, 1, "`set_multicycle_path <cycles> [options]`") {
            return;
        }
        let Some(cycles) = self.number(&words[0]) else {
            return;
        };
        let hold = options.iter().any(|(name, _)| name == "hold");
        let span = self.line.span;
        out.multicycle_paths.push(MulticyclePath {
            cycles,
            hold,
            path: path_spec(&options, span),
        });
    }

    fn clock_domain(&mut self, out: &mut Constraints) {
        let (_, words) = self.split(&[], &[]);
        if words.len() < 2 {
            let span = self.line.span;
            self.error(span, "expected `clock_domain <name> <net>...`");
            return;
        }
        out.clock_domains.push(ClockDomain {
            name: words[0].as_str().to_owned(),
            nets: words[1..].iter().map(|t| t.as_str().to_owned()).collect(),
            span: self.line.span,
        });
    }
}

fn path_spec(options: &[(String, Token)], span: Span) -> PathSpec {
    let find = |key: &str| {
        options
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str().to_owned())
    };
    PathSpec {
        from: find("from"),
        to: find("to"),
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::target;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{CellKind, Design, Name, Type};
    use crate::source::SourceMap;

    fn parse(text: &str) -> (Constraints, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let file = map.add("top.rcf", text).unwrap();
        let mut diags = Diagnostics::new();
        let constraints = Constraints::parse(text, file, &mut diags);
        (constraints, diags, map)
    }

    /// A design with a clock, a reset, an 8-bit output and one flip-flop.
    fn design() -> Design {
        let mut map = SourceMap::new();
        let file = map.add("top.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bits(8));
        let q = b.output("q", Type::bits(8));
        let unclocked = b.add_net("spare", Type::bit());
        let (clk_e, d_e) = (b.net(clk), b.net(d));
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clk_e), (Name::new("d"), d_e)],
            vec![(Name::new("q"), q)],
        );
        let _ = unclocked;
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);
        design
    }

    fn check(text: &str, device_name: &str) -> String {
        let (constraints, mut diags, map) = parse(text);
        let device = target(device_name).unwrap();
        constraints.check(&design(), device, &mut diags);
        diags.render(&map)
    }

    #[test]
    fn parses_every_directive() {
        let (c, diags, map) = parse(
            "\
# a comment
set_io -io_standard LVCMOS33 -drive 8 -slew FAST -pullup yes clk 21
set_io q[0] 99
region core 0 0 7 7
assign ff region core
rloc ff 1 -2 group adder
keep_hierarchy ff
create_clock -name sys -period 83.3 clk
clock_domain sys clk
set_false_path -from d -to q
set_multicycle_path 2 -hold -from d -to q
",
        );
        assert_eq!(diags.render(&map), "");
        assert_eq!(c.pins.len(), 2);
        let clk = &c.pins[0];
        assert_eq!(clk.port, "clk");
        assert_eq!(clk.pin, "21");
        assert_eq!(clk.io.io_standard.as_deref(), Some("LVCMOS33"));
        assert_eq!(clk.io.drive, Some(8));
        assert_eq!(clk.io.slew.as_deref(), Some("fast"));
        assert_eq!(clk.io.pullup, Some(true));
        assert_eq!(clk.signal(), "clk");
        assert_eq!(c.pins[1].bit, Some(0));
        assert_eq!(c.pins[1].signal(), "q[0]");
        assert_eq!(c.regions[0].x1, 7);
        assert_eq!(c.region_assignments[0].region, "core");
        assert_eq!((c.rlocs[0].dx, c.rlocs[0].dy), (1, -2));
        assert_eq!(c.rlocs[0].group.as_deref(), Some("adder"));
        assert!(c.keeps_hierarchy("ff"));
        assert!(!c.keeps_hierarchy("other"));
        assert_eq!(c.clocks[0].name, "sys");
        assert!((c.clocks[0].period_ns - 83.3).abs() < 1e-9);
        assert!((c.clocks[0].frequency_mhz() - 12.004801).abs() < 1e-5);
        assert_eq!(c.clock_domains[0].nets, vec!["clk"]);
        assert_eq!(c.false_paths[0].from.as_deref(), Some("d"));
        assert!(c.multicycle_paths[0].hold);
        assert_eq!(c.multicycle_paths[0].cycles, 2);
        assert!(!c.is_empty());
        assert_eq!(c.pin_of("clk", None).map(|p| p.pin.as_str()), Some("21"));
        assert_eq!(c.pin_of("q", Some(0)).map(|p| p.pin.as_str()), Some("99"));
        assert!(c.pin_of("nope", None).is_none());
        assert!(c.clock_of_net("clk").is_some());
    }

    #[test]
    fn reports_malformed_constraints() {
        let (_, diags, map) = parse(
            "\
set_io clk
set_io -nonsense x clk 21
set_io -drive two clk 21
set_io -pullup maybe clk 21
region core 0 0 7
assign ff pblock core
rloc ff 1
rloc ff 1 2 grp adder
create_clock clk
create_clock -period nope clk
set_multicycle_path -setup
clock_domain sys
frobnicate
",
        );
        let text = diags.render(&map);
        assert!(text.contains("expected a port and a pin"), "{text}");
        assert!(text.contains("unknown option `-nonsense`"), "{text}");
        assert!(text.contains("expected a number, found `two`"), "{text}");
        assert!(text.contains("expected `yes` or `no`"), "{text}");
        assert!(
            text.contains("`region <name> <x0> <y0> <x1> <y1>`"),
            "{text}"
        );
        assert!(text.contains("expected `region`"), "{text}");
        assert!(text.contains("expected `rloc <pattern>"), "{text}");
        assert!(text.contains("expected `group`"), "{text}");
        assert!(text.contains("needs `-period <ns>`"), "{text}");
        assert!(text.contains("expected a number, found `nope`"), "{text}");
        assert!(text.contains("expected `set_multicycle_path"), "{text}");
        assert!(text.contains("expected `clock_domain"), "{text}");
        assert!(text.contains("unknown constraint `frobnicate`"), "{text}");
    }

    #[test]
    fn checks_ports_and_pins() {
        let text = check("set_io nope 99\nset_io q[9] 99\n", "ice40-hx1k-tq144");
        assert!(text.contains("no port named `nope`"), "{text}");
        assert!(
            text.contains("port `q` has 8 bits, so there is no bit 9"),
            "{text}"
        );

        let text = check("set_io clk 4242\n", "ice40-hx1k-tq144");
        assert!(text.contains("has no pin named 4242"), "{text}");

        let text = check("set_io clk 99\nset_io d[0] 99\n", "ice40-hx1k-tq144");
        assert!(text.contains("pin 99 is assigned to two signals"), "{text}");

        let text = check("set_io clk 21\nset_io clk 99\n", "ice40-hx1k-tq144");
        assert!(
            text.contains("port `clk` is assigned a pin twice"),
            "{text}"
        );

        let text = check("set_io q[0] 99\n", "generic");
        assert!(
            text.contains("the database records no package pins"),
            "{text}"
        );
    }

    #[test]
    fn checks_io_standards_and_banks() {
        let text = check("set_io -io_standard LVCMOS99 clk 21\n", "ice40-hx1k-tq144");
        assert!(
            text.contains("does not offer the IO standard `LVCMOS99`"),
            "{text}"
        );
        assert!(text.contains("known standards: LVCMOS33"), "{text}");

        let text = check(
            "set_io -io_standard LVCMOS33 -drive 9 clk 21\n",
            "ecp5-25f-CABGA381",
        );
        assert!(text.contains("drive strength of 9 mA"), "{text}");
        assert!(text.contains("available: 4, 8, 12, 16, 20"), "{text}");

        let text = check(
            "set_io -io_standard LVCMOS33 -slew medium clk 21\n",
            "ecp5-25f-CABGA381",
        );
        assert!(text.contains("slew rate `medium`"), "{text}");

        // Two standards needing different bank voltages on one bank.
        let text = check(
            "set_io -io_standard LVCMOS33 clk 21\nset_io -io_standard LVCMOS18 q[0] 99\n",
            "ice40-hx1k-tq144",
        );
        assert!(text.contains("would need 1.8 V"), "{text}");
        assert!(text.contains("shares one supply voltage"), "{text}");

        // The same standard twice is fine.
        let text = check(
            "set_io -io_standard LVCMOS33 clk 21\nset_io -io_standard LVCMOS33 q[0] 99\n",
            "ice40-hx1k-tq144",
        );
        assert_eq!(text, "");
    }

    #[test]
    fn checks_placement_and_timing() {
        let text = check(
            "region r 0 0 99 99\nassign ff region r\n",
            "ice40-hx1k-tq144",
        );
        assert!(text.contains("outside the 14x18 grid"), "{text}");

        let text = check("region r 4 4 1 1\n", "ice40-hx1k-tq144");
        assert!(text.contains("region `r` is empty"), "{text}");

        let text = check("region r 0 0 1 1\nregion r 0 0 2 2\n", "ice40-hx1k-tq144");
        assert!(text.contains("region `r` is declared twice"), "{text}");

        let text = check("assign ff region nope\n", "ice40-hx1k-tq144");
        assert!(text.contains("no region named `nope`"), "{text}");

        let text = check(
            "region r 0 0 1 1\nassign nothing* region r\n",
            "ice40-hx1k-tq144",
        );
        assert!(text.contains("no instance matches `nothing*`"), "{text}");

        let text = check("region r 0 0 1 1\n", "ecp5-25f-CABGA381");
        assert!(text.contains("records no tile grid"), "{text}");

        let text = check("create_clock -period 10 nope\n", "generic");
        assert!(text.contains("no net named `nope`"), "{text}");

        let text = check("create_clock -period 10 spare\n", "generic");
        assert!(text.contains("does not clock anything"), "{text}");

        let text = check("create_clock -period 0 clk\n", "generic");
        assert!(text.contains("has a period of 0 ns"), "{text}");

        let text = check(
            "create_clock -name a -period 10 clk\ncreate_clock -name a -period 10 clk\n",
            "generic",
        );
        assert!(text.contains("clock `a` is defined twice"), "{text}");

        let text = check("set_multicycle_path 0 -from d -to q\n", "generic");
        assert!(text.contains("must span at least one cycle"), "{text}");

        let text = check("set_false_path -from nowhere\n", "generic");
        assert!(text.contains("no object matches `nowhere`"), "{text}");

        let text = check("clock_domain sys nope\n", "generic");
        assert!(text.contains("no net named `nope`"), "{text}");

        let text = check("keep_hierarchy nope*\n", "generic");
        assert!(text.contains("no instance matches `nope*`"), "{text}");

        // A well formed set passes silently.
        let text = check(
            "set_io clk 21\nregion r 0 0 3 3\nassign ff region r\ncreate_clock -period 10 clk\n",
            "ice40-hx1k-tq144",
        );
        assert_eq!(text, "");
    }

    #[test]
    fn checks_need_a_top_module() {
        let mut design = Design::new();
        design.add_module(crate::ir::Module::new(
            "m",
            Span::new(SourceMap::new().add("x", "").unwrap(), 0, 0),
        ));
        let mut diags = Diagnostics::new();
        Constraints::new().check(&design, target("generic").unwrap(), &mut diags);
        assert_eq!(diags.error_count(), 1);
    }

    #[test]
    fn attributes_merge_behind_the_file() {
        let mut map = SourceMap::new();
        let file = map.add("top.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        b.net_attr(clk, "PIN", "21");
        b.net_attr(clk, "io_standard", "LVCMOS33");
        b.net_attr(clk, "clock_period", "20.0");
        let q = b.output("q", Type::bits(1));
        b.net_attr(q, "pin", "99");
        b.net_attr(q, "drive", 8);
        b.net_attr(q, "pullup", "yes");
        let e = b.net(clk);
        let cell = b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), e), (Name::new("d"), e)],
            vec![(Name::new("q"), q)],
        );
        b.module_mut().cells[cell].attrs.set("keep_hierarchy", 1);
        b.module_mut().cells[cell].attrs.set("rloc", "X1Y2");
        b.module_mut().cells[cell].attrs.set("region", "core");
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        design.top = Some(top);

        // The file assigns `clk` elsewhere and says nothing about `q`.
        let rcf = "set_io clk 98\n";
        let rcf_file = map.add("top.rcf", rcf).unwrap();
        let mut diags = Diagnostics::new();
        let mut constraints = Constraints::parse(rcf, rcf_file, &mut diags);
        constraints.merge_attrs(&design, top, &mut diags);

        let clk_pin = constraints.pin_of("clk", None).unwrap();
        assert_eq!(clk_pin.pin, "98", "the file wins");
        assert_eq!(
            clk_pin.io.io_standard.as_deref(),
            Some("LVCMOS33"),
            "options the file leaves open come from the attributes"
        );
        let q_pin = constraints.pin_of("q", None).unwrap();
        assert_eq!(q_pin.pin, "99");
        assert_eq!(q_pin.io.drive, Some(8));
        assert_eq!(q_pin.io.pullup, Some(true));
        assert_eq!(q_pin.origin, Origin::Attribute);
        assert!(constraints.keeps_hierarchy("ff"));
        assert_eq!((constraints.rlocs[0].dx, constraints.rlocs[0].dy), (1, 2));
        assert_eq!(constraints.region_assignments[0].region, "core");
        assert!((constraints.clocks[0].period_ns - 20.0).abs() < 1e-9);
        // The override is reported, as a note rather than an error.
        let text = diags.render(&map);
        assert!(text.contains("overriding pin 21"), "{text}");
        assert_eq!(diags.error_count(), 0);
    }

    #[test]
    fn writes_vendor_formats() {
        let text = "\
set_io -io_standard LVCMOS33 -drive 8 -slew fast -pullup yes clk 21
set_io q[0] 99
region core 0 0 7 7
assign ff region core
rloc ff 1 2 group adder
keep_hierarchy ff
create_clock -name sys -period 40.0 clk
set_false_path -from d -to q
set_multicycle_path 2 -from d -to q
";
        let (c, _, _) = parse(text);
        let device = target("ice40-hx1k-tq144").unwrap();
        let pcf = c.write_pcf(device);
        assert!(pcf.contains("set_io -pullup yes clk 21\n"), "{pcf}");
        assert!(pcf.contains("set_io q[0] 99\n"), "{pcf}");
        assert!(pcf.contains("set_frequency clk 25.000\n"), "{pcf}");
        assert!(pcf.contains("not expressible"), "{pcf}");

        let lpf = c.write_lpf(target("ecp5-25f-CABGA381").unwrap());
        assert!(lpf.contains("LOCATE COMP \"clk\" SITE \"21\";"), "{lpf}");
        assert!(
            lpf.contains("IOBUF PORT \"clk\" IO_TYPE=LVCMOS33 DRIVE=8 SLEWRATE=FAST PULLMODE=UP;"),
            "{lpf}"
        );
        assert!(lpf.contains("FREQUENCY PORT \"clk\" 25.000 MHZ;"), "{lpf}");

        let xdc = c.write_xdc(device);
        assert!(
            xdc.contains("set_property PACKAGE_PIN 21 [get_ports {clk}]"),
            "{xdc}"
        );
        assert!(xdc.contains("set_property IOSTANDARD LVCMOS33 [get_ports {clk}]"));
        assert!(xdc.contains("set_property DRIVE 8 [get_ports {clk}]"));
        assert!(xdc.contains("set_property SLEW FAST [get_ports {clk}]"));
        assert!(xdc.contains("set_property PULLUP TRUE [get_ports {clk}]"));
        assert!(xdc.contains("create_pblock core"));
        assert!(xdc.contains("-add {SLICE_X0Y0:SLICE_X7Y7}"));
        assert!(xdc.contains("add_cells_to_pblock [get_pblocks core] [get_cells {ff}]"));
        assert!(xdc.contains("set_property RLOC X1Y2 [get_cells {ff}]"));
        assert!(xdc.contains("set_property U_SET adder [get_cells {ff}]"));
        assert!(xdc.contains("set_property KEEP_HIERARCHY TRUE [get_cells {ff}]"));
        assert!(xdc.contains("create_clock -name sys -period 40.000 [get_ports {clk}]"));
        assert!(xdc.contains("set_false_path -from [get_cells {d}] -to [get_ports {q}]"));
        assert!(xdc.contains("set_multicycle_path 2 -setup -from [get_cells {d}]"));

        let sdc = c.write_sdc(device);
        assert!(
            sdc.contains("create_clock -name sys -period 40.000"),
            "{sdc}"
        );
        assert!(!sdc.contains("PACKAGE_PIN"), "{sdc}");
        assert!(
            sdc.contains("not expressible in this format: 1 placement region(s)"),
            "{sdc}"
        );
    }

    #[test]
    fn empty_constraints_write_only_a_header() {
        let device = target("generic").unwrap();
        let c = Constraints::new();
        assert!(c.is_empty());
        assert_eq!(c.write_pcf(device).lines().count(), 1);
        assert_eq!(c.write_lpf(device).lines().count(), 1);
        assert_eq!(c.write_xdc(device).lines().count(), 1);
        assert_eq!(c.write_sdc(device).lines().count(), 1);
    }

    #[test]
    fn globs_match_like_shells() {
        assert!(matches_glob("*", "anything"));
        assert!(matches_glob("u_*", "u_cpu"));
        assert!(matches_glob("u_cpu/*", "u_cpu/alu"));
        assert!(matches_glob("*_reg", "count_reg"));
        assert!(matches_glob("a?c", "abc"));
        assert!(matches_glob("a*b*c", "axxbyyc"));
        assert!(matches_glob("exact", "exact"));
        assert!(!matches_glob("exact", "exac"));
        assert!(!matches_glob("a?c", "ac"));
        assert!(!matches_glob("u_*", "v_cpu"));
        assert!(!matches_glob("a*b", "axxc"));
        assert!(matches_glob("**", "x"));
    }

    #[test]
    fn helpers_parse_their_shapes() {
        assert_eq!(split_port_bit("d[3]"), ("d".to_owned(), Some(3)));
        assert_eq!(split_port_bit("d"), ("d".to_owned(), None));
        assert_eq!(split_port_bit("d[x]"), ("d[x]".to_owned(), None));
        assert_eq!(parse_rloc("X1Y2"), Some((1, 2)));
        assert_eq!(parse_rloc("x1y2"), Some((1, 2)));
        assert_eq!(parse_rloc("3,-4"), Some((3, -4)));
        assert_eq!(parse_rloc("nonsense"), None);
        assert_eq!(Origin::File.describe(), "the constraints file");
        let mut io = IoAttrs::default();
        assert!(io.is_empty());
        io.fill_from(&IoAttrs {
            io_standard: Some("LVCMOS33".into()),
            drive: Some(4),
            slew: Some("fast".into()),
            pullup: Some(false),
            ddr: Some("clk".into()),
            delay: Some(12),
        });
        assert_eq!(io.ddr.as_deref(), Some("clk"));
        assert_eq!(io.delay, Some(12));
        assert_eq!(io.drive, Some(4));
        assert!(!io.is_empty());
    }
}
