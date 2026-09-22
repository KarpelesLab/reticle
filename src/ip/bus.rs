//! Bus interfaces: described once, then generated and checked.
//!
//! An AXI4-Lite subordinate has nineteen ports. Written by hand they are
//! nineteen chances to swap `awready` for `awvalid`, to give `wstrb` the
//! data width instead of an eighth of it, or to make an output an input.
//! Instantiating one costs nineteen more lines, and connecting two costs
//! nineteen nets. This module removes all of that by describing a bus
//! *once*, as data.
//!
//! # The model
//!
//! A [`BusInterface`] is a name, a list of parameters with defaults, and
//! a list of [`BusSignal`]s. Each signal has a name, the direction it has
//! **seen from the manager**, a [`Width`] that is either a number or a
//! parameter (possibly divided, which is how `wstrb` is `DATA_WIDTH/8`),
//! and whether it is required. A [`BusRole`] then decides what the
//! directions mean on a particular module: a [`BusRole::Manager`] takes
//! them as written, a [`BusRole::Subordinate`] mirrors them, and a
//! [`BusRole::Monitor`] reads everything.
//!
//! Clocks and resets are deliberately not part of a bus definition: one
//! clock usually serves several interfaces, so an IP declares it with a
//! `port` line in its manifest.
//!
//! # The definitions are data
//!
//! The built-in buses live in `src/ip/buses/*.bus`, in the same
//! line-oriented format as the manifests, and are pulled in with
//! `include_str!`:
//!
//! ```text
//! bus axi4lite
//!   param ADDR_WIDTH 32
//!   param DATA_WIDTH 32
//!   signal awaddr out ADDR_WIDTH
//!   signal awprot out 3 optional
//!   signal awvalid out 1
//!   signal awready in 1
//!   ...
//! end
//! ```
//!
//! AXI4, AXI4-Lite, AXI4-Stream, Wishbone (classic and pipelined), APB
//! and Avalon-MM ship this way. A user who needs another bus writes
//! another such file and calls [`BusInterface::parse_all`]; no Rust
//! changes are needed, which is the point.
//!
//! # Recognising an interface
//!
//! [`match_ports`] takes a module, a bus, a role and a port-name prefix,
//! and reports what it finds: a [`PortMapping`] naming the real port
//! behind every signal, or a list of [`BusProblem`]s. Matching is by
//! naming convention — prefix plus the standard signal name, compared
//! without regard to case — and the checks are the three that actually
//! catch bugs:
//!
//! - a required signal with no port (`P0201`),
//! - a port whose direction is the wrong way round (`P0202`),
//! - a port whose width disagrees with the parameters (`P0203`).
//!
//! Widths are *inferred* where the module does not declare the
//! parameter: the first port that pins `DATA_WIDTH` down binds it, and
//! every later signal is checked against that binding. A 32-bit `wdata`
//! with a 2-bit `wstrb` is therefore reported even when the module never
//! says what `DATA_WIDTH` is.
//!
//! # Wiring two instances
//!
//! [`connect`] matches the interface on both ends, pairs the signals up
//! and returns the [`Connection`]s that join them; [`wire`] applies them,
//! creating one net per signal and adding the port connections to both
//! instances. Forty port lines become one call.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use super::text::{Line, closest, quote, tokenize};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::builder::ModuleBuilder;
use crate::ir::{
    Design, ExprKind, Instance, InstanceId, Module, ModuleId, Name, Net, NetKind, Port, PortDir,
    Type,
};
use crate::source::{SourceId, Span};

/// Diagnostic code for a required bus signal with no matching port.
pub const MISSING_SIGNAL: &str = "P0201";
/// Diagnostic code for a bus port declared in the wrong direction.
pub const BAD_DIRECTION: &str = "P0202";
/// Diagnostic code for a bus port whose width contradicts the parameters.
pub const BAD_WIDTH: &str = "P0203";
/// Diagnostic code for a reference to a bus nobody has defined.
pub const NO_SUCH_BUS: &str = "P0204";
/// Diagnostic code for a malformed line in a `.bus` file.
pub const BUS_SYNTAX: &str = "P0205";

// ---------------------------------------------------------------------------
// Roles and widths
// ---------------------------------------------------------------------------

/// Which side of a bus a module is.
///
/// The words are AMBA's fifth-revision vocabulary. "Master" and "slave"
/// are not accepted: a format that has to be read by everyone is a bad
/// place to keep the old spelling alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BusRole {
    /// Starts transactions: the directions of the definition as written.
    Manager,
    /// Answers them: every direction mirrored.
    Subordinate,
    /// Watches: every signal an input, nothing driven.
    Monitor,
}

impl BusRole {
    /// The three roles, in the order the keyword table lists them.
    pub const ALL: [BusRole; 3] = [BusRole::Manager, BusRole::Subordinate, BusRole::Monitor];

    /// The words a manifest writes, for a "did you mean" note.
    pub const KEYWORDS: [&'static str; 3] = ["manager", "subordinate", "monitor"];

    /// The word used in a manifest.
    pub fn keyword(self) -> &'static str {
        match self {
            BusRole::Manager => "manager",
            BusRole::Subordinate => "subordinate",
            BusRole::Monitor => "monitor",
        }
    }

    /// The role named by `word`.
    pub fn from_keyword(word: &str) -> Option<BusRole> {
        BusRole::ALL.into_iter().find(|r| r.keyword() == word)
    }

    /// The other side of the bus; a monitor has no other side.
    pub fn opposite(self) -> BusRole {
        match self {
            BusRole::Manager => BusRole::Subordinate,
            BusRole::Subordinate => BusRole::Manager,
            BusRole::Monitor => BusRole::Monitor,
        }
    }
}

impl fmt::Display for BusRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// How wide a bus signal is.
///
/// Either a fixed number, or a parameter of the interface, optionally
/// divided by a constant. The division is not a general expression on
/// purpose: `DATA_WIDTH/8` is the one shape real buses need (byte
/// strobes), and a format with arithmetic in it is a format with a parser
/// in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    /// A literal number of bits.
    Fixed(u32),
    /// `PARAM` or `PARAM/divisor`, rounded up when it does not divide.
    Derived {
        /// The parameter's name.
        param: String,
        /// What the parameter is divided by; `1` for a plain reference.
        divisor: u32,
    },
}

impl Width {
    /// A plain reference to a parameter.
    pub fn param(name: impl Into<String>) -> Width {
        Width::Derived {
            param: name.into(),
            divisor: 1,
        }
    }

    /// Parses `8`, `DATA_WIDTH` or `DATA_WIDTH/8`.
    ///
    /// ```
    /// use reticle::ip::Width;
    /// assert_eq!(Width::parse("8"), Some(Width::Fixed(8)));
    /// assert_eq!(Width::parse("W"), Some(Width::param("W")));
    /// assert_eq!(Width::parse("W/0"), None);
    /// ```
    pub fn parse(text: &str) -> Option<Width> {
        if let Ok(n) = text.parse::<u32>() {
            return Some(Width::Fixed(n));
        }
        let (name, divisor) = match text.split_once('/') {
            Some((name, d)) => (name, d.parse::<u32>().ok()?),
            None => (text, 1),
        };
        if divisor == 0 || name.is_empty() {
            return None;
        }
        let mut chars = name.chars();
        let first = chars.next()?;
        if !(first.is_ascii_alphabetic() || first == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        Some(Width::Derived {
            param: name.to_owned(),
            divisor,
        })
    }

    /// The width in bits given the parameter bindings, if they suffice.
    pub fn resolve(&self, bindings: &BTreeMap<String, u32>) -> Option<u32> {
        match self {
            Width::Fixed(n) => Some(*n),
            Width::Derived { param, divisor } => {
                Some(bindings.get(param)?.div_ceil(*divisor).max(1))
            }
        }
    }

    /// The parameter this width reads, if any.
    pub fn parameter(&self) -> Option<&str> {
        match self {
            Width::Fixed(_) => None,
            Width::Derived { param, .. } => Some(param),
        }
    }

    /// The value the parameter must have for this width to be `bits`.
    ///
    /// This is how a bus width is inferred from the ports rather than
    /// declared: an 8-bit `wstrb` means `DATA_WIDTH` is 64.
    fn implied_binding(&self, bits: u32) -> Option<(String, u32)> {
        match self {
            Width::Fixed(_) => None,
            Width::Derived { param, divisor } => Some((param.clone(), bits * divisor)),
        }
    }
}

impl fmt::Display for Width {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Width::Fixed(n) => write!(f, "{n}"),
            Width::Derived { param, divisor } if *divisor == 1 => f.write_str(param),
            Width::Derived { param, divisor } => write!(f, "{param}/{divisor}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Interfaces
// ---------------------------------------------------------------------------

/// One signal of a bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusSignal {
    /// The signal's name, without any prefix.
    pub name: String,
    /// Its direction **as the manager sees it**.
    pub direction_for_manager: PortDir,
    /// How wide it is.
    pub width: Width,
    /// False when a module may leave the signal out.
    pub required: bool,
}

impl BusSignal {
    /// The direction this signal has on a module playing `role`.
    pub fn direction(&self, role: BusRole) -> PortDir {
        match role {
            BusRole::Manager => self.direction_for_manager,
            BusRole::Subordinate => match self.direction_for_manager {
                PortDir::In => PortDir::Out,
                PortDir::Out => PortDir::In,
                PortDir::InOut => PortDir::InOut,
            },
            BusRole::Monitor => PortDir::In,
        }
    }
}

/// A whole bus: a name, its parameters and its signals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusInterface {
    /// The bus's name, as a manifest's `interface` line writes it.
    pub name: String,
    /// The parameters and their defaults, in declaration order.
    pub params: Vec<(String, u32)>,
    /// The signals, in declaration order.
    pub signals: Vec<BusSignal>,
}

impl BusInterface {
    /// An empty bus, for building one from Rust.
    pub fn new(name: impl Into<String>) -> Self {
        BusInterface {
            name: name.into(),
            params: Vec::new(),
            signals: Vec::new(),
        }
    }

    /// The signal with the given name.
    pub fn signal(&self, name: &str) -> Option<&BusSignal> {
        self.signals.iter().find(|s| s.name == name)
    }

    /// The parameter defaults as a binding map.
    pub fn defaults(&self) -> BTreeMap<String, u32> {
        self.params.iter().cloned().collect()
    }

    /// The port name a signal has under `prefix`.
    pub fn port_name(&self, prefix: &str, signal: &str) -> String {
        format!("{prefix}{signal}")
    }

    /// Parses every `bus ... end` block in a `.bus` file.
    ///
    /// Malformed lines are reported into `diags` and skipped, so one bad
    /// line does not lose the file.
    pub fn parse_all(text: &str, file: SourceId, diags: &mut Diagnostics) -> Vec<BusInterface> {
        let mut out: Vec<BusInterface> = Vec::new();
        let mut current: Option<BusInterface> = None;
        for line in tokenize(text, file) {
            match line.keyword() {
                "bus" => {
                    if let Some(open) = current.take() {
                        // A missing `end` is not worth losing the block
                        // over; say so and keep what was read.
                        diags.push(
                            Diagnostic::error(format!("bus `{}` has no `end`", open.name))
                                .with_code(BUS_SYNTAX)
                                .with_span(line.span),
                        );
                        out.push(open);
                    }
                    match line.args() {
                        [name] => current = Some(BusInterface::new(name.as_str())),
                        _ => shape(diags, &line, "`bus <name>`"),
                    }
                }
                "end" => match current.take() {
                    Some(bus) => out.push(bus),
                    None => shape(diags, &line, "a `bus` block to close"),
                },
                "param" => {
                    let Some(bus) = current.as_mut() else {
                        shape(diags, &line, "to be inside a `bus` block");
                        continue;
                    };
                    match line.args() {
                        [name, value] => match value.as_str().parse::<u32>() {
                            Ok(v) => bus.params.push((name.as_str().to_owned(), v)),
                            Err(_) => shape(diags, &line, "`param <name> <default>`"),
                        },
                        _ => shape(diags, &line, "`param <name> <default>`"),
                    }
                }
                "signal" => {
                    let Some(bus) = current.as_mut() else {
                        shape(diags, &line, "to be inside a `bus` block");
                        continue;
                    };
                    if let Some(signal) = parse_signal(&line, diags) {
                        bus.signals.push(signal);
                    }
                }
                other => {
                    let mut d = Diagnostic::error(format!("unknown bus directive `{other}`"))
                        .with_code(BUS_SYNTAX)
                        .with_span(line.keyword_span());
                    if let Some(s) = closest(other, &["bus", "param", "signal", "end"]) {
                        d = d.with_note(format!("did you mean `{s}`?"));
                    }
                    diags.push(d);
                }
            }
        }
        if let Some(open) = current {
            diags.push(
                Diagnostic::error(format!("bus `{}` has no `end`", open.name))
                    .with_code(BUS_SYNTAX)
                    .with_note("add `end` after the last signal"),
            );
            out.push(open);
        }
        out
    }

    /// Renders the bus in the canonical form, which [`parse_all`] reads
    /// back unchanged.
    ///
    /// [`parse_all`]: BusInterface::parse_all
    pub fn to_text(&self) -> String {
        let mut out = format!("bus {}\n", quote(&self.name));
        for (name, value) in &self.params {
            out.push_str(&format!("  param {} {value}\n", quote(name)));
        }
        for signal in &self.signals {
            out.push_str(&format!(
                "  signal {} {} {}{}\n",
                quote(&signal.name),
                signal.direction_for_manager.keyword(),
                signal.width,
                if signal.required { "" } else { " optional" }
            ));
        }
        out.push_str("end\n");
        out
    }
}

fn shape(diags: &mut Diagnostics, line: &Line, expected: &str) {
    diags.push(
        Diagnostic::error(format!("`{}` takes {expected}", line.keyword()))
            .with_code(BUS_SYNTAX)
            .with_span(line.span),
    );
}

fn parse_signal(line: &Line, diags: &mut Diagnostics) -> Option<BusSignal> {
    let args = line.args();
    if args.len() < 3 || args.len() > 4 {
        shape(
            diags,
            line,
            "`signal <name> <in|out|inout> <width> [optional]`",
        );
        return None;
    }
    let Some(dir) = PortDir::from_keyword(args[1].as_str()) else {
        shape(diags, line, "a direction of `in`, `out` or `inout`");
        return None;
    };
    let Some(width) = Width::parse(args[2].as_str()) else {
        shape(diags, line, "a width of `n`, `PARAM` or `PARAM/n`");
        return None;
    };
    let mut required = true;
    if let Some(flag) = args.get(3) {
        if !flag.is("optional") {
            shape(diags, line, "`optional` as its last word, or nothing");
            return None;
        }
        required = false;
    }
    Some(BusSignal {
        name: args[0].as_str().to_owned(),
        direction_for_manager: dir,
        width,
        required,
    })
}

/// The built-in bus definition files, as `(name, contents)` pairs.
///
/// The name is only used in diagnostics, which a well-formed file never
/// produces. Adding a family means adding a file and a line here — or, if
/// it is not everyone's bus, parsing your own file with
/// [`BusInterface::parse_all`] and never touching this crate.
pub const BUILTIN_FILES: [(&str, &str); 4] = [
    ("axi.bus", include_str!("buses/axi.bus")),
    ("wishbone.bus", include_str!("buses/wishbone.bus")),
    ("apb.bus", include_str!("buses/apb.bus")),
    ("avalon.bus", include_str!("buses/avalon.bus")),
];

static BUILTINS: OnceLock<Vec<BusInterface>> = OnceLock::new();

/// Every bus compiled into the crate, in file order.
///
/// The files are parsed once, on first use. They are constants checked by
/// the test suite, so a parse error in one is an internal invariant
/// violation and panics rather than producing a half-built table.
pub fn builtin_buses() -> &'static [BusInterface] {
    BUILTINS.get_or_init(|| {
        let mut map = crate::source::SourceMap::new();
        let mut diags = Diagnostics::new();
        let mut out = Vec::new();
        for (name, text) in BUILTIN_FILES {
            let file = map
                .add(name, text)
                .expect("built-in bus file fits in a source map");
            out.extend(BusInterface::parse_all(text, file, &mut diags));
        }
        assert!(
            !diags.has_errors(),
            "built-in bus definitions are malformed:\n{}",
            diags.render(&map)
        );
        out
    })
}

/// The built-in bus with the given name.
///
/// ```
/// let axi = reticle::ip::bus::builtin("axi4lite").unwrap();
/// assert_eq!(axi.signals.len(), 19);
/// assert!(reticle::ip::bus::builtin("spacewire").is_none());
/// ```
pub fn builtin(name: &str) -> Option<&'static BusInterface> {
    builtin_buses().iter().find(|b| b.name == name)
}

/// A diagnostic for a bus name nothing defines, with a "did you mean"
/// over the built-ins.
pub fn no_such_bus(name: &str, span: Span) -> Diagnostic {
    let names: Vec<&str> = builtin_buses().iter().map(|b| b.name.as_str()).collect();
    let mut d = Diagnostic::error(format!("unknown bus `{name}`"))
        .with_code(NO_SUCH_BUS)
        .with_span(span);
    if let Some(s) = closest(name, &names) {
        d = d.with_note(format!("did you mean `{s}`?"));
    } else {
        d = d.with_note(format!("the built-in buses are: {}", names.join(", ")));
    }
    d
}

// ---------------------------------------------------------------------------
// Matching an interface against a module
// ---------------------------------------------------------------------------

/// One signal matched to one real port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalMapping {
    /// The bus signal's name.
    pub signal: String,
    /// The module port carrying it.
    pub port: String,
    /// Its width in bits.
    pub width: u32,
    /// Its direction on this module.
    pub dir: PortDir,
}

/// What [`match_ports`] found: every signal of a bus on one module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortMapping {
    /// The bus's name.
    pub bus: String,
    /// The role the module plays.
    pub role: BusRole,
    /// The prefix its port names carry.
    pub prefix: String,
    /// The signals that are present, in the bus's declaration order.
    pub signals: Vec<SignalMapping>,
    /// What each bus parameter turned out to be, whether the module
    /// declared it or the port widths implied it.
    pub bindings: BTreeMap<String, u32>,
}

impl PortMapping {
    /// The mapping for one signal, if it is present.
    pub fn signal(&self, name: &str) -> Option<&SignalMapping> {
        self.signals.iter().find(|s| s.signal == name)
    }

    /// The module port carrying one signal.
    pub fn port(&self, signal: &str) -> Option<&str> {
        self.signal(signal).map(|s| s.port.as_str())
    }
}

/// Something wrong with a module's implementation of a bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BusProblem {
    /// A required signal has no port.
    Missing {
        /// The bus's name.
        bus: String,
        /// The signal that is absent.
        signal: String,
        /// The port name that was looked for.
        port: String,
        /// Where to point: the module's own span.
        span: Span,
    },
    /// A port exists but points the wrong way.
    Direction {
        /// The bus's name.
        bus: String,
        /// The signal.
        signal: String,
        /// The port.
        port: String,
        /// The direction the module declares.
        found: PortDir,
        /// The direction the role requires.
        expected: PortDir,
        /// The port's span.
        span: Span,
    },
    /// A port's width contradicts the bus parameters.
    Width {
        /// The bus's name.
        bus: String,
        /// The signal.
        signal: String,
        /// The port.
        port: String,
        /// The width the module declares.
        found: u32,
        /// The width the parameters require.
        expected: u32,
        /// How that width was arrived at, for the note.
        because: String,
        /// The port's span.
        span: Span,
    },
    /// An instance whose module the design does not contain, so nothing
    /// can be checked.
    Unresolved {
        /// The instance's name.
        instance: String,
        /// The module it names.
        module: String,
        /// The instance's span.
        span: Span,
    },
}

impl BusProblem {
    /// The rustc-style diagnostic for this problem.
    pub fn diagnostic(&self) -> Diagnostic {
        match self {
            BusProblem::Missing {
                bus,
                signal,
                port,
                span,
            } => Diagnostic::error(format!("`{bus}` needs a port `{port}`"))
                .with_code(MISSING_SIGNAL)
                .with_span(*span)
                .with_note(format!("it carries the required signal `{signal}`")),
            BusProblem::Direction {
                bus,
                signal,
                port,
                found,
                expected,
                span,
            } => Diagnostic::error(format!(
                "`{port}` is `{}` but `{bus}` needs `{}`",
                found.keyword(),
                expected.keyword()
            ))
            .with_code(BAD_DIRECTION)
            .with_span(*span)
            .with_note(format!("`{signal}` is driven by the other side")),
            BusProblem::Width {
                bus,
                signal,
                port,
                found,
                expected,
                because,
                span,
            } => Diagnostic::error(format!(
                "`{port}` is {found} bits wide but `{bus}` needs {expected}"
            ))
            .with_code(BAD_WIDTH)
            .with_span(*span)
            .with_note(format!("`{signal}` is {because} bits wide")),
            BusProblem::Unresolved {
                instance,
                module,
                span,
            } => Diagnostic::error(format!(
                "instance `{instance}` has no module `{module}` in the design"
            ))
            .with_code(NO_SUCH_BUS)
            .with_span(*span)
            .with_note("a black box cannot be checked against a bus"),
        }
    }
}

impl fmt::Display for BusProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostic().message)
    }
}

/// Recognises `interface` on `module`'s ports under `prefix`.
///
/// Port names are matched case-insensitively, so `S_AXI_AWVALID` and
/// `s_axi_awvalid` are the same port. Parameter widths come from the
/// module's own [`crate::ir::Param`]s where it declares them, and are
/// otherwise inferred from the first port that pins each one down.
///
/// ```
/// use reticle::ip::bus::{self, BusRole};
/// use reticle::ir::builder::ModuleBuilder;
/// use reticle::ir::{PortDir, Type};
/// use reticle::source::{Span, SourceMap};
///
/// let mut map = SourceMap::new();
/// let span = Span::new(map.add("x", "").unwrap(), 0, 0);
/// let mut b = ModuleBuilder::new("apb_gpio", span);
/// let apb = bus::builtin("apb").unwrap();
/// for signal in &apb.signals {
///     let width = signal.width.resolve(&apb.defaults()).unwrap();
///     let name = format!("s_{}", signal.name);
///     match signal.direction(BusRole::Subordinate) {
///         PortDir::In => { b.input(name, Type::bits(width)); }
///         _ => { b.output(name, Type::bits(width)); }
///     }
/// }
/// let module = b.finish();
/// let mapping = bus::match_ports(&module, apb, BusRole::Subordinate, "s_").unwrap();
/// assert_eq!(mapping.port("pwdata"), Some("s_pwdata"));
/// assert_eq!(mapping.bindings["DATA_WIDTH"], 32);
/// ```
pub fn match_ports(
    module: &Module,
    interface: &BusInterface,
    role: BusRole,
    prefix: &str,
) -> Result<PortMapping, Vec<BusProblem>> {
    // Ports by lower-cased name. Duplicates cannot happen: names are
    // unique per module.
    let mut ports: BTreeMap<String, &Port> = BTreeMap::new();
    for port in &module.ports {
        ports.insert(port.name.as_str().to_ascii_lowercase(), port);
    }

    // Seed the bindings with whatever the module states about the bus
    // parameters; anything else is inferred below.
    let mut bindings: BTreeMap<String, u32> = BTreeMap::new();
    for (name, _) in &interface.params {
        if let Some(param) = module.param(name)
            && let Some(value) = param.value.as_int()
            && let Ok(value) = u32::try_from(value)
            && value > 0
        {
            bindings.insert(name.clone(), value);
        }
    }

    let mut problems = Vec::new();
    let mut signals = Vec::new();
    for signal in &interface.signals {
        let wanted = interface.port_name(prefix, &signal.name);
        let Some(port) = ports.get(&wanted.to_ascii_lowercase()) else {
            if signal.required {
                problems.push(BusProblem::Missing {
                    bus: interface.name.clone(),
                    signal: signal.name.clone(),
                    port: wanted,
                    span: module.span,
                });
            }
            continue;
        };
        let expected_dir = signal.direction(role);
        if port.dir != expected_dir {
            problems.push(BusProblem::Direction {
                bus: interface.name.clone(),
                signal: signal.name.clone(),
                port: port.name.as_str().to_owned(),
                found: port.dir,
                expected: expected_dir,
                span: port.span,
            });
        }
        let found = module
            .nets
            .get(port.net)
            .and_then(|n| n.ty.width())
            .unwrap_or(0);
        match signal.width.resolve(&bindings) {
            Some(expected) if expected != found => problems.push(BusProblem::Width {
                bus: interface.name.clone(),
                signal: signal.name.clone(),
                port: port.name.as_str().to_owned(),
                found,
                expected,
                because: signal.width.to_string(),
                span: port.span,
            }),
            Some(_) => {}
            None => {
                // The parameter is still open: this port decides it.
                if let Some((param, value)) = signal.width.implied_binding(found) {
                    bindings.insert(param, value);
                }
            }
        }
        signals.push(SignalMapping {
            signal: signal.name.clone(),
            port: port.name.as_str().to_owned(),
            width: found,
            dir: port.dir,
        });
    }

    // Anything still unbound keeps the bus's default, so a caller always
    // finds every parameter in the map.
    for (name, default) in &interface.params {
        bindings.entry(name.clone()).or_insert(*default);
    }

    if problems.is_empty() {
        Ok(PortMapping {
            bus: interface.name.clone(),
            role,
            prefix: prefix.to_owned(),
            signals,
            bindings,
        })
    } else {
        Err(problems)
    }
}

// ---------------------------------------------------------------------------
// Connecting two instances
// ---------------------------------------------------------------------------

/// One end of a bus connection: an instance and the prefix its bus ports
/// carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusEndpoint {
    /// The instance inside the parent module.
    pub instance: InstanceId,
    /// The prefix of that instance's bus ports.
    pub prefix: String,
}

impl BusEndpoint {
    /// An endpoint.
    pub fn new(instance: InstanceId, prefix: impl Into<String>) -> Self {
        BusEndpoint {
            instance,
            prefix: prefix.into(),
        }
    }
}

/// One net joining a manager's port to a subordinate's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    /// The bus signal being carried.
    pub signal: String,
    /// The name of the net to create in the parent module.
    pub net: String,
    /// Its width in bits.
    pub width: u32,
    /// The manager's port name.
    pub manager_port: String,
    /// The subordinate's port name.
    pub subordinate_port: String,
}

/// Works out the nets that join two instances over one bus.
///
/// Both ends are matched with [`match_ports`] — the manager as a
/// [`BusRole::Manager`], the subordinate as a [`BusRole::Subordinate`] —
/// so every problem either end has is reported before a single net is
/// invented. A signal that only one end has is joined only when the other
/// end does not need it: an optional output nobody listens to is fine, an
/// input nobody drives is a [`BusProblem::Missing`].
///
/// Net names are `<instance>_<prefix><signal>` on the manager's side,
/// which is unique even when one instance has several buses.
pub fn connect(
    design: &Design,
    parent: ModuleId,
    manager: &BusEndpoint,
    subordinate: &BusEndpoint,
    interface: &BusInterface,
) -> Result<Vec<Connection>, Vec<BusProblem>> {
    let module = design.module(parent);
    let mut problems = Vec::new();
    let mgr = endpoint_mapping(
        design,
        module,
        manager,
        interface,
        BusRole::Manager,
        &mut problems,
    );
    let sub = endpoint_mapping(
        design,
        module,
        subordinate,
        interface,
        BusRole::Subordinate,
        &mut problems,
    );
    let (Some(mgr), Some(sub)) = (mgr, sub) else {
        return Err(problems);
    };
    let manager_name = module.instances[manager.instance].name.as_str().to_owned();
    let mut out = Vec::new();
    for signal in &interface.signals {
        let m = mgr.signal(&signal.name);
        let s = sub.signal(&signal.name);
        match (m, s) {
            (Some(m), Some(s)) => {
                if m.width != s.width {
                    problems.push(BusProblem::Width {
                        bus: interface.name.clone(),
                        signal: signal.name.clone(),
                        port: s.port.clone(),
                        found: s.width,
                        expected: m.width,
                        because: format!("{} bits on the manager", m.width),
                        span: module.instances[subordinate.instance].span,
                    });
                    continue;
                }
                out.push(Connection {
                    signal: signal.name.clone(),
                    net: format!("{manager_name}_{}{}", manager.prefix, signal.name),
                    width: m.width,
                    manager_port: m.port.clone(),
                    subordinate_port: s.port.clone(),
                });
            }
            // One side has the signal as an input and the other does not
            // drive it: that input would float.
            (Some(m), None) if m.dir == PortDir::In => problems.push(BusProblem::Missing {
                bus: interface.name.clone(),
                signal: signal.name.clone(),
                port: interface.port_name(&subordinate.prefix, &signal.name),
                span: module.instances[subordinate.instance].span,
            }),
            (None, Some(s)) if s.dir == PortDir::In => problems.push(BusProblem::Missing {
                bus: interface.name.clone(),
                signal: signal.name.clone(),
                port: interface.port_name(&manager.prefix, &signal.name),
                span: module.instances[manager.instance].span,
            }),
            _ => {}
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

fn endpoint_mapping(
    design: &Design,
    parent: &Module,
    endpoint: &BusEndpoint,
    interface: &BusInterface,
    role: BusRole,
    problems: &mut Vec<BusProblem>,
) -> Option<PortMapping> {
    let instance: &Instance = &parent.instances[endpoint.instance];
    let Some(id) = instance.module.id() else {
        problems.push(BusProblem::Unresolved {
            instance: instance.name.as_str().to_owned(),
            module: match &instance.module {
                crate::ir::ModuleRef::Unresolved(name) => name.as_str().to_owned(),
                crate::ir::ModuleRef::Resolved(_) => unreachable!("just matched as unresolved"),
            },
            span: instance.span,
        });
        return None;
    };
    match match_ports(design.module(id), interface, role, &endpoint.prefix) {
        Ok(mapping) => Some(mapping),
        Err(found) => {
            problems.extend(found);
            None
        }
    }
}

/// Applies the connections: one net per signal, joined to both instances.
///
/// A net whose name is already taken is reused rather than duplicated, so
/// wiring the same bus twice is idempotent. Returns the names of the nets
/// that were created.
pub fn wire(
    builder: &mut ModuleBuilder,
    manager: InstanceId,
    subordinate: InstanceId,
    connections: &[Connection],
) -> Vec<String> {
    let mut created = Vec::new();
    for connection in connections {
        let net = match builder.module().net_by_name(&connection.net) {
            Some(existing) => existing,
            None => {
                created.push(connection.net.clone());
                let span = builder.span;
                builder.module_mut().nets.push(Net {
                    name: Name::new(connection.net.clone()),
                    ty: Type::bits(connection.width),
                    kind: NetKind::Wire,
                    attrs: crate::ir::Attrs::new(),
                    span,
                })
            }
        };
        let expr = builder.expr(ExprKind::Net(net));
        let module = builder.module_mut();
        module.instances[manager]
            .connections
            .push((Name::new(connection.manager_port.clone()), expr));
        module.instances[subordinate]
            .connections
            .push((Name::new(connection.subordinate_port.clone()), expr));
    }
    created
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ModuleRef;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        Span::new(map.add("bus-test", "").unwrap(), 0, 0)
    }

    /// A module with every signal of `bus` under `prefix`, playing
    /// `role`, with the widths `tweak` returns (`None` keeps the
    /// default) and the directions `flip` names reversed.
    fn bus_module(
        name: &str,
        bus: &BusInterface,
        role: BusRole,
        prefix: &str,
        skip: &[&str],
        tweak: &[(&str, u32)],
        flip: &[&str],
    ) -> Module {
        let defaults = bus.defaults();
        let mut b = ModuleBuilder::new(name, span());
        for signal in &bus.signals {
            if skip.contains(&signal.name.as_str()) {
                continue;
            }
            let width = tweak
                .iter()
                .find(|(n, _)| *n == signal.name)
                .map(|(_, w)| *w)
                .unwrap_or_else(|| signal.width.resolve(&defaults).unwrap());
            let mut dir = signal.direction(role);
            if flip.contains(&signal.name.as_str()) {
                dir = match dir {
                    PortDir::In => PortDir::Out,
                    _ => PortDir::In,
                };
            }
            let port = format!("{prefix}{}", signal.name);
            match dir {
                PortDir::In => b.input(port, Type::bits(width)),
                PortDir::Out => b.output(port, Type::bits(width)),
                PortDir::InOut => b.inout(port, Type::bits(width)),
            };
        }
        b.finish()
    }

    #[test]
    fn built_ins_parse_and_round_trip() {
        let buses = builtin_buses();
        let names: Vec<&str> = buses.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "axi4lite",
                "axi4",
                "axi4stream",
                "wishbone",
                "wishbone_pipelined",
                "apb",
                "avalon_mm"
            ]
        );
        for bus in buses {
            let text = bus.to_text();
            let mut map = SourceMap::new();
            let file = map.add(format!("{}.bus", bus.name), &text).unwrap();
            let mut diags = Diagnostics::new();
            let again = BusInterface::parse_all(&text, file, &mut diags);
            assert_eq!(diags.render(&map), "", "{}", bus.name);
            assert_eq!(again, vec![bus.clone()]);
            assert!(!bus.signals.is_empty());
            // Every derived width must name a declared parameter, or the
            // definition is unusable.
            for signal in &bus.signals {
                if let Some(param) = signal.width.parameter() {
                    assert!(
                        bus.params.iter().any(|(n, _)| n == param),
                        "{}: `{}` uses undeclared `{param}`",
                        bus.name,
                        signal.name
                    );
                }
            }
        }
    }

    #[test]
    fn roles_mirror_directions() {
        let axi = builtin("axi4lite").unwrap();
        let awvalid = axi.signal("awvalid").unwrap();
        assert_eq!(awvalid.direction(BusRole::Manager), PortDir::Out);
        assert_eq!(awvalid.direction(BusRole::Subordinate), PortDir::In);
        assert_eq!(awvalid.direction(BusRole::Monitor), PortDir::In);
        assert_eq!(BusRole::Manager.opposite(), BusRole::Subordinate);
        assert_eq!(BusRole::Monitor.opposite(), BusRole::Monitor);
        for role in BusRole::ALL {
            assert_eq!(BusRole::from_keyword(role.keyword()), Some(role));
            assert_eq!(role.to_string(), role.keyword());
        }
        assert_eq!(BusRole::from_keyword("slave"), None);
    }

    #[test]
    fn widths_parse_resolve_and_render() {
        assert_eq!(Width::parse("32"), Some(Width::Fixed(32)));
        assert_eq!(
            Width::parse("DATA_WIDTH/8"),
            Some(Width::Derived {
                param: "DATA_WIDTH".to_owned(),
                divisor: 8
            })
        );
        assert_eq!(Width::parse(""), None);
        assert_eq!(Width::parse("9lives"), None);
        assert_eq!(Width::parse("a-b"), None);
        assert_eq!(Width::parse("W/x"), None);
        let mut bindings = BTreeMap::new();
        bindings.insert("DATA_WIDTH".to_owned(), 32);
        assert_eq!(
            Width::parse("DATA_WIDTH/8").unwrap().resolve(&bindings),
            Some(4)
        );
        // A width never collapses to zero: a one-byte bus still has one
        // strobe bit.
        bindings.insert("DATA_WIDTH".to_owned(), 8);
        assert_eq!(
            Width::parse("DATA_WIDTH/8").unwrap().resolve(&bindings),
            Some(1)
        );
        assert_eq!(Width::param("X").resolve(&BTreeMap::new()), None);
        assert_eq!(Width::Fixed(3).to_string(), "3");
        assert_eq!(Width::param("W").to_string(), "W");
        assert_eq!(Width::parse("W/8").unwrap().to_string(), "W/8");
        assert_eq!(Width::Fixed(3).parameter(), None);
    }

    #[test]
    fn a_correct_subordinate_matches() {
        let axi = builtin("axi4lite").unwrap();
        let module = bus_module("regs", axi, BusRole::Subordinate, "s_axi_", &[], &[], &[]);
        let mapping = match_ports(&module, axi, BusRole::Subordinate, "s_axi_").unwrap();
        assert_eq!(mapping.signals.len(), axi.signals.len());
        assert_eq!(mapping.port("awaddr"), Some("s_axi_awaddr"));
        assert_eq!(mapping.signal("wstrb").unwrap().width, 4);
        assert_eq!(mapping.bindings["ADDR_WIDTH"], 32);
        assert_eq!(mapping.bindings["DATA_WIDTH"], 32);
        assert_eq!(mapping.role, BusRole::Subordinate);
        assert_eq!(mapping.bus, "axi4lite");
        assert!(mapping.signal("nope").is_none());
    }

    #[test]
    fn prefixes_are_matched_without_regard_to_case() {
        let apb = builtin("apb").unwrap();
        let mut module = bus_module("gpio", apb, BusRole::Subordinate, "S_", &[], &[], &[]);
        for port in &mut module.ports {
            let upper = port.name.as_str().to_ascii_uppercase();
            port.name = Name::new(upper);
        }
        let mapping = match_ports(&module, apb, BusRole::Subordinate, "s_").unwrap();
        assert_eq!(mapping.port("pwdata"), Some("S_PWDATA"));
    }

    #[test]
    fn a_missing_required_signal_is_reported() {
        let axi = builtin("axi4lite").unwrap();
        let module = bus_module(
            "regs",
            axi,
            BusRole::Subordinate,
            "s_axi_",
            &["wready", "bresp"],
            &[],
            &[],
        );
        let problems = match_ports(&module, axi, BusRole::Subordinate, "s_axi_").unwrap_err();
        // `bresp` is optional, so only `wready` is a problem.
        assert_eq!(problems.len(), 1);
        let text = problems[0].diagnostic().message.clone();
        assert_eq!(text, "`axi4lite` needs a port `s_axi_wready`");
        assert_eq!(problems[0].diagnostic().code, Some(MISSING_SIGNAL));
        assert_eq!(problems[0].to_string(), text);
    }

    #[test]
    fn a_wrong_width_is_reported_against_the_inferred_parameter() {
        let axi = builtin("axi4lite").unwrap();
        // `wdata` binds DATA_WIDTH to 64; `wstrb` should then be 8.
        let module = bus_module(
            "regs",
            axi,
            BusRole::Subordinate,
            "s_axi_",
            &[],
            &[("wdata", 64), ("rdata", 64)],
            &[],
        );
        let problems = match_ports(&module, axi, BusRole::Subordinate, "s_axi_").unwrap_err();
        assert_eq!(problems.len(), 1);
        let d = problems[0].diagnostic();
        assert_eq!(
            d.message,
            "`s_axi_wstrb` is 4 bits wide but `axi4lite` needs 8"
        );
        assert_eq!(d.code, Some(BAD_WIDTH));
        assert!(d.notes[0].contains("DATA_WIDTH/8"));
    }

    #[test]
    fn a_declared_parameter_beats_inference() {
        let axi = builtin("axi4lite").unwrap();
        let mut module = bus_module(
            "regs",
            axi,
            BusRole::Subordinate,
            "s_axi_",
            &[],
            &[("araddr", 16)],
            &[],
        );
        // Without a declaration `awaddr` binds ADDR_WIDTH to 32 and
        // `araddr` is the odd one out either way; declaring it 16 moves
        // the complaint to `awaddr`.
        module.params.push(crate::ir::Param {
            name: Name::new("ADDR_WIDTH"),
            value: 16i64.into(),
            attrs: crate::ir::Attrs::new(),
            span: span(),
        });
        let problems = match_ports(&module, axi, BusRole::Subordinate, "s_axi_").unwrap_err();
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0]
                .diagnostic()
                .message
                .contains("`s_axi_awaddr` is 32 bits wide but `axi4lite` needs 16"),
            "{:?}",
            problems[0]
        );
    }

    #[test]
    fn a_reversed_direction_is_reported() {
        let axi = builtin("axi4lite").unwrap();
        let module = bus_module(
            "regs",
            axi,
            BusRole::Subordinate,
            "s_axi_",
            &[],
            &[],
            &["awready"],
        );
        let problems = match_ports(&module, axi, BusRole::Subordinate, "s_axi_").unwrap_err();
        assert_eq!(problems.len(), 1);
        let d = problems[0].diagnostic();
        assert_eq!(
            d.message,
            "`s_axi_awready` is `in` but `axi4lite` needs `out`"
        );
        assert_eq!(d.code, Some(BAD_DIRECTION));
    }

    #[test]
    fn a_manager_module_matched_as_a_subordinate_is_all_wrong() {
        let wb = builtin("wishbone").unwrap();
        let module = bus_module("cpu", wb, BusRole::Manager, "wb_", &[], &[], &[]);
        assert!(match_ports(&module, wb, BusRole::Manager, "wb_").is_ok());
        let problems = match_ports(&module, wb, BusRole::Subordinate, "wb_").unwrap_err();
        // Every signal points the wrong way; `sel`, `err` and `rty` are
        // optional but present, so they are checked too.
        assert_eq!(problems.len(), wb.signals.len());
        assert!(
            problems
                .iter()
                .all(|p| matches!(p, BusProblem::Direction { .. }))
        );
    }

    #[test]
    fn a_monitor_reads_everything() {
        let axi = builtin("axi4lite").unwrap();
        let module = bus_module("mon", axi, BusRole::Monitor, "m_", &[], &[], &[]);
        let mapping = match_ports(&module, axi, BusRole::Monitor, "m_").unwrap();
        assert!(mapping.signals.iter().all(|s| s.dir == PortDir::In));
    }

    #[test]
    fn connect_wires_a_manager_to_a_subordinate() {
        let apb = builtin("apb").unwrap();
        let mut design = Design::new();
        let mgr = design.add_module(bus_module(
            "cpu",
            apb,
            BusRole::Manager,
            "apb_",
            &[],
            &[],
            &[],
        ));
        let sub = design.add_module(bus_module(
            "gpio",
            apb,
            BusRole::Subordinate,
            "s_",
            &[],
            &[],
            &[],
        ));
        let mut b = ModuleBuilder::new("top", span());
        let u_cpu = b.instance("u_cpu", ModuleRef::Resolved(mgr), Vec::new());
        let u_gpio = b.instance("u_gpio", ModuleRef::Resolved(sub), Vec::new());
        let top = design.add_module(b.finish());

        let connections = connect(
            &design,
            top,
            &BusEndpoint::new(u_cpu, "apb_"),
            &BusEndpoint::new(u_gpio, "s_"),
            apb,
        )
        .unwrap();
        assert_eq!(connections.len(), apb.signals.len());
        let paddr = connections.iter().find(|c| c.signal == "paddr").unwrap();
        assert_eq!(paddr.net, "u_cpu_apb_paddr");
        assert_eq!(paddr.manager_port, "apb_paddr");
        assert_eq!(paddr.subordinate_port, "s_paddr");
        assert_eq!(paddr.width, 32);

        let module = design.modules[top].clone();
        let mut b = ModuleBuilder::from_module(module, span());
        let created = wire(&mut b, u_cpu, u_gpio, &connections);
        assert_eq!(created.len(), apb.signals.len());
        // Wiring again reuses the nets rather than duplicating them.
        assert!(wire(&mut b, u_cpu, u_gpio, &connections).is_empty());
        let module = b.finish();
        assert_eq!(module.nets.len(), apb.signals.len());
        assert_eq!(
            module.instances[u_cpu].connections.len(),
            2 * apb.signals.len()
        );
    }

    #[test]
    fn connect_reports_both_ends_and_unresolved_instances() {
        let apb = builtin("apb").unwrap();
        let mut design = Design::new();
        let mgr = design.add_module(bus_module(
            "cpu",
            apb,
            BusRole::Manager,
            "apb_",
            &["pready"],
            &[],
            &[],
        ));
        let sub = design.add_module(bus_module(
            "gpio",
            apb,
            BusRole::Subordinate,
            "s_",
            &[],
            &[("pwdata", 16)],
            &[],
        ));
        let mut b = ModuleBuilder::new("top", span());
        let u_cpu = b.instance("u_cpu", ModuleRef::Resolved(mgr), Vec::new());
        let u_gpio = b.instance("u_gpio", ModuleRef::Resolved(sub), Vec::new());
        let u_box = b.instance(
            "u_box",
            ModuleRef::Unresolved(Name::new("vendor")),
            Vec::new(),
        );
        let top = design.add_module(b.finish());

        let problems = connect(
            &design,
            top,
            &BusEndpoint::new(u_cpu, "apb_"),
            &BusEndpoint::new(u_gpio, "s_"),
            apb,
        )
        .unwrap_err();
        // The manager is missing a required signal and the subordinate
        // has a width that contradicts its own `prdata`.
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, BusProblem::Missing { .. }))
        );
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, BusProblem::Width { .. }))
        );

        let problems = connect(
            &design,
            top,
            &BusEndpoint::new(u_box, "apb_"),
            &BusEndpoint::new(u_gpio, "s_"),
            apb,
        )
        .unwrap_err();
        let d = problems[0].diagnostic();
        assert_eq!(
            d.message,
            "instance `u_box` has no module `vendor` in the design"
        );
    }

    #[test]
    fn connect_reports_a_width_that_only_disagrees_across_the_link() {
        let apb = builtin("apb").unwrap();
        let mut design = Design::new();
        // Each side is internally consistent; only together are they
        // wrong, which is exactly what `connect` is for.
        let mgr = design.add_module(bus_module(
            "cpu",
            apb,
            BusRole::Manager,
            "apb_",
            &[],
            &[("pwdata", 64), ("prdata", 64), ("pstrb", 8)],
            &[],
        ));
        let sub = design.add_module(bus_module(
            "gpio",
            apb,
            BusRole::Subordinate,
            "s_",
            &[],
            &[],
            &[],
        ));
        let mut b = ModuleBuilder::new("top", span());
        let u_cpu = b.instance("u_cpu", ModuleRef::Resolved(mgr), Vec::new());
        let u_gpio = b.instance("u_gpio", ModuleRef::Resolved(sub), Vec::new());
        let top = design.add_module(b.finish());
        let problems = connect(
            &design,
            top,
            &BusEndpoint::new(u_cpu, "apb_"),
            &BusEndpoint::new(u_gpio, "s_"),
            apb,
        )
        .unwrap_err();
        assert!(problems.iter().any(|p| {
            p.diagnostic()
                .message
                .contains("`s_pwdata` is 32 bits wide but `apb` needs 64")
        }));
    }

    #[test]
    fn an_optional_input_nobody_drives_is_reported() {
        let wb = builtin("wishbone").unwrap();
        let mut design = Design::new();
        // The manager listens to `err`; the subordinate does not have it.
        let mgr = design.add_module(bus_module(
            "cpu",
            wb,
            BusRole::Manager,
            "wb_",
            &["rty"],
            &[],
            &[],
        ));
        let sub = design.add_module(bus_module(
            "ram",
            wb,
            BusRole::Subordinate,
            "s_",
            &["err", "rty"],
            &[],
            &[],
        ));
        let mut b = ModuleBuilder::new("top", span());
        let u_cpu = b.instance("u_cpu", ModuleRef::Resolved(mgr), Vec::new());
        let u_ram = b.instance("u_ram", ModuleRef::Resolved(sub), Vec::new());
        let top = design.add_module(b.finish());
        let problems = connect(
            &design,
            top,
            &BusEndpoint::new(u_cpu, "wb_"),
            &BusEndpoint::new(u_ram, "s_"),
            wb,
        )
        .unwrap_err();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].diagnostic().message.contains("`s_err`"));
    }

    #[test]
    fn bus_file_errors_are_reported() {
        let cases: [(&str, &str); 7] = [
            ("bus\n", "`bus` takes `bus <name>`"),
            ("end\n", "`end` takes a `bus` block to close"),
            ("param A 1\n", "`param` takes to be inside a `bus` block"),
            (
                "bus a\nparam X y\nend\n",
                "`param` takes `param <name> <default>`",
            ),
            (
                "bus a\nsignal s out\nend\n",
                "`signal` takes `signal <name>",
            ),
            ("bus a\nsignal s sideways 1\nend\n", "a direction of `in`"),
            ("bus a\nsignal s out ?\nend\n", "a width of `n`"),
        ];
        for (text, expected) in cases {
            let mut map = SourceMap::new();
            let file = map.add("t.bus", text).unwrap();
            let mut diags = Diagnostics::new();
            BusInterface::parse_all(text, file, &mut diags);
            let rendered = diags.render(&map);
            assert!(rendered.contains(expected), "{text:?} gave {rendered}");
        }
    }

    #[test]
    fn a_bus_without_end_is_still_returned() {
        let text = "bus a\nsignal s out 1\nbus b\nsignal t out 1\n";
        let mut map = SourceMap::new();
        let file = map.add("t.bus", text).unwrap();
        let mut diags = Diagnostics::new();
        let buses = BusInterface::parse_all(text, file, &mut diags);
        assert_eq!(buses.len(), 2);
        let rendered = diags.render(&map);
        assert!(rendered.contains("bus `a` has no `end`"), "{rendered}");
        assert!(rendered.contains("bus `b` has no `end`"), "{rendered}");
    }

    #[test]
    fn unknown_directives_suggest_a_neighbour() {
        let text = "bus a\nsignl s out 1\nend\n";
        let mut map = SourceMap::new();
        let file = map.add("t.bus", text).unwrap();
        let mut diags = Diagnostics::new();
        BusInterface::parse_all(text, file, &mut diags);
        let rendered = diags.render(&map);
        assert!(
            rendered.contains("unknown bus directive `signl`"),
            "{rendered}"
        );
        assert!(rendered.contains("did you mean `signal`?"), "{rendered}");
    }

    #[test]
    fn unknown_bus_names_suggest_a_neighbour() {
        let d = no_such_bus("axi4lit", span());
        assert_eq!(d.message, "unknown bus `axi4lit`");
        assert_eq!(d.notes[0], "did you mean `axi4lite`?");
        let d = no_such_bus("spacewire", span());
        assert!(d.notes[0].starts_with("the built-in buses are: axi4lite"));
    }

    #[test]
    fn optional_signals_may_simply_be_absent() {
        let stream = builtin("axi4stream").unwrap();
        let module = bus_module(
            "src",
            stream,
            BusRole::Manager,
            "m_",
            &["tstrb", "tkeep", "tlast", "tid", "tdest", "tuser"],
            &[],
            &[],
        );
        let mapping = match_ports(&module, stream, BusRole::Manager, "m_").unwrap();
        assert_eq!(mapping.signals.len(), 3);
        assert_eq!(mapping.port("tdata"), Some("m_tdata"));
        assert_eq!(mapping.port("tlast"), None);
    }
}
